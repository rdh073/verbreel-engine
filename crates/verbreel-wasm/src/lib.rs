//! verbreel-wasm — browser preview engine entry point.
//!
//! The exported embedding scope is [`EmbeddingScope::PreviewOnly`]
//! (decision #404): browser WASM owns preview playback/scrubbing, while
//! editor mutation and export remain on the native/HTTP/MCP surfaces.
//! The crate bridges a [`session::PreviewSession`] to
//! `verbreel-codec-web`'s decode loop — [`EngineHandle::open_preview_session`]
//! opens a session, then `seek` / `frame_at` drive decode (Research 01
//! §0 "web preview build" rationale, §6.2 preview transport).
//!
//! ## Surface
//!
//! - [`EngineHandle`] — `Send + Sync` lifecycle/factory handle; mints
//!   preview sessions but holds no decoder itself.
//! - [`session::PreviewSession`] — owns the (`!Send`) codec-web decoder
//!   on wasm32; `open` / `seek` / `frame_at`.
//! - [`session::PreviewFrame`] — one decoded frame handle (dimensions,
//!   pts, plane bytes) exposed to JS.
//! - [`diagnostics::init`] — opt-in `console.error` panic hook + tracing
//!   bridge; no telemetry.
//! - The `frame::run_frame` wgpu composite path is still deferred to
//!   Spike S2 ([`WasmError::NotYetImplemented`]).
//!
//! Mutation/persistence is deliberately absent from the browser handle.
//! Any future in-memory browser preview path must surface
//! [`WasmError::BrowserNoPersistence`].
//!
//! ## Crate dependency rule
//!
//! ```text
//! verbreel-wasm → verbreel-state (no-default-features), verbreel-ir,
//!                 verbreel-codec-web
//! ```
//!
//! `verbreel-state`'s `native` feature gates the fs4-backed event
//! backend which is wasm32-incompatible, hence `default-features =
//! false`. codec-web's browser deps live under its own wasm32
//! target-cfg block, so the native `cargo check --workspace` build pulls
//! no browser toolchain through this crate. Frame bytes stream through
//! [`session::PreviewSession::frame_at`] and never enter the event log.

#![deny(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]

pub mod diagnostics;
pub mod engine;
pub mod error;
pub mod frame;
pub mod scope;
pub mod session;

pub use diagnostics::init as init_diagnostics;
pub use engine::EngineHandle;
pub use error::{W_BROWSER_NO_PERSISTENCE, WasmError};
pub use frame::run_frame;
pub use scope::{
    EMBEDDING_SCOPE_WIRE, EmbeddingScope, embedding_scope_wire_export,
    supports_editor_embedding_export, supports_preview_embedding_export,
};
pub use session::{PreviewFrame, PreviewSession, tick_to_micros};
