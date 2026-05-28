//! [`EncodeParams`] — full parameter shape for one encode pass.
//!
//! Mirrors the field set `render.queue.add` already exports. Field order
//! is canonical and stable; serde keys are the lowercase struct names.

use serde::{Deserialize, Serialize};

use crate::codec::Codec;
use crate::preset::CodecPreset;

/// Inputs to a single encode pass.
///
/// `fps_num` / `fps_den` carry the rational frame rate (e.g. `30000/1001`
/// for NTSC 29.97). They mirror the rational-rate pattern used elsewhere
/// in the engine — see `verbreel_types::tick` — and avoid the float
/// drift that a single `f64 fps` field would introduce.
///
/// `bit_rate` is `Option<u32>` because the `ProRes` path is bitrate-
/// codec-controlled (advisory only on that codec); H.264 callers should
/// pass `Some(_)` for predictable file sizes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EncodeParams {
    /// Output codec.
    pub codec: Codec,
    /// Encoder preset — picks the §11.1 deterministic or §11.2
    /// performance libx264 parameter set.
    pub preset: CodecPreset,
    /// Output width in pixels. Must be even on the H.264 path
    /// (libx264 yuv420p constraint); validation lands with Spike S1.
    pub width: u32,
    /// Output height in pixels. Must be even on the H.264 path.
    pub height: u32,
    /// Frame-rate numerator (e.g. `30000` for NTSC 29.97).
    pub fps_num: u32,
    /// Frame-rate denominator (e.g. `1001` for NTSC 29.97).
    /// Must be non-zero; validation lands with Spike S1.
    pub fps_den: u32,
    /// Optional target bit rate in bits per second. `None` lets the
    /// encoder pick its default; `ProRes` ignores this field entirely.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub bit_rate: Option<u32>,
}
