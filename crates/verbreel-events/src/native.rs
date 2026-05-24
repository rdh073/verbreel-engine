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
    use crate::event::{Event, EventBuilder};
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

    fn write_event(be: &NativeBackend, ev: &Event) {
        let line = serde_json::to_string(ev).unwrap();
        be.append(line.as_bytes()).unwrap();
    }

    fn fresh_event(verb: &str) -> Event {
        Event::new(verb, serde_json::json!({}), json_patch::Patch(Vec::new()))
    }

    #[test]
    fn read_by_id_returns_event_when_present() {
        // Append 3 events; read_by_id on the middle one returns it.
        let dir = tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        let be = NativeBackend::open(&path).unwrap();

        let e1 = fresh_event("clip.add");
        let e2 = fresh_event("clip.remove");
        let e3 = fresh_event("project.set_metadata");
        write_event(&be, &e1);
        write_event(&be, &e2);
        write_event(&be, &e3);

        let got = be.read_by_id(&e2.id).unwrap();
        let Some(got) = got else {
            panic!("expected to find the middle event, got None");
        };
        assert_eq!(got.id, e2.id);
        assert_eq!(got.verb, "clip.remove");
    }

    #[test]
    fn read_by_id_returns_none_when_absent() {
        // A fresh id not in the log produces None.
        let dir = tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        let be = NativeBackend::open(&path).unwrap();

        let e1 = fresh_event("clip.add");
        write_event(&be, &e1);

        let stranger = verbreel_types::EventId::now();
        assert_ne!(stranger, e1.id, "test relies on monotonic UUIDv7");
        assert!(be.read_by_id(&stranger).unwrap().is_none());
    }

    #[test]
    fn read_by_id_empty_log_returns_none() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        let be = NativeBackend::open(&path).unwrap();

        let probe = verbreel_types::EventId::now();
        assert!(be.read_by_id(&probe).unwrap().is_none());
    }

    #[test]
    fn read_by_id_torn_line_returns_error() {
        // Append a valid line, then append garbage (no trailing newline,
        // invalid JSON). read_by_id walks line-by-line — the garbage at
        // the tail must surface as BackendError::TornLine.
        let dir = tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        let be = NativeBackend::open(&path).unwrap();

        let e1 = fresh_event("clip.add");
        write_event(&be, &e1);

        // Append corrupt bytes without a trailing newline. NativeBackend
        // adds one on b"...\n"-terminated lines only if the caller didn't
        // — pass garbage with no terminator, which the backend will then
        // \n-terminate. Make sure the result is invalid JSON regardless.
        be.append(b"this is not json{").unwrap();

        // Search for some EventId that is NOT e1.id so we have to walk
        // past the torn line.
        let stranger = verbreel_types::EventId::now();
        assert_ne!(stranger, e1.id);
        let err = be.read_by_id(&stranger).expect_err("torn line must error");
        assert!(
            matches!(err, BackendError::TornLine { .. }),
            "expected TornLine, got {err:?}"
        );
    }

    #[test]
    fn eventbuilder_warnings_field_persists_to_disk() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        let backend = NativeBackend::open(&path).unwrap();
        let warnings = vec![serde_json::json!({
            "code": "W_TEST",
            "message": "unit test warning",
        })];
        let event = EventBuilder::new()
            .verb("project.set_metadata")
            .args(serde_json::json!({}))
            .patch(json_patch::Patch(Vec::new()))
            .warnings(warnings.clone())
            .build();
        let line = serde_json::to_string(&event).unwrap();
        backend.append(line.as_bytes()).unwrap();

        let read_back = backend.read_by_id(&event.id).unwrap().unwrap();
        assert_eq!(read_back.warnings, warnings);
        assert_eq!(read_back.verb, event.verb);
    }
}
