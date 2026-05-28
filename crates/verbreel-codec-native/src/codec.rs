//! [`Codec`] — v0 enum mirroring Research 01 §5's pluggable-codec set.
//!
//! Two variants are committed in v0:
//!
//! - [`Codec::H264`] — libx264 software encoder. The deterministic-mode
//!   default per Research 01 §5 + §11.1.
//! - [`Codec::ProRes`] — proxy / archival path (lossless-ish intra-frame).
//!
//! AV1, HEVC, and hardware-encoder backends (NVENC / VAAPI / `VideoToolbox`)
//! are intentionally NOT committed here. They appear when Research 01 §6
//! lands the hwaccel-design follow-up — committing them now would pin
//! variants whose API surface has not yet been negotiated. See Research 01
//! §8 R8 for the determinism risk that hwaccel variants must address
//! before they enter the enum.

use serde::{Deserialize, Serialize};

/// Output codec selected for an encode pass.
///
/// Variants serialize as the lowercase canonical token shown in their
/// docstrings — this matches the on-the-wire shape `render.queue.add`
/// will accept once the surface is wired.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Codec {
    /// H.264 / AVC via libx264 (software encoder).
    ///
    /// Wire token: `"h264"`. This is the deterministic-mode default —
    /// pair with [`crate::CodecPreset::Deterministic`] for byte-identical
    /// MP4 output per Research 01 §11.1.
    H264,

    /// Apple `ProRes` (intra-frame, proxy / archival).
    ///
    /// Wire token: `"prores"`. Intra-frame coding makes `ProRes` naturally
    /// deterministic across encoder threads — the §11.1 x264 preset
    /// gymnastics are not required for this codec. Bitrate is
    /// codec-controlled, so `bit_rate` in [`crate::EncodeParams`] is
    /// advisory only on this path.
    ProRes,
}
