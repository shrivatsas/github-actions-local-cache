use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;
use std::time::Instant;

use sha2::{Digest, Sha256};

use crate::error::{CacheError, IoResultExt, Result};
use crate::limits::OPERATION_TIMEOUT;

pub fn sha256(value: impl AsRef<[u8]>) -> String {
    hex::encode(Sha256::digest(value.as_ref()))
}

pub fn sha256_file(path: &Path, started: Instant) -> Result<String> {
    let file = File::open(path).cache_err("payload-read")?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 128 * 1024];
    loop {
        if started.elapsed() > OPERATION_TIMEOUT {
            return Err(CacheError::new(
                "operation-timeout",
                "cache operation exceeded 20 minutes",
            ));
        }
        let count = reader.read(&mut buffer).cache_err("payload-read")?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hex::encode(hasher.finalize()))
}

pub fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
