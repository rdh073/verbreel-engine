//! [`EventBackend`] trait — storage abstraction for the append-only event log.
//!
//! Per spec §0.8 write-ordering: a mutating verb's lifecycle is
//!   (1) compute patch + validate without mutating in-memory state
//!   (2) write event line + `fsync()`
//!   (3) apply patch to in-memory state
//!
//! Step (2) is the backend's job. The trait surface is intentionally minimal
//! (append, flush, truncate, read) — concurrency control (per-project write
//! mutex, cross-process flock) is concrete-backend territory, not part of the
//! trait surface.

use thiserror::Error;
use verbreel_types::EventId;

use crate::event::Event;

/// Errors returned by backend operations.
#[derive(Debug, Error)]
pub enum BackendError {
    /// Underlying IO failure (file system, OPFS).
    #[error("backend IO failure: {0}")]
    Io(String),

    /// The backend's lock is held by another process / tab.
    #[error("backend locked by another writer")]
    Locked,

    /// A line being read could not be parsed as JSON.
    #[error("torn line at offset {offset}: {detail}")]
    TornLine {
        /// Byte offset where parsing failed.
        offset: u64,
        /// Free-form detail.
        detail: String,
    },

    /// Operation not supported by this backend (e.g. wasm stub).
    #[error("not implemented: {0}")]
    NotImplemented(&'static str),
}

/// Append-only event log backend. Concrete impls live in [`crate::native`]
/// (filesystem + flock, default-feature) and `crate::wasm` (OPFS, Phase 2).
///
/// Implementors MUST honor the §0.8 write-ordering invariant: `append` returns
/// `Ok` only after the bytes are durably persisted (i.e. `fsync` returned
/// success on POSIX, `flush()` returned on OPFS). Callers depend on this for
/// crash-safety guarantees.
pub trait EventBackend: Send + Sync {
    /// Append a serialized event line. The implementor adds the trailing `\n`
    /// if not present and ensures bytes are durable before returning.
    ///
    /// # Errors
    /// Returns [`BackendError::Io`] on IO failure, [`BackendError::Locked`]
    /// if cross-process coordination prevents the write.
    fn append(&self, line: &[u8]) -> Result<(), BackendError>;

    /// Read the full log into memory. Used by `project.open` to rebuild the
    /// idempotency index. Caller MUST be prepared to handle a torn final line
    /// (returned as [`BackendError::TornLine`] with the byte offset).
    ///
    /// # Errors
    /// Same as [`Self::append`], plus [`BackendError::TornLine`] for parse failures.
    fn read_all(&self) -> Result<Vec<u8>, BackendError>;

    /// Truncate the log at the given byte offset. Used for §0.8 torn-line
    /// recovery: on `project.open`, if the last line failed to parse, the
    /// engine truncates back to the last well-formed `\n`.
    ///
    /// # Errors
    /// Returns [`BackendError::Io`] on IO failure.
    fn truncate(&self, offset: u64) -> Result<(), BackendError>;

    /// Current log length in bytes. Cheap operation; backends should not
    /// re-read the full file.
    ///
    /// # Errors
    /// Returns [`BackendError::Io`] on IO failure.
    fn len(&self) -> Result<u64, BackendError>;

    /// True if the log is empty (length 0). Default impl computes from [`Self::len`].
    ///
    /// # Errors
    /// Same as [`Self::len`].
    fn is_empty(&self) -> Result<bool, BackendError> {
        Ok(self.len()? == 0)
    }

    /// Read a single event by id. Walks [`Self::read_all`] and returns the
    /// first event whose `id` matches. Returns `Ok(None)` if the id is not
    /// present.
    ///
    /// This is a convenience for verb-layer replay where the caller knows the
    /// event id (from an idempotency-index lookup) and needs the recorded
    /// 5-tuple `(args, patch, warnings, post_state)` to reconstruct the
    /// envelope `data` per §0.8.
    ///
    /// Default impl walks [`Self::read_all`] + parses lines + returns the
    /// first match. Concrete backends MAY override with a more efficient
    /// implementation (e.g. an in-memory index by id) but are not required to.
    ///
    /// # Errors
    /// - [`BackendError::Io`] on read failure.
    /// - [`BackendError::TornLine`] if a line in the log fails to parse —
    ///   the caller is expected to run torn-line recovery the same way
    ///   `project.open` does.
    fn read_by_id(&self, id: &EventId) -> Result<Option<Event>, BackendError> {
        let bytes = self.read_all()?;
        for (offset, line) in line_offsets(&bytes) {
            if line.is_empty() {
                continue;
            }
            let event: Event =
                serde_json::from_slice(line).map_err(|e| BackendError::TornLine {
                    offset,
                    detail: e.to_string(),
                })?;
            if &event.id == id {
                return Ok(Some(event));
            }
        }
        Ok(None)
    }
}

/// Yields `(byte_offset, line_without_trailing_newline)` for each `\n`-
/// terminated line in `bytes`. The final segment is yielded too if it has
/// no terminator — callers decide whether to treat that as a torn line.
fn line_offsets(bytes: &[u8]) -> impl Iterator<Item = (u64, &[u8])> {
    let mut pos: usize = 0;
    let total = bytes.len();
    std::iter::from_fn(move || {
        if pos >= total {
            return None;
        }
        let rest = &bytes[pos..];
        let line_offset = pos as u64;
        if let Some(rel) = rest.iter().position(|&b| b == b'\n') {
            let line = &bytes[pos..pos + rel];
            pos += rel + 1;
            Some((line_offset, line))
        } else {
            // Untruncated tail (no \n). Hand it to the caller — the
            // default `read_by_id` body decides what to do with it
            // (today: attempt parse, surface TornLine on failure).
            let line = &bytes[pos..];
            pos = total;
            Some((line_offset, line))
        }
    })
}
