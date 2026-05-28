//! Tests for [`verbreel_storage::flock`].
//!
//! Verify the basic RAII contract: acquire succeeds on an unlocked
//! file, a second acquire on the same path fails with `Contended`,
//! and dropping the guard releases the lock so a subsequent acquire
//! succeeds.

use std::fs::OpenOptions;
use std::path::Path;

use tempfile::TempDir;
use verbreel_storage::flock::{FlockError, acquire_exclusive};

fn open_rw(path: &Path) -> std::fs::File {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .expect("open failed")
}

#[test]
fn acquire_on_unlocked_file_succeeds() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("lockfile");

    let file = open_rw(&path);
    let guard = acquire_exclusive(file).expect("acquire failed");
    drop(guard);
}

#[test]
fn second_acquire_on_same_path_is_contended() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("lockfile");

    let _guard1 = acquire_exclusive(open_rw(&path)).expect("first acquire");

    let err = acquire_exclusive(open_rw(&path)).expect_err("second acquire must fail");
    assert!(
        matches!(err, FlockError::Contended),
        "expected Contended, got {err:?}"
    );
}

#[test]
fn drop_releases_lock_for_subsequent_acquire() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("lockfile");

    let guard = acquire_exclusive(open_rw(&path)).expect("first acquire");
    drop(guard);

    let _next = acquire_exclusive(open_rw(&path)).expect("after-drop acquire must succeed");
}

#[test]
fn guard_borrows_underlying_file() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("lockfile");

    let guard = acquire_exclusive(open_rw(&path)).expect("acquire failed");
    // Sanity: the borrowed File reports valid metadata.
    let meta = guard.file().metadata().expect("metadata");
    assert!(meta.is_file());
}

#[test]
fn many_serial_acquire_release_cycles_work() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("lockfile");

    for _ in 0..32 {
        let guard = acquire_exclusive(open_rw(&path)).expect("acquire failed");
        drop(guard);
    }
}

#[test]
fn distinct_files_lock_independently() {
    let dir = TempDir::new().unwrap();
    let a = dir.path().join("a.lock");
    let b = dir.path().join("b.lock");

    let _ga = acquire_exclusive(open_rw(&a)).expect("acquire a");
    let _gb = acquire_exclusive(open_rw(&b)).expect("acquire b");
}

#[test]
fn acquire_does_not_truncate_existing_content() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("lockfile");
    std::fs::write(&path, b"keep me").unwrap();

    let _guard = acquire_exclusive(open_rw(&path)).expect("acquire failed");

    assert_eq!(std::fs::read(&path).unwrap(), b"keep me");
}
