//! verbreel-ir — Composition IR, tick-rate math, `cache_hash` derivation.
//!
//! First content slice: composition-cache primitives only.
//!
//! - [`IrNodeId`] — strict `UUIDv7` newtype identifying a composition-graph
//!   node (§0.3 IDs).
//! - [`CacheKey`] — four-field struct + pure-function [`CacheKey::derive`]
//!   that returns a deterministic SHA-256 over a pinned byte layout.
//!
//! Research 01 line 114 calls out `cache_hash` as a pure function of
//! `(node_id, args_hash, upstream_cache_hashes, tick)`. Shipping the
//! primitive now — without committing to the IR Node enum shape — unblocks
//! every future slice that wants to address cache identity without
//! re-litigating the hashing contract.
//!
//! Tick-rate math intentionally NOT redeclared here — it lives in
//! `verbreel_types::tick`.

#![deny(missing_docs)]
#![deny(unsafe_op_in_unsafe_fn)]
#![warn(clippy::pedantic)]

pub mod cache_key;
pub mod node_id;

pub use cache_key::CacheKey;
pub use node_id::IrNodeId;
