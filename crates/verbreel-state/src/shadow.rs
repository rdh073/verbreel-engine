//! [`Shadow`] — drop shadow parameters for text rendering. Mirrors
//! `spec/project-schema.json` `$defs/Shadow`.

use serde::{Deserialize, Serialize};

use crate::newtypes::Color;

/// Drop-shadow parameters, in clip pixels.
///
/// Schema reference: `spec/project-schema.json` `$defs/Shadow`.
///
/// All 4 fields are optional with non-trivial defaults; the
/// [`Default`] impl reproduces them.
///
/// Note: `Eq` is intentionally NOT derived because `blur_px` /
/// `offset_x` / `offset_y` are `f64` and `f64::NaN != f64::NaN`. The
/// canonical-storage layer never produces NaN (JSON has no NaN), so
/// in practice equality matches structural equality, but the trait
/// bound stays at `PartialEq` to avoid lying about IEEE 754.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Shadow {
    /// Shadow color. Default `"#000000aa"` (semi-transparent black).
    #[serde(default = "default_shadow_color")]
    pub color: Color,
    /// Gaussian blur radius in pixels. Default `4`. Spec
    /// `minimum: 0`.
    #[serde(default = "default_blur_px")]
    pub blur_px: f64,
    /// Horizontal offset in pixels. Default `0`.
    #[serde(default)]
    pub offset_x: f64,
    /// Vertical offset in pixels. Default `2`.
    #[serde(default = "default_offset_y")]
    pub offset_y: f64,
}

impl Default for Shadow {
    fn default() -> Self {
        Shadow {
            color: default_shadow_color(),
            blur_px: 4.0,
            offset_x: 0.0,
            offset_y: 2.0,
        }
    }
}

fn default_shadow_color() -> Color {
    Color::new("#000000aa".to_string()).expect("compile-time-valid color literal")
}

fn default_blur_px() -> f64 {
    4.0
}

fn default_offset_y() -> f64 {
    2.0
}
