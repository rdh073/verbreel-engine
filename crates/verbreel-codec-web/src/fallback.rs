//! fMP4/MSE fallback envelope.
//!
//! When [`crate::codec_for_preview`] selects [`PREVIEW_CODEC_MSE`], the
//! browser plays preview through a `MediaSource` + `SourceBuffer`
//! rather than the `WebCodecs` chunk path. The browser needs the MSE
//! MIME type (`video/mp4; codecs="…"`) to construct the `SourceBuffer`,
//! and the segments must be fragmented MP4 (fMP4) so they can be
//! appended incrementally.
//!
//! [`MseFallbackEnvelope`] is the preview-session metadata that carries
//! that MIME string to the client. It serializes (preview-session
//! metadata only — frame bytes never travel in it, per Research 01
//! §6.2), so the handshake can hand the client everything it needs to
//! open the `SourceBuffer` before any media arrives.
//!
//! [`PREVIEW_CODEC_MSE`]: crate::PREVIEW_CODEC_MSE

use serde::{Deserialize, Serialize};

use crate::decoder::WebDecoder;

/// fMP4/MSE source-buffer MIME for an H.264 preview stream.
///
/// `avc1.640028` mirrors the `WebCodecs` H.264 baseline so the fallback
/// decodes the same bitstream the primary path would.
const MSE_MIME_H264: &str = "video/mp4; codecs=\"avc1.640028\"";

/// fMP4/MSE source-buffer MIME for an H.265 preview stream.
///
/// `hvc1.1.6.L93.B0` mirrors the `WebCodecs` H.265 baseline.
const MSE_MIME_H265: &str = "video/mp4; codecs=\"hvc1.1.6.L93.B0\"";

/// Preview-session metadata for the fMP4/MSE fallback transport.
///
/// Holds the `MediaSource` MIME type the client passes to
/// `addSourceBuffer`, plus the fragmentation contract the segments
/// follow. Carries no frame bytes — it is pure session metadata.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MseFallbackEnvelope {
    /// `MediaSource` source-buffer MIME, e.g.
    /// `video/mp4; codecs="avc1.640028"`.
    pub mime_type: String,
    /// True — segments are fragmented MP4 so they can be appended to a
    /// `SourceBuffer` incrementally. Pinned in the envelope so the
    /// client does not have to infer it from the byte stream.
    pub fragmented: bool,
}

impl MseFallbackEnvelope {
    /// Build the fallback envelope for `codec`.
    #[must_use]
    pub fn for_codec(codec: WebDecoder) -> Self {
        let mime_type = match codec {
            WebDecoder::H264 => MSE_MIME_H264,
            WebDecoder::H265 => MSE_MIME_H265,
        };
        Self {
            mime_type: mime_type.to_string(),
            fragmented: true,
        }
    }
}
