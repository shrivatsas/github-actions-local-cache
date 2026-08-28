use std::time::Duration;

pub const SCHEMA: u32 = 1;
pub const ARCHIVE_POLICY: u32 = 1;
pub const CACHE_VERSION: &str = "v1";
pub const MAX_COMPRESSED_BYTES: u64 = 2 * 1024 * 1024 * 1024;
pub const MAX_EXTRACTED_BYTES: u64 = 8 * 1024 * 1024 * 1024;
pub const MAX_ENTRIES: usize = 100_000;
pub const MAX_FILE_BYTES: u64 = 1024 * 1024 * 1024;
pub const MAX_METADATA_BYTES: u64 = 1024 * 1024;
pub const OPERATION_TIMEOUT: Duration = Duration::from_secs(20 * 60);
pub const LOCK_TIMEOUT: Duration = Duration::from_secs(30);
pub const MAX_FALLBACK_CANDIDATES: usize = 256;
