use std::fs;
use std::path::{Path, PathBuf};

use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use crate::archive::{collect_entries, create_archive, extract_archive, materialize};
use crate::digest::{sha256, sha256_file};
use crate::entry::{
    ensure_repository_directory, entry_directory, inspect_entry, path_exists, quarantine_entry,
    repository_directory, validate_entry,
};
use crate::error::{CacheError, IoResultExt, Result};
use crate::fsutil::{create_private_dir, sync_directory, write_json_exclusive};
use crate::limits::{ARCHIVE_POLICY, LOCK_TIMEOUT, MAX_FALLBACK_CANDIDATES, SCHEMA};
use crate::lock::with_entry_lock;
use crate::model::{
    CompleteMarker, EntryMetadata, RestoreMatch, RestoreRequest, RestoreResult, SaveRequest,
    SaveResult,
};

fn lock_path(repository: &Path, digest: &str) -> PathBuf {
    repository.join(format!("{digest}.lock"))
}

fn quarantinable(error: &CacheError) -> bool {
    matches!(error.code, "invalid-entry" | "incompatible-entry")
}

fn exact_candidate(
    request: &RestoreRequest,
    digest: &str,
) -> Result<Option<(PathBuf, EntryMetadata)>> {
    let repository = repository_directory(&request.context);
    let path = entry_directory(&request.context, digest);
    if !path_exists(&path)? {
        return Ok(None);
    }
    with_entry_lock(
        &lock_path(&repository, digest),
        LOCK_TIMEOUT,
        || match validate_entry(&path, &request.context) {
            Ok(metadata) => Ok(Some((path.clone(), metadata))),
            Err(error) if quarantinable(&error) => {
                quarantine_entry(&path)?;
                Ok(None)
            }
            Err(error) => Err(error),
        },
    )
}

fn fallback_candidate(request: &RestoreRequest) -> Result<Option<(PathBuf, EntryMetadata)>> {
    if request.restore_keys.is_empty() {
        return Ok(None);
    }
    let repository = repository_directory(&request.context);
    let mut candidates = Vec::new();
    for item in fs::read_dir(&repository).cache_err("fallback-scan")? {
        let item = item.cache_err("fallback-scan")?;
        let digest = match item.file_name().to_str() {
            Some(name)
                if name.len() == 64
                    && name
                        .bytes()
                        .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()) =>
            {
                name.to_owned()
            }
            _ => continue,
        };
        match inspect_entry(&item.path(), &request.context) {
            Ok(metadata) => candidates.push((item.path(), digest, metadata)),
            Err(error) if quarantinable(&error) => {}
            Err(error) => return Err(error),
        }
    }
    candidates.sort_by(|left, right| {
        right
            .2
            .created_at
            .cmp(&left.2.created_at)
            .then_with(|| left.1.cmp(&right.1))
    });
    candidates.truncate(MAX_FALLBACK_CANDIDATES);
    for prefix in &request.restore_keys {
        for (path, _, inspected) in &candidates {
            if !inspected.key.starts_with(prefix) {
                continue;
            }
            match validate_entry(path, &request.context) {
                Ok(metadata) => return Ok(Some((path.clone(), metadata))),
                Err(error) if quarantinable(&error) => continue,
                Err(error) => return Err(error),
            }
        }
    }
    Ok(None)
}

pub fn restore_cache(request: RestoreRequest) -> Result<RestoreResult> {
    ensure_repository_directory(&request.context)?;
    let requested_digest = sha256(&request.key);
    let (candidate, cache_match) = match exact_candidate(&request, &requested_digest)? {
        Some(candidate) => (Some(candidate), RestoreMatch::Exact),
        None => match fallback_candidate(&request)? {
            Some(candidate) => (Some(candidate), RestoreMatch::Fallback),
            None => (None, RestoreMatch::Miss),
        },
    };
    let Some((path, metadata)) = candidate else {
        return Ok(RestoreResult {
            cache_match,
            digest: None,
            files: 0,
            bytes: 0,
        });
    };
    let parent = request
        .context
        .workspace
        .parent()
        .ok_or_else(|| CacheError::new("restore-staging", "workspace has no parent"))?;
    let temporary = tempfile::Builder::new()
        .prefix(".local-cache-")
        .tempdir_in(parent)
        .cache_err("restore-staging")?;
    let staging = temporary.path().join("content");
    let (files, bytes) = extract_archive(&path.join("payload.tar.zst"), &staging, &metadata.paths)?;
    materialize(&staging, &request.context.workspace)?;
    Ok(RestoreResult {
        cache_match,
        digest: Some(sha256(&metadata.key)),
        files,
        bytes,
    })
}

pub fn save_cache(request: SaveRequest) -> Result<SaveResult> {
    let repository = ensure_repository_directory(&request.context)?;
    let digest = sha256(&request.key);
    let entries = collect_entries(&request.context.workspace, &request.patterns)?;
    if entries.is_empty() {
        return Ok(SaveResult::SkippedNoPaths);
    }
    with_entry_lock(&lock_path(&repository, &digest), LOCK_TIMEOUT, || {
        let destination = entry_directory(&request.context, &digest);
        if path_exists(&destination)? {
            match validate_entry(&destination, &request.context) {
                Ok(_) => return Ok(SaveResult::Raced),
                Err(error) if quarantinable(&error) => {
                    quarantine_entry(&destination)?;
                }
                Err(error) => return Err(error),
            }
        }
        let staging = repository.join(format!(".tmp-{digest}-{}", Uuid::new_v4()));
        create_private_dir(&staging)?;
        let result = (|| -> Result<SaveResult> {
            let payload = staging.join("payload.tar.zst");
            create_archive(&entries, &payload)?;
            let payload_bytes = fs::metadata(&payload).cache_err("payload-stat")?.len();
            let payload_sha256 = sha256_file(&payload)?;
            let created_at = OffsetDateTime::now_utc()
                .format(&Rfc3339)
                .map_err(|error| CacheError::new("metadata-create", error.to_string()))?;
            let metadata = EntryMetadata {
                schema: SCHEMA,
                archive_policy: ARCHIVE_POLICY,
                key: request.key.clone(),
                created_at,
                payload_bytes,
                payload_sha256: payload_sha256.clone(),
                os: "linux".to_owned(),
                arch: request.context.arch.clone(),
                paths: entries.iter().map(|entry| entry.relative.clone()).collect(),
            };
            write_json_exclusive(&staging.join("metadata.json"), &metadata)?;
            write_json_exclusive(
                &staging.join("complete"),
                &CompleteMarker {
                    schema: SCHEMA,
                    payload_sha256,
                },
            )?;
            sync_directory(&staging)?;
            fs::rename(&staging, &destination).cache_err("publish")?;
            sync_directory(&repository)?;
            Ok(SaveResult::Saved)
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(&staging);
        }
        result
    })
}
