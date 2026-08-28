use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use github_actions_local_cache::CacheContext;
use tempfile::TempDir;

#[allow(dead_code)]
pub struct Fixture {
    pub temporary: TempDir,
    pub cache_root: PathBuf,
    pub workspace: PathBuf,
    pub context: CacheContext,
}

impl Fixture {
    pub fn new() -> Self {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let cache_root = temporary.path().join("cache");
        let workspace = temporary.path().join("workspace");
        fs::create_dir(&cache_root).unwrap();
        fs::set_permissions(&cache_root, fs::Permissions::from_mode(0o700)).unwrap();
        fs::create_dir(&workspace).unwrap();
        let context = CacheContext {
            cache_root: cache_root.clone(),
            workspace: workspace.clone(),
            repository_id: "12345".to_owned(),
            arch: "x64".to_owned(),
        };
        Self {
            temporary,
            cache_root,
            workspace,
            context,
        }
    }
}
