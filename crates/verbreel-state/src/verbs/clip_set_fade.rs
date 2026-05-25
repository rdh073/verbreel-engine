//! `clip.set_fade` (§5.12) — fortieth production verb in the engine.
//!
//! ## Behavior
//!
//! Sets `Clip.fade_in_tk`, `Clip.fade_out_tk`, `Clip.fade_in_curve`,
//! and `Clip.fade_out_curve` as a partial update. All clip kinds are
//! accepted. Audio clips keep sample-accurate fade times; video/image/text
//! clips snap fade times to the nearest project-fps frame boundary and emit
//! `W_TIME_SNAPPED` for each adjusted field.

use crate::clip::FadeCurve;
use crate::invariants::timeline_duration_tk;
use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use crate::track::TrackKind;
use crate::verbs::project_set_fps::is_off_frame;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::{ClipId, ProjectId, TICK_RATE_HZ, Tick};

/// Warning code emitted when the incoming fade state equals current.
pub const W_NOOP_CODE: &str = "W_NOOP";

/// Warning code emitted when a fade tick is snapped to a frame boundary.
pub const W_TIME_SNAPPED_CODE: &str = "W_TIME_SNAPPED";

/// Args for `clip.set_fade`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipSetFadeArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Target clip id as bare `UUIDv7`.
    pub clip: String,

    /// Optional fade-in duration in ticks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade_in_tk: Option<i64>,

    /// Optional fade-out duration in ticks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade_out_tk: Option<i64>,

    /// Optional fade-in curve.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade_in_curve: Option<FadeCurve>,

    /// Optional fade-out curve.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade_out_curve: Option<FadeCurve>,
}

/// Envelope `data` returned by `clip.set_fade`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipSetFadeData {
    /// Target clip id.
    pub clip_id: ClipId,

    /// Post-state fade-in duration in ticks.
    pub fade_in_tk: i64,

    /// Post-state fade-out duration in ticks.
    pub fade_out_tk: i64,

    /// Post-state fade-in curve.
    pub fade_in_curve: FadeCurve,

    /// Post-state fade-out curve.
    pub fade_out_curve: FadeCurve,
}

/// Verb-level validation failures for `clip.set_fade`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ClipSetFadeError {
    /// No partial-update field was supplied.
    #[error("clip.set_fade: at least one fade field must be supplied")]
    BadArgs,

    /// `args.clip` is not parseable as `UUIDv7`.
    #[error("clip.set_fade: `clip` selector parse failed: {detail}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
    },

    /// No clip exists for `clip`.
    #[error("clip.set_fade: clip `{clip_id}` not found")]
    ClipNotFound {
        /// Missing clip id string.
        clip_id: String,
    },

    /// The target clip or its parent track is locked.
    #[error("clip.set_fade: {kind} `{id}` is locked for clip `{clip_id}`")]
    Locked {
        /// Locked entity kind.
        kind: &'static str,
        /// Locked entity id.
        id: String,
        /// Target clip id string.
        clip_id: String,
    },

    /// A provided fade duration is negative.
    #[error("clip.set_fade: `{field}` value {value} must be >= 0")]
    BadTime {
        /// Invalid field name.
        field: &'static str,
        /// Invalid value.
        value: i64,
    },

    /// Proposed fades exceed the clip timeline duration.
    #[error(
        "clip.set_fade: fade sum {fade_sum_tk} exceeds timeline_duration_tk {timeline_duration_tk}"
    )]
    BadRange {
        /// Sum of fade-in and fade-out ticks.
        fade_sum_tk: i64,
        /// Clip timeline duration in ticks.
        timeline_duration_tk: i64,
    },
}

#[derive(Debug, Clone)]
struct LocatedClip<'a> {
    track_idx: usize,
    clip_idx: usize,
    track_kind: TrackKind,
    track_locked: bool,
    track_id: String,
    clip: &'a crate::clip::Clip,
}

/// Build the RFC-6902 patch for `clip.set_fade`.
///
/// # Errors
///
/// Returns [`ClipSetFadeError`] for missing update fields, selector parse
/// failure, missing clip, locked target, negative fade durations, or fades
/// that exceed the clip's timeline duration.
pub fn compute_patch(
    prior: &Project,
    args: &ClipSetFadeArgs,
) -> Result<(Value, Vec<Value>, ClipSetFadeData), ClipSetFadeError> {
    if args.fade_in_tk.is_none()
        && args.fade_out_tk.is_none()
        && args.fade_in_curve.is_none()
        && args.fade_out_curve.is_none()
    {
        return Err(ClipSetFadeError::BadArgs);
    }

    let clip_id = args
        .clip
        .parse::<ClipId>()
        .map_err(|err| ClipSetFadeError::BadSelector {
            detail: err.to_string(),
        })?;

    let located = locate_clip(prior, clip_id).ok_or_else(|| ClipSetFadeError::ClipNotFound {
        clip_id: args.clip.clone(),
    })?;

    if located.track_locked {
        return Err(ClipSetFadeError::Locked {
            kind: "track",
            id: located.track_id,
            clip_id: args.clip.clone(),
        });
    }

    if located.clip.locked {
        return Err(ClipSetFadeError::Locked {
            kind: "clip",
            id: located.clip.id.to_string(),
            clip_id: args.clip.clone(),
        });
    }

    validate_non_negative(args.fade_in_tk, "fade_in_tk")?;
    validate_non_negative(args.fade_out_tk, "fade_out_tk")?;

    let mut fade_in_tk = args
        .fade_in_tk
        .unwrap_or_else(|| located.clip.fade_in_tk.get());
    let mut fade_out_tk = args
        .fade_out_tk
        .unwrap_or_else(|| located.clip.fade_out_tk.get());
    let fade_in_curve = args.fade_in_curve.unwrap_or(located.clip.fade_in_curve);
    let fade_out_curve = args.fade_out_curve.unwrap_or(located.clip.fade_out_curve);

    let duration_tk = timeline_duration_tk(
        located.clip.source_in_tk,
        located.clip.source_out_tk,
        located.clip.speed,
    )
    .get();
    check_fade_sum(fade_in_tk, fade_out_tk, duration_tk)?;

    let mut warnings = Vec::new();
    if matches!(located.track_kind, TrackKind::Video | TrackKind::Text) {
        let snapped_in = snap_fade_field(prior, fade_in_tk, "fade_in_tk", &mut warnings);
        let snapped_out = snap_fade_field(prior, fade_out_tk, "fade_out_tk", &mut warnings);
        fade_in_tk = snapped_in;
        fade_out_tk = snapped_out;
    }

    check_fade_sum(fade_in_tk, fade_out_tk, duration_tk)?;

    let data = ClipSetFadeData {
        clip_id,
        fade_in_tk,
        fade_out_tk,
        fade_in_curve,
        fade_out_curve,
    };

    if fade_in_tk == located.clip.fade_in_tk.get()
        && fade_out_tk == located.clip.fade_out_tk.get()
        && fade_in_curve == located.clip.fade_in_curve
        && fade_out_curve == located.clip.fade_out_curve
    {
        warnings.push(json!({
            "code": W_NOOP_CODE,
            "message": "clip.set_fade no-op",
            "details": {
                "clip_id": clip_id.to_string(),
            }
        }));
        return Ok((json!([]), warnings, data));
    }

    let patch = json!([
        {
            "op": "replace",
            "path": format!("/tracks/{}/clips/{}/fade_in_tk", located.track_idx, located.clip_idx),
            "value": fade_in_tk,
        },
        {
            "op": "replace",
            "path": format!("/tracks/{}/clips/{}/fade_out_tk", located.track_idx, located.clip_idx),
            "value": fade_out_tk,
        },
        {
            "op": "replace",
            "path": format!("/tracks/{}/clips/{}/fade_in_curve", located.track_idx, located.clip_idx),
            "value": fade_in_curve,
        },
        {
            "op": "replace",
            "path": format!("/tracks/{}/clips/{}/fade_out_curve", located.track_idx, located.clip_idx),
            "value": fade_out_curve,
        },
    ]);

    Ok((patch, warnings, data))
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

fn validate_non_negative(value: Option<i64>, field: &'static str) -> Result<(), ClipSetFadeError> {
    if let Some(value) = value
        && value < 0
    {
        return Err(ClipSetFadeError::BadTime { field, value });
    }
    Ok(())
}

fn check_fade_sum(
    fade_in_tk: i64,
    fade_out_tk: i64,
    duration_tk: i64,
) -> Result<(), ClipSetFadeError> {
    let fade_sum_tk = fade_in_tk.saturating_add(fade_out_tk);
    if fade_sum_tk > duration_tk {
        return Err(ClipSetFadeError::BadRange {
            fade_sum_tk,
            timeline_duration_tk: duration_tk,
        });
    }
    Ok(())
}

fn snap_fade_field(
    prior: &Project,
    value_tk: i64,
    field: &'static str,
    warnings: &mut Vec<Value>,
) -> i64 {
    if !is_off_frame(Tick::new(value_tk), prior.fps_num, prior.fps_den) {
        return value_tk;
    }

    let snapped_tk = nearest_frame_tick(value_tk, prior.fps_num, prior.fps_den);
    if snapped_tk != value_tk {
        warnings.push(json!({
            "code": W_TIME_SNAPPED_CODE,
            "message": "time value snapped to frame boundary",
            "details": {
                "from_tk": value_tk,
                "to_tk": snapped_tk,
                "field": field,
            }
        }));
    }
    snapped_tk
}

fn nearest_frame_tick(value_tk: i64, fps_num: u32, fps_den: u32) -> i64 {
    if fps_num == 0 {
        return value_tk;
    }

    let frame_clock = u64::from(TICK_RATE_HZ) * u64::from(fps_den);
    let step_tk = frame_clock / gcd_u64(frame_clock, u64::from(fps_num));
    if step_tk == 0 {
        return value_tk;
    }

    let value = i128::from(value_tk.max(0));
    let step = i128::from(step_tk);
    let snapped = ((value + (step / 2)) / step) * step;
    i64::try_from(snapped).unwrap_or(i64::MAX)
}

fn gcd_u64(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        let rem = a % b;
        a = b;
        b = rem;
    }
    a
}

/// Rebuilds the envelope from `(args, post_state)`.
///
/// # Errors
///
/// Returns [`ReconstructError::TypeMismatch`] when `args.clip` is not
/// a valid `UUIDv7`, or [`ReconstructError::PostStateMissing`] when the
/// post-state does not contain the target clip.
pub fn data_envelope_from_post_state(
    args: &ClipSetFadeArgs,
    post_state: &Project,
) -> Result<ClipSetFadeData, ReconstructError> {
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
                return Ok(ClipSetFadeData {
                    clip_id,
                    fade_in_tk: clip.fade_in_tk.get(),
                    fade_out_tk: clip.fade_out_tk.get(),
                    fade_in_curve: clip.fade_in_curve,
                    fade_out_curve: clip.fade_out_curve,
                });
            }
        }
    }

    Err(ReconstructError::PostStateMissing {
        detail: format!("clip set_fade: clip id {clip_id} not found in post_state"),
    })
}

/// `clip.set_fade` verb registration entry.
#[derive(Debug, Default)]
pub struct ClipSetFadeVerb;

impl From<ClipSetFadeError> for VerbError {
    fn from(value: ClipSetFadeError) -> Self {
        match value {
            ClipSetFadeError::BadArgs
            | ClipSetFadeError::BadSelector { .. }
            | ClipSetFadeError::ClipNotFound { .. }
            | ClipSetFadeError::Locked { .. }
            | ClipSetFadeError::BadTime { .. }
            | ClipSetFadeError::BadRange { .. } => VerbError::BadArgs {
                detail: value.to_string(),
            },
        }
    }
}

impl Verb for ClipSetFadeVerb {
    fn verb(&self) -> &'static str {
        "clip.set_fade"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: ClipSetFadeArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("clip.set_fade: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, _data) = compute_patch(prior, &typed)?;

        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!("clip.set_fade: patch construction failed: {err}"))
            })?;

        let post_state = prior
            .apply(&patch)
            .map_err(|err| VerbError::InvariantViolation {
                detail: format!("clip.set_fade: post-state validation failed: {err}"),
            })?;

        let envelope = data_envelope_from_post_state(&typed, &post_state).map_err(|err| {
            VerbError::Custom(format!(
                "clip.set_fade: data envelope reconstruction failed: {err}"
            ))
        })?;

        let data = serde_json::to_value(&envelope).map_err(|err| {
            VerbError::Custom(format!("clip.set_fade: data serialize failed: {err}"))
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
        let typed: ClipSetFadeArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "ClipSetFadeArgs",
            })?;

        let envelope = data_envelope_from_post_state(&typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
