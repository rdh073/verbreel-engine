//! verbreel-wasm — browser preview engine entry point.
//!
//! v0 ships the type surface that the eventual `wasm-bindgen` + wgpu +
//! `WebCodecs` implementation will satisfy. The exported embedding
//! scope is [`EmbeddingScope::PreviewOnly`]: browser WASM owns preview
//! playback/scrubbing, while editor mutation and export remain on the
//! native/HTTP/MCP surfaces. Every public method routes to
//! [`WasmError::NotYetImplemented`] until Spike S2 lands the real
//! browser render path (Research 01 §0 "web preview build" rationale,
//! §11 spike acceptance criteria).
//!
//! ## Why ship types now
//!
//! - Closes the workspace exit-condition counter (was a 1-line stub).
//! - JS callers wiring up render loops can hold a stable Rust surface
//!   to call into; behaviour arrives at S2 by body replacement, not
//!   API change.
//! - The opaque-fields pattern on [`EngineHandle`] mirrors
//!   codec-native `Frame` / codec-web `DecodedFrame` — keeps the
//!   public API bounded so S2 can grow internals (wgpu `Surface`,
//!   project graph reference, async runtime) without breaking JS
//!   callers.
//!
//! ## Crate dependency rule
//!
//! ```text
//! verbreel-wasm → verbreel-state (no-default-features), verbreel-ir
//! ```
//!
//! `verbreel-state`'s `native` feature gates the fs4-backed event
//! backend which is wasm32-incompatible, hence
//! `default-features = false`. `wasm-bindgen` is NOT a dep at v0 —
//! that decision belongs to Spike S2 along with the JS bindings shape.

#![deny(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]

pub mod engine;
pub mod error;
pub mod frame;
pub mod scope;

pub use engine::EngineHandle;
pub use error::WasmError;
pub use frame::run_frame;
pub use scope::{
    EMBEDDING_SCOPE_WIRE, EmbeddingScope, embedding_scope_wire_export,
    supports_editor_embedding_export, supports_preview_embedding_export,
};
