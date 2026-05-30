//! [`CodecError`] — v0 error surface for the encode/decode/probe paths.
//!
//! Variants:
//!
//! - [`CodecError::NotYetImplemented`] — historical placeholder, retained
//!   only for enum API stability; no live call site emits it.
//! - [`CodecError::FeatureDisabled`] — the entry point needs the
//!   `rsmpeg` cargo feature (system `FFmpeg`) and it was compiled off.
//!   This is the feature-off return for `encode`; `decode`/`probe` are
//!   gated out as a whole module feature-off, so they never reach a
//!   runtime return.
//! - [`CodecError::InvalidParams`] — param-shape failures caught
//!   before any encoder work runs.
//! - [`CodecError::InputExhausted`] — caller-driven decode loop
//!   reached EOF; structural signal, not a failure.
//! - [`CodecError::BackendInternal`] — rsmpeg / libav errors mapped
//!   to a string body, from either the decode/probe or encode half.

use thiserror::Error;

/// Errors returned by [`encode`](crate::encode()) and its sibling
/// codec entry points.
#[derive(Debug, Error)]
pub enum CodecError {
    /// The codec backend body has not been implemented yet. No live call
    /// site emits this variant now that the rsmpeg+libx264 path is wired;
    /// it is retained only for enum API stability.
    #[error("codec backend not yet implemented: {detail}")]
    NotYetImplemented {
        /// Free-form context — typically the Spike S1 citation.
        detail: String,
    },

    /// The requested entry point needs the `rsmpeg` cargo feature
    /// (which links system `FFmpeg` 6.x) and the crate was compiled
    /// without it. This is the deterministic feature-off return for
    /// [`encode`](crate::encode()); `decode`/`probe` live in a module
    /// gated out feature-off, so they fail at compile time rather than
    /// returning this. CI builds the crate feature-off and never links
    /// `FFmpeg`, so `encode` fails closed with a clear signal rather
    /// than a link error.
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
    /// any further, from EITHER codec half — the decode/probe path
    /// (open input, find best stream, receive frame, sws-null, scale)
    /// or the encode path (open encoder, write header, send frame,
    /// mux). The body names which operation failed so the catch-all
    /// does not hide which half broke. v0 body is a free-form string;
    /// Spike S1 will pin a stricter shape once the real libav error
    /// codes are known.
    #[error("codec backend error: {detail}")]
    BackendInternal {
        /// Free-form context — typically the libav error string,
        /// prefixed with the failing operation (e.g. `open input: ...`).
        detail: String,
    },
}
