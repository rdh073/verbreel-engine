//! [`DecodeError`] — v0 error surface for the `WebCodecs` decode path.
//!
//! Four variants in v0:
//!
//! - [`DecodeError::NotYetImplemented`] — temporary placeholder while
//!   the `web_sys::VideoDecoder` body is deferred to Spike S2.
//! - [`DecodeError::UnsupportedCodec`] — caller asked for a codec that
//!   is not in [`crate::WebDecoder`].
//! - [`DecodeError::MalformedBitstream`] — input bytes do not satisfy
//!   the codec's start-code / NAL framing.
//! - [`DecodeError::DecoderInternal`] — `WebCodecs` / browser errors
//!   mapped to a string body. Body lands with Spike S2.

use thiserror::Error;

/// Errors returned by [`crate::decode`] and its eventual sibling
/// decoder entry points.
#[derive(Debug, Error)]
pub enum DecodeError {
    /// The decoder backend body has not been implemented yet. v0 stub
    /// returns this from every call until Spike S2 lands the real
    /// `web_sys::VideoDecoder` path per Research 01 §11.
    #[error("decoder backend not yet implemented: {detail}")]
    NotYetImplemented {
        /// Free-form context — typically the Spike S2 citation.
        detail: String,
    },

    /// Caller asked for a codec the `WebCodecs` shim does not know
    /// about. The `requested` field carries the offending token so the
    /// caller can decide whether to fall back to a software decode
    /// path.
    #[error("unsupported codec: {requested}")]
    UnsupportedCodec {
        /// The codec token the caller requested
        /// (e.g. `"av1"`, `"vp9"`).
        requested: String,
    },

    /// Input bytes do not satisfy the selected codec's start-code /
    /// NAL framing. Caught pre-decode; no decoder work runs.
    #[error("malformed bitstream: {detail}")]
    MalformedBitstream {
        /// Free-form context — typically which framing rule failed.
        detail: String,
    },

    /// `WebCodecs` surfaced an error the wrapper cannot classify any
    /// further. v0 body is a free-form string; Spike S2 will pin a
    /// stricter shape once the real browser `VideoDecoderError` codes
    /// are known across the playwright matrix.
    #[error("decoder internal error: {detail}")]
    DecoderInternal {
        /// Free-form context — typically the browser error string.
        detail: String,
    },
}
