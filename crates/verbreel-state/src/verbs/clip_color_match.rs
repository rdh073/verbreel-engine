//! `clip.color_match` (issue #479) — reference-frame color match via a
//! deterministic Reinhard mean/std transfer, emitted as an editable
//! `color_correct` grade.
//!
//! ## Behavior
//!
//! Given a **reference** frame's statistics and the **target** frame's
//! statistics (both supplied as args), computes a Reinhard mean/std
//! transfer in Oklab (see [`crate::color_dsp::color_match_grade`]) and
//! mints a `color_correct` effect on the target clip carrying the
//! derived params. Matching a frame to itself yields the identity grade.
//!
//! ## Why statistics arrive as args
//!
//! Same runtime-floor split as `clip.auto_color`: pure
//! [`crate::reconstructor::Verb::compute_patch`] must not decode frames.
//! The reference is *selected* upstream (by timeline frame or by asset);
//! the selector string is carried through to `data.reference` for
//! provenance, while the already-reduced reference statistics drive the
//! transfer math so the output is byte-deterministic.
//!
//! ## Selector
//!
//! `target` MUST be the qualified `clip:<UUIDv7>` form (spec §0.4).
//! `reference` is a free-form provenance string (e.g.
//! `frame:<clip>@<tk>` or `asset:<UUIDv7>`) and is not resolved by this
//! pure verb. Errors mirror `clip.auto_color`.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::{ClipId, EffectId, ProjectId};

use crate::color_dsp::{ChannelStats, GradeParams, color_match_grade};
use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use crate::verbs::color_grade::{build_grade_patch, grade_params_map, locate_grade_target};

/// Internal warning code carrying the minted effect id for replay.
pub const W_COLOR_MATCH_ENVELOPE_CODE: &str = "W_CLIP_COLOR_MATCH_ENVELOPE";

const SELECTOR_HINT: &str = "target must be qualified `clip:<UUIDv7>` (spec §0.4)";

/// Per-channel frame statistics carried in the `clip.color_match` args.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameStats {
    /// Mean of the red channel in `[0, 1]` sRGB.
    pub mean_r: f64,
    /// Mean of the green channel in `[0, 1]` sRGB.
    pub mean_g: f64,
    /// Mean of the blue channel in `[0, 1]` sRGB.
    pub mean_b: f64,
    /// Standard deviation of the red channel in `[0, 1]`.
    pub std_r: f64,
    /// Standard deviation of the green channel in `[0, 1]`.
    pub std_g: f64,
    /// Standard deviation of the blue channel in `[0, 1]`.
    pub std_b: f64,
    /// Low luma percentile in `[0, 1]`.
    ///
    /// Reserved: accepted and range-validated for shape-parity with
    /// `clip.auto_color`'s stats, but the Reinhard mean/std transfer in
    /// [`crate::color_dsp::color_match_grade`] consumes only the means and
    /// stds — the percentiles do not influence the emitted grade.
    #[serde(default)]
    pub black_point: f64,
    /// High luma percentile in `[0, 1]`.
    ///
    /// Reserved — see [`FrameStats::black_point`]; unused by the transfer.
    #[serde(default = "default_white_point")]
    pub white_point: f64,
}

fn default_white_point() -> f64 {
    1.0
}

impl From<FrameStats> for ChannelStats {
    fn from(value: FrameStats) -> Self {
        ChannelStats {
            mean_r: value.mean_r,
            mean_g: value.mean_g,
            mean_b: value.mean_b,
            std_r: value.std_r,
            std_g: value.std_g,
            std_b: value.std_b,
            black_point: value.black_point,
            white_point: value.white_point,
        }
    }
}

/// Args for `clip.color_match`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClipColorMatchArgs {
    /// Target project id.
    pub project_id: ProjectId,
    /// Qualified `clip:<UUIDv7>` selector for the clip to grade.
    pub target: String,
    /// Free-form provenance string naming the reference (timeline frame
    /// or asset). Not resolved by this pure verb.
    pub reference: String,
    /// Reference frame statistics (the look to match).
    pub reference_stats: FrameStats,
    /// Target frame statistics (the look to transform).
    pub target_stats: FrameStats,
}

/// Envelope `data` returned by `clip.color_match`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipColorMatchData {
    /// Freshly minted `color_correct` effect id.
    pub effect_id: EffectId,
    /// Target clip id.
    pub clip_id: ClipId,
    /// Reference provenance string echoed back.
    pub reference: String,
    /// Derived brightness param.
    pub brightness: f64,
    /// Derived contrast param.
    pub contrast: f64,
    /// Derived saturation param.
    pub saturation: f64,
    /// Derived temperature param.
    pub temperature: f64,
    /// Derived tint param.
    pub tint: f64,
}

impl ClipColorMatchData {
    fn from_grade(
        effect_id: EffectId,
        clip_id: ClipId,
        reference: String,
        grade: GradeParams,
    ) -> Self {
        Self {
            effect_id,
            clip_id,
            reference,
            brightness: grade.brightness,
            contrast: grade.contrast,
            saturation: grade.saturation,
            temperature: grade.temperature,
            tint: grade.tint,
        }
    }
}

/// Verb-level validation failures for `clip.color_match`.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum ClipColorMatchError {
    /// `args.target` is empty, unqualified, or uses a non-clip prefix.
    #[error("clip.color_match: E_BAD_SELECTOR — {detail}; hint: {hint}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
        /// Recovery hint.
        hint: &'static str,
    },

    /// `args.reference` is empty.
    #[error("clip.color_match: E_BAD_SELECTOR — reference selector is empty; hint: {hint}")]
    EmptyReference {
        /// Recovery hint.
        hint: &'static str,
    },

    /// No clip exists for `args.target`.
    #[error("clip.color_match: E_NOT_FOUND — clip `{clip_id}` not found")]
    NotFound {
        /// Missing clip id string.
        clip_id: String,
    },

    /// Target clip or parent track is locked.
    #[error("clip.color_match: E_LOCKED — {kind} `{id}` is locked")]
    Locked {
        /// Locked entity kind (`"clip"` or `"track"`).
        kind: &'static str,
        /// Locked entity id.
        id: String,
    },

    /// A supplied statistic is non-finite or out of its `[0, 1]` range.
    #[error("clip.color_match: E_BAD_RANGE — `{field}` value {value} out of range [{min}, {max}]")]
    BadRange {
        /// Offending field name.
        field: &'static str,
        /// Offending value.
        value: f64,
        /// Inclusive lower bound.
        min: f64,
        /// Inclusive upper bound.
        max: f64,
    },
}

fn parse_clip_target(target: &str) -> Result<ClipId, ClipColorMatchError> {
    let (prefix, body) =
        target
            .split_once(':')
            .ok_or_else(|| ClipColorMatchError::BadSelector {
                detail: format!("selector `{target}` is unqualified (missing `clip:` prefix)"),
                hint: SELECTOR_HINT,
            })?;
    if prefix != "clip" {
        return Err(ClipColorMatchError::BadSelector {
            detail: format!("unknown selector prefix `{prefix}`"),
            hint: SELECTOR_HINT,
        });
    }
    body.parse::<ClipId>()
        .map_err(|err| ClipColorMatchError::BadSelector {
            detail: format!("clip body parse failed: {err}"),
            hint: SELECTOR_HINT,
        })
}

fn validate_frame(prefix: &'static str, stats: &FrameStats) -> Result<(), ClipColorMatchError> {
    // The field labels carry the reference/target prefix so the caller
    // knows which stat block was malformed. `qualified_field` maps each
    // (prefix, field) pair to a single static label without allocation.
    let pairs: [(&'static str, f64); 8] = [
        ("mean_r", stats.mean_r),
        ("mean_g", stats.mean_g),
        ("mean_b", stats.mean_b),
        ("std_r", stats.std_r),
        ("std_g", stats.std_g),
        ("std_b", stats.std_b),
        ("black_point", stats.black_point),
        ("white_point", stats.white_point),
    ];
    for (field, value) in pairs {
        if !(value.is_finite() && (0.0..=1.0).contains(&value)) {
            return Err(ClipColorMatchError::BadRange {
                field: qualified_field(prefix, field),
                value,
                min: 0.0,
                max: 1.0,
            });
        }
    }
    Ok(())
}

/// Map a `(prefix, field)` pair to a single static label without
/// per-call allocation. Covers the full Cartesian product the two
/// validated frames can produce.
fn qualified_field(prefix: &str, field: &str) -> &'static str {
    match (prefix, field) {
        ("reference", "mean_r") => "reference_stats.mean_r",
        ("reference", "mean_g") => "reference_stats.mean_g",
        ("reference", "mean_b") => "reference_stats.mean_b",
        ("reference", "std_r") => "reference_stats.std_r",
        ("reference", "std_g") => "reference_stats.std_g",
        ("reference", "std_b") => "reference_stats.std_b",
        ("reference", "black_point") => "reference_stats.black_point",
        ("reference", "white_point") => "reference_stats.white_point",
        ("target", "mean_r") => "target_stats.mean_r",
        ("target", "mean_g") => "target_stats.mean_g",
        ("target", "mean_b") => "target_stats.mean_b",
        ("target", "std_r") => "target_stats.std_r",
        ("target", "std_g") => "target_stats.std_g",
        ("target", "std_b") => "target_stats.std_b",
        ("target", "black_point") => "target_stats.black_point",
        ("target", "white_point") => "target_stats.white_point",
        // `prefix` is a compile-time-fixed "reference" | "target" and
        // `field` ranges over the eight stat names in `validate_frame`;
        // the 16 arms above are exhaustive over that product. Any other
        // pair signals a new caller passing an unmapped prefix/field —
        // surface it loudly rather than mislabel the error field.
        (other_prefix, other_field) => unreachable!(
            "qualified_field: unmapped (prefix, field) = ({other_prefix}, {other_field})"
        ),
    }
}

/// Build the RFC-6902 patch for `clip.color_match`.
///
/// # Errors
///
/// Returns [`ClipColorMatchError`] for selector parse failure, empty
/// reference, missing clip, locked clip/track, or out-of-range
/// statistics.
pub fn compute_patch(
    prior: &Project,
    args: &ClipColorMatchArgs,
) -> Result<(Value, Vec<Value>, ClipColorMatchData), ClipColorMatchError> {
    if args.reference.trim().is_empty() {
        return Err(ClipColorMatchError::EmptyReference {
            hint: "reference must name a timeline frame or an asset",
        });
    }
    validate_frame("reference", &args.reference_stats)?;
    validate_frame("target", &args.target_stats)?;

    let clip_id = parse_clip_target(&args.target)?;
    let located =
        locate_grade_target(prior, clip_id).ok_or_else(|| ClipColorMatchError::NotFound {
            clip_id: clip_id.to_string(),
        })?;
    if located.clip_locked {
        return Err(ClipColorMatchError::Locked {
            kind: "clip",
            id: located.clip_id,
        });
    }
    if located.track_locked {
        return Err(ClipColorMatchError::Locked {
            kind: "track",
            id: located.track_id,
        });
    }

    let grade = color_match_grade(&args.reference_stats.into(), &args.target_stats.into());
    let params = grade_params_map(grade);
    let effect_id = EffectId::now();

    let patch = build_grade_patch(&located, effect_id, &params);
    let data = ClipColorMatchData::from_grade(effect_id, clip_id, args.reference.clone(), grade);
    let warnings = vec![envelope_warning(&data)];

    Ok((patch, warnings, data))
}

fn envelope_warning(data: &ClipColorMatchData) -> Value {
    json!({
        "code": W_COLOR_MATCH_ENVELOPE_CODE,
        "message": "clip.color_match envelope",
        "details": data,
    })
}

/// Rebuild [`ClipColorMatchData`] from the recorded envelope warning.
///
/// # Errors
///
/// Returns [`ReconstructError`] if the internal envelope warning is
/// missing or malformed.
pub fn data_envelope_from_warnings(
    warnings: &[Value],
) -> Result<ClipColorMatchData, ReconstructError> {
    for warning in warnings {
        if warning.get("code").and_then(Value::as_str) != Some(W_COLOR_MATCH_ENVELOPE_CODE) {
            continue;
        }
        let details = warning
            .get("details")
            .ok_or(ReconstructError::MissingField {
                name: "warnings[].W_CLIP_COLOR_MATCH_ENVELOPE.details",
            })?;
        return serde_json::from_value(details.clone()).map_err(|_| {
            ReconstructError::TypeMismatch {
                name: "warnings[].W_CLIP_COLOR_MATCH_ENVELOPE.details",
                expected: "ClipColorMatchData",
            }
        });
    }
    Err(ReconstructError::MissingField {
        name: "warnings[].W_CLIP_COLOR_MATCH_ENVELOPE",
    })
}

impl From<ClipColorMatchError> for VerbError {
    fn from(value: ClipColorMatchError) -> Self {
        VerbError::BadArgs {
            detail: value.to_string(),
        }
    }
}

/// `clip.color_match` verb registration entry.
#[derive(Debug, Default)]
pub struct ClipColorMatchVerb;

impl Verb for ClipColorMatchVerb {
    fn verb(&self) -> &'static str {
        "clip.color_match"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: ClipColorMatchArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("clip.color_match: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, _data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!(
                    "clip.color_match: patch construction failed: {err}"
                ))
            })?;

        let post_state = prior
            .apply(&patch)
            .map_err(|err| VerbError::InvariantViolation {
                detail: format!("clip.color_match: post-state validation failed: {err}"),
            })?;
        drop(post_state);

        // Intentional round-trip: `_data` is discarded and rebuilt from
        // the recorded warning so the write path exercises the exact
        // envelope decode that `reconstruct` (replay) relies on. A drift
        // between the in-memory `data` and the serialized warning surfaces
        // here at write time instead of silently at replay.
        let envelope = data_envelope_from_warnings(&warnings).map_err(|err| {
            VerbError::Custom(format!(
                "clip.color_match: data envelope reconstruction failed: {err}"
            ))
        })?;
        let data = serde_json::to_value(&envelope).map_err(|err| {
            VerbError::Custom(format!("clip.color_match: data serialize failed: {err}"))
        })?;

        Ok((patch, data, warnings))
    }

    fn reconstruct(
        &self,
        _args: &Value,
        _patch: &Value,
        warnings: &[Value],
        _post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let envelope = data_envelope_from_warnings(warnings)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
