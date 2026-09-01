use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::de::DeserializeOwned;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use crate::config::CacheContext;
use crate::digest::{is_sha256, sha256, sha256_file};
use crate::error::{CacheError, IoResultExt, Result};
use crate::fsutil::{create_private_dir, sync_directory};
use crate::limits::{CACHE_VERSION, MAX_COMPRESSED_BYTES, MAX_METADATA_BYTES, SCHEMA};
use crate::model::{CompleteMarker, EntryMetadata};

pub fn repository_directory(context: &CacheContext) -> PathBuf {
    context
        .cache_root
        .join(CACHE_VERSION)
        .join(&context.repository_id)
}

pub fn entry_directory(context: &CacheContext, digest: &str) -> PathBuf {
    repository_directory(context).join(digest)
}

pub fn ensure_repository_directory(context: &CacheContext) -> Result<PathBuf> {
    let version = context.cache_root.join(CACHE_VERSION);
    let repository = version.join(&context.repository_id);
    claim_repository_root(context, &version)?;
    create_private_dir(&version)?;
    create_private_dir(&repository)?;
    verify_private_directory(&version)?;
    verify_private_directory(&repository)?;
    sync_directory(&context.cache_root)?;
    sync_directory(&version)?;
    Ok(repository)
}

fn claim_repository_root(context: &CacheContext, version: &Path) -> Result<()> {
    reject_foreign_repository_namespace(version, &context.repository_id)?;

    let claim = context.cache_root.join(".local-cache-repository-id");
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&claim)
    {
        Ok(mut file) => {
            file.write_all(context.repository_id.as_bytes())
                .cache_err("root-claim")?;
            file.write_all(b"\n").cache_err("root-claim")?;
            file.sync_all().cache_err("root-claim")?;
            sync_directory(&context.cache_root)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let metadata = fs::symlink_metadata(&claim).cache_err("invalid-root")?;
            if !metadata.is_file()
                || metadata.file_type().is_symlink()
                || metadata.uid() != unsafe { libc::geteuid() }
                || metadata.mode() & 0o777 != 0o600
                || metadata.len() > 32
            {
                return Err(CacheError::new(
                    "invalid-root",
                    "cache root claim must be a runner-owned mode-0600 regular file",
                ));
            }
            let mut value = String::new();
            File::open(&claim)
                .cache_err("invalid-root")?
                .take(33)
                .read_to_string(&mut value)
                .cache_err("invalid-root")?;
            if value != format!("{}\n", context.repository_id) {
                return Err(CacheError::new(
                    "shared-root-detected",
                    "cache-dir is already claimed by a different repository",
                ));
            }
        }
        Err(error) => return Err(CacheError::io("root-claim", error)),
    }

    // A pre-marker v1 layout may have been created by an earlier release. Check it
    // both before and after claiming so a legacy shared root is never adopted.
    reject_foreign_repository_namespace(version, &context.repository_id)
}

fn reject_foreign_repository_namespace(version: &Path, repository_id: &str) -> Result<()> {
    let entries = match fs::read_dir(version) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(CacheError::io("invalid-root", error)),
    };
    for entry in entries {
        let entry = entry.cache_err("invalid-root")?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name != repository_id
            && !name.is_empty()
            && name.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(CacheError::new(
                "shared-root-detected",
                "cache-dir already contains a namespace for a different repository",
            ));
        }
    }
    Ok(())
}

fn verify_private_directory(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path).cache_err("invalid-root")?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() || metadata.mode() & 0o777 != 0o700 {
        return Err(CacheError::new(
            "invalid-root",
            "cache namespaces must be non-symlinked mode-0700 directories",
        ));
    }
    if metadata.uid() != unsafe { libc::geteuid() } {
        return Err(CacheError::new(
            "invalid-root",
            "cache namespaces must be owned by the runner user",
        ));
    }
    Ok(())
}

fn read_bounded_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let metadata = fs::symlink_metadata(path).cache_err("invalid-entry")?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > MAX_METADATA_BYTES
    {
        return Err(CacheError::new(
            "invalid-entry",
            "entry JSON file is invalid",
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    File::open(path)
        .cache_err("invalid-entry")?
        .take(MAX_METADATA_BYTES + 1)
        .read_to_end(&mut bytes)
        .cache_err("invalid-entry")?;
    serde_json::from_slice(&bytes)
        .map_err(|error| CacheError::new("invalid-entry", error.to_string()))
}

fn validate_metadata(
    metadata: &EntryMetadata,
    context: &CacheContext,
    directory_name: &str,
) -> Result<()> {
    if !metadata.compatible(context) {
        return Err(CacheError::new(
            "incompatible-entry",
            "entry is not compatible with this runner",
        ));
    }
    if metadata.key.is_empty()
        || metadata.key.len() > 512
        || sha256(&metadata.key) != directory_name
    {
        return Err(CacheError::new(
            "invalid-entry",
            "entry key or address is invalid",
        ));
    }
    if metadata.payload_bytes > MAX_COMPRESSED_BYTES || !is_sha256(&metadata.payload_sha256) {
        return Err(CacheError::new(
            "invalid-entry",
            "entry payload fields are invalid",
        ));
    }
    if OffsetDateTime::parse(&metadata.created_at, &Rfc3339).is_err() {
        return Err(CacheError::new(
            "invalid-entry",
            "entry timestamp is invalid",
        ));
    }
    if metadata.paths.iter().any(|path| {
        path.is_empty()
            || path.starts_with('/')
            || path.split('/').any(|part| part == ".." || part.is_empty())
    }) {
        return Err(CacheError::new("invalid-entry", "entry paths are invalid"));
    }
    Ok(())
}

pub fn inspect_entry(path: &Path, context: &CacheContext) -> Result<EntryMetadata> {
    let directory = fs::symlink_metadata(path).cache_err("invalid-entry")?;
    if !directory.is_dir() || directory.file_type().is_symlink() {
        return Err(CacheError::new(
            "invalid-entry",
            "entry is not a regular directory",
        ));
    }
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| CacheError::new("invalid-entry", "entry address is invalid"))?;
    let metadata: EntryMetadata = read_bounded_json(&path.join("metadata.json"))?;
    let complete: CompleteMarker = read_bounded_json(&path.join("complete"))?;
    validate_metadata(&metadata, context, name)?;
    if complete.schema != SCHEMA
        || !is_sha256(&complete.payload_sha256)
        || complete.payload_sha256 != metadata.payload_sha256
    {
        return Err(CacheError::new(
            "invalid-entry",
            "complete marker is invalid",
        ));
    }
    let payload = fs::symlink_metadata(path.join("payload.tar.zst")).cache_err("invalid-entry")?;
    if !payload.is_file()
        || payload.file_type().is_symlink()
        || payload.nlink() != 1
        || payload.len() != metadata.payload_bytes
    {
        return Err(CacheError::new(
            "invalid-entry",
            "payload type or size is invalid",
        ));
    }
    Ok(metadata)
}

pub fn validate_entry(
    path: &Path,
    context: &CacheContext,
    started: Instant,
) -> Result<EntryMetadata> {
    let metadata = inspect_entry(path, context)?;
    if sha256_file(&path.join("payload.tar.zst"), started)? != metadata.payload_sha256 {
        return Err(CacheError::new(
            "invalid-entry",
            "payload digest does not match metadata",
        ));
    }
    Ok(metadata)
}

pub fn quarantine_entry(path: &Path) -> Result<Option<PathBuf>> {
    match fs::symlink_metadata(path) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(CacheError::io("quarantine", error)),
    }
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("invalid");
    let destination = path.with_file_name(format!(".quarantine-{name}-{}", Uuid::new_v4()));
    create_private_dir(&destination)?;
    if let Err(error) = fs::rename(path, destination.join("entry")) {
        let _ = fs::remove_dir(&destination);
        return Err(CacheError::io("quarantine", error));
    }
    sync_directory(&destination)?;
    sync_directory(path.parent().expect("entry has a parent"))?;
    Ok(Some(destination))
}

pub fn path_exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(CacheError::io("path-check", error)),
    }
}
