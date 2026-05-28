//! Error type for the AI orchestration layer.

use thiserror::Error;

/// Errors surfaced by the AI provider entry points.
///
/// v0 only produces [`Self::NotYetImplemented`]; the other three
/// variants are committed now so downstream callers can pattern-
/// match against the eventual real-implementation surface without
/// a breaking add.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum AiError {
    /// Method body deferred to the AI orchestration spike (Research
    /// 04 §3).
    #[error("verbreel-ai method not yet implemented: {detail}")]
    NotYetImplemented {
        /// Pointer to where the real implementation lands.
        detail: String,
    },

    /// Engine failed to spawn the Python sidecar process (binary
    /// missing, exec permission denied, OOM). Reserved at v0;
    /// produced by the sidecar-bound paths.
    #[error("ai sidecar launch failed: {detail}")]
    SidecarLaunchFailed {
        /// Underlying spawn error message.
        detail: String,
    },

    /// `ort` failed to load the ONNX model weights (missing file,
    /// version mismatch, hardware-accelerator unavailable).
    #[error("ai model load failed: {detail}")]
    ModelLoadFailed {
        /// `ort` diagnostic message.
        detail: String,
    },

    /// The Python sidecar exited before producing a complete JSON
    /// response, or sent malformed JSON. Distinct from
    /// `SidecarLaunchFailed` so callers can distinguish "binary
    /// missing" from "binary crashed mid-stream".
    #[error("ai sidecar process exited prematurely: {detail}")]
    ProcessExited {
        /// Exit code + stderr tail if available.
        detail: String,
    },
}
