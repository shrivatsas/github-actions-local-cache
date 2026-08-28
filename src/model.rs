use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::config::CacheContext;
use crate::limits::{ARCHIVE_POLICY, SCHEMA};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct EntryMetadata {
    pub schema: u32,
    pub archive_policy: u32,
    pub key: String,
    pub created_at: String,
    pub payload_bytes: u64,
    pub payload_sha256: String,
    pub os: String,
    pub arch: String,
    pub paths: Vec<String>,
}

impl EntryMetadata {
    pub fn compatible(&self, context: &CacheContext) -> bool {
        self.schema == SCHEMA
            && self.archive_policy == ARCHIVE_POLICY
            && self.os == "linux"
            && self.arch == context.arch
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct CompleteMarker {
    pub schema: u32,
    pub payload_sha256: String,
}

#[derive(Debug)]
pub struct RestoreRequest {
    pub context: CacheContext,
    pub key: String,
    pub patterns: Vec<String>,
    pub restore_keys: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreMatch {
    Exact,
    Fallback,
    Miss,
}

impl RestoreMatch {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Fallback => "fallback",
            Self::Miss => "miss",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct RestoreResult {
    pub cache_match: RestoreMatch,
    pub digest: Option<String>,
    pub files: usize,
    pub bytes: u64,
}

#[derive(Debug)]
pub struct SaveRequest {
    pub context: CacheContext,
    pub key: String,
    pub patterns: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveResult {
    Saved,
    Raced,
    SkippedNoPaths,
}

impl SaveResult {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Saved => "saved",
            Self::Raced => "raced",
            Self::SkippedNoPaths => "skipped-no-paths",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Directory,
}

#[derive(Debug, Clone)]
pub struct ArchiveEntry {
    pub relative: String,
    pub absolute: PathBuf,
    pub kind: EntryKind,
    pub mode: u32,
    pub size: u64,
    pub modified_nanos: i128,
    pub device: u64,
    pub inode: u64,
}
