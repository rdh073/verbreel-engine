//! Per-variant asset metadata structs + [`RotationDeg`] enum +
//! [`FileFingerprint`].
//!
//! Mirrors `spec/project-schema.json` `$defs/VideoAssetMetadata`,
//! `$defs/AudioAssetMetadata`, `$defs/ImageAssetMetadata`,
//! `$defs/SubtitleAssetMetadata`, `$defs/FileFingerprint`.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use verbreel_types::Tick;

// ---------------------------------------------------------------------
// RotationDeg
// ---------------------------------------------------------------------

/// EXIF/container rotation in degrees. Schema constrains to
/// `enum: [0, 90, 180, 270]`. Used by [`VideoAssetMetadata`] and
/// [`ImageAssetMetadata`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "i64", into = "i64")]
pub enum RotationDeg {
    /// 0° rotation (upright).
    Deg0 = 0,
    /// 90° clockwise rotation.
    Deg90 = 90,
    /// 180° rotation (upside-down).
    Deg180 = 180,
    /// 270° clockwise rotation (90° counter-clockwise).
    Deg270 = 270,
}

/// Error returned when constructing a [`RotationDeg`] from an
/// out-of-spec integer.
#[derive(Debug, Error)]
#[error(
    "RotationDeg must be one of {{0, 90, 180, 270}} (schema $defs/{{Video,Image}}AssetMetadata.\
     rotation_deg enum), got {got}"
)]
pub struct RotationDegError {
    /// The offending value.
    pub got: i64,
}

impl TryFrom<i64> for RotationDeg {
    type Error = RotationDegError;
    fn try_from(value: i64) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(RotationDeg::Deg0),
            90 => Ok(RotationDeg::Deg90),
            180 => Ok(RotationDeg::Deg180),
            270 => Ok(RotationDeg::Deg270),
            other => Err(RotationDegError { got: other }),
        }
    }
}

impl From<RotationDeg> for i64 {
    fn from(value: RotationDeg) -> Self {
        value as i64
    }
}

// ---------------------------------------------------------------------
// FileFingerprint
// ---------------------------------------------------------------------

/// Quick file-identity check carried alongside each asset. Schema
/// `$defs/FileFingerprint`: `mtime_ms` (POSIX milliseconds since
/// epoch) + `size_bytes` (source file size at import). Both are
/// `integer >= 0`; the typed shape uses `u64` so the non-negativity
/// constraint is type-enforced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileFingerprint {
    /// POSIX mtime in milliseconds since epoch, captured at import.
    pub mtime_ms: u64,
    /// Source file size in bytes at import.
    pub size_bytes: u64,
}

// ---------------------------------------------------------------------
// VideoAssetMetadata
// ---------------------------------------------------------------------

/// Video-asset metadata. Schema `$defs/VideoAssetMetadata`.
///
/// Required: `duration_tk`, `width`, `height`, `fps_num`, `fps_den`,
/// `video_codec`, `container`, `fingerprint`. The rest is optional —
/// audio sub-stream fields appear when the source carries audio, and
/// `bitrate_bps` / `color_*` / `has_alpha` / `rotation_deg` are
/// best-effort probe results.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VideoAssetMetadata {
    /// Duration in ticks at 240,000 Hz. Spec `minimum: 1`.
    pub duration_tk: Tick,
    /// Width in pixels. Spec `minimum: 1`.
    pub width: u32,
    /// Height in pixels. Spec `minimum: 1`.
    pub height: u32,
    /// Frame-rate numerator. Spec `minimum: 1`.
    pub fps_num: u32,
    /// Frame-rate denominator. Spec `minimum: 1`.
    pub fps_den: u32,
    /// Video codec identifier (`"h264"`, `"hevc"`, `"av1"`, etc.).
    pub video_codec: String,
    /// Audio codec identifier if the source has an audio sub-stream.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_codec: Option<String>,
    /// Number of audio channels, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_channels: Option<u32>,
    /// Audio sample rate in Hz, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_sample_rate_hz: Option<u32>,
    /// Probe bitrate in bps. Spec `minimum: 0`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bitrate_bps: Option<u64>,
    /// Color space (`"bt709"`, `"bt2020"`, etc.).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_space: Option<String>,
    /// Color primaries identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_primaries: Option<String>,
    /// Container format identifier (`"mp4"`, `"mov"`, `"webm"`, ...).
    pub container: String,
    /// Whether the source carries an alpha channel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_alpha: Option<bool>,
    /// EXIF/container rotation. See [`RotationDeg`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation_deg: Option<RotationDeg>,
    /// Quick file-identity check.
    pub fingerprint: FileFingerprint,
}

// ---------------------------------------------------------------------
// AudioAssetMetadata
// ---------------------------------------------------------------------

/// Audio-asset metadata. Schema `$defs/AudioAssetMetadata`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioAssetMetadata {
    /// Duration in ticks. Spec `minimum: 1`.
    pub duration_tk: Tick,
    /// Audio codec identifier (`"aac"`, `"opus"`, `"pcm_s16le"`, ...).
    pub audio_codec: String,
    /// Number of audio channels. Spec `minimum: 1`.
    pub audio_channels: u32,
    /// Audio sample rate in Hz. Spec `minimum: 1`.
    pub audio_sample_rate_hz: u32,
    /// Bitrate in bps. Spec `minimum: 0`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bitrate_bps: Option<u64>,
    /// Container format identifier (`"mp3"`, `"m4a"`, `"wav"`, ...).
    pub container: String,
    /// Quick file-identity check.
    pub fingerprint: FileFingerprint,
}

// ---------------------------------------------------------------------
// ImageAssetMetadata
// ---------------------------------------------------------------------

/// Image-asset metadata. Schema `$defs/ImageAssetMetadata`.
///
/// Images have no intrinsic duration — clips set `source_out_tk` to
/// the desired display duration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageAssetMetadata {
    /// Width in pixels. Spec `minimum: 1`.
    pub width: u32,
    /// Height in pixels. Spec `minimum: 1`.
    pub height: u32,
    /// Image format identifier (`"png"`, `"jpg"`, `"webp"`, ...).
    pub container: String,
    /// Whether the source carries an alpha channel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_alpha: Option<bool>,
    /// Color space (`"srgb"`, `"display-p3"`, ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_space: Option<String>,
    /// EXIF/container rotation. See [`RotationDeg`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation_deg: Option<RotationDeg>,
    /// Quick file-identity check.
    pub fingerprint: FileFingerprint,
}

// ---------------------------------------------------------------------
// SubtitleAssetMetadata
// ---------------------------------------------------------------------

/// Subtitle-asset metadata. Schema `$defs/SubtitleAssetMetadata`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubtitleAssetMetadata {
    /// Subtitle format identifier (`"srt"`, `"vtt"`, `"ass"`).
    pub container: String,
    /// BCP 47 language tag, e.g. `"en"`, `"en-US"`, `"id"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Number of subtitle segments. Spec `minimum: 0`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub segment_count: Option<u32>,
    /// Quick file-identity check.
    pub fingerprint: FileFingerprint,
}
