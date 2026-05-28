//! [`WebDecoder`] — v0 enum committing Research 01 §0's decode-only
//! `WebCodecs` codec set.
//!
//! Two variants are committed in v0:
//!
//! - [`WebDecoder::H264`] — `WebCodecs` `avc1.*` codec strings.
//! - [`WebDecoder::H265`] — `WebCodecs` `hev1.*` / `hvc1.*` codec strings.
//!
//! AV1, VP9, and other browser-mediated codecs are intentionally NOT
//! committed here. Research 01 candidates §1 scopes `WebCodecs` to
//! "common formats reaching parity on Safari as of 2026" (Risks §R3);
//! pinning more variants now would freeze the wire shape before the
//! cross-browser support matrix has settled. New variants land when
//! Spike S2's playwright matrix confirms the `WebCodecs` decoder
//! enumeration includes them on the target browsers.

use serde::{Deserialize, Serialize};

/// Decode codec selected for a preview pass.
///
/// Variants serialize as the lowercase canonical token shown in their
/// docstrings — this matches the on-the-wire shape `preview.session.*`
/// will accept once the surface is wired in Spike S2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WebDecoder {
    /// H.264 / AVC via the browser's `WebCodecs` `VideoDecoder`.
    ///
    /// Wire token: `"h264"`. Corresponds to `WebCodecs` codec strings
    /// of the form `avc1.<profile><level>` (e.g. `avc1.640028`).
    /// Required baseline per Research 01 candidates §1 — every
    /// `WebCodecs`-shipping browser supports H.264 decode.
    H264,

    /// H.265 / HEVC via the browser's `WebCodecs` `VideoDecoder`.
    ///
    /// Wire token: `"h265"`. Corresponds to `WebCodecs` codec strings
    /// of the form `hev1.*` or `hvc1.*`. Safari 17+ and Chrome 107+
    /// ship hardware-accelerated HEVC decode; Firefox is the lagging
    /// implementation. Spike S2's playwright matrix gates whether this
    /// variant is wired on a per-browser basis.
    H265,
}
