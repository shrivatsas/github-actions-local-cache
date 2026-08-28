use std::fs;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use github_actions_local_cache::lock::with_entry_lock;

#[test]
fn lock_wait_is_bounded_and_releases_after_owner_finishes() {
    let temporary = tempfile::tempdir().unwrap();
    let lock = temporary.path().join("entry.lock");
    let thread_lock = lock.clone();
    let (sender, receiver) = mpsc::channel();
    let owner = thread::spawn(move || {
        with_entry_lock(&thread_lock, Duration::from_secs(1), || {
            sender.send(()).unwrap();
            thread::sleep(Duration::from_millis(100));
            Ok(())
        })
        .unwrap()
    });
    receiver.recv().unwrap();
    let error = with_entry_lock(&lock, Duration::from_millis(20), || Ok(())).unwrap_err();
    assert_eq!(error.code, "lock-timeout");
    owner.join().unwrap();
    with_entry_lock(&lock, Duration::from_millis(20), || Ok(())).unwrap();
}

#[test]
fn stale_lock_from_another_boot_is_recovered() {
    let temporary = tempfile::tempdir().unwrap();
    let lock = temporary.path().join("entry.lock");
    fs::write(
        &lock,
        r#"{"pid":1,"bootId":"different","createdAtMs":1,"nonce":"old"}"#,
    )
    .unwrap();
    with_entry_lock(&lock, Duration::from_millis(50), || Ok(())).unwrap();
    assert!(!lock.exists());
}
