//! [`CodecError`] — v0 error surface for the encode/decode/probe paths.
//!
//! Variants:
//!
//! - [`CodecError::NotYetImplemented`] — historical placeholder kept for
//!   callers that still grep for it; no live call site emits it now that
//!   the rsmpeg path is wired.
//! - [`CodecError::FeatureDisabled`] — the entry point needs the
//!   `rsmpeg` cargo feature (system `FFmpeg`) and it was compiled off.
//!   This is the feature-off return for every decode/encode/probe call.
//! - [`CodecError::InvalidParams`] — param-shape failures caught
//!   before any encoder work runs.
//! - [`CodecError::InputExhausted`] — caller-driven decode loop
//!   reached EOF; structural signal, not a failure.
//! - [`CodecError::EncoderInternal`] — rsmpeg / libav errors mapped
//!   to a string body.

use thiserror::Error;

/// Errors returned by [`encode`](crate::encode()) and its sibling
/// codec entry points.
#[derive(Debug, Error)]
pub enum CodecError {
    /// The codec backend body has not been implemented yet. Retained
    /// for callers that still grep the v0 string; the rsmpeg+libx264
    /// path is now wired, so no live call site emits this variant.
    #[error("codec backend not yet implemented: {detail}")]
    NotYetImplemented {
        /// Free-form context — typically the Spike S1 citation.
        detail: String,
    },

    /// The requested entry point needs the `rsmpeg` cargo feature
    /// (which links system `FFmpeg` 6.x) and the crate was compiled
    /// without it. This is the deterministic feature-off return for
    /// every decode / encode / probe call — CI builds the crate
    /// feature-off and never links `FFmpeg`, so these calls fail closed
    /// with a clear signal rather than a link error.
    #[error("codec backend requires the `rsmpeg` feature: {detail}")]
    FeatureDisabled {
        /// Free-form context — typically the entry-point name.
        detail: String,
    },

    /// Caller passed parameters the encoder cannot satisfy
    /// (e.g. odd width on the H.264 yuv420p path, zero `fps_den`,
    /// zero-frame input). Caught pre-encode; no encoder work runs.
    #[error("invalid encode params: {detail}")]
    InvalidParams {
        /// Free-form context — typically the offending field name.
        detail: String,
    },

    /// Caller-driven decode loop hit EOF. Structural signal that the
    /// pull-based caller should stop, not a failure to surface.
    #[error("input exhausted")]
    InputExhausted,

    /// rsmpeg / libav surfaced an error the wrapper cannot classify
    /// any further. v0 body is a free-form string; Spike S1 will
    /// pin a stricter shape once the real libav error codes are
    /// known.
    #[error("encoder internal error: {detail}")]
    EncoderInternal {
        /// Free-form context — typically the libav error string.
        detail: String,
    },
}
