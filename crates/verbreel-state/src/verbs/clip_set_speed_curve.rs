//! `clip.set_speed_curve` (§5.20) — fifty-second production verb in the engine.
//!
//! Sets or clears [`crate::clip::Clip::speed_curve`] on source-slice
//! clips. `None` serializes as JSON `null` and clears the curve.
//!
//! ## v1.1 deferrals
//!
//! This slice is setter-only. The closed-form integration of the
//! piecewise-linear curve is deferred, so `effective_duration_tk` uses
//! the scalar v1.0 formula `ceil((source_out_tk - source_in_tk) /
//! Clip.speed)`. The fade clamp cascade (`W_FADE_CLAMPED`), keyframe
//! overflow cascade (`W_KEYFRAMES_REMOVED`), effect window clamp cascade
//! (`W_EFFECT_WINDOW_CLAMPED`), `W_SPEED_CURVE_EXTREME` emission,
//! `E_BAD_TIME` overflow guard on integrated duration, `E_CLIP_OVERLAP`
//! post-integration check, and `time_stretch` managed-effect coupling
//! are also deferred to the integration arm.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::{ClipId, ProjectId};

use crate::asset::Asset;
use crate::clip::{Clip, SpeedCurvePoint};
use crate::invariants::timeline_duration_tk;
use crate::newtypes::AssetRef;
use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use crate::track::TrackKind;

/// Warning code emitted when the requested curve equals current state.
pub const W_NOOP_CODE: &str = "W_NOOP";

const MIN_POINT_COUNT: usize = 2;
const MAX_POINT_COUNT: usize = 256;
const POINT_COUNT_BOUND: &str = "[2, 256]";
const MIN_FACTOR: f64 = 0.001;
const MAX_FACTOR: f64 = 100.0;
const FACTOR_BOUND: &str = "[0.001, 100]";

/// Arguments for `clip.set_speed_curve`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipSetSpeedCurveArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Target clip id as bare `UUIDv7`.
    pub clip: String,

    /// New speed-curve value. `None` serializes as JSON `null` and clears it.
    pub points: Option<Vec<SpeedCurvePoint>>,
}

/// Envelope returned by `clip.set_speed_curve`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipSetSpeedCurveData {
    /// Target clip id.
    pub clip_id: ClipId,

    /// Post-state speed curve.
    pub speed_curve: Option<Vec<SpeedCurvePoint>>,

    /// Post-state effective duration. Setter-only v1.1 slice uses scalar v1.0 formula.
    pub effective_duration_tk: i64,

    /// Reserved for future cross-clip speed-curve sync. Always empty in this slice.
    pub linked_audio_clip_ids: Vec<ClipId>,
}

/// Verb-level validation failures for `clip.set_speed_curve`.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum ClipSetSpeedCurveError {
    /// `args.clip` is not parseable as `UUIDv7`.
    #[error("E_BAD_SELECTOR: clip.set_speed_curve: `clip` selector parse failed: {detail}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
    },

    /// No clip exists for `clip`.
    #[error("E_NOT_FOUND: clip.set_speed_curve: clip `{clip_id}` not found")]
    ClipNotFound {
        /// Missing clip id string.
        clip_id: String,
    },

    /// Target clip has display-duration semantics.
    #[error(
        "E_CLIP_KIND_MISMATCH: clip.set_speed_curve: clip `{clip_id}` is `{actual_kind}`, expected source-slice clip"
    )]
    ClipKindMismatch {
        /// Target clip id.
        clip_id: ClipId,
        /// Actual semantic kind.
        actual_kind: &'static str,
    },

    /// The target clip or its parent track is locked.
    #[error("E_LOCKED: clip.set_speed_curve: {kind} `{id}` is locked for clip `{clip_id}`")]
    Locked {
        /// Locked entity kind.
        kind: &'static str,
        /// Locked entity id.
        id: String,
        /// Target clip id string.
        clip_id: String,
    },

    /// `points.length` is outside `[2, 256]`.
    #[error("E_BAD_SPEED_CURVE: clip.set_speed_curve: points length {length} must satisfy {bound}")]
    BadSpeedCurveLength {
        /// Details violation discriminator.
        violation: &'static str,
        /// Actual point count.
        length: usize,
        /// Human-readable bound.
        bound: &'static str,
    },

    /// A point factor is outside `[0.001, 100]`.
    #[error(
        "E_BAD_SPEED_CURVE: clip.set_speed_curve: points[{index}].factor {factor} must satisfy {bound}"
    )]
    BadSpeedCurveFactor {
        /// Details violation discriminator.
        violation: &'static str,
        /// Offending point index.
        index: usize,
        /// Offending factor.
        factor: f64,
        /// Human-readable bound.
        bound: &'static str,
    },

    /// A point time is outside the source-relative domain.
    #[error(
        "E_BAD_SPEED_CURVE: clip.set_speed_curve: points[{index}].time_tk {time_tk} must satisfy {bound}"
    )]
    BadSpeedCurveTime {
        /// Details violation discriminator.
        violation: &'static str,
        /// Offending point index.
        index: usize,
        /// Offending time.
        time_tk: i64,
        /// Human-readable bound.
        bound: String,
    },

    /// Point times are not strictly ascending.
    #[error(
        "E_BAD_SPEED_CURVE: clip.set_speed_curve: points[{index}].time_tk {time_tk} must be greater than previous {previous_time_tk}"
    )]
    BadSpeedCurveMonotonic {
        /// Details violation discriminator.
        violation: &'static str,
        /// Offending point index.
        index: usize,
        /// Offending time.
        time_tk: i64,
        /// Previous point time.
        previous_time_tk: i64,
    },
}

#[derive(Debug, Clone)]
struct LocatedClip<'a> {
    track_idx: usize,
    clip_idx: usize,
    track_kind: TrackKind,
    track_locked: bool,
    track_id: String,
    clip: &'a Clip,
}

/// Build the RFC-6902 patch for `clip.set_speed_curve`.
///
/// # Errors
///
/// Returns [`ClipSetSpeedCurveError`] for selector parse failure, missing
/// clip, display-duration clip kind, locked target, or invalid curve
/// points.
pub fn compute_patch(
    prior: &Project,
    args: &ClipSetSpeedCurveArgs,
) -> Result<(Value, Vec<Value>, ClipSetSpeedCurveData), ClipSetSpeedCurveError> {
    let clip_id =
        args.clip
            .parse::<ClipId>()
            .map_err(|err| ClipSetSpeedCurveError::BadSelector {
                detail: err.to_string(),
            })?;

    let located =
        locate_clip(prior, clip_id).ok_or_else(|| ClipSetSpeedCurveError::ClipNotFound {
            clip_id: args.clip.clone(),
        })?;

    if let Some(actual_kind) =
        display_duration_kind(prior, located.track_kind, &located.clip.asset_id)
    {
        return Err(ClipSetSpeedCurveError::ClipKindMismatch {
            clip_id,
            actual_kind,
        });
    }

    if located.track_locked {
        return Err(ClipSetSpeedCurveError::Locked {
            kind: "track",
            id: located.track_id.clone(),
            clip_id: args.clip.clone(),
        });
    }

    if located.clip.locked {
        return Err(ClipSetSpeedCurveError::Locked {
            kind: "clip",
            id: located.clip.id.to_string(),
            clip_id: args.clip.clone(),
        });
    }

    if let Some(points) = args.points.as_ref() {
        validate_points(points, located.clip)?;
    }

    let data = ClipSetSpeedCurveData {
        clip_id,
        speed_curve: args.points.clone(),
        effective_duration_tk: scalar_effective_duration_tk(located.clip),
        linked_audio_clip_ids: Vec::new(),
    };

    if args.points == located.clip.speed_curve {
        return Ok((
            json!([]),
            vec![json!({
                "code": W_NOOP_CODE,
                "message": "clip.set_speed_curve no-op",
                "details": {
                    "clip_id": clip_id.to_string(),
                }
            })],
            data,
        ));
    }

    let path = format!(
        "/tracks/{}/clips/{}/speed_curve",
        located.track_idx, located.clip_idx
    );
    let mut ops = Vec::new();
    if located.clip.speed_curve.is_none() && args.points.is_some() {
        ops.push(json!({
            "op": "add",
            "path": path.clone(),
            "value": Value::Null,
        }));
    }
    ops.push(json!({
        "op": "replace",
        "path": path,
        "value": args.points,
    }));

    Ok((Value::Array(ops), Vec::new(), data))
}

fn locate_clip(prior: &Project, clip_id: ClipId) -> Option<LocatedClip<'_>> {
    for (track_idx, track) in prior.tracks.iter().enumerate() {
        for (clip_idx, clip) in track.clips.iter().enumerate() {
            if clip.id == clip_id {
                return Some(LocatedClip {
                    track_idx,
                    clip_idx,
                    track_kind: track.kind,
                    track_locked: track.locked,
                    track_id: track.id.to_string(),
                    clip,
                });
            }
        }
    }
    None
}

fn display_duration_kind(
    project: &Project,
    track_kind: TrackKind,
    asset_id: &AssetRef,
) -> Option<&'static str> {
    if track_kind == TrackKind::Text {
        return Some("text");
    }

    asset_id.id().and_then(|asset_id| {
        project.assets.iter().find_map(|asset| {
            (asset.id() == asset_id && matches!(asset, Asset::Image(_))).then_some("image")
        })
    })
}

fn validate_points(points: &[SpeedCurvePoint], clip: &Clip) -> Result<(), ClipSetSpeedCurveError> {
    if !(MIN_POINT_COUNT..=MAX_POINT_COUNT).contains(&points.len()) {
        return Err(ClipSetSpeedCurveError::BadSpeedCurveLength {
            violation: "length",
            length: points.len(),
            bound: POINT_COUNT_BOUND,
        });
    }

    let max_tk = clip
        .source_out_tk
        .get()
        .saturating_sub(clip.source_in_tk.get());
    let time_bound = format!("[0, {max_tk}]");
    for (index, point) in points.iter().enumerate() {
        if !(MIN_FACTOR..=MAX_FACTOR).contains(&point.factor) {
            return Err(ClipSetSpeedCurveError::BadSpeedCurveFactor {
                violation: "factor_out_of_range",
                index,
                factor: point.factor,
                bound: FACTOR_BOUND,
            });
        }

        let time_tk = point.time_tk.get();
        if !(0..=max_tk).contains(&time_tk) {
            return Err(ClipSetSpeedCurveError::BadSpeedCurveTime {
                violation: "time_tk_out_of_range",
                index,
                time_tk,
                bound: time_bound,
            });
        }

        if index > 0 {
            let previous_time_tk = points[index - 1].time_tk.get();
            if time_tk <= previous_time_tk {
                return Err(ClipSetSpeedCurveError::BadSpeedCurveMonotonic {
                    violation: "monotonic",
                    index,
                    time_tk,
                    previous_time_tk,
                });
            }
        }
    }

    Ok(())
}

fn scalar_effective_duration_tk(clip: &Clip) -> i64 {
    timeline_duration_tk(clip.source_in_tk, clip.source_out_tk, clip.speed).get()
}

/// Rebuilds the envelope from `(args, post_state)`.
///
/// # Errors
/// Returns [`ReconstructError::TypeMismatch`] when `args.clip` is not
/// a valid `UUIDv7`, or [`ReconstructError::PostStateMissing`] when the
/// post-state does not contain the target clip.
pub fn data_envelope_from_post_state(
    args: &ClipSetSpeedCurveArgs,
    post_state: &Project,
) -> Result<ClipSetSpeedCurveData, ReconstructError> {
    let clip_id = args
        .clip
        .parse::<ClipId>()
        .map_err(|_| ReconstructError::TypeMismatch {
            name: "args.clip",
            expected: "UUIDv7 ClipId string",
        })?;

    for track in &post_state.tracks {
        for clip in &track.clips {
            if clip.id == clip_id {
                return Ok(ClipSetSpeedCurveData {
                    clip_id,
                    speed_curve: clip.speed_curve.clone(),
                    effective_duration_tk: scalar_effective_duration_tk(clip),
                    linked_audio_clip_ids: Vec::new(),
                });
            }
        }
    }

    Err(ReconstructError::PostStateMissing {
        detail: format!("clip.set_speed_curve: clip id {clip_id} not found in post_state"),
    })
}

impl From<ClipSetSpeedCurveError> for VerbError {
    fn from(value: ClipSetSpeedCurveError) -> Self {
        VerbError::BadArgs {
            detail: value.to_string(),
        }
    }
}

/// `clip.set_speed_curve` verb registration entry.
#[derive(Debug, Default)]
pub struct ClipSetSpeedCurveVerb;

impl Verb for ClipSetSpeedCurveVerb {
    fn verb(&self) -> &'static str {
        "clip.set_speed_curve"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: ClipSetSpeedCurveArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("clip.set_speed_curve: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, _data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!(
                    "clip.set_speed_curve: patch construction failed: {err}"
                ))
            })?;

        let post_state = prior
            .apply(&patch)
            .map_err(|err| VerbError::InvariantViolation {
                detail: format!("clip.set_speed_curve: post-state validation failed: {err}"),
            })?;

        let envelope = data_envelope_from_post_state(&typed, &post_state).map_err(|err| {
            VerbError::Custom(format!(
                "clip.set_speed_curve: data envelope reconstruction failed: {err}"
            ))
        })?;
        let data = serde_json::to_value(&envelope).map_err(|err| {
            VerbError::Custom(format!(
                "clip.set_speed_curve: data serialize failed: {err}"
            ))
        })?;

        Ok((patch, data, warnings))
    }

    fn reconstruct(
        &self,
        args: &Value,
        _patch: &Value,
        _warnings: &[Value],
        post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let typed: ClipSetSpeedCurveArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "ClipSetSpeedCurveArgs",
            })?;

        let envelope = data_envelope_from_post_state(&typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
