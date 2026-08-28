mod common;

use std::fs::{self, File};
use std::io;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::time::Instant;

use github_actions_local_cache::archive::extract_archive;
use github_actions_local_cache::digest::sha256;
use github_actions_local_cache::entry::{ensure_repository_directory, entry_directory};
use github_actions_local_cache::{
    RestoreMatch, RestoreRequest, SaveRequest, restore_cache, save_cache,
};
use tar::{Builder, EntryType, Header};

use common::Fixture;

#[test]
fn save_rejects_symlinks_and_hard_links() {
    let fixture = Fixture::new();
    fs::write(fixture.workspace.join("target"), "x").unwrap();
    symlink("target", fixture.workspace.join("link")).unwrap();
    let error = save_cache(SaveRequest {
        context: fixture.context.clone(),
        key: "links".to_owned(),
        patterns: vec!["link".to_owned()],
    })
    .unwrap_err();
    assert_eq!(error.code, "unsupported-file");

    fs::hard_link(
        fixture.workspace.join("target"),
        fixture.workspace.join("hard"),
    )
    .unwrap();
    let error = save_cache(SaveRequest {
        context: fixture.context.clone(),
        key: "hard".to_owned(),
        patterns: vec!["target".to_owned()],
    })
    .unwrap_err();
    assert_eq!(error.code, "unsupported-file");
}

fn malicious_archive(fixture: &Fixture, name: &[u8], kind: EntryType) -> std::path::PathBuf {
    let payload = fixture
        .temporary
        .path()
        .join(format!("malicious-{}.tar.zst", kind.as_byte()));
    let encoder = zstd::stream::write::Encoder::new(File::create(&payload).unwrap(), 1).unwrap();
    let mut builder = Builder::new(encoder);
    let mut header = Header::new_gnu();
    header.as_mut_bytes()[..name.len()].copy_from_slice(name);
    header.set_entry_type(kind);
    header.set_mode(0o644);
    header.set_size(if kind == EntryType::Regular { 1 } else { 0 });
    if kind == EntryType::Symlink {
        header.set_link_name("target").unwrap();
    }
    header.set_cksum();
    if kind == EntryType::Regular {
        builder.append(&header, &b"x"[..]).unwrap();
    } else {
        builder.append(&header, io::empty()).unwrap();
    }
    builder.into_inner().unwrap().finish().unwrap();
    payload
}

#[test]
fn restore_rejects_traversal_and_link_archives() {
    let fixture = Fixture::new();
    let traversal = malicious_archive(&fixture, b"../escape", EntryType::Regular);
    let staging = fixture.temporary.path().join("extract-traversal");
    assert_eq!(
        extract_archive(
            &traversal,
            &staging,
            &["../escape".to_owned()],
            Instant::now()
        )
        .unwrap_err()
        .code,
        "unsafe-archive"
    );
    assert!(!fixture.temporary.path().join("escape").exists());

    let link = malicious_archive(&fixture, b"link", EntryType::Symlink);
    let staging = fixture.temporary.path().join("extract-link");
    assert_eq!(
        extract_archive(&link, &staging, &["link".to_owned()], Instant::now())
            .unwrap_err()
            .code,
        "unsafe-archive"
    );
}

#[test]
fn quarantine_does_not_follow_a_malicious_entry_symlink() {
    let fixture = Fixture::new();
    ensure_repository_directory(&fixture.context).unwrap();
    let target = fixture.temporary.path().join("outside");
    fs::create_dir(&target).unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).unwrap();
    let exact = entry_directory(&fixture.context, &sha256("malicious"));
    symlink(&target, &exact).unwrap();

    let result = restore_cache(RestoreRequest {
        context: fixture.context.clone(),
        key: "malicious".to_owned(),
        patterns: vec!["**".to_owned()],
        restore_keys: vec![],
    })
    .unwrap();
    assert_eq!(result.cache_match, RestoreMatch::Miss);
    assert_eq!(
        fs::metadata(&target).unwrap().permissions().mode() & 0o777,
        0o755
    );
}
