//! [`Clip`] + supporting enums and substructs. Mirrors
//! `spec/project-schema.json` `$defs/Clip`.
//!
//! `Clip.effects` and `Clip.keyframes` remain
//! `Vec<serde_json::Value>` placeholders this slice — those types
//! land in the next two slices per the topology-dictated dependency
//! order (Effect → Keyframe → `apply()`).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use verbreel_types::{ClipId, LinkGroupId, Tick};

use crate::newtypes::AssetRef;
use crate::text_element::TextElement;
use crate::transform::Transform;

// ---------------------------------------------------------------------
// FadeCurve
// ---------------------------------------------------------------------

/// Fade-in / fade-out interpolation curve. Schema
/// `$defs/Clip.fade_in_curve` and `fade_out_curve` enums.
///
/// The string forms are Verbreel-level identifiers; the engine maps
/// them to ffmpeg `afade=curve=` values at render time per
/// `Clip.fade_in_curve`'s schema description.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum FadeCurve {
    /// Linear ramp. Schema default.
    #[default]
    Linear,
    /// Exponential rise/fall (mapped to ffmpeg `esin`).
    Exp,
    /// Logarithmic rise/fall.
    Log,
}

// ---------------------------------------------------------------------
// BlendMode
// ---------------------------------------------------------------------

/// Per-clip compositor blend mode. Schema `$defs/Clip.blend_mode`
/// enum. **v1.1-additive** — v1.0 engines treat absent `blend_mode`
/// as [`BlendMode::Normal`] (the v1.1 default).
///
/// `rename_all = "kebab-case"` matches the schema literals:
/// `SoftLight` ↔ `"soft-light"`, `ColorDodge` ↔ `"color-dodge"`, etc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum BlendMode {
    /// Straight alpha-over composite. Schema default; v1.0 fallback.
    #[default]
    Normal,
    /// Multiply blend.
    Multiply,
    /// Screen blend.
    Screen,
    /// Overlay blend.
    Overlay,
    /// Soft-light blend.
    SoftLight,
    /// Hard-light blend.
    HardLight,
    /// Darken-only blend.
    Darken,
    /// Lighten-only blend.
    Lighten,
    /// Difference blend.
    Difference,
    /// Color-dodge blend.
    ColorDodge,
    /// Color-burn blend.
    ColorBurn,
}

// ---------------------------------------------------------------------
// MaskKind / ClipMask
// ---------------------------------------------------------------------

/// Mask shape kind. Schema `$defs/Clip.mask.kind` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MaskKind {
    /// Rectangular mask. Params: `{x, y, w, h}`.
    Rect,
    /// Elliptical mask. Params: `{cx, cy, rx, ry}`.
    Ellipse,
    /// Polygonal mask. Params: `{points: [[x, y], ...]}`.
    Polygon,
    /// Asset-driven mask. Params: `{asset_id, threshold?}`.
    Asset,
}

/// Per-clip shape- or asset-driven alpha mask. Schema
/// `$defs/Clip.mask` (non-null branch). **v1.1-additive**.
///
/// `params` is deliberately an opaque `serde_json::Map<String,
/// serde_json::Value>` — the schema uses `additionalProperties:
/// true` for this field so each `kind`'s nested shape is validated
/// by the engine layer (`E_MASK_INVALID_PARAMS`), not the JSON
/// Schema layer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipMask {
    /// Mask shape kind. Determines the expected shape of `params`.
    pub kind: MaskKind,
    /// Per-kind opaque params bag (validated by the engine, not the
    /// type or schema layer).
    pub params: Map<String, Value>,
    /// Gaussian edge feather in clip pixels. Default `0`.
    #[serde(default)]
    pub feather_px: f64,
    /// When true, flips the mask's pass/block regions. Default
    /// `false`.
    #[serde(default)]
    pub inverted: bool,
}

// ---------------------------------------------------------------------
// SpeedCurvePoint
// ---------------------------------------------------------------------

/// One control point on a non-uniform speed-ramp curve. Schema
/// `$defs/Clip.speed_curve.items`. **v1.1-additive**.
///
/// Bounds (`factor ∈ [0.001, 100]`, `time_tk` monotonic, `2 ≤ len ≤
/// 256`) are engine-layer invariants — surface as `E_BAD_TIME` /
/// `E_SCHEMA_VIOLATION` — not type-layer.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SpeedCurvePoint {
    /// Source-time-relative tick measured from `Clip.source_in_tk`.
    /// `0` is the first source tick, `source_out_tk - source_in_tk`
    /// is the last.
    pub time_tk: Tick,
    /// Multiplier applied on top of `Clip.speed` at this control
    /// point. Schema bound `[0.001, 100]`.
    pub factor: f64,
}

// ---------------------------------------------------------------------
// Clip
// ---------------------------------------------------------------------

/// A clip on a track in the project graph.
///
/// Schema reference: `spec/project-schema.json` `$defs/Clip`.
///
/// 6 required fields (`id`, `name`, `asset_id`, `track_position_tk`,
/// `source_in_tk`, `source_out_tk`) + 17 optional fields with
/// defaults matching the schema. Field order matches the schema's
/// property listing.
///
/// `clippy::struct_excessive_bools` allowed — Clip carries 2 bools
/// directly (`reversed`, `locked`) and is well under the lint's
/// threshold, but a future per-field expansion could push past it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    /// Clip ID.
    pub id: ClipId,
    /// Display name. Spec: 1..=128 chars.
    pub name: String,
    /// Reference to an asset in the project's `assets[]`, OR the
    /// nil-UUID sentinel for text clips. See [`AssetRef`].
    pub asset_id: AssetRef,
    /// Timeline start tick of this clip. Spec `minimum: 0`.
    pub track_position_tk: Tick,
    /// Source media slice start. Spec `minimum: 0`. Image / text
    /// clips have this engine-pinned to `0`.
    pub source_in_tk: Tick,
    /// Source slice end (exclusive). Spec `minimum: 1`. For images
    /// and text clips, this is the display duration in ticks.
    pub source_out_tk: Tick,
    /// Playback speed multiplier. Schema range `[0.001, 100]`,
    /// default `1`. Bounds enforced at the engine layer.
    #[serde(default = "default_speed")]
    pub speed: f64,
    /// Reverse playback. Default `false`.
    #[serde(default)]
    pub reversed: bool,
    /// 2D affine transform applied to the clip's render output.
    /// Default = [`Transform::default`].
    #[serde(default)]
    pub transform: Transform,
    /// Clip opacity. Schema range `[0, 1]`, default `1`.
    #[serde(default = "default_opacity")]
    pub opacity: f64,
    /// Per-clip linear gain. Schema range `[0, 4]`, default `1`.
    #[serde(default = "default_volume")]
    pub volume: f64,
    /// Fade-in length in ticks. Default `0`.
    #[serde(default)]
    pub fade_in_tk: Tick,
    /// Fade-out length in ticks. Default `0`.
    #[serde(default)]
    pub fade_out_tk: Tick,
    /// Fade-in interpolation curve. Default [`FadeCurve::Linear`].
    #[serde(default)]
    pub fade_in_curve: FadeCurve,
    /// Fade-out interpolation curve. Default [`FadeCurve::Linear`].
    #[serde(default)]
    pub fade_out_curve: FadeCurve,
    /// Clip-level effects. **Placeholder** — typing lands in the
    /// Effect slice.
    ///
    // TODO(slice-4 follow-up): replace `Vec<serde_json::Value>` with
    // `Vec<Effect>` once `Effect` is typed.
    #[serde(default)]
    pub effects: Vec<Value>,
    /// Clip-level keyframes. **Placeholder** — typing lands in the
    /// Keyframe slice.
    ///
    // TODO(slice-5 follow-up): replace `Vec<serde_json::Value>` with
    // `Vec<Keyframe>` once `Keyframe` is typed.
    #[serde(default)]
    pub keyframes: Vec<Value>,
    /// Text-element parameters (present on text-track clips).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<TextElement>,
    /// When true, mutating verbs return `E_LOCKED`. Default `false`.
    #[serde(default)]
    pub locked: bool,
    /// Link-group ID. Clips sharing this id move and trim as a unit.
    /// Engine-managed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_group: Option<LinkGroupId>,
    /// Per-clip blend mode. **v1.1-additive**. Default
    /// [`BlendMode::Normal`].
    #[serde(default)]
    pub blend_mode: BlendMode,
    /// Per-clip alpha mask. **v1.1-additive**. `None` (schema `null`)
    /// = no mask.
    #[serde(default)]
    pub mask: Option<ClipMask>,
    /// Non-uniform speed ramp. **v1.1-additive**. `None` = scalar
    /// `Clip.speed` only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_curve: Option<Vec<SpeedCurvePoint>>,
}

fn default_speed() -> f64 {
    1.0
}

fn default_opacity() -> f64 {
    1.0
}

fn default_volume() -> f64 {
    1.0
}
