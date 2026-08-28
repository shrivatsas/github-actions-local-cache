use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path};
use std::time::Instant;

use filetime::FileTime;
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use tar::{Archive, Builder, EntryType, Header};
use walkdir::WalkDir;

use crate::error::{CacheError, IoResultExt, Result};
use crate::limits::{
    MAX_COMPRESSED_BYTES, MAX_ENTRIES, MAX_EXTRACTED_BYTES, MAX_FILE_BYTES, OPERATION_TIMEOUT,
};
use crate::model::{ArchiveEntry, EntryKind};

fn portable_path(workspace: &Path, path: &Path) -> Result<String> {
    let relative = path
        .strip_prefix(workspace)
        .map_err(|_| CacheError::new("invalid-path", "path escapes workspace"))?;
    if relative.as_os_str().is_empty() {
        return Err(CacheError::new(
            "invalid-path",
            "workspace root cannot be archived",
        ));
    }
    let mut parts = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_str().ok_or_else(|| {
                CacheError::new("invalid-path", "non-UTF-8 paths are unsupported")
            })?),
            _ => {
                return Err(CacheError::new(
                    "invalid-path",
                    "archive path is not portable",
                ));
            }
        }
    }
    Ok(parts.join("/"))
}

fn build_matcher(patterns: &[String]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        let portable = pattern.replace('\\', "/");
        let glob = GlobBuilder::new(&portable)
            .literal_separator(true)
            .build()
            .map_err(|error| CacheError::new("invalid-input", error.to_string()))?;
        builder.add(glob);
    }
    builder
        .build()
        .map_err(|error| CacheError::new("invalid-input", error.to_string()))
}

/// Returns whether every archived path is within the restore request's declared
/// path scope.  Directory ancestors are retained in an archive so that empty
/// directories and permissions can be restored; a literal directory pattern
/// therefore also permits its descendants.
pub fn paths_match_patterns(paths: &[String], patterns: &[String]) -> Result<bool> {
    let matcher = build_matcher(patterns)?;
    let literal_directories = patterns
        .iter()
        .map(|pattern| pattern.replace('\\', "/"))
        .filter(|pattern| {
            !pattern
                .bytes()
                .any(|byte| matches!(byte, b'*' | b'?' | b'[' | b'{'))
        })
        .collect::<Vec<_>>();
    let matched = paths
        .iter()
        .filter(|path| matcher.is_match(path))
        .collect::<HashSet<_>>();

    Ok(paths.iter().all(|path| {
        matched.contains(path)
            || literal_directories.iter().any(|directory| {
                path == directory
                    || path
                        .strip_prefix(directory)
                        .is_some_and(|suffix| suffix.starts_with('/'))
            })
            || matched.iter().any(|matched_path| {
                matched_path
                    .strip_prefix(path)
                    .is_some_and(|suffix| suffix.starts_with('/'))
            })
    }))
}

fn reject_symlink_prefixes(workspace: &Path, pattern: &str) -> Result<()> {
    let portable = pattern.replace('\\', "/");
    let mut cursor = workspace.to_path_buf();
    for part in portable.split('/') {
        if part
            .bytes()
            .any(|byte| matches!(byte, b'*' | b'?' | b'[' | b'{'))
        {
            break;
        }
        if part.is_empty() {
            continue;
        }
        cursor.push(part);
        match fs::symlink_metadata(&cursor) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(CacheError::new(
                    "unsupported-file",
                    "source path has a symlink component",
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => break,
            Err(error) => return Err(CacheError::io("source-stat", error)),
        }
    }
    Ok(())
}

fn walk(
    path: &Path,
) -> impl Iterator<Item = std::result::Result<walkdir::DirEntry, walkdir::Error>> {
    WalkDir::new(path)
        .follow_links(false)
        .sort_by_file_name()
        .into_iter()
}

fn snapshot(workspace: &Path, path: &Path) -> Result<ArchiveEntry> {
    let metadata = fs::symlink_metadata(path).cache_err("source-stat")?;
    let relative = portable_path(workspace, path)?;
    if metadata.file_type().is_symlink() {
        return Err(CacheError::new(
            "unsupported-file",
            "symlinks are unsupported",
        ));
    }
    let kind = if metadata.is_file() {
        EntryKind::File
    } else if metadata.is_dir() {
        EntryKind::Directory
    } else {
        return Err(CacheError::new(
            "unsupported-file",
            "special files are unsupported",
        ));
    };
    if kind == EntryKind::File && metadata.nlink() != 1 {
        return Err(CacheError::new(
            "unsupported-file",
            "hard-linked files are unsupported",
        ));
    }
    if metadata.len() > MAX_FILE_BYTES {
        return Err(CacheError::new("file-limit", "file exceeds 1 GiB"));
    }
    Ok(ArchiveEntry {
        relative,
        absolute: path.to_path_buf(),
        kind,
        mode: metadata.mode() & 0o777,
        size: metadata.len(),
        modified_nanos: i128::from(metadata.mtime()) * 1_000_000_000
            + i128::from(metadata.mtime_nsec()),
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

fn add_with_ancestors(
    workspace: &Path,
    path: &Path,
    entries: &mut BTreeMap<String, ArchiveEntry>,
) -> Result<()> {
    let mut current = Some(path);
    while let Some(item) = current {
        if item == workspace {
            break;
        }
        let entry = snapshot(workspace, item)?;
        entries.entry(entry.relative.clone()).or_insert(entry);
        current = item.parent();
    }
    Ok(())
}

pub fn collect_entries(
    workspace: &Path,
    patterns: &[String],
    started: Instant,
) -> Result<Vec<ArchiveEntry>> {
    let matcher = build_matcher(patterns)?;
    for pattern in patterns {
        reject_symlink_prefixes(workspace, pattern)?;
    }
    let mut matched = Vec::new();
    for item in walk(workspace) {
        check_operation_timeout(started)?;
        let item = item.map_err(|error| CacheError::new("source-walk", error.to_string()))?;
        if item.depth() == 0 {
            continue;
        }
        let relative = portable_path(workspace, item.path())?;
        if matcher.is_match(&relative) {
            if item.file_type().is_symlink() {
                return Err(CacheError::new(
                    "unsupported-file",
                    "symlinks are unsupported",
                ));
            }
            matched.push(item.path().to_path_buf());
        }
    }

    let mut entries = BTreeMap::new();
    for root in matched {
        let metadata = fs::symlink_metadata(&root).cache_err("source-stat")?;
        if metadata.is_dir() {
            for item in walk(&root) {
                check_operation_timeout(started)?;
                let item =
                    item.map_err(|error| CacheError::new("source-walk", error.to_string()))?;
                add_with_ancestors(workspace, item.path(), &mut entries)?;
                if entries.len() > MAX_ENTRIES {
                    return Err(CacheError::new(
                        "entry-limit",
                        "archive exceeds 100,000 entries",
                    ));
                }
            }
        } else {
            add_with_ancestors(workspace, &root, &mut entries)?;
        }
    }
    if entries.len() > MAX_ENTRIES {
        return Err(CacheError::new(
            "entry-limit",
            "archive exceeds 100,000 entries",
        ));
    }
    let extracted = entries
        .values()
        .filter(|entry| entry.kind == EntryKind::File)
        .try_fold(0_u64, |total, entry| total.checked_add(entry.size))
        .ok_or_else(|| CacheError::new("size-limit", "archive size overflow"))?;
    if extracted > MAX_EXTRACTED_BYTES {
        return Err(CacheError::new("size-limit", "archive exceeds 8 GiB"));
    }
    Ok(entries.into_values().collect())
}

struct LimitedWriter<W> {
    inner: W,
    written: u64,
    started: Instant,
}

impl<W: Write> Write for LimitedWriter<W> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if self.started.elapsed() > OPERATION_TIMEOUT {
            return Err(io::Error::other("save exceeded 20 minutes"));
        }
        if self.written.saturating_add(buffer.len() as u64) > MAX_COMPRESSED_BYTES {
            return Err(io::Error::other("compressed archive exceeds 2 GiB"));
        }
        let count = self.inner.write(buffer)?;
        self.written += count as u64;
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

pub fn create_archive(entries: &[ArchiveEntry], output: &Path, started: Instant) -> Result<()> {
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(output)
        .cache_err("archive-create")?;
    let limited = LimitedWriter {
        inner: file,
        written: 0,
        started,
    };
    let encoder = zstd::stream::write::Encoder::new(limited, 3).cache_err("archive-create")?;
    let mut builder = Builder::new(encoder);
    for item in entries {
        if started.elapsed() > OPERATION_TIMEOUT {
            return Err(CacheError::new(
                "operation-timeout",
                "save exceeded 20 minutes",
            ));
        }
        let mut header = Header::new_gnu();
        header.set_entry_type(if item.kind == EntryKind::Directory {
            EntryType::Directory
        } else {
            EntryType::Regular
        });
        header.set_mode(item.mode);
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime((item.modified_nanos / 1_000_000_000).max(0) as u64);
        header.set_size(if item.kind == EntryKind::File {
            item.size
        } else {
            0
        });
        header.set_cksum();
        if item.kind == EntryKind::File {
            let mut source = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&item.absolute)
                .cache_err("source-read")?;
            let opened = source.metadata().cache_err("source-stat")?;
            if !opened.is_file()
                || opened.nlink() != 1
                || opened.len() != item.size
                || opened.dev() != item.device
                || opened.ino() != item.inode
            {
                return Err(CacheError::new(
                    "source-mutated",
                    "source changed before archiving",
                ));
            }
            builder
                .append_data(&mut header, &item.relative, &mut source)
                .cache_err("archive-write")?;
            let after = fs::symlink_metadata(&item.absolute).cache_err("source-stat")?;
            let modified =
                i128::from(after.mtime()) * 1_000_000_000 + i128::from(after.mtime_nsec());
            if !after.is_file()
                || after.file_type().is_symlink()
                || after.nlink() != 1
                || after.len() != item.size
                || after.dev() != item.device
                || after.ino() != item.inode
                || modified != item.modified_nanos
            {
                return Err(CacheError::new(
                    "source-mutated",
                    "source changed while archiving",
                ));
            }
        } else {
            builder
                .append_data(&mut header, &item.relative, io::empty())
                .cache_err("archive-write")?;
        }
    }
    for item in entries {
        let after = fs::symlink_metadata(&item.absolute).cache_err("source-stat")?;
        let modified = i128::from(after.mtime()) * 1_000_000_000 + i128::from(after.mtime_nsec());
        let expected_file = item.kind == EntryKind::File;
        if after.file_type().is_symlink()
            || (expected_file && (!after.is_file() || after.nlink() != 1))
            || (!expected_file && !after.is_dir())
            || after.len() != item.size
            || after.dev() != item.device
            || after.ino() != item.inode
            || modified != item.modified_nanos
        {
            return Err(CacheError::new(
                "source-mutated",
                "source changed while archiving",
            ));
        }
    }
    let encoder = builder.into_inner().cache_err("archive-write")?;
    let limited = encoder.finish().cache_err("archive-write")?;
    limited.inner.sync_all().cache_err("archive-sync")
}

fn checked_entry_path(path: &Path) -> Result<String> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(
                part.to_str()
                    .ok_or_else(|| CacheError::new("unsafe-archive", "non-UTF-8 archive path"))?,
            ),
            _ => {
                return Err(CacheError::new(
                    "unsafe-archive",
                    "absolute or traversal archive path",
                ));
            }
        }
    }
    if parts.is_empty() {
        return Err(CacheError::new("unsafe-archive", "empty archive path"));
    }
    Ok(parts.join("/"))
}

pub fn extract_archive(
    payload: &Path,
    staging: &Path,
    expected_paths: &[String],
    started: Instant,
) -> Result<(usize, u64)> {
    fs::create_dir(staging).cache_err("restore-staging")?;
    fs::set_permissions(staging, fs::Permissions::from_mode(0o700)).cache_err("restore-staging")?;
    let decoder = zstd::stream::read::Decoder::new(File::open(payload).cache_err("payload-read")?)
        .cache_err("archive-read")?;
    let mut archive = Archive::new(TimedReader {
        inner: decoder,
        started,
    });
    let mut seen = BTreeMap::<String, EntryKind>::new();
    let mut directories = Vec::new();
    let mut files = 0_usize;
    let mut bytes = 0_u64;
    let archive_entries = archive.entries().cache_err("archive-read")?;
    for item in archive_entries {
        if started.elapsed() > OPERATION_TIMEOUT {
            return Err(CacheError::new(
                "operation-timeout",
                "restore exceeded 20 minutes",
            ));
        }
        let mut item = item.cache_err("archive-read")?;
        let path = checked_entry_path(&item.path().cache_err("unsafe-archive")?)?;
        let kind = match item.header().entry_type() {
            EntryType::Regular => EntryKind::File,
            EntryType::Directory => EntryKind::Directory,
            _ => {
                return Err(CacheError::new(
                    "unsafe-archive",
                    "links and special archive entries are unsupported",
                ));
            }
        };
        if seen.contains_key(&path) {
            return Err(CacheError::new(
                "unsafe-archive",
                "archive contains duplicate paths",
            ));
        }
        if let Some((parent, _)) = path.rsplit_once('/')
            && seen.get(parent) != Some(&EntryKind::Directory)
        {
            return Err(CacheError::new(
                "unsafe-archive",
                "archive path has a missing or non-directory parent",
            ));
        }
        files += 1;
        if files > MAX_ENTRIES {
            return Err(CacheError::new(
                "entry-limit",
                "archive exceeds 100,000 entries",
            ));
        }
        let size = item.header().size().cache_err("archive-read")?;
        if size > MAX_FILE_BYTES {
            return Err(CacheError::new("file-limit", "archive file exceeds 1 GiB"));
        }
        bytes = bytes
            .checked_add(size)
            .ok_or_else(|| CacheError::new("size-limit", "archive size overflow"))?;
        if bytes > MAX_EXTRACTED_BYTES {
            return Err(CacheError::new("size-limit", "archive exceeds 8 GiB"));
        }
        let destination = path
            .split('/')
            .fold(staging.to_path_buf(), |base, part| base.join(part));
        let mode = item.header().mode().cache_err("archive-read")? & 0o777;
        match kind {
            EntryKind::Directory => {
                fs::create_dir(&destination).cache_err("archive-extract")?;
                fs::set_permissions(&destination, fs::Permissions::from_mode(0o700))
                    .cache_err("archive-extract")?;
                let mtime = item.header().mtime().unwrap_or(0);
                directories.push((destination, mode, mtime));
            }
            EntryKind::File => {
                let mut output = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(mode)
                    .open(&destination)
                    .cache_err("archive-extract")?;
                let copied = io::copy(&mut item, &mut output).cache_err("archive-extract")?;
                if copied != size {
                    return Err(CacheError::new(
                        "unsafe-archive",
                        "archive entry size mismatch",
                    ));
                }
                output.sync_all().cache_err("archive-extract")?;
                if let Ok(mtime) = item.header().mtime() {
                    filetime::set_file_mtime(
                        &destination,
                        FileTime::from_unix_time(mtime as i64, 0),
                    )
                    .cache_err("archive-extract")?;
                }
            }
        }
        seen.insert(path, kind);
    }
    let actual: BTreeSet<_> = seen.into_keys().collect();
    let expected: BTreeSet<_> = expected_paths.iter().cloned().collect();
    if actual != expected || actual.len() != expected_paths.len() {
        return Err(CacheError::new(
            "invalid-entry",
            "archive paths do not match metadata",
        ));
    }
    for (directory, mode, mtime) in directories.into_iter().rev() {
        File::open(&directory)
            .cache_err("archive-extract")?
            .sync_all()
            .cache_err("archive-extract")?;
        filetime::set_file_mtime(&directory, FileTime::from_unix_time(mtime as i64, 0))
            .cache_err("archive-extract")?;
        fs::set_permissions(&directory, fs::Permissions::from_mode(mode))
            .cache_err("archive-extract")?;
    }
    File::open(staging)
        .cache_err("archive-extract")?
        .sync_all()
        .cache_err("archive-extract")?;
    Ok((files, bytes))
}

pub fn materialize(staging: &Path, workspace: &Path, started: Instant) -> Result<()> {
    let mut names = fs::read_dir(staging)
        .cache_err("materialize")?
        .map(|item| {
            item.map(|entry| entry.file_name())
                .map_err(|error| CacheError::io("materialize", error))
        })
        .collect::<Result<Vec<_>>>()?;
    names.sort();
    let mut top_directory_modes = BTreeMap::new();
    for name in &names {
        check_operation_timeout(started)?;
        match fs::symlink_metadata(workspace.join(name)) {
            Ok(_) => {
                return Err(CacheError::new(
                    "destination-exists",
                    "restore destination already exists",
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(CacheError::io("destination-check", error)),
        }
        let source = staging.join(name);
        let metadata = fs::symlink_metadata(&source).cache_err("materialize")?;
        if metadata.is_dir() {
            top_directory_modes.insert(
                name.clone(),
                (
                    metadata.mode() & 0o777,
                    FileTime::from_unix_time(metadata.mtime(), metadata.mtime_nsec() as u32),
                ),
            );
            fs::set_permissions(&source, fs::Permissions::from_mode(0o700))
                .cache_err("materialize")?;
        }
    }
    let mut moved = Vec::new();
    for name in &names {
        check_operation_timeout(started)?;
        if let Err(error) = fs::rename(staging.join(name), workspace.join(name)) {
            for prior in moved.iter().rev() {
                fs::rename(workspace.join(prior), staging.join(prior))
                    .cache_err("materialize-rollback")?;
            }
            restore_top_directory_modes(staging, &top_directory_modes)?;
            return Err(CacheError::io("materialize", error));
        }
        moved.push(name.clone());
    }
    if let Err(error) = File::open(workspace).and_then(|file| file.sync_all()) {
        for prior in moved.iter().rev() {
            fs::rename(workspace.join(prior), staging.join(prior))
                .cache_err("materialize-rollback")?;
        }
        restore_top_directory_modes(staging, &top_directory_modes)?;
        return Err(CacheError::io("materialize", error));
    }
    if let Err(error) = restore_top_directory_modes(workspace, &top_directory_modes) {
        for name in top_directory_modes.keys() {
            fs::set_permissions(workspace.join(name), fs::Permissions::from_mode(0o700))
                .cache_err("materialize-rollback")?;
        }
        for prior in moved.iter().rev() {
            fs::rename(workspace.join(prior), staging.join(prior))
                .cache_err("materialize-rollback")?;
        }
        restore_top_directory_modes(staging, &top_directory_modes)?;
        return Err(error);
    }
    Ok(())
}

fn check_operation_timeout(started: Instant) -> Result<()> {
    if started.elapsed() > OPERATION_TIMEOUT {
        return Err(CacheError::new(
            "operation-timeout",
            "cache operation exceeded 20 minutes",
        ));
    }
    Ok(())
}

fn restore_top_directory_modes(
    parent: &Path,
    modes: &BTreeMap<std::ffi::OsString, (u32, FileTime)>,
) -> Result<()> {
    for (name, (mode, mtime)) in modes {
        let path = parent.join(name);
        filetime::set_file_mtime(&path, *mtime).cache_err("materialize")?;
        fs::set_permissions(&path, fs::Permissions::from_mode(*mode)).cache_err("materialize")?;
    }
    Ok(())
}

struct TimedReader<R> {
    inner: R,
    started: Instant,
}

impl<R: Read> Read for TimedReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.started.elapsed() > OPERATION_TIMEOUT {
            return Err(io::Error::other("restore exceeded 20 minutes"));
        }
        self.inner.read(buffer)
    }
}
