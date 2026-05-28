//! [`WebPreviewCodec`] and capability mapping for `preview.session`.
//!
//! Research 03 §4 fixes the v1 policy: browser preview prefers
//! `webcodecs`, and falls back to fMP4/MSE when `WebCodecs` decode is
//! unavailable. This module pins that decision in typed API surface so
//! integration layers do not infer policy from prose.

use serde::{Deserialize, Serialize};

/// Canonical `preview_session_formats` token for the `WebCodecs` path.
pub const PREVIEW_CODEC_WEBCODECS: &str = "webcodecs";

/// Canonical `preview_session_formats` token for the fMP4/MSE fallback path.
pub const PREVIEW_CODEC_MSE: &str = "mse";

/// Browser family relevant to preview transport selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BrowserFamily {
    /// Apple Safari / `WebKit` family.
    Safari,
    /// Any non-Safari browser family.
    Other,
}

/// Client decode capability input used to choose preview transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PreviewClientCapabilities {
    /// Browser family (Safari vs non-Safari).
    pub browser_family: BrowserFamily,
    /// True when `VideoDecoder` / `WebCodecs` decode support is available.
    pub has_webcodecs_decode: bool,
}

/// Preview transport selected for browser playback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WebPreviewCodec {
    /// `WebCodecs` chunk path (`"webcodecs"`).
    #[serde(rename = "webcodecs")]
    WebCodecs,
    /// fMP4/MSE fallback path (`"mse"`).
    #[serde(rename = "mse")]
    MseFmp4,
}

impl WebPreviewCodec {
    /// Constructor for the primary `WebCodecs` path.
    #[must_use]
    pub const fn webcodecs() -> Self {
        Self::WebCodecs
    }

    /// Constructor for the fallback fMP4/MSE path.
    #[must_use]
    pub const fn mse_fmp4() -> Self {
        Self::MseFmp4
    }

    /// Canonical wire literal used in capability reporting.
    #[must_use]
    pub const fn wire_literal(self) -> &'static str {
        match self {
            Self::WebCodecs => PREVIEW_CODEC_WEBCODECS,
            Self::MseFmp4 => PREVIEW_CODEC_MSE,
        }
    }
}

/// Decide preview transport from browser capabilities.
///
/// v1 policy is intentionally strict and additive-safe:
/// - if `WebCodecs` decode exists, use `webcodecs`;
/// - otherwise fall back to fMP4/MSE, including Safari.
#[must_use]
pub const fn codec_for_preview(caps: PreviewClientCapabilities) -> WebPreviewCodec {
    if caps.has_webcodecs_decode {
        WebPreviewCodec::webcodecs()
    } else {
        safari_fallback_codec(caps.browser_family)
    }
}

/// Safari fallback contract for v1.
///
/// This pins issue #376 decision A at the API boundary: Safari without
/// `WebCodecs` does not degrade to NDJSON-only and is not marked unsupported.
#[must_use]
pub const fn safari_fallback_codec(_browser_family: BrowserFamily) -> WebPreviewCodec {
    WebPreviewCodec::mse_fmp4()
}
