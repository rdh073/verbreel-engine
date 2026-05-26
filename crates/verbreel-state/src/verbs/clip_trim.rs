//! `clip.trim` (§5.2) — forty-seventh production verb in the engine.
//!
//! Changes a clip's source window. Source-window deltas propagate
//! across the target clip's link group per §5.15; lock, bounds, and
//! overlap checks are atomic across the whole sync set.

use crate::asset::Asset;
use crate::clip::Clip;
use crate::invariants::timeline_duration_tk;
use crate::newtypes::AssetRef;
use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use crate::track::TrackKind;
use crate::verbs::project_set_fps::is_off_frame;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::{ClipId, KeyframeId, LinkGroupId, ProjectId, TICK_RATE_HZ, Tick};

/// Warning code emitted when `keep_end` is ignored.
pub const W_NOOP_FLAG_CODE: &str = "W_NOOP_FLAG";

/// Warning code emitted when a trim-adjusted position snaps to a frame.
pub const W_TIME_SNAPPED_CODE: &str = "W_TIME_SNAPPED";

/// Warning code emitted when fades are clamped to the new duration.
pub const W_FADE_CLAMPED_CODE: &str = "W_FADE_CLAMPED";

/// Warning code emitted when keyframes beyond the new duration are removed.
pub const W_KEYFRAMES_REMOVED_CODE: &str = "W_KEYFRAMES_REMOVED";

/// Recovery hint returned when no trim field is supplied.
pub const ARGS_INCOMPATIBLE_HINT: &str = "supply at least one of source_in_tk or source_out_tk";

/// Recovery hint returned with `E_LINK_GROUP_SEMANTICS_MIX`.
pub const LINK_GROUP_SEMANTICS_MIX_HINT: &str =
    "call clip.unlink first, then mutate each clip independently";

/// Args for `clip.trim`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipTrimArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Target clip id as bare `UUIDv7`.
    pub clip: String,

    /// Optional new source in-point in ticks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_in_tk: Option<i64>,

    /// Optional new source out-point in ticks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_out_tk: Option<i64>,

    /// Anchor the trailing timeline edge when only `source_in_tk`
    /// changes. Defaults to `false`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_end: Option<bool>,
}

/// Envelope `data` returned by `clip.trim`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipTrimData {
    /// Target clip id.
    pub clip_id: ClipId,
    /// Target clip's post-trim source in-point.
    pub source_in_tk: i64,
    /// Target clip's post-trim source out-point.
    pub source_out_tk: i64,
    /// Target clip's post-trim timeline position.
    pub track_position_tk: i64,
    /// Target clip's post-trim timeline duration.
    pub duration_tk: i64,
    /// Link group id, if the target is linked.
    pub link_group: Option<LinkGroupId>,
    /// Other members of the target link group, sorted lexicographically.
    pub linked_clip_ids: Vec<ClipId>,
}

/// Per-kind member counts returned in `E_LINK_GROUP_SEMANTICS_MIX`
/// details.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberKindCounts {
    /// Video-kind members.
    pub video: usize,
    /// Audio-kind members.
    pub audio: usize,
    /// Image-kind members.
    pub image: usize,
    /// Text-kind members.
    pub text: usize,
}

/// Per-semantics-class counts returned in
/// `E_LINK_GROUP_SEMANTICS_MIX` details.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticsClassCounts {
    /// Video/audio source-slice members.
    pub source_slice: usize,
    /// Image/text display-duration members.
    pub display_duration: usize,
}

/// Verb-level validation failures for `clip.trim`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ClipTrimError {
    /// Neither `source_in_tk` nor `source_out_tk` was supplied.
    #[error("E_ARGS_INCOMPATIBLE: clip.trim: {hint}")]
    ArgsIncompatible {
        /// Recovery hint.
        hint: &'static str,
    },

    /// A selector is not parseable as `UUIDv7`.
    #[error("E_BAD_SELECTOR: clip.trim: `{field}` selector parse failed: {detail}")]
    BadSelector {
        /// Argument field name.
        field: &'static str,
        /// Parse failure detail.
        detail: String,
    },

    /// No clip exists for `clip`.
    #[error("E_NOT_FOUND: clip.trim: clip `{clip_id}` not found")]
    ClipNotFound {
        /// Missing clip id string.
        clip_id: String,
    },

    /// A provided source time is negative or degenerate.
    #[error("E_BAD_TIME: clip.trim: `{field}` value {value} is invalid")]
    BadTime {
        /// Invalid field name.
        field: &'static str,
        /// Invalid value.
        value: i64,
    },

    /// A link group mixes source-slice and display-duration classes.
    #[error(
        "E_LINK_GROUP_SEMANTICS_MIX: clip.trim: link group `{link_group}` mixes source-time semantics classes"
    )]
    LinkGroupSemanticsMix {
        /// Mixed link group id.
        link_group: LinkGroupId,
        /// Per-member kind counts.
        member_kinds: MemberKindCounts,
        /// Per-semantics-class counts.
        semantics_classes: SemanticsClassCounts,
        /// Recovery hint for callers.
        hint: &'static str,
    },

    /// A sync-set member's projected source window is outside bounds.
    #[error(
        "E_CLIP_OUT_OF_BOUNDS: clip.trim: clip `{failed_clip}` proposed source window [{proposed_in}, {proposed_out}) outside [{bound_min}, {bound_max}]"
    )]
    ClipOutOfBounds {
        /// First failed member in deterministic track/clip order.
        failed_clip: String,
        /// Active lower bound.
        bound_min: i64,
        /// Active upper bound.
        bound_max: i64,
        /// Proposed source in-point.
        proposed_in: i64,
        /// Proposed source out-point.
        proposed_out: i64,
    },

    /// A sync-set member or its parent track is locked.
    #[error("E_LOCKED: clip.trim: clip `{failed_clip}` or its parent track is locked")]
    Locked {
        /// First failed member in deterministic track/clip order.
        failed_clip: String,
    },

    /// A sync-set member would overlap another clip after the trim.
    #[error("E_CLIP_OVERLAP: clip.trim: clip `{failed_clip}` would overlap on its track")]
    ClipOverlap {
        /// First failed member in deterministic track/clip order.
        failed_clip: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MemberKind {
    Video,
    Audio,
    Image,
    Text,
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
struct TrimPlan {
    track_idx: usize,
    clip_idx: usize,
    track_locked: bool,
    clip: Clip,
    new_position_tk: i64,
    new_duration_tk: i64,
    is_target: bool,
}

/// Build the RFC-6902 patch for `clip.trim`.
///
/// # Errors
///
/// Returns [`ClipTrimError`] for missing trim args, selector parse
/// failure, missing target clip, bad source times, mixed link-group
/// semantics, per-member source bounds failures, lock conflicts, or
/// post-trim overlap.
pub fn compute_patch(
    prior: &Project,
    args: &ClipTrimArgs,
) -> Result<(Value, Vec<Value>, ClipTrimData), ClipTrimError> {
    if args.source_in_tk.is_none() && args.source_out_tk.is_none() {
        return Err(ClipTrimError::ArgsIncompatible {
            hint: ARGS_INCOMPATIBLE_HINT,
        });
    }

    let clip_id = args
        .clip
        .parse::<ClipId>()
        .map_err(|err| ClipTrimError::BadSelector {
            field: "clip",
            detail: err.to_string(),
        })?;

    let target = locate_clip(prior, clip_id).ok_or_else(|| ClipTrimError::ClipNotFound {
        clip_id: args.clip.clone(),
    })?;

    validate_non_negative(args.source_in_tk, "source_in_tk")?;
    validate_non_negative(args.source_out_tk, "source_out_tk")?;

    let target_source_out = args
        .source_out_tk
        .unwrap_or_else(|| target.clip.source_out_tk.get());
    if let Some(source_in_tk) = args.source_in_tk
        && source_in_tk >= target_source_out
    {
        return Err(ClipTrimError::BadTime {
            field: "source_in_tk",
            value: source_in_tk,
        });
    }

    let members = resolve_sync_set(prior, target);
    if let Some(target_group) = target.clip.link_group
        && members.len() > 1
    {
        reject_semantics_mix(prior, &members, target_group)?;
    }

    let new_source_in = args
        .source_in_tk
        .unwrap_or_else(|| target.clip.source_in_tk.get());
    let new_source_out = args
        .source_out_tk
        .unwrap_or_else(|| target.clip.source_out_tk.get());
    let delta_in = new_source_in.saturating_sub(target.clip.source_in_tk.get());
    let delta_out = new_source_out.saturating_sub(target.clip.source_out_tk.get());
    let keep_end = args.keep_end.unwrap_or(false);

    let mut warnings = Vec::new();
    if keep_end && args.source_out_tk.is_some() {
        warnings.push(json!({
            "code": W_NOOP_FLAG_CODE,
            "message": "source_out_tk supplied; keep_end ignored",
            "details": {
                "flag": "keep_end",
                "message": "source_out_tk supplied; keep_end ignored",
            }
        }));
    }

    let shift_position =
        keep_end && args.source_out_tk.is_none() && args.source_in_tk.is_some() && delta_in != 0;
    let plans = build_plans(
        prior,
        &members,
        clip_id,
        delta_in,
        delta_out,
        shift_position,
        &mut warnings,
    )?;

    for plan in &plans {
        if plan.track_locked || plan.clip.locked {
            return Err(ClipTrimError::Locked {
                failed_clip: plan.clip.id.to_string(),
            });
        }
    }

    check_planned_overlaps(prior, &plans)?;

    let Some(target_plan) = plans.iter().find(|plan| plan.is_target) else {
        return Err(ClipTrimError::ClipNotFound {
            clip_id: args.clip.clone(),
        });
    };

    let data = ClipTrimData {
        clip_id,
        source_in_tk: target_plan.clip.source_in_tk.get(),
        source_out_tk: target_plan.clip.source_out_tk.get(),
        track_position_tk: target_plan.new_position_tk,
        duration_tk: target_plan.new_duration_tk,
        link_group: target_plan.clip.link_group,
        linked_clip_ids: sorted_linked_clip_ids(&members, clip_id),
    };

    let patch = build_patch(prior, &plans);
    Ok((patch, warnings, data))
}

fn validate_non_negative(value: Option<i64>, field: &'static str) -> Result<(), ClipTrimError> {
    if let Some(value) = value
        && value < 0
    {
        return Err(ClipTrimError::BadTime { field, value });
    }
    Ok(())
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

fn resolve_sync_set<'a>(project: &'a Project, target: LocatedClip<'a>) -> Vec<LocatedClip<'a>> {
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

fn build_plans(
    prior: &Project,
    members: &[LocatedClip<'_>],
    target_id: ClipId,
    delta_in: i64,
    delta_out: i64,
    shift_position: bool,
    warnings: &mut Vec<Value>,
) -> Result<Vec<TrimPlan>, ClipTrimError> {
    members
        .iter()
        .map(|member| {
            let proposed_source_in = member.clip.source_in_tk.get().saturating_add(delta_in);
            let proposed_source_out = member.clip.source_out_tk.get().saturating_add(delta_out);
            check_source_bounds(prior, member, proposed_source_in, proposed_source_out)?;

            let mut new_position_tk = member.clip.track_position_tk.get();
            if shift_position {
                new_position_tk = new_position_tk.saturating_add(delta_in);
            }
            new_position_tk = snap_position(prior, member, new_position_tk, warnings);

            let new_duration_tk = timeline_duration_tk(
                Tick::new(proposed_source_in),
                Tick::new(proposed_source_out),
                member.clip.speed,
            )
            .get();

            let mut clip = member.clip.clone();
            clip.source_in_tk = Tick::new(proposed_source_in);
            clip.source_out_tk = Tick::new(proposed_source_out);
            clip.track_position_tk = Tick::new(new_position_tk);
            clamp_fades(member.clip, &mut clip, new_duration_tk, warnings);
            remove_overflow_keyframes(member.clip, &mut clip, new_duration_tk, warnings);

            Ok(TrimPlan {
                track_idx: member.track_idx,
                clip_idx: member.clip_idx,
                track_locked: member.track_locked,
                clip,
                new_position_tk,
                new_duration_tk,
                is_target: member.clip.id == target_id,
            })
        })
        .collect()
}

fn check_source_bounds(
    prior: &Project,
    member: &LocatedClip<'_>,
    proposed_in: i64,
    proposed_out: i64,
) -> Result<(), ClipTrimError> {
    let bound_min = 0;
    let bound_max = source_bound_max(prior, member);
    if proposed_in < bound_min
        || proposed_out > bound_max
        || proposed_out <= proposed_in
        || (matches!(
            classify_member(prior, member.track_kind, &member.clip.asset_id),
            MemberKind::Image | MemberKind::Text
        ) && proposed_in != 0)
    {
        return Err(ClipTrimError::ClipOutOfBounds {
            failed_clip: member.clip.id.to_string(),
            bound_min,
            bound_max,
            proposed_in,
            proposed_out,
        });
    }
    Ok(())
}

fn source_bound_max(prior: &Project, member: &LocatedClip<'_>) -> i64 {
    match classify_member(prior, member.track_kind, &member.clip.asset_id) {
        MemberKind::Video | MemberKind::Audio => member
            .clip
            .asset_id
            .id()
            .and_then(|asset_id| {
                prior.assets.iter().find_map(|asset| {
                    if asset.id() != asset_id {
                        return None;
                    }
                    match asset {
                        Asset::Video(asset) => Some(asset.metadata.duration_tk.get()),
                        Asset::Audio(asset) => Some(asset.metadata.duration_tk.get()),
                        Asset::Image(_) | Asset::Subtitle(_) => None,
                    }
                })
            })
            .unwrap_or(0),
        MemberKind::Image | MemberKind::Text => i64::MAX,
    }
}

fn snap_position(
    prior: &Project,
    member: &LocatedClip<'_>,
    value_tk: i64,
    warnings: &mut Vec<Value>,
) -> i64 {
    if matches!(
        classify_member(prior, member.track_kind, &member.clip.asset_id),
        MemberKind::Audio
    ) || !is_off_frame(Tick::new(value_tk), prior.fps_num, prior.fps_den)
    {
        return value_tk;
    }

    let snapped_tk = nearest_frame_tick(value_tk, prior.fps_num, prior.fps_den);
    if snapped_tk != value_tk {
        warnings.push(json!({
            "code": W_TIME_SNAPPED_CODE,
            "message": "time value snapped to frame boundary",
            "details": {
                "clip_id": member.clip.id.to_string(),
                "from_tk": value_tk,
                "to_tk": snapped_tk,
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
        "message": "clip fades clamped to fit trimmed duration",
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
    warnings.push(keyframes_removed_warning(clip.id, &removed));
}

fn keyframes_removed_warning(clip_id: ClipId, removed_keyframe_ids: &[KeyframeId]) -> Value {
    json!({
        "code": W_KEYFRAMES_REMOVED_CODE,
        "message": "clip keyframes beyond the trimmed duration were removed",
        "details": {
            "clip_id": clip_id.to_string(),
            "removed_keyframe_ids": stringify_ids(removed_keyframe_ids),
        }
    })
}

fn stringify_ids<T: ToString>(ids: &[T]) -> Vec<String> {
    ids.iter().map(ToString::to_string).collect()
}

fn check_planned_overlaps(prior: &Project, plans: &[TrimPlan]) -> Result<(), ClipTrimError> {
    for plan in plans {
        let start = plan.new_position_tk;
        let end = start.saturating_add(plan.new_duration_tk);

        for other_plan in plans
            .iter()
            .filter(|other| other.track_idx == plan.track_idx && other.clip.id != plan.clip.id)
        {
            let other_start = other_plan.new_position_tk;
            let other_end = other_start.saturating_add(other_plan.new_duration_tk);
            if intervals_overlap(start, end, other_start, other_end) {
                return Err(ClipTrimError::ClipOverlap {
                    failed_clip: plan.clip.id.to_string(),
                });
            }
        }

        for (clip_idx, other) in prior.tracks[plan.track_idx].clips.iter().enumerate() {
            if plans
                .iter()
                .any(|trimmed| trimmed.track_idx == plan.track_idx && trimmed.clip_idx == clip_idx)
            {
                continue;
            }
            let other_start = other.track_position_tk.get();
            let other_end = other_start.saturating_add(
                timeline_duration_tk(other.source_in_tk, other.source_out_tk, other.speed).get(),
            );
            if intervals_overlap(start, end, other_start, other_end) {
                return Err(ClipTrimError::ClipOverlap {
                    failed_clip: plan.clip.id.to_string(),
                });
            }
        }
    }
    Ok(())
}

fn intervals_overlap(a_start: i64, a_end: i64, b_start: i64, b_end: i64) -> bool {
    a_start < b_end && b_start < a_end
}

fn build_patch(prior: &Project, plans: &[TrimPlan]) -> Value {
    let mut ops = Vec::new();

    for plan in plans {
        let old_clip = &prior.tracks[plan.track_idx].clips[plan.clip_idx];
        push_replace_if_changed(
            &mut ops,
            &format!(
                "/tracks/{}/clips/{}/source_in_tk",
                plan.track_idx, plan.clip_idx
            ),
            old_clip.source_in_tk.get(),
            plan.clip.source_in_tk.get(),
        );
        push_replace_if_changed(
            &mut ops,
            &format!(
                "/tracks/{}/clips/{}/source_out_tk",
                plan.track_idx, plan.clip_idx
            ),
            old_clip.source_out_tk.get(),
            plan.clip.source_out_tk.get(),
        );
        push_replace_if_changed(
            &mut ops,
            &format!(
                "/tracks/{}/clips/{}/track_position_tk",
                plan.track_idx, plan.clip_idx
            ),
            old_clip.track_position_tk.get(),
            plan.clip.track_position_tk.get(),
        );
        push_replace_if_changed(
            &mut ops,
            &format!(
                "/tracks/{}/clips/{}/fade_in_tk",
                plan.track_idx, plan.clip_idx
            ),
            old_clip.fade_in_tk.get(),
            plan.clip.fade_in_tk.get(),
        );
        push_replace_if_changed(
            &mut ops,
            &format!(
                "/tracks/{}/clips/{}/fade_out_tk",
                plan.track_idx, plan.clip_idx
            ),
            old_clip.fade_out_tk.get(),
            plan.clip.fade_out_tk.get(),
        );
        if old_clip.keyframes != plan.clip.keyframes {
            ops.push(json!({
                "op": "replace",
                "path": format!(
                    "/tracks/{}/clips/{}/keyframes",
                    plan.track_idx, plan.clip_idx
                ),
                "value": plan.clip.keyframes,
            }));
        }
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

fn push_replace_if_changed(ops: &mut Vec<Value>, path: &str, old_value: i64, new_value: i64) {
    if old_value != new_value {
        ops.push(json!({
            "op": "replace",
            "path": path,
            "value": new_value,
        }));
    }
}

fn planned_project_duration_tk(prior: &Project, plans: &[TrimPlan]) -> i64 {
    let mut computed = 0_i64;
    for (track_idx, track) in prior.tracks.iter().enumerate() {
        for (clip_idx, clip) in track.clips.iter().enumerate() {
            if let Some(planned) = plans
                .iter()
                .find(|plan| plan.track_idx == track_idx && plan.clip_idx == clip_idx)
            {
                computed = computed.max(
                    planned
                        .new_position_tk
                        .saturating_add(planned.new_duration_tk),
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

fn sorted_linked_clip_ids(members: &[LocatedClip<'_>], target_id: ClipId) -> Vec<ClipId> {
    let mut linked = members
        .iter()
        .map(|member| member.clip.id)
        .filter(|id| *id != target_id)
        .collect::<Vec<_>>();
    linked.sort_by_key(ToString::to_string);
    linked
}

fn reject_semantics_mix(
    project: &Project,
    members: &[LocatedClip<'_>],
    link_group: LinkGroupId,
) -> Result<(), ClipTrimError> {
    let mut member_kinds = MemberKindCounts::default();
    let mut semantics_classes = SemanticsClassCounts::default();

    for member in members {
        match classify_member(project, member.track_kind, &member.clip.asset_id) {
            MemberKind::Video => {
                member_kinds.video += 1;
                semantics_classes.source_slice += 1;
            }
            MemberKind::Audio => {
                member_kinds.audio += 1;
                semantics_classes.source_slice += 1;
            }
            MemberKind::Image => {
                member_kinds.image += 1;
                semantics_classes.display_duration += 1;
            }
            MemberKind::Text => {
                member_kinds.text += 1;
                semantics_classes.display_duration += 1;
            }
        }
    }

    if semantics_classes.source_slice > 0 && semantics_classes.display_duration > 0 {
        return Err(ClipTrimError::LinkGroupSemanticsMix {
            link_group,
            member_kinds,
            semantics_classes,
            hint: LINK_GROUP_SEMANTICS_MIX_HINT,
        });
    }

    Ok(())
}

fn classify_member(project: &Project, track_kind: TrackKind, asset_id: &AssetRef) -> MemberKind {
    if track_kind == TrackKind::Text {
        return MemberKind::Text;
    }
    if let Some(asset_id) = asset_id.id()
        && project
            .assets
            .iter()
            .any(|asset| asset.id() == asset_id && matches!(asset, Asset::Image(_)))
    {
        return MemberKind::Image;
    }
    match track_kind {
        TrackKind::Audio => MemberKind::Audio,
        TrackKind::Text => MemberKind::Text,
        TrackKind::Video | TrackKind::Effect => MemberKind::Video,
    }
}

/// Rebuilds the envelope from `(args, post_state)`.
///
/// # Errors
///
/// Returns [`ReconstructError::TypeMismatch`] when `args.clip` is not
/// a valid `UUIDv7`, or [`ReconstructError::PostStateMissing`] when the
/// post-state does not contain the target clip.
pub fn data_envelope_from_post_state(
    args: &ClipTrimArgs,
    post_state: &Project,
) -> Result<ClipTrimData, ReconstructError> {
    let clip_id = args
        .clip
        .parse::<ClipId>()
        .map_err(|_| ReconstructError::TypeMismatch {
            name: "args.clip",
            expected: "UUIDv7 ClipId string",
        })?;

    let target =
        locate_clip(post_state, clip_id).ok_or_else(|| ReconstructError::PostStateMissing {
            detail: format!("clip.trim: clip id {clip_id} not found in post_state"),
        })?;
    let members = resolve_sync_set(post_state, target);
    let duration_tk = timeline_duration_tk(
        target.clip.source_in_tk,
        target.clip.source_out_tk,
        target.clip.speed,
    )
    .get();

    Ok(ClipTrimData {
        clip_id,
        source_in_tk: target.clip.source_in_tk.get(),
        source_out_tk: target.clip.source_out_tk.get(),
        track_position_tk: target.clip.track_position_tk.get(),
        duration_tk,
        link_group: target.clip.link_group,
        linked_clip_ids: sorted_linked_clip_ids(&members, clip_id),
    })
}

/// `clip.trim` verb registration entry.
#[derive(Debug, Default)]
pub struct ClipTrimVerb;

impl From<ClipTrimError> for VerbError {
    fn from(value: ClipTrimError) -> Self {
        VerbError::BadArgs {
            detail: value.to_string(),
        }
    }
}

impl Verb for ClipTrimVerb {
    fn verb(&self) -> &'static str {
        "clip.trim"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: ClipTrimArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("clip.trim: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, _data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!("clip.trim: patch construction failed: {err}"))
            })?;

        let post_state = prior
            .apply(&patch)
            .map_err(|err| VerbError::InvariantViolation {
                detail: format!("clip.trim: post-state validation failed: {err}"),
            })?;

        let envelope = data_envelope_from_post_state(&typed, &post_state).map_err(|err| {
            VerbError::Custom(format!(
                "clip.trim: data envelope reconstruction failed: {err}"
            ))
        })?;
        let data = serde_json::to_value(&envelope)
            .map_err(|err| VerbError::Custom(format!("clip.trim: data serialize failed: {err}")))?;

        Ok((patch, data, warnings))
    }

    fn reconstruct(
        &self,
        args: &Value,
        _patch: &Value,
        _warnings: &[Value],
        post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let typed: ClipTrimArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "ClipTrimArgs",
            })?;

        let envelope = data_envelope_from_post_state(&typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
