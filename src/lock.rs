use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{CacheError, IoResultExt, Result};
use crate::fsutil::sync_directory;
use crate::limits::{LOCK_TIMEOUT, OPERATION_TIMEOUT};

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
struct LockRecord {
    pid: u32,
    boot_id: String,
    created_at_ms: u128,
    nonce: String,
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn boot_id() -> String {
    fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .map(|value| value.trim().to_owned())
        .unwrap_or_else(|_| "unknown".to_owned())
}

fn file_age(path: &Path) -> Duration {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .unwrap_or_default()
}

fn is_stale(path: &Path, current_boot_id: &str) -> bool {
    match fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<LockRecord>(&bytes).ok())
    {
        Some(record) => {
            record.boot_id != current_boot_id
                || now_ms().saturating_sub(record.created_at_ms)
                    > (OPERATION_TIMEOUT + LOCK_TIMEOUT).as_millis()
        }
        None => file_age(path) > LOCK_TIMEOUT,
    }
}

pub fn with_entry_lock<T>(
    path: &Path,
    timeout: Duration,
    operation: impl FnOnce() -> Result<T>,
) -> Result<T> {
    let started = Instant::now();
    let current_boot_id = boot_id();
    let record = LockRecord {
        pid: std::process::id(),
        boot_id: current_boot_id.clone(),
        created_at_ms: now_ms(),
        nonce: Uuid::new_v4().to_string(),
    };
    let bytes = serde_json::to_vec(&record)
        .map_err(|error| CacheError::new("lock-create", error.to_string()))?;
    let mut backoff = Duration::from_millis(20);

    loop {
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
        {
            Ok(mut file) => {
                file.write_all(&bytes).cache_err("lock-create")?;
                file.write_all(b"\n").cache_err("lock-create")?;
                file.sync_all().cache_err("lock-create")?;
                sync_directory(path.parent().expect("lock has a parent"))?;
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                if is_stale(path, &current_boot_id) {
                    match fs::remove_file(path) {
                        Ok(()) => {
                            sync_directory(path.parent().expect("lock has a parent"))?;
                            continue;
                        }
                        Err(unlink_error)
                            if unlink_error.kind() == std::io::ErrorKind::NotFound =>
                        {
                            continue;
                        }
                        Err(unlink_error) => {
                            return Err(CacheError::io("lock-recovery", unlink_error));
                        }
                    }
                }
                if started.elapsed() >= timeout {
                    return Err(CacheError::new(
                        "lock-timeout",
                        "timed out waiting for cache entry lock",
                    ));
                }
                thread::sleep(backoff.min(timeout.saturating_sub(started.elapsed())));
                backoff = (backoff * 2).min(Duration::from_millis(500));
            }
            Err(error) => return Err(CacheError::io("lock-create", error)),
        }
    }

    let result = operation();
    let unlock_result = (|| -> Result<()> {
        let current = fs::read(path).cache_err("lock-release")?;
        let current: LockRecord = serde_json::from_slice(&current)
            .map_err(|error| CacheError::new("lock-release", error.to_string()))?;
        if current.nonce == record.nonce {
            fs::remove_file(path).cache_err("lock-release")?;
            sync_directory(path.parent().expect("lock has a parent"))?;
        }
        Ok(())
    })();
    match (result, unlock_result) {
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Ok(value), Ok(())) => Ok(value),
    }
}
