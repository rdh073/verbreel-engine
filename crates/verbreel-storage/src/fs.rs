//! Atomic file writes.
//!
//! [`atomic_write_bytes`] mirrors the inline code in
//! `verbreel-state::lifecycle::save()` (~lines 975–985):
//!
//! 1. `NamedTempFile::new_in(parent_of(target))` — same filesystem
//!    so the eventual rename is atomic per POSIX semantics.
//! 2. Write the bytes.
//! 3. `sync_data()` on the temp file — content is on disk.
//! 4. `persist(target)` — rename(2) replaces any existing file at
//!    the destination atomically.
//! 5. `sync_data()` on the parent directory — the rename itself
//!    is durable across a crash.

use std::fs::File;
use std::io::{self, Write};
use std::path::Path;

use tempfile::NamedTempFile;

/// Atomically write `contents` to `path`.
///
/// The write goes through a `NamedTempFile` in the parent directory of
/// `path` so the final `rename(2)` lands on the same filesystem. After
/// the rename the parent directory is fsynced so the rename itself
/// survives a power loss.
///
/// If `path` already exists it is replaced atomically — readers see
/// either the old bytes or the new bytes, never a half-written file.
///
/// # Errors
///
/// Returns [`io::Error`] if any IO step fails:
///
/// - The parent directory cannot be resolved (path has no parent),
///   surfaced as [`io::ErrorKind::InvalidInput`].
/// - Creating the temp file, writing, syncing, or persisting fails.
/// - Opening the parent directory for fsync fails.
pub fn atomic_write_bytes(path: &Path, contents: &[u8]) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("atomic_write_bytes: path has no parent: {}", path.display()),
        )
    })?;

    let mut tmp = NamedTempFile::new_in(parent)?;
    tmp.as_file_mut().write_all(contents)?;
    tmp.as_file_mut().sync_data()?;
    tmp.persist(path).map_err(|e| e.error)?;

    // Parent-dir fsync — POSIX guarantee that the rename is durable.
    // An empty parent ("") means current-directory; resolve it so
    // File::open does not fail with ENOENT.
    let dir_to_fsync: &Path = if parent.as_os_str().is_empty() {
        Path::new(".")
    } else {
        parent
    };
    let dir = File::open(dir_to_fsync)?;
    dir.sync_data()?;

    Ok(())
}
