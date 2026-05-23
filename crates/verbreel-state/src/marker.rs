//! [`Marker`] — project-level time markers. Mirrors `$defs/Marker`
//! from `spec/project-schema.json`.

use serde::{Deserialize, Serialize};
use verbreel_types::{MarkerId, Tick};

/// A point-in-time annotation on the project timeline.
///
/// Schema reference: `spec/project-schema.json` `$defs/Marker`.
///
/// Required: `id`, `time_tk`, `label`. Optional: `color` (default
/// `"#ffaa00ff"`), `note` (max 4096 chars, schema-enforced).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Marker {
    /// Marker ID — strongly-typed [`MarkerId`] (`UuidV7`).
    pub id: MarkerId,

    /// Marker position on the project timeline, in ticks at
    /// `TICK_RATE_HZ` (240,000 Hz). Spec requires `minimum: 0`; that
    /// range check is currently schema-only and moves into the typed
    /// validator with §0.13.
    pub time_tk: Tick,

    /// Display label. Spec: 1..=256 chars.
    pub label: String,

    /// Marker color as lower-case `#rrggbbaa`. Default
    /// `"#ffaa00ff"` per schema.
    ///
    // TODO(#19 follow-up): replace `String` with a `Color` newtype
    // (deferred per the lib-level docstring).
    #[serde(default = "default_color")]
    pub color: String,

    /// Optional freeform note (spec: ≤ 4096 chars).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

fn default_color() -> String {
    "#ffaa00ff".to_string()
}
