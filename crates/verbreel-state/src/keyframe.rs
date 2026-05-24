//! [`Keyframe`] and its supporting types ([`KeyframeProperty`],
//! [`Easing`]). Mirrors `spec/project-schema.json` `$defs/Keyframe`.
//!
//! ## Why the [`Easing`] enum carries `bezier` inside `CubicBezier`
//!
//! Schema `$defs/Keyframe.allOf[0]` is an if/then conditional —
//! `easing == "cubic-bezier"` implies `bezier` is required. A naïve
//! mapping would carry `easing: String` + `bezier: Option<[f64; 4]>`
//! at the [`Keyframe`] level and enforce the coupling at the engine
//! layer; that leaves the invalid `easing="cubic-bezier" + bezier=None`
//! state representable at the type level. Instead we model
//! [`Easing::CubicBezier`] as a struct variant carrying the bezier
//! array inline — the type system makes the half-state
//! unrepresentable.
//!
//! The on-disk JSON shape is unchanged (flat `easing` + optional
//! flat `bezier` at [`Keyframe`] level, NOT nested) because the
//! private [`KeyframeSerde`] adapter splits the variant on serialize
//! and re-joins on deserialize.
//!
//! ## Why [`KeyframeProperty`] is a regex newtype and not an enum
//!
//! The `effects[<uuidv7>].params.<dotted-path>` form has unbounded
//! variants — any v7 UUID, any dotted identifier path. Lifting that
//! into a Rust enum would require parsing into typed-then-string
//! state, which would (a) lose info on round-trip and (b) require
//! the engine to re-parse a typed variant back into the schema's
//! string form. The schema's intent is exactly a regex-validated
//! string; the type layer mirrors that with a `try_from = "String"`
//! newtype that compiles the regex once via [`OnceLock`].
//!
//! ## `value` is `serde_json::Value`
//!
//! Schema `$defs/Keyframe.value` has no type constraint — schema
//! says only "JSON value matching the target property's type".
//! Per-property value type validation (e.g. opacity must be number
//! in [0, 1]) is engine-layer, evaluated at patch-compute. The type
//! layer carries the bag as `serde_json::Value`.

use std::fmt;
use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use verbreel_types::{KeyframeId, Tick};

// ---------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------

/// Errors surfaced when constructing a [`KeyframeProperty`] from a
/// raw string or when [`KeyframeSerde`] resolves the easing+bezier
/// pair into a typed [`Easing`].
#[derive(Debug, Error)]
pub enum KeyframeNewtypeError {
    /// String did not match the schema's `$defs/Keyframe.property` pattern.
    #[error(
        "KeyframeProperty must match schema $defs/Keyframe.property regex (one of: \
         transform.<leaf>, opacity, volume, mask.feather_px, mask.params.<leaf>, \
         effects[<uuidv7>].params.<dotted-path>), got {got:?}"
    )]
    InvalidProperty {
        /// The offending input.
        got: String,
    },

    /// Schema requires `bezier` when `easing == "cubic-bezier"`.
    #[error(
        "Keyframe easing=cubic-bezier requires bezier: [x1, y1, x2, y2] (schema \
         $defs/Keyframe.allOf if/then violation)"
    )]
    MissingBezier,

    /// Unknown easing literal — not one of the 6 schema enum values.
    #[error(
        "Keyframe easing must be one of linear|ease-in|ease-out|ease-in-out|step|cubic-bezier \
         (schema $defs/Keyframe.easing enum), got {got:?}"
    )]
    UnknownEasing {
        /// The offending easing string.
        got: String,
    },
}

// ---------------------------------------------------------------------
// KeyframeProperty
// ---------------------------------------------------------------------

/// Schema `$defs/Keyframe.property` regex, copied verbatim. See the
/// module docs for the form summary and §0.13 for non-regex
/// invariants (dangling effect-ref check, mask-leaf existence, etc.)
/// that the engine enforces on top of this pattern.
const KEYFRAME_PROPERTY_PATTERN: &str = r"^(transform\.(x|y|scale_x|scale_y|rotation_deg|anchor_x|anchor_y|skew_x_deg|skew_y_deg)|opacity|volume|mask\.(feather_px|params\.(x|y|w|h|cx|cy|rx|ry|threshold))|effects\[[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}\]\.params(\.[a-zA-Z_][a-zA-Z0-9_]*)+)$";

fn keyframe_property_regex() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(KEYFRAME_PROPERTY_PATTERN).expect("KEYFRAME_PROPERTY_PATTERN compiles")
    })
}

/// Dotted-path identifier into the parent clip's keyframable surface.
/// Schema `$defs/Keyframe.property`.
///
/// Examples (all valid):
/// - `transform.x`, `transform.scale_y`, `transform.rotation_deg`
/// - bare `opacity`, `volume`
/// - `mask.feather_px`, `mask.params.cx`
/// - `effects[01890000-0000-7000-8000-0000000000aa].params.radius_px`
///
/// Serialized as a bare JSON string.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct KeyframeProperty(String);

impl KeyframeProperty {
    /// Construct, validating against schema's regex pattern.
    ///
    /// # Errors
    ///
    /// Returns [`KeyframeNewtypeError::InvalidProperty`] if `s` does
    /// not match the schema's regex.
    pub fn new(s: String) -> Result<Self, KeyframeNewtypeError> {
        if keyframe_property_regex().is_match(&s) {
            Ok(KeyframeProperty(s))
        } else {
            Err(KeyframeNewtypeError::InvalidProperty { got: s })
        }
    }

    /// Borrow the inner string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for KeyframeProperty {
    type Error = KeyframeNewtypeError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        KeyframeProperty::new(value)
    }
}

impl From<KeyframeProperty> for String {
    fn from(value: KeyframeProperty) -> Self {
        value.0
    }
}

impl fmt::Display for KeyframeProperty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

// ---------------------------------------------------------------------
// Easing
// ---------------------------------------------------------------------

/// Keyframe interpolation curve. Schema `$defs/Keyframe.easing` enum.
///
/// `Linear` is the default per schema. `CubicBezier` carries the
/// `[x1, y1, x2, y2]` control points inline; the schema's if/then
/// "bezier required when easing=cubic-bezier" constraint is
/// compile-enforced — `CubicBezier` without a bezier array cannot
/// be constructed.
///
/// On-disk shape is flat (the [`KeyframeSerde`] adapter handles the
/// split). Use [`Easing::Linear`] / [`Easing::EaseIn`] /
/// [`Easing::EaseOut`] / [`Easing::EaseInOut`] / [`Easing::Step`] /
/// `Easing::CubicBezier { bezier: [...] }`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Easing {
    /// Linear interpolation. Schema default.
    #[default]
    Linear,
    /// Ease-in curve.
    EaseIn,
    /// Ease-out curve.
    EaseOut,
    /// Ease-in-out curve.
    EaseInOut,
    /// Step / hold-then-jump.
    Step,
    /// Cubic bezier with the schema-mandated `[x1, y1, x2, y2]`
    /// control points carried inline.
    CubicBezier {
        /// Control points `[x1, y1, x2, y2]`. Schema requires
        /// exactly 4 numbers.
        bezier: [f64; 4],
    },
}

// ---------------------------------------------------------------------
// Keyframe
// ---------------------------------------------------------------------

/// A keyframe on a clip property. Mirrors `spec/project-schema.json`
/// `$defs/Keyframe`.
///
/// Required: `id`, `property`, `time_tk`, `value`. Optional:
/// `easing` (default [`Easing::Linear`]).
///
/// `time_tk` is timeline-relative — measured from the parent clip's
/// `track_position_tk`, independent of `clip.speed` and
/// `clip.reversed` (schema's documented semantics).
///
/// `value` is `serde_json::Value` — the schema does not constrain
/// its type. Engine-layer evaluates type-compat against the property
/// the keyframe targets (e.g. `opacity` must be a number in `[0, 1]`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "KeyframeSerde", into = "KeyframeSerde")]
pub struct Keyframe {
    /// Strongly-typed keyframe ID.
    pub id: KeyframeId,
    /// Regex-validated property path. See [`KeyframeProperty`].
    pub property: KeyframeProperty,
    /// Timeline-relative tick from the parent clip's
    /// `track_position_tk`. Schema `minimum: 0`.
    pub time_tk: Tick,
    /// Target value at this keyframe. Schema-untyped at this level.
    pub value: Value,
    /// Interpolation curve into this keyframe. See [`Easing`].
    pub easing: Easing,
}

impl Keyframe {
    /// Construct a minimal [`Keyframe`] — id, property, `time_tk`,
    /// value, default [`Easing::Linear`].
    #[must_use]
    pub fn new(id: KeyframeId, property: KeyframeProperty, time_tk: Tick, value: Value) -> Self {
        Self {
            id,
            property,
            time_tk,
            value,
            easing: Easing::Linear,
        }
    }
}

// ---------------------------------------------------------------------
// KeyframeSerde adapter
// ---------------------------------------------------------------------

/// Internal serde adapter shape — flat on-disk form per schema. The
/// `try_from` / `into` derives on [`Keyframe`] wire this in.
///
/// Carries both the easing literal and an optional bezier array;
/// [`Keyframe::try_from`] (via [`Easing`]) resolves the pair into a
/// typed [`Easing::CubicBezier`] variant or rejects when easing is
/// `cubic-bezier` without bezier (schema if/then violation).
///
/// The schema's deliberate v1 permissiveness — allowing a leftover
/// `bezier` under non-bezier easings (see schema description) — is
/// honored on deserialize: bezier under any non-bezier easing is
/// silently dropped (mirrors the engine's render-pipeline behavior
/// that "ignores bezier whenever easing != cubic-bezier"). Future
/// schema versions may tighten this to a hard rejection (§0.15);
/// when they do, this adapter will switch to erroring there.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyframeSerde {
    id: KeyframeId,
    property: KeyframeProperty,
    time_tk: Tick,
    value: Value,
    #[serde(default = "default_easing_str")]
    easing: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bezier: Option<[f64; 4]>,
}

fn default_easing_str() -> String {
    "linear".to_string()
}

impl TryFrom<KeyframeSerde> for Keyframe {
    type Error = KeyframeNewtypeError;
    fn try_from(s: KeyframeSerde) -> Result<Self, Self::Error> {
        let easing = match s.easing.as_str() {
            "linear" => Easing::Linear,
            "ease-in" => Easing::EaseIn,
            "ease-out" => Easing::EaseOut,
            "ease-in-out" => Easing::EaseInOut,
            "step" => Easing::Step,
            "cubic-bezier" => match s.bezier {
                Some(bezier) => Easing::CubicBezier { bezier },
                None => return Err(KeyframeNewtypeError::MissingBezier),
            },
            other => {
                return Err(KeyframeNewtypeError::UnknownEasing {
                    got: other.to_string(),
                });
            }
        };
        Ok(Keyframe {
            id: s.id,
            property: s.property,
            time_tk: s.time_tk,
            value: s.value,
            easing,
        })
    }
}

impl From<Keyframe> for KeyframeSerde {
    fn from(k: Keyframe) -> Self {
        let (easing_str, bezier) = match k.easing {
            Easing::Linear => ("linear".to_string(), None),
            Easing::EaseIn => ("ease-in".to_string(), None),
            Easing::EaseOut => ("ease-out".to_string(), None),
            Easing::EaseInOut => ("ease-in-out".to_string(), None),
            Easing::Step => ("step".to_string(), None),
            Easing::CubicBezier { bezier } => ("cubic-bezier".to_string(), Some(bezier)),
        };
        KeyframeSerde {
            id: k.id,
            property: k.property,
            time_tk: k.time_tk,
            value: k.value,
            easing: easing_str,
            bezier,
        }
    }
}
