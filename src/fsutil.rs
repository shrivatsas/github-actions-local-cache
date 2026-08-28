use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::Path;

use serde::Serialize;

use crate::error::{CacheError, IoResultExt, Result};
use crate::limits::MAX_METADATA_BYTES;

pub fn create_private_dir(path: &Path) -> Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.mode(0o700);
    match builder.create(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(CacheError::io("directory-create", error)),
    }
}

pub fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)
        .cache_err("directory-open")?
        .sync_all()
        .cache_err("directory-sync")
}

pub fn write_json_exclusive(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut data = serde_json::to_vec(value)
        .map_err(|error| CacheError::new("metadata-serialize", error.to_string()))?;
    data.push(b'\n');
    if data.len() as u64 > MAX_METADATA_BYTES {
        return Err(CacheError::new("metadata-limit", "metadata exceeds 1 MiB"));
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .cache_err("metadata-create")?;
    file.write_all(&data).cache_err("metadata-write")?;
    file.sync_all().cache_err("metadata-sync")
}
