//! `clip.set_speed_curve` (§5.20) — fifty-second production verb in the engine.
//!
//! Sets or clears [`crate::clip::Clip::speed_curve`] on source-slice
//! clips. `None` serializes as JSON `null` and clears the curve, reverting
//! the clip to the v1.0 scalar-speed duration formula.
//!
//! ## Integration coverage (PR closing #330)
//!
//! The v1.1-additive speed-ramp field is integrated into the clip's
//! effective `timeline_duration_tk` at patch-computation time per the
//! §5.20 closed-form integral. The new duration drives the standard
//! duration-shortening cascade from §5.7 maintenance:
//!
//! - **Integral** (`integrate_speed_curve` in [`crate::invariants`]):
//!   piecewise-linear over the source range with boundary-held points,
//!   summed in source-tick ascending order so the result is byte-stable
//!   across implementations.
//! - **`E_BAD_TIME`** — fired when the integrated duration would
//!   overflow `Number.MAX_SAFE_INTEGER`; `details.field: "speed_curve"`,
//!   `details.computed_duration_tk` carry the pre-`ceil` value.
//! - **`E_CLIP_OVERLAP`** — post-integration scan of the target track:
//!   rejects when the new range collides with a sibling. Clearing
//!   `points: None` is overlap-checked only when the resulting duration
//!   exceeds the previous duration (purely shortening clears never
//!   trigger overlap).
//! - **`W_FADE_CLAMPED`** — fades clamp proportionally when
//!   `fade_in_tk + fade_out_tk > new_duration_tk`.
//! - **`W_KEYFRAMES_REMOVED`** — keyframes with `time_tk >
//!   new_duration_tk` are removed in the same patch.
//! - **`W_EFFECT_WINDOW_CLAMPED`** — effect windows whose `out_tk >
//!   new_duration_tk` clamp to the new duration; effects with `in_tk >=
//!   new_duration_tk` are removed entirely. Dangling effect-targeting
//!   keyframes are cascade-removed with a second
//!   `W_KEYFRAMES_REMOVED` per §6.2 dangling-keyframe rule.
//! - **`W_SPEED_CURVE_EXTREME`** — when
//!   `max(Clip.speed * point.factor) > 16` AND a `time_stretch` effect
//!   is attached, emits informational quality warning with
//!   `details.segment_index` naming the offending control point.
//!
//! ## Out of scope (still deferred — tracked separately)
//!
//! - `time_stretch` managed-effect creation / removal on curve set or
//!   clear (the `--preserve_pitch` half of §5.7's resolution table).
//! - `time_stretch.params.factor` per-tick render-time coupling (lives
//!   in the render pipeline, not the engine).
//! - Cross-clip speed-curve propagation across link groups (deferred to
//!   v1.x per §5.20 "Linked clips").
//! - `clip.split` curve partition with inverse-integral lookup (own
//!   slice).
//! - `linked_audio_clip_ids[]` is always `[]` at v1.1 per spec.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::{ClipId, EffectId, KeyframeId, ProjectId, Tick};

use crate::asset::Asset;
use crate::clip::{Clip, SpeedCurvePoint};
use crate::effect::Effect;
use crate::invariants::{
    IntegrationOverflow, clip_timeline_duration_tk, extract_effect_id_from_property,
    integrate_speed_curve, timeline_duration_tk,
};
use crate::newtypes::AssetRef;
use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use crate::track::TrackKind;

/// Warning code emitted when the requested curve equals current state.
pub const W_NOOP_CODE: &str = "W_NOOP";

/// Warning code emitted when fades are clamped to the new duration.
pub const W_FADE_CLAMPED_CODE: &str = "W_FADE_CLAMPED";

/// Warning code emitted when keyframes are removed.
pub const W_KEYFRAMES_REMOVED_CODE: &str = "W_KEYFRAMES_REMOVED";

/// Warning code emitted when an effect window is clamped or the effect
/// is removed because its window collapsed past the new duration.
pub const W_EFFECT_WINDOW_CLAMPED_CODE: &str = "W_EFFECT_WINDOW_CLAMPED";

/// Warning code emitted when a curve drives an attached `time_stretch`
/// effect at a max effective factor above 16.
pub const W_SPEED_CURVE_EXTREME_CODE: &str = "W_SPEED_CURVE_EXTREME";

/// Quality-warning threshold from §5.20.
const EXTREME_FACTOR_THRESHOLD: f64 = 16.0;

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

    /// Post-state effective timeline duration in ticks. Curve-aware:
    /// when `speed_curve.is_some()`, this is the §5.20 closed-form
    /// integral; otherwise the v1.0 scalar formula.
    pub effective_duration_tk: i64,

    /// Reserved for v1.x cross-clip speed-curve sync. Always empty at
    /// v1.1 per spec.
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

    /// Closed-form integration of the curve produced a duration that
    /// would overflow `Number.MAX_SAFE_INTEGER`.
    #[error(
        "E_BAD_TIME: clip.set_speed_curve: integrated `{field}` for clip `{clip_id}` overflows MAX_SAFE_INTEGER (computed={computed_duration_tk})"
    )]
    BadTime {
        /// Invalid field name — always `"speed_curve"` for this verb.
        field: &'static str,
        /// Failed clip id.
        clip_id: String,
        /// Pre-`ceil` integrated duration that caused the overflow.
        computed_duration_tk: f64,
    },

    /// The post-integration timeline range collides with a sibling clip
    /// on the target's track.
    #[error(
        "E_CLIP_OVERLAP: clip.set_speed_curve: clip `{failed_clip}` would overlap on its track"
    )]
    ClipOverlap {
        /// Target clip id (overlap is per §5.20 attributable to the
        /// target — not a sibling).
        failed_clip: String,
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

/// Resolved cascade: the new clip state plus the per-warning evidence
/// needed to emit the §5.20 warning set.
#[derive(Debug, Clone)]
struct CurvePlan {
    /// Cloned clip with `speed_curve`, fades, keyframes, and effects
    /// already mutated to the post-cascade state.
    clip: Clip,
    /// Post-integration timeline duration.
    new_duration_tk: i64,
}

/// Build the RFC-6902 patch for `clip.set_speed_curve`.
///
/// # Errors
///
/// Returns [`ClipSetSpeedCurveError`] for selector parse failure, missing
/// clip, display-duration clip kind, locked target, invalid curve
/// points, integrated-duration overflow (`E_BAD_TIME`), or
/// post-integration sibling overlap (`E_CLIP_OVERLAP`).
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

    // W_NOOP — same curve as before. Skip integration entirely.
    if args.points == located.clip.speed_curve {
        let data = ClipSetSpeedCurveData {
            clip_id,
            speed_curve: args.points.clone(),
            effective_duration_tk: clip_timeline_duration_tk(located.clip).get(),
            linked_audio_clip_ids: Vec::new(),
        };
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

    let prior_duration_tk = clip_timeline_duration_tk(located.clip).get();
    let new_duration_tk = compute_new_duration_tk(located.clip, args.points.as_deref(), clip_id)?;

    let mut warnings = Vec::new();
    let plan = build_plan(
        located.clip,
        args.points.clone(),
        new_duration_tk,
        &mut warnings,
    );

    // Overlap re-check: the post-integration range must not collide
    // with siblings on the target's track. Clearing the curve to a
    // strictly shorter (or equal) duration cannot create new
    // collisions, so skip the scan in that case.
    if new_duration_tk > prior_duration_tk {
        check_planned_overlap(prior, &located, new_duration_tk)?;
    }

    // Extreme-rate quality warning. Reads the prior effect set since
    // the warning's clip_id is the target and the effects haven't been
    // touched yet by the cascade besides removal-on-collapse.
    warn_extreme_curve(located.clip, args.points.as_deref(), &mut warnings);

    let data = ClipSetSpeedCurveData {
        clip_id,
        speed_curve: plan.clip.speed_curve.clone(),
        effective_duration_tk: new_duration_tk,
        linked_audio_clip_ids: Vec::new(),
    };

    let patch = build_patch(prior, &located, &plan);
    Ok((patch, warnings, data))
}

fn compute_new_duration_tk(
    clip: &Clip,
    points: Option<&[SpeedCurvePoint]>,
    clip_id: ClipId,
) -> Result<i64, ClipSetSpeedCurveError> {
    match points {
        Some(points) => {
            integrate_speed_curve(clip.source_in_tk, clip.source_out_tk, clip.speed, points)
                .map_err(|err: IntegrationOverflow| ClipSetSpeedCurveError::BadTime {
                    field: "speed_curve",
                    clip_id: clip_id.to_string(),
                    computed_duration_tk: err.computed,
                })
        }
        None => Ok(timeline_duration_tk(clip.source_in_tk, clip.source_out_tk, clip.speed).get()),
    }
}

fn build_plan(
    prior_clip: &Clip,
    new_points: Option<Vec<SpeedCurvePoint>>,
    new_duration_tk: i64,
    warnings: &mut Vec<Value>,
) -> CurvePlan {
    let mut clip = prior_clip.clone();
    clip.speed_curve = new_points;
    clamp_fades(prior_clip, &mut clip, new_duration_tk, warnings);
    remove_overflow_keyframes(prior_clip, &mut clip, new_duration_tk, warnings);
    clamp_or_remove_effect_windows(prior_clip, &mut clip, new_duration_tk, warnings);

    CurvePlan {
        clip,
        new_duration_tk,
    }
}

fn clamp_fades(old_clip: &Clip, clip: &mut Clip, new_duration_tk: i64, warnings: &mut Vec<Value>) {
    let old_fade_in = old_clip.fade_in_tk.get();
    let old_fade_out = old_clip.fade_out_tk.get();
    let fade_sum = old_fade_in.saturating_add(old_fade_out);
    if fade_sum == 0 || fade_sum <= new_duration_tk {
        return;
    }

    let new_fade_in = ((i128::from(new_duration_tk) * i128::from(old_fade_in))
        / i128::from(fade_sum))
    .try_into()
    .unwrap_or(i64::MAX);
    let new_fade_out = new_duration_tk.saturating_sub(new_fade_in);
    clip.fade_in_tk = Tick::new(new_fade_in);
    clip.fade_out_tk = Tick::new(new_fade_out);
    warnings.push(json!({
        "code": W_FADE_CLAMPED_CODE,
        "message": "clip fades clamped to fit curve-adjusted duration",
        "details": {
            "clip_id": clip.id.to_string(),
            "from_in_tk": old_fade_in,
            "from_out_tk": old_fade_out,
            "to_in_tk": new_fade_in,
            "to_out_tk": new_fade_out,
        }
    }));
}

fn remove_overflow_keyframes(
    old_clip: &Clip,
    clip: &mut Clip,
    new_duration_tk: i64,
    warnings: &mut Vec<Value>,
) {
    let mut removed = Vec::new();
    let mut filtered = Vec::with_capacity(old_clip.keyframes.len());

    for keyframe in &old_clip.keyframes {
        if keyframe.time_tk.get() > new_duration_tk {
            removed.push(keyframe.id);
        } else {
            filtered.push(keyframe.clone());
        }
    }

    if removed.is_empty() {
        return;
    }

    removed.sort_by_key(ToString::to_string);
    clip.keyframes = filtered;
    warnings.push(keyframes_removed_warning(
        clip.id,
        &removed,
        "clip keyframes beyond the curve-adjusted duration were removed",
    ));
}

fn clamp_or_remove_effect_windows(
    old_clip: &Clip,
    clip: &mut Clip,
    new_duration_tk: i64,
    warnings: &mut Vec<Value>,
) {
    let mut effects = Vec::with_capacity(old_clip.effects.len());
    let mut removed_effect_ids = Vec::new();
    let mut changed = false;

    for effect in &old_clip.effects {
        let mut next = effect.clone();
        let Some(mut window) = next.window else {
            effects.push(next);
            continue;
        };

        if window.in_tk.get() >= new_duration_tk {
            removed_effect_ids.push(effect.id);
            changed = true;
            warnings.push(json!({
                "code": W_EFFECT_WINDOW_CLAMPED_CODE,
                "message": "effect removed because its window started past curve-adjusted duration",
                "details": {
                    "effect_id": effect.id.to_string(),
                    "from_in_tk": window.in_tk.get(),
                    "from_out_tk": window.out_tk.get(),
                    "parent_clip_id": clip.id.to_string(),
                    "removed": true,
                }
            }));
            continue;
        }

        if window.out_tk.get() > new_duration_tk {
            let from_out_tk = window.out_tk.get();
            window.out_tk = Tick::new(new_duration_tk);
            next.window = Some(window);
            changed = true;
            warnings.push(json!({
                "code": W_EFFECT_WINDOW_CLAMPED_CODE,
                "message": "effect window clamped to fit curve-adjusted duration",
                "details": {
                    "effect_id": effect.id.to_string(),
                    "from_out_tk": from_out_tk,
                    "to_out_tk": new_duration_tk,
                    "parent_clip_id": clip.id.to_string(),
                }
            }));
        }

        effects.push(next);
    }

    if changed {
        clip.effects = effects;
    }

    remove_keyframes_for_effects(clip, &removed_effect_ids, warnings);
}

fn remove_keyframes_for_effects(
    clip: &mut Clip,
    removed_effect_ids: &[EffectId],
    warnings: &mut Vec<Value>,
) {
    if removed_effect_ids.is_empty() {
        return;
    }

    let removed_effect_id_strings = removed_effect_ids
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let mut removed_keyframes = Vec::new();
    let mut filtered = Vec::with_capacity(clip.keyframes.len());

    for keyframe in &clip.keyframes {
        let target = extract_effect_id_from_property(keyframe.property.as_str());
        if target.is_some_and(|id| {
            removed_effect_id_strings
                .iter()
                .any(|removed| removed == id)
        }) {
            removed_keyframes.push(keyframe.id);
        } else {
            filtered.push(keyframe.clone());
        }
    }

    if removed_keyframes.is_empty() {
        return;
    }

    removed_keyframes.sort_by_key(ToString::to_string);
    clip.keyframes = filtered;
    warnings.push(keyframes_removed_warning(
        clip.id,
        &removed_keyframes,
        "effect keyframes targeting removed effects were removed",
    ));
}

fn warn_extreme_curve(clip: &Clip, points: Option<&[SpeedCurvePoint]>, warnings: &mut Vec<Value>) {
    let Some(points) = points else {
        return;
    };
    if !clip.effects.iter().any(is_time_stretch) {
        return;
    }
    let mut max_effective: f64 = 0.0;
    let mut segment_index: usize = 0;
    for (index, point) in points.iter().enumerate() {
        let effective = clip.speed * point.factor;
        if effective > max_effective {
            max_effective = effective;
            segment_index = index;
        }
    }
    if max_effective <= EXTREME_FACTOR_THRESHOLD {
        return;
    }
    warnings.push(json!({
        "code": W_SPEED_CURVE_EXTREME_CODE,
        "message": "extreme curve rate with time_stretch effect may produce artifacts",
        "details": {
            "clip_id": clip.id.to_string(),
            "max_effective_factor": max_effective,
            "segment_index": segment_index,
        }
    }));
}

fn is_time_stretch(effect: &Effect) -> bool {
    effect.kind.as_str() == "time_stretch"
}

fn keyframes_removed_warning(
    clip_id: ClipId,
    removed_keyframe_ids: &[KeyframeId],
    message: &'static str,
) -> Value {
    json!({
        "code": W_KEYFRAMES_REMOVED_CODE,
        "message": message,
        "details": {
            "clip_id": clip_id.to_string(),
            "removed_keyframe_ids": stringify_ids(removed_keyframe_ids),
        }
    })
}

fn stringify_ids<T: ToString>(ids: &[T]) -> Vec<String> {
    ids.iter().map(ToString::to_string).collect()
}

fn check_planned_overlap(
    prior: &Project,
    located: &LocatedClip<'_>,
    new_duration_tk: i64,
) -> Result<(), ClipSetSpeedCurveError> {
    let start = located.clip.track_position_tk.get();
    let end = start.saturating_add(new_duration_tk);

    for (clip_idx, other) in prior.tracks[located.track_idx].clips.iter().enumerate() {
        if clip_idx == located.clip_idx {
            continue;
        }
        let other_start = other.track_position_tk.get();
        let other_end = other_start.saturating_add(clip_timeline_duration_tk(other).get());
        if intervals_overlap(start, end, other_start, other_end) {
            return Err(ClipSetSpeedCurveError::ClipOverlap {
                failed_clip: located.clip.id.to_string(),
            });
        }
    }
    Ok(())
}

fn intervals_overlap(a_start: i64, a_end: i64, b_start: i64, b_end: i64) -> bool {
    a_start < b_end && b_start < a_end
}

fn build_patch(prior: &Project, located: &LocatedClip<'_>, plan: &CurvePlan) -> Value {
    let track_idx = located.track_idx;
    let clip_idx = located.clip_idx;
    let old_clip = located.clip;
    let new_clip = &plan.clip;
    let speed_curve_path = format!("/tracks/{track_idx}/clips/{clip_idx}/speed_curve");
    let mut ops = Vec::new();

    // Patch op for `Clip.speed_curve` first so the rest of the cascade
    // reads cleanly when an event-log replayer audits the diff.
    if old_clip.speed_curve.is_none() && new_clip.speed_curve.is_some() {
        ops.push(json!({
            "op": "add",
            "path": speed_curve_path.clone(),
            "value": Value::Null,
        }));
    }
    if old_clip.speed_curve != new_clip.speed_curve {
        ops.push(json!({
            "op": "replace",
            "path": speed_curve_path,
            "value": new_clip.speed_curve,
        }));
    }

    push_replace_i64_if_changed(
        &mut ops,
        &format!("/tracks/{track_idx}/clips/{clip_idx}/fade_in_tk"),
        old_clip.fade_in_tk.get(),
        new_clip.fade_in_tk.get(),
    );
    push_replace_i64_if_changed(
        &mut ops,
        &format!("/tracks/{track_idx}/clips/{clip_idx}/fade_out_tk"),
        old_clip.fade_out_tk.get(),
        new_clip.fade_out_tk.get(),
    );
    if old_clip.keyframes != new_clip.keyframes {
        ops.push(json!({
            "op": "replace",
            "path": format!("/tracks/{track_idx}/clips/{clip_idx}/keyframes"),
            "value": new_clip.keyframes,
        }));
    }
    if old_clip.effects != new_clip.effects {
        ops.push(json!({
            "op": "replace",
            "path": format!("/tracks/{track_idx}/clips/{clip_idx}/effects"),
            "value": new_clip.effects,
        }));
    }

    let new_project_duration = planned_project_duration_tk(prior, located, plan.new_duration_tk);
    if new_project_duration != prior.duration_tk.get() {
        ops.push(json!({
            "op": "replace",
            "path": "/duration_tk",
            "value": new_project_duration,
        }));
    }

    Value::Array(ops)
}

fn push_replace_i64_if_changed(ops: &mut Vec<Value>, path: &str, old_value: i64, new_value: i64) {
    if old_value != new_value {
        ops.push(json!({
            "op": "replace",
            "path": path,
            "value": new_value,
        }));
    }
}

fn planned_project_duration_tk(
    prior: &Project,
    located: &LocatedClip<'_>,
    new_target_duration_tk: i64,
) -> i64 {
    let mut computed = 0_i64;
    for (track_idx, track) in prior.tracks.iter().enumerate() {
        for (clip_idx, clip) in track.clips.iter().enumerate() {
            let duration = if track_idx == located.track_idx && clip_idx == located.clip_idx {
                new_target_duration_tk
            } else {
                clip_timeline_duration_tk(clip).get()
            };
            computed = computed.max(clip.track_position_tk.get().saturating_add(duration));
        }
    }
    computed
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

/// Rebuilds the envelope from `(args, post_state)`.
///
/// Reads the curve-aware [`clip_timeline_duration_tk`] on the post-state
/// target so `effective_duration_tk` matches what `compute_patch`
/// produced. Cascade-warning evidence (pre-clamp fade values, removed
/// keyframe ids, removed effect ids) is NOT carried in the envelope
/// and so does not need reconstruction.
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
                    effective_duration_tk: clip_timeline_duration_tk(clip).get(),
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
