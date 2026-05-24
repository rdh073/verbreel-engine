//! [`Transform`] — 2D affine transform applied to a clip's render
//! output. Mirrors `spec/project-schema.json` `$defs/Transform`.

use serde::{Deserialize, Serialize};

/// 2D affine transform, in canvas pixels.
///
/// Schema reference: `spec/project-schema.json` `$defs/Transform`.
///
/// All 11 fields are optional in the schema with defaults; the typed
/// shape carries every field directly so the canonical-storage form
/// (per spec §0.5.2 — canonical JSON includes defaults) round-trips
/// byte-equivalently at the `serde_json::Value` layer. The
/// [`Default`] impl reproduces the schema defaults bit-for-bit.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    /// Translation along canvas X, in pixels. Default `0`.
    #[serde(default)]
    pub x: f64,
    /// Translation along canvas Y, in pixels. Default `0`.
    #[serde(default)]
    pub y: f64,
    /// Scale along clip X. Default `1`.
    #[serde(default = "one_f64")]
    pub scale_x: f64,
    /// Scale along clip Y. Default `1`.
    #[serde(default = "one_f64")]
    pub scale_y: f64,
    /// Rotation in degrees (canvas-relative). Default `0`.
    #[serde(default)]
    pub rotation_deg: f64,
    /// Normalized anchor inside the clip on X (0=left, 1=right).
    /// Default `0.5`.
    #[serde(default = "half_f64")]
    pub anchor_x: f64,
    /// Normalized anchor inside the clip on Y (0=top, 1=bottom).
    /// Default `0.5`.
    #[serde(default = "half_f64")]
    pub anchor_y: f64,
    /// Skew along X in degrees. Default `0`.
    #[serde(default)]
    pub skew_x_deg: f64,
    /// Skew along Y in degrees. Default `0`.
    #[serde(default)]
    pub skew_y_deg: f64,
    /// Mirror horizontally. Default `false`.
    #[serde(default)]
    pub flip_h: bool,
    /// Mirror vertically. Default `false`.
    #[serde(default)]
    pub flip_v: bool,
}

impl Default for Transform {
    fn default() -> Self {
        Transform {
            x: 0.0,
            y: 0.0,
            scale_x: 1.0,
            scale_y: 1.0,
            rotation_deg: 0.0,
            anchor_x: 0.5,
            anchor_y: 0.5,
            skew_x_deg: 0.0,
            skew_y_deg: 0.0,
            flip_h: false,
            flip_v: false,
        }
    }
}

fn one_f64() -> f64 {
    1.0
}

fn half_f64() -> f64 {
    0.5
}
