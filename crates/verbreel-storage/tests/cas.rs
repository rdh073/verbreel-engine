//! Integration tests for the §0.13 content-addressed storage
//! primitives.
//!
//! Two surfaces under test:
//!
//! - [`verbreel_storage::cas::cas_path`] — path-shape construction +
//!   hex validation.
//! - [`verbreel_storage::cas::sha256_file`] — streaming SHA-256 over an
//!   on-disk file.

use std::path::Path;

use tempfile::TempDir;
use verbreel_storage::cas::{CasError, cas_path, sha256_file};

// SHA-256 of the empty byte string. Canonical fixture from RFC 6234.
const SHA256_EMPTY: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

// SHA-256 of the ASCII string "abc". Canonical fixture from FIPS 180-4 App. B.
const SHA256_ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

// SHA-256 of 1 MiB of zero bytes. Confirms the 64 KiB streaming buffer
// handles multi-chunk reads correctly.
const SHA256_1MIB_ZEROS: &str = "30e14955ebf1352266dc2ff8067e68104607e750abb9d3b36582b8af909fcb58";

// --- cas_path: path shape ------------------------------------------------

#[test]
fn cas_path_emits_two_char_shard_and_full_basename() {
    let root = Path::new("/tmp/verbreel");
    let p = cas_path(root, SHA256_EMPTY, "bin").expect("valid hex must succeed");
    let expected = Path::new(
        "/tmp/verbreel/assets/e3/e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855.bin",
    );
    assert_eq!(p, expected);
}

#[test]
fn cas_path_preserves_root_prefix_and_appends_assets_segment() {
    let root = Path::new("/data/projects/example");
    let p = cas_path(root, SHA256_ABC, "txt").unwrap();
    let s = p.to_string_lossy();
    assert!(
        s.starts_with("/data/projects/example/assets/"),
        "path must be rooted at <root>/assets, got {s}"
    );
}

#[test]
fn cas_path_shard_matches_first_two_digest_chars() {
    let p = cas_path(Path::new("/r"), SHA256_ABC, "x").unwrap();
    let shard = p.parent().unwrap().file_name().unwrap().to_string_lossy();
    assert_eq!(shard, &SHA256_ABC[..2]);
}

#[test]
fn cas_path_basename_is_sha_plus_ext() {
    let p = cas_path(Path::new("/r"), SHA256_EMPTY, "mp4").unwrap();
    let basename = p.file_name().unwrap().to_string_lossy();
    assert_eq!(basename, format!("{SHA256_EMPTY}.mp4"));
}

#[test]
fn cas_path_ext_appended_verbatim_no_leading_dot_stripped() {
    // Caller is responsible for passing the ext without a leading dot.
    // If they pass "mp4" the basename ends with ".mp4"; if they pass
    // ".mp4" the basename ends with "..mp4" — documented contract.
    let p = cas_path(Path::new("/r"), SHA256_EMPTY, ".mp4").unwrap();
    let basename = p.file_name().unwrap().to_string_lossy();
    assert_eq!(basename, format!("{SHA256_EMPTY}..mp4"));
}

#[test]
fn cas_path_relative_root_remains_relative() {
    let p = cas_path(Path::new("project"), SHA256_EMPTY, "bin").unwrap();
    assert!(p.is_relative());
    assert!(p.starts_with("project/assets"));
}

// --- cas_path: hex validation -------------------------------------------

#[test]
fn cas_path_rejects_short_hex() {
    let short = "abc";
    let err = cas_path(Path::new("/r"), short, "bin").unwrap_err();
    assert_eq!(err, CasError::BadHexLength(3));
}

#[test]
fn cas_path_rejects_long_hex() {
    let long = "a".repeat(65);
    let err = cas_path(Path::new("/r"), &long, "bin").unwrap_err();
    assert_eq!(err, CasError::BadHexLength(65));
}

#[test]
fn cas_path_rejects_empty_hex() {
    let err = cas_path(Path::new("/r"), "", "bin").unwrap_err();
    assert_eq!(err, CasError::BadHexLength(0));
}

#[test]
fn cas_path_rejects_uppercase_hex() {
    let mixed = "E3B0C44298FC1C149AFBF4C8996FB92427AE41E4649B934CA495991B7852B855";
    assert_eq!(mixed.len(), 64);
    let err = cas_path(Path::new("/r"), mixed, "bin").unwrap_err();
    assert!(matches!(err, CasError::NotLowercaseHex('E')));
}

#[test]
fn cas_path_rejects_non_hex_char() {
    let bad = "g".to_owned() + "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b85";
    assert_eq!(bad.len(), 64);
    let err = cas_path(Path::new("/r"), &bad, "bin").unwrap_err();
    assert!(matches!(err, CasError::NotLowercaseHex('g')));
}

// --- sha256_file: known-fixture round-trips -----------------------------

#[test]
fn sha256_file_of_empty_file_matches_rfc6234_fixture() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("empty");
    std::fs::write(&path, b"").unwrap();
    let digest = sha256_file(&path).unwrap();
    assert_eq!(digest, SHA256_EMPTY);
}

#[test]
fn sha256_file_of_abc_matches_fips_180_4_fixture() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("abc");
    std::fs::write(&path, b"abc").unwrap();
    let digest = sha256_file(&path).unwrap();
    assert_eq!(digest, SHA256_ABC);
}

#[test]
fn sha256_file_streams_through_buffer_boundary() {
    // 1 MiB > the 64 KiB streaming buffer; tests the multi-chunk loop.
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("zeros");
    std::fs::write(&path, vec![0u8; 1024 * 1024]).unwrap();
    let digest = sha256_file(&path).unwrap();
    assert_eq!(digest, SHA256_1MIB_ZEROS);
}

#[test]
fn sha256_file_returns_64_lowercase_hex_chars() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("anything");
    std::fs::write(&path, b"arbitrary content").unwrap();
    let digest = sha256_file(&path).unwrap();
    assert_eq!(digest.len(), 64);
    assert!(
        digest
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
        "digest must be lowercase hex, got {digest}"
    );
}

#[test]
fn sha256_file_errors_on_missing_path() {
    let dir = TempDir::new().unwrap();
    let missing = dir.path().join("does-not-exist");
    let err = sha256_file(&missing).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
}

#[test]
fn sha256_file_errors_on_directory_target() {
    // Opening a directory for reading yields IsADirectory on Linux;
    // assert it surfaces as an error rather than producing a digest.
    let dir = TempDir::new().unwrap();
    let err = sha256_file(dir.path()).unwrap_err();
    // We don't pin the specific ErrorKind because it varies by OS.
    assert!(
        !err.to_string().is_empty(),
        "directory read must surface a non-empty error"
    );
}

#[test]
fn sha256_file_round_trips_through_cas_path() {
    // End-to-end: hash a file, build its CAS path, write to that path,
    // re-hash the destination — digests must match.
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("source.bin");
    let payload = b"verbreel cas round-trip";
    std::fs::write(&src, payload).unwrap();

    let digest = sha256_file(&src).unwrap();
    let dest = cas_path(dir.path(), &digest, "bin").unwrap();
    std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
    std::fs::copy(&src, &dest).unwrap();

    let dest_digest = sha256_file(&dest).unwrap();
    assert_eq!(digest, dest_digest);
}
