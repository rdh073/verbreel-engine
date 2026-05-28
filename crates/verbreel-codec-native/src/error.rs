//! [`CodecError`] — v0 error surface for the encode path.
//!
//! Four variants in v0:
//!
//! - [`CodecError::NotYetImplemented`] — temporary placeholder while
//!   the rsmpeg+libx264 body is deferred to Spike S1.
//! - [`CodecError::InvalidParams`] — param-shape failures caught
//!   before any encoder work runs.
//! - [`CodecError::InputExhausted`] — caller-driven decode loop
//!   reached EOF; structural signal, not a failure.
//! - [`CodecError::EncoderInternal`] — rsmpeg / libav errors mapped
//!   to a string body. Body lands with Spike S1.

use thiserror::Error;

/// Errors returned by [`crate::encode`] and its eventual sibling
/// codec entry points.
#[derive(Debug, Error)]
pub enum CodecError {
    /// The codec backend body has not been implemented yet. v0 stub
    /// returns this from every call until Spike S1 lands the real
    /// rsmpeg+libx264 path per Research 01 §11.
    #[error("codec backend not yet implemented: {detail}")]
    NotYetImplemented {
        /// Free-form context — typically the Spike S1 citation.
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
