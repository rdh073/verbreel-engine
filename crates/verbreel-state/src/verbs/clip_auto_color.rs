//! `clip.auto_color` (issue #479) — one-click auto white-balance +
//! auto-levels as an editable `color_correct` grade.
//!
//! ## Behavior
//!
//! Given a representative frame's per-channel statistics (mean/std +
//! black/white percentiles, supplied as args), computes a deterministic
//! gray-world white-balance + level-normalization grade and mints a
//! `color_correct` effect on the target clip carrying the derived
//! brightness / contrast / saturation / temperature / tint params. The
//! result is an editable grade, not a black box — the operator can
//! tweak any param afterwards with `effect.set_param`.
//!
//! ## Why statistics arrive as args
//!
//! [`crate::reconstructor::Verb::compute_patch`] is pure and must not
//! decode frames or touch the filesystem — the same runtime-floor split
//! `audio.analyze` / `tracker.run` / `asset.probe` observe. Frame decode
//! and per-pixel reduction are a separate runtime concern; this verb
//! consumes the already-reduced statistics so its output is byte-
//! deterministic given identical input. See [`crate::color_dsp`] for the
//! grade math and the param conventions.
//!
//! ## Selector
//!
//! `target` MUST be the qualified `clip:<UUIDv7>` form (spec §0.4). A
//! bare or unknown-prefix selector is rejected with `E_BAD_SELECTOR`; a
//! missing clip with `E_NOT_FOUND`; a locked clip or parent track with
//! `E_LOCKED`.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use thiserror::Error;
use verbreel_types::{ClipId, EffectId, ProjectId};

use crate::color_dsp::{ChannelStats, GradeParams, auto_color_grade};
use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use crate::verbs::color_grade::{
    GRADE_KIND, build_grade_patch, grade_params_map, locate_grade_target,
};

/// Internal warning code carrying the minted effect id for replay.
pub const W_AUTO_COLOR_ENVELOPE_CODE: &str = "W_CLIP_AUTO_COLOR_ENVELOPE";

const SELECTOR_HINT: &str = "target must be qualified `clip:<UUIDv7>` (spec §0.4)";

/// Args for `clip.auto_color`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClipAutoColorArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Qualified `clip:<UUIDv7>` selector.
    pub target: String,

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
    /// Low luma percentile in `[0, 1]` mapped to 0 by the levels stretch.
    pub black_point: f64,
    /// High luma percentile in `[0, 1]` mapped to 1 by the levels stretch.
    pub white_point: f64,
}

impl ClipAutoColorArgs {
    fn channel_stats(&self) -> ChannelStats {
        ChannelStats {
            mean_r: self.mean_r,
            mean_g: self.mean_g,
            mean_b: self.mean_b,
            std_r: self.std_r,
            std_g: self.std_g,
            std_b: self.std_b,
            black_point: self.black_point,
            white_point: self.white_point,
        }
    }
}

/// Envelope `data` returned by `clip.auto_color`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipAutoColorData {
    /// Freshly minted `color_correct` effect id.
    pub effect_id: EffectId,
    /// Target clip id.
    pub clip_id: ClipId,
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

impl ClipAutoColorData {
    fn from_grade(effect_id: EffectId, clip_id: ClipId, grade: GradeParams) -> Self {
        Self {
            effect_id,
            clip_id,
            brightness: grade.brightness,
            contrast: grade.contrast,
            saturation: grade.saturation,
            temperature: grade.temperature,
            tint: grade.tint,
        }
    }
}

/// Verb-level validation failures for `clip.auto_color`.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum ClipAutoColorError {
    /// `args.target` is empty, unqualified, or uses a non-clip prefix.
    #[error("clip.auto_color: E_BAD_SELECTOR — {detail}; hint: {hint}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
        /// Recovery hint.
        hint: &'static str,
    },

    /// No clip exists for `args.target`.
    #[error("clip.auto_color: E_NOT_FOUND — clip `{clip_id}` not found")]
    NotFound {
        /// Missing clip id string.
        clip_id: String,
    },

    /// Target clip or parent track is locked.
    #[error("clip.auto_color: E_LOCKED — {kind} `{id}` is locked")]
    Locked {
        /// Locked entity kind (`"clip"` or `"track"`).
        kind: &'static str,
        /// Locked entity id.
        id: String,
    },

    /// A supplied statistic is non-finite or out of its `[0, 1]` range.
    #[error("clip.auto_color: E_BAD_RANGE — `{field}` value {value} out of range [{min}, {max}]")]
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

fn parse_clip_target(target: &str) -> Result<ClipId, ClipAutoColorError> {
    let (prefix, body) = target
        .split_once(':')
        .ok_or_else(|| ClipAutoColorError::BadSelector {
            detail: format!("selector `{target}` is unqualified (missing `clip:` prefix)"),
            hint: SELECTOR_HINT,
        })?;
    if prefix != "clip" {
        return Err(ClipAutoColorError::BadSelector {
            detail: format!("unknown selector prefix `{prefix}`"),
            hint: SELECTOR_HINT,
        });
    }
    body.parse::<ClipId>()
        .map_err(|err| ClipAutoColorError::BadSelector {
            detail: format!("clip body parse failed: {err}"),
            hint: SELECTOR_HINT,
        })
}

fn check_unit(field: &'static str, value: f64) -> Result<(), ClipAutoColorError> {
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        Ok(())
    } else {
        Err(ClipAutoColorError::BadRange {
            field,
            value,
            min: 0.0,
            max: 1.0,
        })
    }
}

fn validate_stats(args: &ClipAutoColorArgs) -> Result<(), ClipAutoColorError> {
    check_unit("mean_r", args.mean_r)?;
    check_unit("mean_g", args.mean_g)?;
    check_unit("mean_b", args.mean_b)?;
    check_unit("std_r", args.std_r)?;
    check_unit("std_g", args.std_g)?;
    check_unit("std_b", args.std_b)?;
    check_unit("black_point", args.black_point)?;
    check_unit("white_point", args.white_point)?;
    if args.white_point < args.black_point {
        return Err(ClipAutoColorError::BadRange {
            field: "white_point",
            value: args.white_point,
            min: args.black_point,
            max: 1.0,
        });
    }
    Ok(())
}

/// Build the RFC-6902 patch for `clip.auto_color`.
///
/// # Errors
///
/// Returns [`ClipAutoColorError`] for selector parse failure, missing
/// clip, locked clip/track, or out-of-range statistics.
pub fn compute_patch(
    prior: &Project,
    args: &ClipAutoColorArgs,
) -> Result<(Value, Vec<Value>, ClipAutoColorData), ClipAutoColorError> {
    validate_stats(args)?;
    let clip_id = parse_clip_target(&args.target)?;

    let located =
        locate_grade_target(prior, clip_id).ok_or_else(|| ClipAutoColorError::NotFound {
            clip_id: clip_id.to_string(),
        })?;
    if located.clip_locked {
        return Err(ClipAutoColorError::Locked {
            kind: "clip",
            id: located.clip_id,
        });
    }
    if located.track_locked {
        return Err(ClipAutoColorError::Locked {
            kind: "track",
            id: located.track_id,
        });
    }

    let grade = auto_color_grade(&args.channel_stats());
    let params = grade_params_map(grade);
    let effect_id = EffectId::now();

    let patch = build_grade_patch(&located, effect_id, &params);
    let data = ClipAutoColorData::from_grade(effect_id, clip_id, grade);
    let warnings = vec![envelope_warning(&data)];

    Ok((patch, warnings, data))
}

fn envelope_warning(data: &ClipAutoColorData) -> Value {
    json!({
        "code": W_AUTO_COLOR_ENVELOPE_CODE,
        "message": "clip.auto_color envelope",
        "details": data,
    })
}

/// Rebuild [`ClipAutoColorData`] from the recorded envelope warning.
///
/// # Errors
///
/// Returns [`ReconstructError`] if the internal envelope warning is
/// missing or malformed.
pub fn data_envelope_from_warnings(
    warnings: &[Value],
) -> Result<ClipAutoColorData, ReconstructError> {
    for warning in warnings {
        if warning.get("code").and_then(Value::as_str) != Some(W_AUTO_COLOR_ENVELOPE_CODE) {
            continue;
        }
        let details = warning
            .get("details")
            .ok_or(ReconstructError::MissingField {
                name: "warnings[].W_CLIP_AUTO_COLOR_ENVELOPE.details",
            })?;
        return serde_json::from_value(details.clone()).map_err(|_| {
            ReconstructError::TypeMismatch {
                name: "warnings[].W_CLIP_AUTO_COLOR_ENVELOPE.details",
                expected: "ClipAutoColorData",
            }
        });
    }
    Err(ReconstructError::MissingField {
        name: "warnings[].W_CLIP_AUTO_COLOR_ENVELOPE",
    })
}

impl From<ClipAutoColorError> for VerbError {
    fn from(value: ClipAutoColorError) -> Self {
        VerbError::BadArgs {
            detail: value.to_string(),
        }
    }
}

/// `clip.auto_color` verb registration entry.
#[derive(Debug, Default)]
pub struct ClipAutoColorVerb;

impl Verb for ClipAutoColorVerb {
    fn verb(&self) -> &'static str {
        "clip.auto_color"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: ClipAutoColorArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("clip.auto_color: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, _data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!("clip.auto_color: patch construction failed: {err}"))
            })?;

        let post_state = prior
            .apply(&patch)
            .map_err(|err| VerbError::InvariantViolation {
                detail: format!("clip.auto_color: post-state validation failed: {err}"),
            })?;
        drop(post_state);

        let envelope = data_envelope_from_warnings(&warnings).map_err(|err| {
            VerbError::Custom(format!(
                "clip.auto_color: data envelope reconstruction failed: {err}"
            ))
        })?;
        let data = serde_json::to_value(&envelope).map_err(|err| {
            VerbError::Custom(format!("clip.auto_color: data serialize failed: {err}"))
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

/// The `color_correct` kind both color verbs emit. Re-exported so callers
/// and tests can assert no new bundled kind is introduced.
pub const EMITTED_EFFECT_KIND: &str = GRADE_KIND;

/// Build the params map for a grade (re-export for tests).
#[must_use]
pub fn params_for(grade: GradeParams) -> Map<String, Value> {
    grade_params_map(grade)
}
