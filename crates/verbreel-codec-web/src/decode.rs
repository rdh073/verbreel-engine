//! [`decode`] — v0 entry point.
//!
//! The body returns [`DecodeError::NotYetImplemented`] in v0. The real
//! `WebCodecs` pipeline lands with Spike S2 per Research 01 §11:
//! `web_sys::VideoDecoder` → `VideoFrame` → wgpu zero-copy upload →
//! shared-WGSL composite (pixel diff ≤1/255 per channel vs the native
//! codec path).
//!
//! Surface shape is stable now so downstream crates
//! (`verbreel-render`, the preview-session verbs in `verbreel-state`)
//! can wire against this signature without waiting for S2 to land.

use crate::decoder::WebDecoder;
use crate::error::DecodeError;
use crate::frame::DecodedFrame;

/// Spike S2 deferral marker — surfaced verbatim in
/// [`DecodeError::NotYetImplemented::detail`] so callers (and the
/// orchestrator) can grep for the exact string. Tests pin this so the
/// detail cannot drift without a corresponding test update.
pub(crate) const SPIKE_S2_DEFERRAL_DETAIL: &str =
    "WebCodecs decode path lands with Spike S2 (Research 01 §11)";

/// Decode a codec-framed bitstream into a sequence of frames.
///
/// # Errors
///
/// v0 stub always returns [`DecodeError::NotYetImplemented`] with
/// [`SPIKE_S2_DEFERRAL_DETAIL`] as the `detail`. Real error variants
/// ([`DecodeError::UnsupportedCodec`], [`DecodeError::MalformedBitstream`],
/// [`DecodeError::DecoderInternal`]) come online when Spike S2 lands
/// the real `web_sys::VideoDecoder` pipeline.
pub fn decode(_codec: WebDecoder, _bitstream: &[u8]) -> Result<Vec<DecodedFrame>, DecodeError> {
    Err(DecodeError::NotYetImplemented {
        detail: SPIKE_S2_DEFERRAL_DETAIL.to_string(),
    })
}
