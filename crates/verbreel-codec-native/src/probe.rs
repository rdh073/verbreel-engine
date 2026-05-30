//! [`ProbeMetadata`] — container/stream metadata for `asset.probe` and
//! `asset.import`.
//!
//! Pure data: no `FFmpeg` dependency, so the struct compiles feature-off and
//! is part of the stable type surface. The feature-gated
//! `decode::probe` populates it from an `rsmpeg` open; feature-off
//! callers still get the type to deserialize a previously probed asset's
//! stored metadata.

use serde::{Deserialize, Serialize};

use crate::codec::Codec;

/// Metadata for the primary video stream of a media container.
///
/// `fps_num` / `fps_den` carry the rational frame rate (e.g. `30000/1001`
/// for NTSC 29.97), mirroring [`crate::EncodeParams`] and avoiding the float
/// drift a single `f64` field would introduce. `duration_us` is the stream
/// duration in microseconds (`FFmpeg`'s `AV_TIME_BASE` unit), `None` when the
/// container does not report one.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProbeMetadata {
    /// Detected codec of the primary video stream.
    pub codec: Codec,
    /// Coded width in pixels.
    pub width: u32,
    /// Coded height in pixels.
    pub height: u32,
    /// Frame-rate numerator (e.g. `30000` for NTSC 29.97).
    pub fps_num: u32,
    /// Frame-rate denominator (e.g. `1001` for NTSC 29.97). Non-zero.
    pub fps_den: u32,
    /// Stream duration in microseconds, `None` when the container reports
    /// no duration.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub duration_us: Option<u64>,
}
