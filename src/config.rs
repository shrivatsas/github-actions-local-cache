use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

use crate::error::{CacheError, IoResultExt, Result};

#[derive(Debug, Clone)]
pub struct CacheContext {
    pub cache_root: PathBuf,
    pub workspace: PathBuf,
    pub repository_id: String,
    pub arch: String,
}

impl CacheContext {
    pub fn from_environment(cache_dir: Option<&str>) -> Result<Self> {
        let root = cache_dir
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .or_else(|| std::env::var("CACHE_DIR").ok())
            .unwrap_or_default();
        let workspace = std::env::var("GITHUB_WORKSPACE").unwrap_or_default();
        let repository_id = std::env::var("GITHUB_REPOSITORY_ID").unwrap_or_default();
        Self::new_for_platform(
            &root,
            &workspace,
            &repository_id,
            std::env::consts::OS,
            std::env::consts::ARCH,
        )
    }

    pub fn new_for_platform(
        root: &str,
        workspace: &str,
        repository_id: &str,
        os: &str,
        arch: &str,
    ) -> Result<Self> {
        if root.is_empty() {
            return Err(CacheError::new(
                "invalid-root",
                "cache-dir or CACHE_DIR is required",
            ));
        }
        if workspace.is_empty() {
            return Err(CacheError::new(
                "invalid-environment",
                "GITHUB_WORKSPACE is required",
            ));
        }
        let cache_root = normalize_absolute(Path::new(root), "cache-dir")?;
        let workspace = normalize_absolute(Path::new(workspace), "GITHUB_WORKSPACE")?;
        if !repository_id.bytes().all(|byte| byte.is_ascii_digit()) || repository_id.is_empty() {
            return Err(CacheError::new(
                "invalid-environment",
                "GITHUB_REPOSITORY_ID must be numeric",
            ));
        }
        if os != "linux" {
            return Err(CacheError::new(
                "unsupported-platform",
                "v1 supports Linux only",
            ));
        }
        let arch = match arch {
            "x86_64" | "x64" => "x64",
            "aarch64" | "arm64" => "arm64",
            _ => {
                return Err(CacheError::new(
                    "unsupported-platform",
                    "v1 supports x64 and arm64 only",
                ));
            }
        };
        if cache_root.starts_with(&workspace) || workspace.starts_with(&cache_root) {
            return Err(CacheError::new(
                "invalid-root",
                "cache-dir and workspace cannot contain one another",
            ));
        }
        reject_symlink_components(&cache_root)?;
        let metadata = fs::metadata(&cache_root).cache_err("invalid-root")?;
        if !metadata.is_dir() {
            return Err(CacheError::new(
                "invalid-root",
                "cache-dir must be a directory",
            ));
        }
        if metadata.uid() != unsafe { libc::geteuid() } {
            return Err(CacheError::new(
                "invalid-root",
                "cache-dir must be owned by the runner user",
            ));
        }
        if metadata.mode() & 0o777 != 0o700 {
            return Err(CacheError::new(
                "invalid-root",
                "cache-dir mode must be 0700",
            ));
        }
        Ok(Self {
            cache_root,
            workspace,
            repository_id: repository_id.to_owned(),
            arch: arch.to_owned(),
        })
    }
}

fn normalize_absolute(path: &Path, name: &str) -> Result<PathBuf> {
    if !path.is_absolute() {
        return Err(CacheError::new(
            "invalid-path",
            format!("{name} must be absolute"),
        ));
    }
    let mut normalized = PathBuf::from("/");
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::Prefix(_) => {
                return Err(CacheError::new(
                    "invalid-path",
                    format!("{name} cannot contain parent traversal"),
                ));
            }
        }
    }
    Ok(normalized)
}

fn reject_symlink_components(path: &Path) -> Result<()> {
    let mut cursor = PathBuf::from("/");
    for component in path.components() {
        if let Component::Normal(part) = component {
            cursor.push(part);
            if fs::symlink_metadata(&cursor)
                .cache_err("invalid-root")?
                .file_type()
                .is_symlink()
            {
                return Err(CacheError::new(
                    "invalid-root",
                    "cache-dir cannot contain symlinks",
                ));
            }
        }
    }
    Ok(())
}

pub fn parse_lines(value: &str) -> Vec<String> {
    value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

pub fn parse_boolean(value: &str, name: &str) -> Result<bool> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(CacheError::new(
            "invalid-input",
            format!("{name} must be exactly true or false"),
        )),
    }
}

pub fn validate_key(value: &str) -> Result<()> {
    if value.is_empty() || value.len() > 512 {
        return Err(CacheError::new(
            "invalid-input",
            "key must contain 1-512 UTF-8 bytes",
        ));
    }
    Ok(())
}

pub fn validate_patterns(patterns: &[String]) -> Result<()> {
    if patterns.is_empty() {
        return Err(CacheError::new(
            "invalid-input",
            "path must contain at least one path",
        ));
    }
    for pattern in patterns {
        let portable = pattern.replace('\\', "/");
        if portable.starts_with('!') {
            return Err(CacheError::new(
                "invalid-input",
                "negated globs are unsupported",
            ));
        }
        if portable.starts_with('/') || portable.split('/').any(|part| part == "..") {
            return Err(CacheError::new(
                "invalid-input",
                "paths must be workspace-relative",
            ));
        }
    }
    Ok(())
}
