//! Preview-session handshake: capability report → chosen transport.
//!
//! `preview.session` opens with the client reporting its decode
//! capabilities. The server resolves those into a transport decision
//! via [`crate::codec_for_preview`] (policy #405 option A: `webcodecs`
//! when present, else `mse` including Safari) and replies with a
//! [`PreviewSessionPlan`] — the chosen wire literal plus the
//! transport-specific session metadata the client needs to start
//! playback.
//!
//! This module is the composition point that turns the pure policy
//! decision into a serializable session reply. It carries no frame
//! bytes (Research 01 §6.2 — only session metadata serializes).

use serde::{Deserialize, Serialize};

use crate::decoder::WebDecoder;
use crate::fallback::MseFallbackEnvelope;
use crate::preview_codec::{PreviewClientCapabilities, WebPreviewCodec, codec_for_preview};

/// Transport-specific session metadata returned to the client.
///
/// The `WebCodecs` path needs only the chosen codec literal (the client
/// already has `VideoDecoder`); the MSE path additionally carries the
/// [`MseFallbackEnvelope`] so the client can open its `SourceBuffer`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "transport", rename_all = "lowercase")]
pub enum PreviewSessionTransport {
    /// `WebCodecs` chunk transport.
    WebCodecs,
    /// fMP4/MSE fallback transport, carrying the source-buffer MIME.
    Mse {
        /// The fMP4/MSE envelope the client opens its `SourceBuffer`
        /// with.
        envelope: MseFallbackEnvelope,
    },
}

/// The server's reply to a `preview.session` capability report.
///
/// `codec` is the canonical wire literal (`"webcodecs"` or `"mse"`) the
/// client echoes back; `transport` carries any extra metadata the
/// chosen path needs.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PreviewSessionPlan {
    /// Selected preview transport.
    pub codec: WebPreviewCodec,
    /// Transport-specific session metadata.
    pub transport: PreviewSessionTransport,
}

impl PreviewSessionPlan {
    /// Resolve a client capability report into a session plan for the
    /// preview stream's `codec`.
    ///
    /// Applies policy #405 option A through
    /// [`codec_for_preview`]: `webcodecs` when the client reports
    /// `WebCodecs` decode, otherwise the `mse` fallback (including
    /// Safari).
    #[must_use]
    pub fn resolve(caps: PreviewClientCapabilities, codec: WebDecoder) -> Self {
        match codec_for_preview(caps) {
            WebPreviewCodec::WebCodecs => Self {
                codec: WebPreviewCodec::WebCodecs,
                transport: PreviewSessionTransport::WebCodecs,
            },
            WebPreviewCodec::MseFmp4 => Self {
                codec: WebPreviewCodec::MseFmp4,
                transport: PreviewSessionTransport::Mse {
                    envelope: MseFallbackEnvelope::for_codec(codec),
                },
            },
        }
    }

    /// The chosen transport's canonical wire literal (`"webcodecs"` or
    /// `"mse"`).
    #[must_use]
    pub const fn codec_literal(&self) -> &'static str {
        self.codec.wire_literal()
    }
}
