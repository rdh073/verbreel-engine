//! verbreel-canon — RFC 8785 JSON Canonicalization Scheme + `project_hash`.
//!
//! This crate is the single source of canonical-JSON serialization for the engine.
//! Every callsite that needs a `project_hash` (§12.1), an idempotency-key
//! fingerprint (§0.8), or a cache-key filename (§14.1 / §14.2) MUST route
//! through this crate. Hand-rolled `serde_json::to_string` does NOT produce
//! RFC 8785 output (see spec §0.5.2 "Migration note").
//!
//! Spec references:
//! - §0.5.2 canonical JSON — [`canonicalize`], [`sha256_hex`]
//! - §0.5.2 `project_hash` field projection — [`project_hash`]
//! - §0.13 engine invariants — [`assert_rfc8785_conformance`]

#![deny(missing_docs)]
#![deny(unsafe_op_in_unsafe_fn)]
#![warn(clippy::pedantic)]

pub mod conformance;
pub mod jcs;
pub mod project;

pub use conformance::{ConformanceError, assert_rfc8785_conformance};
pub use jcs::{CanonError, canonicalize, sha256_hex};
pub use project::project_hash;
