//! Content-addressed storage primitives.
//!
//! `verbreel` stores binary assets at the spec §0.13 path shape:
//!
//! ```text
//! <root>/assets/<aa>/<sha256>.<ext>
//! ```
//!
//! where `<aa>` is the first 2 hex characters of the full 64-character
//! lowercase SHA-256 hex digest of the file bytes. The two-character
//! shard avoids putting tens of thousands of files in a single
//! directory.

use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use thiserror::Error;

/// 64 KiB streaming buffer for SHA-256 hashing. Larger buffers do not
/// measurably improve throughput on modern kernels; smaller buffers
/// waste syscalls.
const HASH_BUF_SIZE: usize = 64 * 1024;

/// Error returned by [`cas_path`] when the input hex string fails the
/// §0.13 shape check.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CasError {
    /// `sha256_hex` was not exactly 64 characters long.
    #[error("sha256_hex must be 64 chars, got {0}")]
    BadHexLength(usize),

    /// `sha256_hex` contained a non-lowercase-hex character. The bad
    /// character is included for diagnostics.
    #[error("sha256_hex must be lowercase hex; bad char {0:?}")]
    NotLowercaseHex(char),
}

/// Build the spec §0.13 content-addressed asset path
/// `<root>/assets/<aa>/<sha>.<ext>`.
///
/// `sha256_hex` is validated to be 64 lowercase hex characters; an
/// invalid hex string returns [`CasError`] rather than producing a
/// garbage path. `ext` is appended verbatim — callers are expected to
/// pass the canonical extension for the asset family (no leading dot).
///
/// # Errors
///
/// - [`CasError::BadHexLength`] — `sha256_hex.len() != 64`.
/// - [`CasError::NotLowercaseHex`] — at least one character is outside
///   `[0-9a-f]`.
pub fn cas_path(root: &Path, sha256_hex: &str, ext: &str) -> Result<PathBuf, CasError> {
    if sha256_hex.len() != 64 {
        return Err(CasError::BadHexLength(sha256_hex.len()));
    }
    if let Some(bad) = sha256_hex
        .chars()
        .find(|c| !c.is_ascii_digit() && !matches!(c, 'a'..='f'))
    {
        return Err(CasError::NotLowercaseHex(bad));
    }

    let shard = &sha256_hex[..2];
    Ok(root
        .join("assets")
        .join(shard)
        .join(format!("{sha}.{ext}", sha = sha256_hex, ext = ext)))
}

/// Stream the file at `path` through SHA-256 and return the digest as
/// 64 lowercase hex characters.
///
/// Reads in 64 KiB chunks; suitable for files of any size — memory use
/// is `O(1)` in the file size.
///
/// # Errors
///
/// Returns [`io::Error`] if opening or reading the file fails.
pub fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; HASH_BUF_SIZE];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex_lower(&hasher.finalize()))
}

/// Local lowercase-hex encoder. Avoids pulling in `hex` just for this.
fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}
