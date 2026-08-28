mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::thread;

use github_actions_local_cache::digest::sha256;
use github_actions_local_cache::entry::entry_directory;
use github_actions_local_cache::{
    RestoreMatch, RestoreRequest, SaveRequest, SaveResult, restore_cache, save_cache,
};

use common::Fixture;

fn save(
    fixture: &Fixture,
    key: &str,
    patterns: &[&str],
) -> github_actions_local_cache::Result<SaveResult> {
    save_cache(SaveRequest {
        context: fixture.context.clone(),
        key: key.to_owned(),
        patterns: patterns.iter().map(|value| (*value).to_owned()).collect(),
    })
}

fn restore(
    fixture: &Fixture,
    key: &str,
    prefixes: &[&str],
) -> github_actions_local_cache::Result<github_actions_local_cache::RestoreResult> {
    restore_cache(RestoreRequest {
        context: fixture.context.clone(),
        key: key.to_owned(),
        restore_keys: prefixes.iter().map(|value| (*value).to_owned()).collect(),
    })
}

#[test]
fn miss_save_exact_restore_preserves_content_empty_directories_and_modes() {
    let fixture = Fixture::new();
    assert_eq!(
        restore(&fixture, "build-v1", &[]).unwrap().cache_match,
        RestoreMatch::Miss
    );
    fs::create_dir_all(fixture.workspace.join("artifacts/empty")).unwrap();
    fs::write(
        fixture.workspace.join("artifacts/result.txt"),
        "verified output\n",
    )
    .unwrap();
    fs::set_permissions(
        fixture.workspace.join("artifacts"),
        fs::Permissions::from_mode(0o500),
    )
    .unwrap();
    assert_eq!(
        save(&fixture, "build-v1", &["artifacts"]).unwrap(),
        SaveResult::Saved
    );
    fs::set_permissions(
        fixture.workspace.join("artifacts"),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    fs::remove_dir_all(fixture.workspace.join("artifacts")).unwrap();

    let result = restore(&fixture, "build-v1", &[]).unwrap();
    assert_eq!(result.cache_match, RestoreMatch::Exact);
    assert_eq!(
        fs::read_to_string(fixture.workspace.join("artifacts/result.txt")).unwrap(),
        "verified output\n"
    );
    assert!(
        fs::read_dir(fixture.workspace.join("artifacts/empty"))
            .unwrap()
            .next()
            .is_none()
    );
    assert_eq!(
        fs::metadata(fixture.workspace.join("artifacts"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o500
    );
}

#[test]
fn complete_entries_are_immutable_and_losers_report_raced() {
    let fixture = Fixture::new();
    fs::write(fixture.workspace.join("result.txt"), "first").unwrap();
    assert_eq!(
        save(&fixture, "same", &["result.txt"]).unwrap(),
        SaveResult::Saved
    );
    fs::write(fixture.workspace.join("result.txt"), "second").unwrap();
    assert_eq!(
        save(&fixture, "same", &["result.txt"]).unwrap(),
        SaveResult::Raced
    );
    fs::remove_file(fixture.workspace.join("result.txt")).unwrap();
    restore(&fixture, "same", &[]).unwrap();
    assert_eq!(
        fs::read_to_string(fixture.workspace.join("result.txt")).unwrap(),
        "first"
    );
}

#[test]
fn ordered_prefixes_restore_a_fallback_without_an_exact_hit() {
    let fixture = Fixture::new();
    fs::write(fixture.workspace.join("data.bin"), "fallback").unwrap();
    save(&fixture, "deps-linux-old", &["data.bin"]).unwrap();
    fs::remove_file(fixture.workspace.join("data.bin")).unwrap();
    let result = restore(&fixture, "deps-linux-new", &["none-", "deps-linux-"]).unwrap();
    assert_eq!(result.cache_match, RestoreMatch::Fallback);
    assert_eq!(
        fs::read_to_string(fixture.workspace.join("data.bin")).unwrap(),
        "fallback"
    );
}

#[test]
fn corrupt_exact_entry_is_quarantined_before_fallback() {
    let fixture = Fixture::new();
    fs::write(fixture.workspace.join("data.bin"), "old").unwrap();
    save(&fixture, "family-old", &["data.bin"]).unwrap();
    fs::remove_file(fixture.workspace.join("data.bin")).unwrap();
    let exact = entry_directory(&fixture.context, &sha256("family-new"));
    fs::create_dir(&exact).unwrap();
    fs::write(exact.join("metadata.json"), "not-json").unwrap();

    let result = restore(&fixture, "family-new", &["family-"]).unwrap();
    assert_eq!(result.cache_match, RestoreMatch::Fallback);
    let namespace = fixture.cache_root.join("v1/12345");
    assert!(fs::read_dir(namespace).unwrap().flatten().any(|item| {
        item.file_name()
            .to_string_lossy()
            .starts_with(&format!(".quarantine-{}-", sha256("family-new")))
    }));
}

#[test]
fn interrupted_entry_without_a_complete_marker_is_quarantined() {
    let fixture = Fixture::new();
    fs::write(fixture.workspace.join("seed"), "seed").unwrap();
    save(&fixture, "initialize", &["seed"]).unwrap();
    let digest = sha256("interrupted");
    let exact = entry_directory(&fixture.context, &digest);
    fs::create_dir(&exact).unwrap();
    fs::write(exact.join("payload.tar.zst"), "partial").unwrap();

    let result = restore(&fixture, "interrupted", &[]).unwrap();
    assert_eq!(result.cache_match, RestoreMatch::Miss);
    assert!(!exact.exists());
    assert!(
        fs::read_dir(fixture.cache_root.join("v1/12345"))
            .unwrap()
            .flatten()
            .any(|item| item
                .file_name()
                .to_string_lossy()
                .starts_with(&format!(".quarantine-{digest}-")))
    );
}

#[test]
fn restore_fails_closed_on_existing_destination() {
    let fixture = Fixture::new();
    fs::write(fixture.workspace.join("output"), "cached").unwrap();
    save(&fixture, "collision", &["output"]).unwrap();
    fs::write(fixture.workspace.join("output"), "workspace").unwrap();
    let error = restore(&fixture, "collision", &[]).unwrap_err();
    assert_eq!(error.code, "destination-exists");
    assert_eq!(
        fs::read_to_string(fixture.workspace.join("output")).unwrap(),
        "workspace"
    );
}

#[test]
fn concurrent_saves_have_one_winner() {
    let fixture = Fixture::new();
    fs::write(fixture.workspace.join("data"), vec![7_u8; 1024 * 1024]).unwrap();
    let left = fixture.context.clone();
    let right = fixture.context.clone();
    let first = thread::spawn(move || {
        save_cache(SaveRequest {
            context: left,
            key: "race".to_owned(),
            patterns: vec!["data".to_owned()],
        })
        .unwrap()
    });
    let second = thread::spawn(move || {
        save_cache(SaveRequest {
            context: right,
            key: "race".to_owned(),
            patterns: vec!["data".to_owned()],
        })
        .unwrap()
    });
    let mut results = [first.join().unwrap(), second.join().unwrap()];
    results.sort_by_key(|result| result.as_str());
    assert_eq!(results, [SaveResult::Raced, SaveResult::Saved]);
}

#[test]
fn unmatched_patterns_are_skipped() {
    let fixture = Fixture::new();
    assert_eq!(
        save(&fixture, "empty", &["missing/**"]).unwrap(),
        SaveResult::SkippedNoPaths
    );
}
