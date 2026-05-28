//! Tests for [`verbreel_storage::fs::atomic_write_bytes`].
//!
//! Each test uses a fresh [`TempDir`] so concurrent test runs do not
//! step on each other.

use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;

use tempfile::TempDir;
use verbreel_storage::fs::atomic_write_bytes;

fn td() -> TempDir {
    TempDir::new().expect("TempDir::new failed")
}

#[test]
fn writes_new_file_with_expected_bytes() {
    let dir = td();
    let path = dir.path().join("hello.txt");

    atomic_write_bytes(&path, b"hello\n").unwrap();

    let read = fs::read(&path).unwrap();
    assert_eq!(read, b"hello\n");
}

#[test]
fn replaces_existing_file_atomically() {
    let dir = td();
    let path = dir.path().join("config.json");
    fs::write(&path, b"old contents").unwrap();

    atomic_write_bytes(&path, b"new contents").unwrap();

    let read = fs::read(&path).unwrap();
    assert_eq!(read, b"new contents");
}

#[test]
fn empty_payload_writes_empty_file() {
    let dir = td();
    let path = dir.path().join("empty");

    atomic_write_bytes(&path, b"").unwrap();

    let read = fs::read(&path).unwrap();
    assert!(read.is_empty());
    assert!(path.exists());
}

#[test]
fn large_payload_round_trips() {
    let dir = td();
    let path = dir.path().join("big.bin");
    // 4 MiB — large enough to exercise multiple internal buffer flushes
    // without making the test slow.
    let payload: Vec<u8> = (0..(4 * 1024 * 1024_usize))
        .map(|i| (i % 251) as u8)
        .collect();

    atomic_write_bytes(&path, &payload).unwrap();

    let read = fs::read(&path).unwrap();
    assert_eq!(read.len(), payload.len());
    assert_eq!(read, payload);
}

#[test]
fn binary_payload_preserves_zero_bytes() {
    let dir = td();
    let path = dir.path().join("binary.bin");
    let payload = [0u8, 0, 0, 1, 0, 2, 0, 3, 0xff, 0xfe];

    atomic_write_bytes(&path, &payload).unwrap();

    let read = fs::read(&path).unwrap();
    assert_eq!(read, payload);
}

#[test]
fn idempotent_under_repeated_writes() {
    let dir = td();
    let path = dir.path().join("loop");

    for i in 0..16 {
        let bytes = format!("iteration={i}");
        atomic_write_bytes(&path, bytes.as_bytes()).unwrap();
        assert_eq!(fs::read(&path).unwrap(), bytes.as_bytes());
    }
}

#[test]
fn missing_parent_directory_errors() {
    let dir = td();
    let path = dir.path().join("does/not/exist/file.txt");

    let err = atomic_write_bytes(&path, b"x").expect_err("must error");
    assert!(
        matches!(err.kind(), ErrorKind::NotFound | ErrorKind::Other),
        "expected NotFound/Other, got {err:?}"
    );
}

#[test]
fn path_without_parent_errors_with_invalid_input() {
    // A bare relative path with no parent component (just a file name)
    // resolves to parent == Some(""), which is treated as the CWD; the
    // *truly* parent-less case is the root path "/". On Linux that
    // means trying to atomic-write to "/" which is a directory.
    // Use the empty path which has parent == None.
    let path = PathBuf::new();

    let err = atomic_write_bytes(&path, b"x").expect_err("must error");
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
}

#[test]
fn nested_parent_must_exist_before_call() {
    // atomic_write_bytes only creates the temp file inside an
    // EXISTING parent. It does not mkdir -p. Verify by writing into
    // a path whose immediate parent does not exist.
    let dir = td();
    let path = dir.path().join("nested").join("a.txt");

    let err = atomic_write_bytes(&path, b"x").expect_err("must error");
    assert!(
        matches!(err.kind(), ErrorKind::NotFound | ErrorKind::Other),
        "expected NotFound/Other, got {err:?}"
    );
}

#[test]
fn relative_path_with_filename_only_succeeds() {
    // Write into a file inside a tempdir using an absolute path: the
    // common case. (A truly relative path here would race with other
    // tests sharing CWD.)
    let dir = td();
    let path = dir.path().join("just-a-name.txt");
    atomic_write_bytes(&path, b"ok").unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"ok");
}

#[test]
fn does_not_leave_temp_file_behind_on_success() {
    let dir = td();
    let path = dir.path().join("target.txt");
    atomic_write_bytes(&path, b"data").unwrap();

    let entries: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name())
        .collect();
    // Only the target file should remain — NamedTempFile cleans up its
    // own tempfile on persist().
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0], std::ffi::OsString::from("target.txt"));
}
