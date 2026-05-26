//! `clip.split` (§5.3) — forty-eighth production verb in the engine.
//!
//! Cuts a clip into left/right halves. Linked clips are structurally
//! split at the same project-time tick without source-semantics-mix
//! rejection per §5.15.

use std::collections::HashMap;

use crate::asset::Asset;
use crate::clip::{Clip, FadeCurve};
use crate::invariants::timeline_duration_tk;
use crate::keyframe::Keyframe;
use crate::newtypes::AssetRef;
use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use crate::track::TrackKind;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::{ClipId, KeyframeId, LinkGroupId, ProjectId, Tick};

/// Warning code emitted when fades are clamped to a split half's duration.
pub const W_FADE_CLAMPED_CODE: &str = "W_FADE_CLAMPED";

/// Warning code emitted when keyframes beyond a split half's duration are removed.
pub const W_KEYFRAMES_REMOVED_CODE: &str = "W_KEYFRAMES_REMOVED";

/// Internal warning code carrying minted IDs for reconstructor replay.
pub const W_CLIP_SPLIT_ENVELOPE_CODE: &str = "W_CLIP_SPLIT_ENVELOPE";

/// Args for `clip.split`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipSplitArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Target clip id as bare `UUIDv7`.
    pub clip: String,

    /// Project-time tick where the clip is split.
    pub at_tk: i64,
}

/// One linked sibling split returned in the `clip.split` envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SiblingSplit {
    /// Source sibling clip id.
    pub source_clip_id: ClipId,
    /// Left half id; same as the source sibling id.
    pub left_clip_id: ClipId,
    /// Freshly minted right half id.
    pub right_clip_id: ClipId,
}

/// Envelope `data` returned by `clip.split`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipSplitData {
    /// Target left half id; same as the original target id.
    pub left_clip_id: ClipId,
    /// Freshly minted target right half id.
    pub right_clip_id: ClipId,
    /// Project-time split tick.
    pub at_tk: i64,
    /// Original link group retained by left halves, when linked.
    pub left_link_group: Option<LinkGroupId>,
    /// Freshly minted link group shared by right halves, when linked.
    pub right_link_group: Option<LinkGroupId>,
    /// Linked siblings split by the same operation, excluding the target.
    pub sibling_splits: Vec<SiblingSplit>,
}

/// Verb-level validation failures for `clip.split`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ClipSplitError {
    /// `args.clip` is not parseable as `UUIDv7`.
    #[error("E_BAD_SELECTOR: clip.split: `clip` selector parse failed: {detail}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
    },

    /// No clip exists for `clip`.
    #[error("E_NOT_FOUND: clip.split: clip `{clip_id}` not found")]
    ClipNotFound {
        /// Missing clip id string.
        clip_id: String,
    },

    /// The split time is negative or would create a degenerate half.
    #[error("E_BAD_TIME: clip.split: `{field}` value {value} is invalid")]
    BadTime {
        /// Invalid field name.
        field: &'static str,
        /// Invalid value.
        value: i64,
    },

    /// The split time is outside a clip's timeline interval.
    #[error(
        "E_CLIP_OUT_OF_BOUNDS: clip.split: `{field}` value {value} outside clip `{failed_clip}` range [{range_start_tk}, {range_end_tk}]"
    )]
    ClipOutOfBounds {
        /// Invalid field name.
        field: &'static str,
        /// Invalid value.
        value: i64,
        /// First failed clip in deterministic track/clip order.
        failed_clip: String,
        /// Clip start tick.
        range_start_tk: i64,
        /// Clip end tick.
        range_end_tk: i64,
    },

    /// A structural-set member or its parent track is locked.
    #[error("E_LOCKED: clip.split: clip `{failed_clip}` or its parent track is locked")]
    Locked {
        /// First failed member in deterministic track/clip order.
        failed_clip: String,
    },
}

#[derive(Debug, Clone, Copy)]
struct LocatedClip<'a> {
    track_idx: usize,
    clip_idx: usize,
    track_kind: TrackKind,
    track_locked: bool,
    clip: &'a Clip,
}

#[derive(Debug, Clone)]
struct SplitPlan {
    track_idx: usize,
    clip_idx: usize,
    left_clip: Clip,
    right_clip: Clip,
    right_duration_tk: i64,
    left_duration_tk: i64,
}

/// Build the RFC-6902 patch for `clip.split`.
///
/// # Errors
///
/// Returns [`ClipSplitError`] for selector parse failure, missing target
/// clip, bad split time, per-member bounds failures, or lock conflicts.
pub fn compute_patch(
    prior: &Project,
    args: &ClipSplitArgs,
) -> Result<(Value, Vec<Value>, ClipSplitData), ClipSplitError> {
    let clip_id = args
        .clip
        .parse::<ClipId>()
        .map_err(|err| ClipSplitError::BadSelector {
            detail: err.to_string(),
        })?;

    let target = locate_clip(prior, clip_id).ok_or_else(|| ClipSplitError::ClipNotFound {
        clip_id: args.clip.clone(),
    })?;

    validate_target_at_tk(target, args.at_tk)?;

    let members = resolve_structural_set(prior, target);
    for member in &members {
        if member.track_locked || member.clip.locked {
            return Err(ClipSplitError::Locked {
                failed_clip: member.clip.id.to_string(),
            });
        }
    }
    for member in &members {
        validate_member_at_tk(member, args.at_tk)?;
    }

    let right_link_group = members
        .iter()
        .any(|member| member.clip.link_group.is_some())
        .then(LinkGroupId::now);
    let right_clip_ids = members
        .iter()
        .map(|member| (member.clip.id, ClipId::now()))
        .collect::<HashMap<_, _>>();

    let mut warnings = Vec::new();
    let plans = members
        .iter()
        .map(|member| {
            build_split_plan(
                prior,
                member,
                args.at_tk,
                right_clip_ids[&member.clip.id],
                right_link_group,
                &mut warnings,
            )
        })
        .collect::<Vec<_>>();

    let target_right_clip_id = right_clip_ids[&clip_id];
    let data = ClipSplitData {
        left_clip_id: clip_id,
        right_clip_id: target_right_clip_id,
        at_tk: args.at_tk,
        left_link_group: target.clip.link_group,
        right_link_group,
        sibling_splits: sibling_splits(&members, clip_id, &right_clip_ids),
    };
    warnings.push(envelope_warning(&data));

    let patch = build_patch(prior, &plans);
    Ok((patch, warnings, data))
}

fn validate_target_at_tk(target: LocatedClip<'_>, at_tk: i64) -> Result<(), ClipSplitError> {
    if at_tk < 0 {
        return Err(ClipSplitError::BadTime {
            field: "at_tk",
            value: at_tk,
        });
    }

    let start = target.clip.track_position_tk.get();
    let end = clip_end_tk(target.clip);
    if at_tk < start || at_tk > end {
        return Err(ClipSplitError::ClipOutOfBounds {
            field: "at_tk",
            value: at_tk,
            failed_clip: target.clip.id.to_string(),
            range_start_tk: start,
            range_end_tk: end,
        });
    }
    if at_tk == start || at_tk == end {
        return Err(ClipSplitError::BadTime {
            field: "at_tk",
            value: at_tk,
        });
    }
    Ok(())
}

fn validate_member_at_tk(member: &LocatedClip<'_>, at_tk: i64) -> Result<(), ClipSplitError> {
    let start = member.clip.track_position_tk.get();
    let end = clip_end_tk(member.clip);
    if at_tk <= start || at_tk >= end {
        return Err(ClipSplitError::ClipOutOfBounds {
            field: "at_tk",
            value: at_tk,
            failed_clip: member.clip.id.to_string(),
            range_start_tk: start,
            range_end_tk: end,
        });
    }
    Ok(())
}

fn clip_end_tk(clip: &Clip) -> i64 {
    clip.track_position_tk.get().saturating_add(
        timeline_duration_tk(clip.source_in_tk, clip.source_out_tk, clip.speed).get(),
    )
}

fn locate_clip(project: &Project, clip_id: ClipId) -> Option<LocatedClip<'_>> {
    for (track_idx, track) in project.tracks.iter().enumerate() {
        for (clip_idx, clip) in track.clips.iter().enumerate() {
            if clip.id == clip_id {
                return Some(LocatedClip {
                    track_idx,
                    clip_idx,
                    track_kind: track.kind,
                    track_locked: track.locked,
                    clip,
                });
            }
        }
    }
    None
}

fn resolve_structural_set<'a>(
    project: &'a Project,
    target: LocatedClip<'a>,
) -> Vec<LocatedClip<'a>> {
    let Some(target_group) = target.clip.link_group else {
        return vec![target];
    };

    project
        .tracks
        .iter()
        .enumerate()
        .flat_map(|(track_idx, track)| {
            track
                .clips
                .iter()
                .enumerate()
                .filter_map(move |(clip_idx, clip)| {
                    (clip.link_group == Some(target_group)).then_some(LocatedClip {
                        track_idx,
                        clip_idx,
                        track_kind: track.kind,
                        track_locked: track.locked,
                        clip,
                    })
                })
        })
        .collect()
}

fn build_split_plan(
    prior: &Project,
    member: &LocatedClip<'_>,
    at_tk: i64,
    right_clip_id: ClipId,
    right_link_group: Option<LinkGroupId>,
    warnings: &mut Vec<Value>,
) -> SplitPlan {
    let split_offset = at_tk.saturating_sub(member.clip.track_position_tk.get());
    let display_duration =
        is_display_duration_clip(prior, member.track_kind, &member.clip.asset_id);
    let source_out_left = if display_duration {
        split_offset
    } else {
        member.clip.source_in_tk.get()
            + timeline_offset_to_source_delta(split_offset, member.clip.speed)
    };

    let mut left_clip = member.clip.clone();
    if display_duration {
        left_clip.source_in_tk = Tick::ZERO;
    }
    left_clip.source_out_tk = Tick::new(source_out_left);
    left_clip.fade_out_tk = Tick::ZERO;
    left_clip.fade_out_curve = FadeCurve::Linear;
    left_clip.keyframes = member
        .clip
        .keyframes
        .iter()
        .filter(|keyframe| keyframe.time_tk.get() < split_offset)
        .cloned()
        .collect();

    let mut right_clip = member.clip.clone();
    right_clip.id = right_clip_id;
    if display_duration {
        let old_duration_tk = timeline_duration_tk(
            member.clip.source_in_tk,
            member.clip.source_out_tk,
            member.clip.speed,
        )
        .get();
        right_clip.source_in_tk = Tick::ZERO;
        right_clip.source_out_tk = Tick::new(old_duration_tk.saturating_sub(split_offset));
    } else {
        right_clip.source_in_tk = Tick::new(source_out_left);
    }
    right_clip.track_position_tk = Tick::new(at_tk);
    right_clip.fade_in_tk = Tick::ZERO;
    right_clip.fade_in_curve = FadeCurve::Linear;
    right_clip.keyframes = rebased_right_keyframes(member.clip, split_offset);
    right_clip.link_group = right_link_group;

    let left_duration_tk = timeline_duration_tk(
        left_clip.source_in_tk,
        left_clip.source_out_tk,
        left_clip.speed,
    )
    .get();
    let right_duration_tk = timeline_duration_tk(
        right_clip.source_in_tk,
        right_clip.source_out_tk,
        right_clip.speed,
    )
    .get();

    clamp_fades(member.clip, &mut left_clip, left_duration_tk, warnings);
    clamp_fades(member.clip, &mut right_clip, right_duration_tk, warnings);
    remove_overflow_keyframes(&mut left_clip, left_duration_tk, warnings);
    remove_overflow_keyframes(&mut right_clip, right_duration_tk, warnings);

    SplitPlan {
        track_idx: member.track_idx,
        clip_idx: member.clip_idx,
        left_clip,
        right_clip,
        right_duration_tk,
        left_duration_tk,
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
fn timeline_offset_to_source_delta(split_offset_tk: i64, speed: f64) -> i64 {
    ((split_offset_tk as f64) * speed) as i64
}

fn is_display_duration_clip(project: &Project, track_kind: TrackKind, asset_id: &AssetRef) -> bool {
    if track_kind == TrackKind::Text {
        return true;
    }
    asset_id.id().is_some_and(|asset_id| {
        project
            .assets
            .iter()
            .any(|asset| asset.id() == asset_id && matches!(asset, Asset::Image(_)))
    })
}

fn rebased_right_keyframes(clip: &Clip, split_offset: i64) -> Vec<Keyframe> {
    clip.keyframes
        .iter()
        .filter(|keyframe| keyframe.time_tk.get() >= split_offset)
        .map(|keyframe| {
            let mut rebased = keyframe.clone();
            rebased.id = KeyframeId::now();
            rebased.time_tk = Tick::new(keyframe.time_tk.get().saturating_sub(split_offset));
            rebased
        })
        .collect()
}

fn clamp_fades(old_clip: &Clip, clip: &mut Clip, new_duration_tk: i64, warnings: &mut Vec<Value>) {
    let old_fade_in = clip.fade_in_tk.get();
    let old_fade_out = clip.fade_out_tk.get();
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
        "message": "clip fades clamped to fit split duration",
        "details": {
            "clip_id": clip.id.to_string(),
            "from_in_tk": old_fade_in,
            "from_out_tk": old_fade_out,
            "to_in_tk": new_fade_in,
            "to_out_tk": new_fade_out,
            "source_clip_id": old_clip.id.to_string(),
        }
    }));
}

fn remove_overflow_keyframes(clip: &mut Clip, new_duration_tk: i64, warnings: &mut Vec<Value>) {
    let mut removed = Vec::new();
    clip.keyframes.retain(|keyframe| {
        if keyframe.time_tk.get() > new_duration_tk {
            removed.push(keyframe.id);
            false
        } else {
            true
        }
    });

    if removed.is_empty() {
        return;
    }

    removed.sort_by_key(ToString::to_string);
    warnings.push(json!({
        "code": W_KEYFRAMES_REMOVED_CODE,
        "message": "clip keyframes beyond the split duration were removed",
        "details": {
            "clip_id": clip.id.to_string(),
            "removed_keyframe_ids": stringify_ids(&removed),
        }
    }));
}

fn sibling_splits(
    members: &[LocatedClip<'_>],
    target_id: ClipId,
    right_clip_ids: &HashMap<ClipId, ClipId>,
) -> Vec<SiblingSplit> {
    members
        .iter()
        .filter(|member| member.clip.id != target_id)
        .map(|member| SiblingSplit {
            source_clip_id: member.clip.id,
            left_clip_id: member.clip.id,
            right_clip_id: right_clip_ids[&member.clip.id],
        })
        .collect()
}

fn build_patch(prior: &Project, plans: &[SplitPlan]) -> Value {
    let mut ops = Vec::new();
    let mut planned_tracks = prior.tracks.clone();

    for (track_idx, track) in planned_tracks.iter_mut().enumerate() {
        let plans_for_track = plans
            .iter()
            .filter(|plan| plan.track_idx == track_idx)
            .collect::<Vec<_>>();
        if plans_for_track.is_empty() {
            continue;
        }

        let mut clips = Vec::with_capacity(track.clips.len() + plans_for_track.len());
        for (clip_idx, clip) in track.clips.iter().enumerate() {
            if let Some(plan) = plans_for_track
                .iter()
                .find(|plan| plan.clip_idx == clip_idx)
            {
                clips.push(plan.left_clip.clone());
                clips.push(plan.right_clip.clone());
            } else {
                clips.push(clip.clone());
            }
        }
        track.clips = clips;
        ops.push(json!({
            "op": "replace",
            "path": format!("/tracks/{track_idx}/clips"),
            "value": track.clips,
        }));
    }

    let new_duration_tk = planned_project_duration_tk(prior, plans);
    if new_duration_tk != prior.duration_tk.get() {
        ops.push(json!({
            "op": "replace",
            "path": "/duration_tk",
            "value": new_duration_tk,
        }));
    }

    Value::Array(ops)
}

fn planned_project_duration_tk(prior: &Project, plans: &[SplitPlan]) -> i64 {
    let mut computed = 0_i64;
    for (track_idx, track) in prior.tracks.iter().enumerate() {
        for (clip_idx, clip) in track.clips.iter().enumerate() {
            if let Some(plan) = plans
                .iter()
                .find(|plan| plan.track_idx == track_idx && plan.clip_idx == clip_idx)
            {
                computed = computed.max(
                    plan.left_clip
                        .track_position_tk
                        .get()
                        .saturating_add(plan.left_duration_tk),
                );
                computed = computed.max(
                    plan.right_clip
                        .track_position_tk
                        .get()
                        .saturating_add(plan.right_duration_tk),
                );
            } else {
                let duration_tk =
                    timeline_duration_tk(clip.source_in_tk, clip.source_out_tk, clip.speed);
                computed = computed.max(
                    clip.track_position_tk
                        .get()
                        .saturating_add(duration_tk.get()),
                );
            }
        }
    }
    computed
}

fn envelope_warning(data: &ClipSplitData) -> Value {
    json!({
        "code": W_CLIP_SPLIT_ENVELOPE_CODE,
        "message": "clip.split envelope",
        "details": data,
    })
}

fn stringify_ids<T: ToString>(ids: &[T]) -> Vec<String> {
    ids.iter().map(ToString::to_string).collect()
}

/// Rebuild `ClipSplitData` from recorded warnings.
///
/// # Errors
///
/// Returns [`ReconstructError`] if the internal envelope warning is
/// missing or malformed.
pub fn data_envelope_from_args_warnings(
    _args: &ClipSplitArgs,
    warnings: &[Value],
) -> Result<ClipSplitData, ReconstructError> {
    for warning in warnings {
        if warning.get("code").and_then(Value::as_str) != Some(W_CLIP_SPLIT_ENVELOPE_CODE) {
            continue;
        }
        let details = warning
            .get("details")
            .ok_or(ReconstructError::MissingField {
                name: "warnings[].W_CLIP_SPLIT_ENVELOPE.details",
            })?;
        return serde_json::from_value(details.clone()).map_err(|_| {
            ReconstructError::TypeMismatch {
                name: "warnings[].W_CLIP_SPLIT_ENVELOPE.details",
                expected: "ClipSplitData",
            }
        });
    }

    Err(ReconstructError::MissingField {
        name: "warnings[].W_CLIP_SPLIT_ENVELOPE",
    })
}

/// `clip.split` verb registration entry.
#[derive(Debug, Default)]
pub struct ClipSplitVerb;

impl From<ClipSplitError> for VerbError {
    fn from(value: ClipSplitError) -> Self {
        VerbError::BadArgs {
            detail: value.to_string(),
        }
    }
}

impl Verb for ClipSplitVerb {
    fn verb(&self) -> &'static str {
        "clip.split"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: ClipSplitArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("clip.split: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!("clip.split: patch construction failed: {err}"))
            })?;

        let post_state = prior
            .apply(&patch)
            .map_err(|err| VerbError::InvariantViolation {
                detail: format!("clip.split: post-state validation failed: {err}"),
            })?;
        drop(post_state);

        let data = serde_json::to_value(&data).map_err(|err| {
            VerbError::Custom(format!("clip.split: data serialize failed: {err}"))
        })?;

        Ok((patch, data, warnings))
    }

    fn reconstruct(
        &self,
        args: &Value,
        _patch: &Value,
        warnings: &[Value],
        _post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let typed: ClipSplitArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "ClipSplitArgs",
            })?;

        let envelope = data_envelope_from_args_warnings(&typed, warnings)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
