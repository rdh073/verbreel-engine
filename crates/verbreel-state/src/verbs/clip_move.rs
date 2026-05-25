//! `clip.move` (§5.4) — forty-sixth production verb in the engine.
//!
//! Moves a clip to a new timeline position and/or to another track of
//! the same kind. Timeline-position changes propagate by delta across
//! the target clip's link group; `to_track` applies only to the target.

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
use verbreel_types::{ClipId, LinkGroupId, ProjectId, TICK_RATE_HZ, Tick, TrackId};

/// Warning code emitted when a moved clip position snaps to a frame.
pub const W_TIME_SNAPPED_CODE: &str = "W_TIME_SNAPPED";

/// Recovery hint returned when no setter field is supplied.
pub const ARGS_INCOMPATIBLE_HINT: &str =
    "supply at least one of track_position_tk or to_track; the verb is a setter, not a query";

/// Recovery hint returned with `E_LINK_GROUP_SEMANTICS_MIX`.
pub const LINK_GROUP_SEMANTICS_MIX_HINT: &str =
    "call clip.unlink first, then mutate each clip independently";

/// Args for `clip.move`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipMoveArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Target clip id as bare `UUIDv7`.
    pub clip: String,

    /// Optional new target timeline position in ticks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_position_tk: Option<i64>,

    /// Optional destination track id as bare `UUIDv7`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_track: Option<String>,
}

/// Envelope `data` returned by `clip.move`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipMoveData {
    /// Target clip id.
    pub clip_id: ClipId,

    /// Target clip's post-move parent track id.
    pub track_id: TrackId,

    /// Target clip's post-move timeline position.
    pub track_position_tk: i64,

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

/// Verb-level validation failures for `clip.move`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ClipMoveError {
    /// Neither `track_position_tk` nor `to_track` was supplied.
    #[error("E_ARGS_INCOMPATIBLE: clip.move: {hint}")]
    ArgsIncompatible {
        /// Recovery hint.
        hint: &'static str,
    },

    /// A selector is not parseable as `UUIDv7`.
    #[error("E_BAD_SELECTOR: clip.move: `{field}` selector parse failed: {detail}")]
    BadSelector {
        /// Argument field name.
        field: &'static str,
        /// Parse failure detail.
        detail: String,
    },

    /// No clip exists for `clip`.
    #[error("E_NOT_FOUND: clip.move: clip `{clip_id}` not found")]
    ClipNotFound {
        /// Missing clip id string.
        clip_id: String,
    },

    /// No track exists for `to_track`.
    #[error("E_TRACK_NOT_FOUND: clip.move: track `{track_id}` not found")]
    TrackNotFound {
        /// Missing track id string.
        track_id: String,
    },

    /// Destination track kind differs from the current target track.
    #[error(
        "E_TRACK_KIND_MISMATCH: clip.move: expected destination track kind `{expected_kind}`, got `{actual_kind}`"
    )]
    TrackKindMismatch {
        /// Current target track kind.
        expected_kind: &'static str,
        /// Destination track kind.
        actual_kind: &'static str,
    },

    /// A provided timeline position is negative.
    #[error("E_BAD_TIME: clip.move: `{field}` value {value} must be >= 0")]
    BadTime {
        /// Invalid field name.
        field: &'static str,
        /// Invalid value.
        value: i64,
    },

    /// A link group mixes source-slice and display-duration classes.
    #[error(
        "E_LINK_GROUP_SEMANTICS_MIX: clip.move: link group `{link_group}` mixes source-time semantics classes"
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

    /// A sync-set member or its new parent track is locked.
    #[error("E_LOCKED: clip.move: clip `{failed_clip}` or its new parent track is locked")]
    Locked {
        /// First failed member in deterministic track/clip order.
        failed_clip: String,
    },

    /// A sync-set member would overlap another clip after the move.
    #[error("E_CLIP_OVERLAP: clip.move: clip `{failed_clip}` would overlap on its track")]
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
    track_id: TrackId,
    track_kind: TrackKind,
    track_locked: bool,
    clip: &'a Clip,
}

#[derive(Debug, Clone, Copy)]
struct LocatedTrack {
    idx: usize,
    id: TrackId,
    kind: TrackKind,
    locked: bool,
}

#[derive(Debug, Clone)]
struct MovePlan {
    old_track_idx: usize,
    old_clip_idx: usize,
    new_track_idx: usize,
    new_track_id: TrackId,
    new_track_locked: bool,
    clip: Clip,
    new_position_tk: i64,
    is_target: bool,
}

/// Build the RFC-6902 patch for `clip.move`.
///
/// # Errors
///
/// Returns [`ClipMoveError`] for selector parse failure, missing clip
/// or track, kind mismatch, negative position, mixed link-group
/// semantics, lock conflicts, or post-move overlap.
pub fn compute_patch(
    prior: &Project,
    args: &ClipMoveArgs,
) -> Result<(Value, Vec<Value>, ClipMoveData), ClipMoveError> {
    if args.track_position_tk.is_none() && args.to_track.is_none() {
        return Err(ClipMoveError::ArgsIncompatible {
            hint: ARGS_INCOMPATIBLE_HINT,
        });
    }

    let clip_id = args
        .clip
        .parse::<ClipId>()
        .map_err(|err| ClipMoveError::BadSelector {
            field: "clip",
            detail: err.to_string(),
        })?;

    let target = locate_clip(prior, clip_id).ok_or_else(|| ClipMoveError::ClipNotFound {
        clip_id: args.clip.clone(),
    })?;

    let target_new_track = if let Some(to_track) = args.to_track.as_ref() {
        let to_track_id =
            to_track
                .parse::<TrackId>()
                .map_err(|err| ClipMoveError::BadSelector {
                    field: "to_track",
                    detail: err.to_string(),
                })?;
        let track =
            locate_track(prior, to_track_id).ok_or_else(|| ClipMoveError::TrackNotFound {
                track_id: to_track.clone(),
            })?;
        if track.kind != target.track_kind {
            return Err(ClipMoveError::TrackKindMismatch {
                expected_kind: kind_name(target.track_kind),
                actual_kind: kind_name(track.kind),
            });
        }
        track
    } else {
        LocatedTrack {
            idx: target.track_idx,
            id: target.track_id,
            kind: target.track_kind,
            locked: target.track_locked,
        }
    };

    if let Some(position_tk) = args.track_position_tk
        && position_tk < 0
    {
        return Err(ClipMoveError::BadTime {
            field: "track_position_tk",
            value: position_tk,
        });
    }

    let members = resolve_sync_set(prior, target);
    if let Some(target_group) = target.clip.link_group
        && members.len() > 1
    {
        reject_semantics_mix(prior, &members, target_group)?;
    }

    let delta_tk = args.track_position_tk.map_or(0, |position_tk| {
        position_tk - target.clip.track_position_tk.get()
    });

    let mut warnings = Vec::new();
    let plans = build_plans(
        prior,
        &members,
        clip_id,
        target_new_track,
        delta_tk,
        &mut warnings,
    );

    for plan in &plans {
        if plan.new_track_locked || plan.clip.locked {
            return Err(ClipMoveError::Locked {
                failed_clip: plan.clip.id.to_string(),
            });
        }
    }

    check_planned_overlaps(prior, &plans)?;

    let Some(target_plan) = plans.iter().find(|plan| plan.is_target) else {
        return Err(ClipMoveError::ClipNotFound {
            clip_id: args.clip.clone(),
        });
    };
    let data = ClipMoveData {
        clip_id,
        track_id: target_plan.new_track_id,
        track_position_tk: target_plan.new_position_tk,
        linked_clip_ids: sorted_linked_clip_ids(&members, clip_id),
    };

    let patch = build_patch(prior, &plans);
    Ok((patch, warnings, data))
}

fn locate_clip(project: &Project, clip_id: ClipId) -> Option<LocatedClip<'_>> {
    for (track_idx, track) in project.tracks.iter().enumerate() {
        for (clip_idx, clip) in track.clips.iter().enumerate() {
            if clip.id == clip_id {
                return Some(LocatedClip {
                    track_idx,
                    clip_idx,
                    track_id: track.id,
                    track_kind: track.kind,
                    track_locked: track.locked,
                    clip,
                });
            }
        }
    }
    None
}

fn locate_track(project: &Project, track_id: TrackId) -> Option<LocatedTrack> {
    project
        .tracks
        .iter()
        .enumerate()
        .find_map(|(track_idx, track)| {
            (track.id == track_id).then_some(LocatedTrack {
                idx: track_idx,
                id: track.id,
                kind: track.kind,
                locked: track.locked,
            })
        })
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
                        track_id: track.id,
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
    target_new_track: LocatedTrack,
    delta_tk: i64,
    warnings: &mut Vec<Value>,
) -> Vec<MovePlan> {
    members
        .iter()
        .map(|member| {
            let is_target = member.clip.id == target_id;
            let new_track = if is_target {
                target_new_track
            } else {
                LocatedTrack {
                    idx: member.track_idx,
                    id: member.track_id,
                    kind: member.track_kind,
                    locked: member.track_locked,
                }
            };
            let unsnapped_position_tk =
                member.clip.track_position_tk.get().saturating_add(delta_tk);
            let new_position_tk = snap_position(prior, member, unsnapped_position_tk, warnings);
            let mut clip = member.clip.clone();
            clip.track_position_tk = Tick::new(new_position_tk);
            MovePlan {
                old_track_idx: member.track_idx,
                old_clip_idx: member.clip_idx,
                new_track_idx: new_track.idx,
                new_track_id: new_track.id,
                new_track_locked: new_track.locked,
                clip,
                new_position_tk,
                is_target,
            }
        })
        .collect()
}

fn snap_position(
    prior: &Project,
    member: &LocatedClip<'_>,
    value_tk: i64,
    warnings: &mut Vec<Value>,
) -> i64 {
    if !matches!(member.track_kind, TrackKind::Video | TrackKind::Text)
        || !is_off_frame(Tick::new(value_tk), prior.fps_num, prior.fps_den)
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

fn check_planned_overlaps(prior: &Project, plans: &[MovePlan]) -> Result<(), ClipMoveError> {
    for plan in plans {
        let start = plan.new_position_tk;
        let end = start.saturating_add(
            timeline_duration_tk(
                plan.clip.source_in_tk,
                plan.clip.source_out_tk,
                plan.clip.speed,
            )
            .get(),
        );

        for other_plan in plans.iter().filter(|other| {
            other.new_track_idx == plan.new_track_idx && other.clip.id != plan.clip.id
        }) {
            let other_start = other_plan.new_position_tk;
            let other_end = other_start.saturating_add(
                timeline_duration_tk(
                    other_plan.clip.source_in_tk,
                    other_plan.clip.source_out_tk,
                    other_plan.clip.speed,
                )
                .get(),
            );
            if intervals_overlap(start, end, other_start, other_end) {
                return Err(ClipMoveError::ClipOverlap {
                    failed_clip: plan.clip.id.to_string(),
                });
            }
        }

        for (clip_idx, other) in prior.tracks[plan.new_track_idx].clips.iter().enumerate() {
            if plans.iter().any(|moved| {
                moved.old_track_idx == plan.new_track_idx && moved.old_clip_idx == clip_idx
            }) {
                continue;
            }
            let other_start = other.track_position_tk.get();
            let other_end = other_start.saturating_add(
                timeline_duration_tk(other.source_in_tk, other.source_out_tk, other.speed).get(),
            );
            if intervals_overlap(start, end, other_start, other_end) {
                return Err(ClipMoveError::ClipOverlap {
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

fn build_patch(prior: &Project, plans: &[MovePlan]) -> Value {
    let mut ops = Vec::new();

    for plan in plans {
        if plan.is_target && plan.new_track_idx != plan.old_track_idx {
            continue;
        }
        if plan.new_position_tk
            != prior.tracks[plan.old_track_idx].clips[plan.old_clip_idx]
                .track_position_tk
                .get()
        {
            ops.push(json!({
                "op": "replace",
                "path": format!(
                    "/tracks/{}/clips/{}/track_position_tk",
                    plan.old_track_idx, plan.old_clip_idx
                ),
                "value": plan.new_position_tk,
            }));
        }
    }

    if let Some(target_plan) = plans
        .iter()
        .find(|plan| plan.is_target && plan.new_track_idx != plan.old_track_idx)
    {
        ops.push(json!({
            "op": "add",
            "path": format!("/tracks/{}/clips/-", target_plan.new_track_idx),
            "value": target_plan.clip,
        }));
        ops.push(json!({
            "op": "remove",
            "path": format!(
                "/tracks/{}/clips/{}",
                target_plan.old_track_idx, target_plan.old_clip_idx
            ),
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

fn planned_project_duration_tk(prior: &Project, plans: &[MovePlan]) -> i64 {
    let mut computed = 0_i64;
    for (track_idx, track) in prior.tracks.iter().enumerate() {
        for (clip_idx, clip) in track.clips.iter().enumerate() {
            let planned = plans
                .iter()
                .find(|plan| plan.old_track_idx == track_idx && plan.old_clip_idx == clip_idx);
            let position_tk =
                planned.map_or_else(|| clip.track_position_tk.get(), |plan| plan.new_position_tk);
            let duration_tk =
                timeline_duration_tk(clip.source_in_tk, clip.source_out_tk, clip.speed);
            computed = computed.max(position_tk.saturating_add(duration_tk.get()));
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
) -> Result<(), ClipMoveError> {
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
        return Err(ClipMoveError::LinkGroupSemanticsMix {
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

fn kind_name(kind: TrackKind) -> &'static str {
    match kind {
        TrackKind::Video => "video",
        TrackKind::Audio => "audio",
        TrackKind::Text => "text",
        TrackKind::Effect => "effect",
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
    args: &ClipMoveArgs,
    post_state: &Project,
) -> Result<ClipMoveData, ReconstructError> {
    let clip_id = args
        .clip
        .parse::<ClipId>()
        .map_err(|_| ReconstructError::TypeMismatch {
            name: "args.clip",
            expected: "UUIDv7 ClipId string",
        })?;

    let target =
        locate_clip(post_state, clip_id).ok_or_else(|| ReconstructError::PostStateMissing {
            detail: format!("clip.move: clip id {clip_id} not found in post_state"),
        })?;
    let members = resolve_sync_set(post_state, target);

    Ok(ClipMoveData {
        clip_id,
        track_id: target.track_id,
        track_position_tk: target.clip.track_position_tk.get(),
        linked_clip_ids: sorted_linked_clip_ids(&members, clip_id),
    })
}

/// `clip.move` verb registration entry.
#[derive(Debug, Default)]
pub struct ClipMoveVerb;

impl From<ClipMoveError> for VerbError {
    fn from(value: ClipMoveError) -> Self {
        VerbError::BadArgs {
            detail: value.to_string(),
        }
    }
}

impl Verb for ClipMoveVerb {
    fn verb(&self) -> &'static str {
        "clip.move"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: ClipMoveArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("clip.move: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, _data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!("clip.move: patch construction failed: {err}"))
            })?;

        let post_state = prior
            .apply(&patch)
            .map_err(|err| VerbError::InvariantViolation {
                detail: format!("clip.move: post-state validation failed: {err}"),
            })?;

        let envelope = data_envelope_from_post_state(&typed, &post_state).map_err(|err| {
            VerbError::Custom(format!(
                "clip.move: data envelope reconstruction failed: {err}"
            ))
        })?;
        let data = serde_json::to_value(&envelope)
            .map_err(|err| VerbError::Custom(format!("clip.move: data serialize failed: {err}")))?;

        Ok((patch, data, warnings))
    }

    fn reconstruct(
        &self,
        args: &Value,
        _patch: &Value,
        _warnings: &[Value],
        post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let typed: ClipMoveArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "ClipMoveArgs",
            })?;

        let envelope = data_envelope_from_post_state(&typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
