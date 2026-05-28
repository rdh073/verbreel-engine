//! RAII exclusive `flock(2)` wrapper.
//!
//! Mirrors the lock pattern used by
//! `verbreel-events::native::NativeBackend::open` —
//! `fs4::FileExt::try_lock` (exclusive) on acquire, `unlock` on drop.
//! The OS releases the lock when the underlying file descriptor is
//! closed; the explicit unlock on drop is for symmetry and to surface
//! lock-release errors via `tracing` rather than ignoring them silently.

use std::fs::File;
use std::io;

use fs4::{FileExt, TryLockError};
use thiserror::Error;

/// Error returned by [`acquire_exclusive`].
#[derive(Debug, Error)]
pub enum FlockError {
    /// Another process (or another file handle in this process) holds
    /// the exclusive lock. Equivalent to `EWOULDBLOCK` from `flock(2)`.
    #[error("file is locked by another process")]
    Contended,

    /// The lock syscall failed for a reason other than contention
    /// (`EINVAL`, `ENOLCK`, …).
    #[error("flock IO error: {0}")]
    Io(#[from] io::Error),
}

/// RAII guard for an exclusive whole-file lock.
///
/// The guard owns the [`File`] it was constructed from. Dropping the
/// guard calls `FileExt::unlock` and then closes the file, which
/// releases the OS-level lock either way. Any error from `unlock` is
/// logged via `tracing::warn!` because Drop cannot return a `Result`.
#[derive(Debug)]
pub struct ExclusiveFlock {
    file: Option<File>,
}

impl ExclusiveFlock {
    /// Borrow the locked file. Useful for the rare caller that wants
    /// to read/write through the same descriptor the lock is bound to.
    #[must_use]
    pub fn file(&self) -> &File {
        // `file` is only `None` between the start and end of `drop`,
        // which is not observable from safe code.
        self.file
            .as_ref()
            .expect("ExclusiveFlock::file called after drop")
    }
}

impl Drop for ExclusiveFlock {
    fn drop(&mut self) {
        if let Some(file) = self.file.take()
            && let Err(e) = FileExt::unlock(&file)
        {
            tracing::warn!(error = %e, "ExclusiveFlock::drop: unlock failed");
        }
    }
}

/// Acquire an exclusive lock on `file`, non-blocking.
///
/// The returned [`ExclusiveFlock`] takes ownership of the file; drop
/// it to release the lock. The lock is also released when the file
/// descriptor is closed, so leaking the guard does not orphan the lock.
///
/// # Errors
///
/// - [`FlockError::Contended`] — another process holds the lock.
/// - [`FlockError::Io`] — the underlying `flock(2)` syscall failed.
pub fn acquire_exclusive(file: File) -> Result<ExclusiveFlock, FlockError> {
    match FileExt::try_lock(&file) {
        Ok(()) => Ok(ExclusiveFlock { file: Some(file) }),
        Err(TryLockError::WouldBlock) => Err(FlockError::Contended),
        Err(TryLockError::Error(io)) => Err(FlockError::Io(io)),
    }
}
