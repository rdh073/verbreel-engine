//! [`Canvas`] — viewport dimensions, background color, and pixel
//! aspect ratio. Mirrors `$defs/Canvas` from `spec/project-schema.json`.

use serde::{Deserialize, Serialize};

use crate::newtypes::Color;

/// The rectangular viewport onto which clips are composited.
///
/// Schema reference: `spec/project-schema.json` `$defs/Canvas`.
///
/// - `width` / `height` are required, 16..=8192 (spec). Validation of the
///   range is currently surfaced only by the schema test; the typed
///   shape carries them as plain `u32` because no engine path mints a
///   `Canvas` outside spec range without going through schema-validated
///   storage first. Range-enforcement will move into a typed validator
///   when `project.open` lands (follow-up slice).
/// - `background` is a [`Color`] newtype, defaulting to `"#000000ff"`
///   (lower-case `#rrggbbaa`). The newtype enforces the
///   `^#[0-9a-f]{8}$` pattern at construction time, so any value
///   that survives serde deserialization or in-engine construction is
///   already in canonical form.
/// - `pixel_aspect_num` / `pixel_aspect_den` default to `1` (square
///   pixels). Spec requires `minimum: 1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Canvas {
    /// Canvas width in pixels. Spec range 16..=8192.
    pub width: u32,

    /// Canvas height in pixels. Spec range 16..=8192.
    pub height: u32,

    /// Background color as lower-case `#rrggbbaa`. Default `"#000000ff"`
    /// per schema; `serde(default)` lets the field be omitted on input.
    #[serde(default = "default_background")]
    pub background: Color,

    /// Pixel aspect ratio numerator. Default `1` (square pixels).
    #[serde(default = "one_u32")]
    pub pixel_aspect_num: u32,

    /// Pixel aspect ratio denominator. Default `1` (square pixels).
    #[serde(default = "one_u32")]
    pub pixel_aspect_den: u32,
}

fn default_background() -> Color {
    Color::new("#000000ff".to_string()).expect("compile-time-valid color literal")
}

#[allow(clippy::unnecessary_wraps)]
fn one_u32() -> u32 {
    1
}
