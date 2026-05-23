//! Native (POSIX/Windows) [`EventBackend`] impl using `fs4` for cross-process locks.
//!
//! ONLY compiled when the `native` feature is enabled. wasm32 targets opt out
//! of this module entirely.

use crate::backend::{BackendError, EventBackend};
use fs4::{FileExt, TryLockError};
use parking_lot::Mutex;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;

/// Native `EventBackend` — opens `events.jsonl` at the given path with
/// `O_APPEND`, uses `fs4::flock` for cross-process exclusion, and
/// `parking_lot::Mutex` for in-process serialization.
pub struct NativeBackend {
    path: PathBuf,
    file: Mutex<File>,
}

impl NativeBackend {
    /// Open (or create) the events.jsonl at `path`. Acquires an exclusive
    /// `flock` for the lifetime of the backend; released on drop.
    ///
    /// # Errors
    /// [`BackendError::Io`] on open failure; [`BackendError::Locked`] if another
    /// process holds the lock.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, BackendError> {
        let path = path.into();
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(&path)
            .map_err(|e| BackendError::Io(e.to_string()))?;

        FileExt::try_lock(&file).map_err(|e| match e {
            TryLockError::WouldBlock => BackendError::Locked,
            TryLockError::Error(io) => BackendError::Io(io.to_string()),
        })?;

        Ok(Self {
            path,
            file: Mutex::new(file),
        })
    }

    /// Path of the underlying log file.
    #[must_use]
    pub fn path(&self) -> &PathBuf {
        &self.path
    }
}

impl EventBackend for NativeBackend {
    fn append(&self, line: &[u8]) -> Result<(), BackendError> {
        let mut file = self.file.lock();
        file.write_all(line)
            .map_err(|e| BackendError::Io(e.to_string()))?;
        if !line.ends_with(b"\n") {
            file.write_all(b"\n")
                .map_err(|e| BackendError::Io(e.to_string()))?;
        }
        file.sync_data()
            .map_err(|e| BackendError::Io(e.to_string()))?;
        Ok(())
    }

    fn read_all(&self) -> Result<Vec<u8>, BackendError> {
        let mut file = self.file.lock();
        let mut buf = Vec::new();
        file.seek(SeekFrom::Start(0))
            .map_err(|e| BackendError::Io(e.to_string()))?;
        file.read_to_end(&mut buf)
            .map_err(|e| BackendError::Io(e.to_string()))?;
        // Restore append position; O_APPEND already seeks to end on every
        // write, but an explicit seek here keeps `stat()`/`len()` honest.
        file.seek(SeekFrom::End(0))
            .map_err(|e| BackendError::Io(e.to_string()))?;
        Ok(buf)
    }

    fn truncate(&self, offset: u64) -> Result<(), BackendError> {
        let file = self.file.lock();
        file.set_len(offset)
            .map_err(|e| BackendError::Io(e.to_string()))?;
        Ok(())
    }

    fn len(&self) -> Result<u64, BackendError> {
        let file = self.file.lock();
        let meta = file
            .metadata()
            .map_err(|e| BackendError::Io(e.to_string()))?;
        Ok(meta.len())
    }
}

impl Drop for NativeBackend {
    fn drop(&mut self) {
        let file = self.file.lock();
        let _ = FileExt::unlock(&*file);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn append_then_read() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        let be = NativeBackend::open(&path).unwrap();
        be.append(b"first").unwrap();
        be.append(b"second\n").unwrap();
        let all = be.read_all().unwrap();
        assert_eq!(all, b"first\nsecond\n");
        assert_eq!(be.len().unwrap(), 13);
    }

    #[test]
    fn truncate_works() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        let be = NativeBackend::open(&path).unwrap();
        be.append(b"a").unwrap();
        be.append(b"b").unwrap();
        be.append(b"c").unwrap();
        assert_eq!(be.len().unwrap(), 6);
        be.truncate(4).unwrap();
        assert_eq!(be.len().unwrap(), 4);
    }
}
