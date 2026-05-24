//! verbreel-events — events.jsonl writer/reader + idempotency index.
//!
//! This crate is the §0.8 / Appendix C implementation: append-only event log
//! with write-ordering invariants. The trait [`EventBackend`] abstracts over
//! the storage layer so the same engine can run on native (file + flock) and
//! wasm32 (OPFS `SyncAccessHandle` + `navigator.locks` — Phase 2 work).
//!
//! Spec references:
//! - §0.8 write ordering, idempotency, fsync, torn-line recovery → [`EventBackend`]
//! - Appendix C events.jsonl line shape → [`event`] module
//! - §0.14 dense normalization → consumer responsibility (state crate)

#![deny(missing_docs)]
#![deny(unsafe_op_in_unsafe_fn)]
#![warn(clippy::pedantic)]

pub mod backend;
pub mod event;

#[cfg(feature = "native")]
pub mod native;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub mod wasm;

pub use backend::{BackendError, EventBackend};
pub use event::{Event, EventBuilder, EventLine, timestamp_rfc3339_now};

#[cfg(feature = "native")]
pub use native::NativeBackend;
