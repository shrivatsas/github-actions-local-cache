mod common;

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};

use github_actions_local_cache::config::{parse_boolean, parse_lines};
use github_actions_local_cache::entry::ensure_repository_directory;
use github_actions_local_cache::{CacheContext, validate_key, validate_patterns};

use common::Fixture;

#[test]
fn parses_strict_input_grammar() {
    assert_eq!(parse_lines(" one\r\n\n two \n"), ["one", "two"]);
    assert!(parse_boolean("true", "flag").unwrap());
    assert!(parse_boolean("True", "flag").is_err());
    assert!(validate_key(&"é".repeat(256)).is_ok());
    assert!(validate_key(&"é".repeat(257)).is_err());
    assert!(validate_patterns(&["dist/**/*.tgz".to_owned()]).is_ok());
    assert!(validate_patterns(&["!ignored".to_owned()]).is_err());
    assert!(validate_patterns(&["../secret".to_owned()]).is_err());
}

#[test]
fn validates_root_ownership_mode_and_location() {
    let fixture = Fixture::new();
    let cache_root = fs::canonicalize(&fixture.cache_root).unwrap();
    let workspace = fs::canonicalize(&fixture.workspace).unwrap();
    let context = CacheContext::new_for_platform(
        cache_root.to_str().unwrap(),
        workspace.to_str().unwrap(),
        "12345",
        "linux",
        "x86_64",
    )
    .unwrap();
    assert_eq!(context.arch, "x64");

    fs::set_permissions(&fixture.cache_root, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(
        CacheContext::new_for_platform(
            cache_root.to_str().unwrap(),
            workspace.to_str().unwrap(),
            "12345",
            "linux",
            "x64"
        )
        .is_err()
    );
}

#[test]
fn rejects_symlinked_cache_root_components() {
    let fixture = Fixture::new();
    let target = fixture.temporary.path().join("target");
    let linked = fixture.temporary.path().join("linked");
    fs::create_dir(&target).unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o700)).unwrap();
    symlink(&target, &linked).unwrap();
    let error = CacheContext::new_for_platform(
        linked.to_str().unwrap(),
        fixture.workspace.to_str().unwrap(),
        "12345",
        "linux",
        "x64",
    )
    .unwrap_err();
    assert_eq!(error.code, "invalid-root");
}

#[test]
fn rejects_cache_root_claimed_by_a_different_repository() {
    let fixture = Fixture::new();
    ensure_repository_directory(&fixture.context).unwrap();
    let other_repository = CacheContext {
        repository_id: "67890".to_owned(),
        ..fixture.context.clone()
    };

    let error = ensure_repository_directory(&other_repository).unwrap_err();
    assert_eq!(error.code, "shared-root-detected");
}

#[test]
fn rejects_legacy_root_with_a_different_repository_namespace() {
    let fixture = Fixture::new();
    let foreign_namespace = fixture.cache_root.join("v1/67890");
    fs::create_dir(fixture.cache_root.join("v1")).unwrap();
    fs::set_permissions(
        fixture.cache_root.join("v1"),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    fs::create_dir(foreign_namespace).unwrap();
    fs::set_permissions(
        fixture.cache_root.join("v1/67890"),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();

    let error = ensure_repository_directory(&fixture.context).unwrap_err();
    assert_eq!(error.code, "shared-root-detected");
}
