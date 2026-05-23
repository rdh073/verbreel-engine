//! wasm32 [`crate::EventBackend`] impl — placeholder.
//!
//! Phase 2 will populate this module with an OPFS-backed impl using
//! `SyncAccessHandle` for sync read/write (only available in dedicated Web
//! Workers) and `navigator.locks` for cross-tab serialization. See
//! `spec/research/05-storage-state.md` §7.2.1.
//!
//! The trait + event types are always available via the crate root, so
//! consumers can write generic code against [`crate::EventBackend`] today;
//! the concrete wasm impl lands when the Phase 2 follow-up issue is closed.

// Module intentionally empty in v0.x. Phase 2 will populate:
//   pub struct OpfsBackend { ... }
//   impl EventBackend for OpfsBackend { ... }
