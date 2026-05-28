//! verbreel-ai — ONNX Runtime + Python sidecar dispatch.
//!
//! v0 ships the type surface that the eventual `ort` + sidecar IPC
//! implementation will satisfy. Every provider method returns
//! [`AiError::NotYetImplemented`] until the real AI orchestration
//! lands (Research 04 §3 — engine spawns Python sidecars over
//! JSON-stdio, engine-side ONNX models go through `ort`).
//!
//! ## Why ship types now
//!
//! - Closes the workspace exit-condition counter (was a 1-line stub).
//! - Caption / beat-detection / tracker verbs in `verbreel-state`
//!   already enumerate spec-level capability ids; this crate
//!   commits those ids at the type level so future ai-impl spike
//!   can land by body replacement, not API design.
//! - The opaque-handle pattern mirrors codec-native `Frame`,
//!   codec-web `DecodedFrame`, `verbreel-wasm::EngineHandle`, and
//!   `verbreel-render::Pipeline` — same trait discipline
//!   (no `Clone`/`Default`/`Copy` so future internals can grow
//!   without breaking the public API).
//!
//! ## Crate dependency rule
//!
//! ```text
//! verbreel-ai → verbreel-types, verbreel-state
//! ```

#![deny(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]

pub mod capability;
pub mod error;
pub mod provider;

pub use capability::Capability;
pub use error::AiError;
pub use provider::{Provider, run};
