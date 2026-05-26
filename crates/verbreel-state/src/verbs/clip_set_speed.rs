//! `clip.set_speed` (§5.7) — fifty-third production verb in the engine.
//!
//! Sets the scalar [`crate::clip::Clip::speed`] value on source-slice
//! clips and propagates the same speed to every member of the target's
//! link group per §5.15.
//!
//! ## Deferrals
//!
//! This slice is scalar-only. Deferred from the full §5.7 contract:
//! `preserve_pitch`, `time_stretch` managed-effect creation / removal /
//! param update, `W_NOOP_FLAG` on `preserve_pitch`, `time_stretch`
//! propagation across audio members, and cross-clip `speed_curve` sync.

use crate::asset::Asset;
use crate::clip::Clip;
use crate::effect::Effect;
use crate::invariants::{extract_effect_id_from_property, timeline_duration_tk};
use crate::newtypes::AssetRef;
use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use crate::track::TrackKind;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::{ClipId, EffectId, KeyframeId, LinkGroupId, ProjectId, Tick};

/// JavaScript `Number.MAX_SAFE_INTEGER`, the host integer ceiling
/// referenced by the spec.
const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

/// Factor must be in this interval.
const FACTOR_ALLOWED: &str = "(0, 100]";

/// Warning code emitted when fades are clamped to the new duration.
pub const W_FADE_CLAMPED_CODE: &str = "W_FADE_CLAMPED";

/// Warning code emitted when keyframes are removed.
pub const W_KEYFRAMES_REMOVED_CODE: &str = "W_KEYFRAMES_REMOVED";

/// Warning code emitted when an effect window is clamped.
pub const W_EFFECT_WINDOW_CLAMPED_CODE: &str = "W_EFFECT_WINDOW_CLAMPED";

/// Warning code emitted when an existing `time_stretch` effect is driven
/// at an extreme scalar speed.
pub const W_SPEED_EXTREME_CODE: &str = "W_SPEED_EXTREME";

/// Recovery hint returned with `E_LINK_GROUP_SEMANTICS_MIX`.
pub const LINK_GROUP_SEMANTICS_MIX_HINT: &str =
    "call clip.unlink first, then mutate each clip independently";

/// Args for `clip.set_speed`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipSetSpeedArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Target clip id as bare `UUIDv7`.
    pub clip: String,

    /// Scalar playback factor.
    pub factor: f64,
}

/// Envelope `data` returned by `clip.set_speed`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipSetSpeedData {
    /// Target clip id.
    pub clip_id: ClipId,
    /// Post-state scalar speed.
    pub speed: f64,
    /// Target clip's post-state timeline duration.
    pub duration_tk: i64,
    /// Deferred managed `time_stretch` result; always `None` in this slice.
    pub time_stretch_effect_id: Option<EffectId>,
    /// Deferred managed `time_stretch` removal; always `None` in this slice.
    pub removed_time_stretch_effect_id: Option<EffectId>,
    /// Other members of the target link group, sorted lexicographically.
    pub linked_clip_ids: Vec<ClipId>,
    /// Deferred linked `time_stretch` results; always empty in this slice.
    pub linked_time_stretch_effect_ids: Vec<EffectId>,
    /// Deferred linked `time_stretch` removals; always empty in this slice.
    pub removed_linked_time_stretch_effect_ids: Vec<EffectId>,
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

/// Verb-level validation failures for `clip.set_speed`.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum ClipSetSpeedError {
    /// `args.clip` is not parseable as `UUIDv7`.
    #[error("E_BAD_SELECTOR: clip.set_speed: `clip` selector parse failed: {detail}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
    },

    /// No clip exists for `clip`.
    #[error("E_NOT_FOUND: clip.set_speed: clip `{clip_id}` not found")]
    ClipNotFound {
        /// Missing clip id string.
        clip_id: String,
    },

    /// Factor is outside `(0, 100]`.
    #[error("E_SCHEMA_VIOLATION: clip.set_speed: factor {value} must satisfy {allowed}")]
    SchemaViolation {
        /// Invalid field name.
        field: &'static str,
        /// Allowed interval.
        allowed: &'static str,
        /// Offending value.
        value: f64,
    },

    /// Target clip has display-duration semantics.
    #[error(
        "E_CLIP_KIND_MISMATCH: clip.set_speed: clip `{clip_id}` is `{actual_kind}`, expected source-slice clip"
    )]
    ClipKindMismatch {
        /// Target clip id.
        clip_id: ClipId,
        /// Actual semantic kind.
        actual_kind: &'static str,
    },

    /// A link group mixes source-slice and display-duration classes.
    #[error(
        "E_LINK_GROUP_SEMANTICS_MIX: clip.set_speed: link group `{link_group}` mixes source-time semantics classes"
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

    /// A sync-set member or its parent track is locked.
    #[error("E_LOCKED: clip.set_speed: clip `{failed_clip}` or its parent track is locked")]
    Locked {
        /// First failed member in deterministic track/clip order.
        failed_clip: String,
    },

    /// New duration would exceed the host safe integer range.
    #[error(
        "E_BAD_TIME: clip.set_speed: `{field}` overflows timeline duration for clip `{clip_id}`"
    )]
    BadTime {
        /// Invalid field name.
        field: &'static str,
        /// Failed member id.
        clip_id: String,
    },

    /// A sync-set member would overlap another clip after speed change.
    #[error("E_CLIP_OVERLAP: clip.set_speed: clip `{failed_clip}` would overlap on its track")]
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
struct SpeedPlan {
    track_idx: usize,
    clip_idx: usize,
    clip: Clip,
    new_duration_tk: i64,
    is_target: bool,
}

/// Build the RFC-6902 patch for `clip.set_speed`.
///
/// # Errors
///
/// Returns [`ClipSetSpeedError`] for selector parse failure, missing
/// target clip, invalid factor, display-duration clip kind, mixed
/// link-group semantics, locked members, duration overflow, or
/// post-speed overlap.
pub fn compute_patch(
    prior: &Project,
    args: &ClipSetSpeedArgs,
) -> Result<(Value, Vec<Value>, ClipSetSpeedData), ClipSetSpeedError> {
    let clip_id = args
        .clip
        .parse::<ClipId>()
        .map_err(|err| ClipSetSpeedError::BadSelector {
            detail: err.to_string(),
        })?;

    let target = locate_clip(prior, clip_id).ok_or_else(|| ClipSetSpeedError::ClipNotFound {
        clip_id: args.clip.clone(),
    })?;

    validate_factor(args.factor)?;

    if let Some(actual_kind) =
        display_duration_kind(prior, target.track_kind, &target.clip.asset_id)
    {
        return Err(ClipSetSpeedError::ClipKindMismatch {
            clip_id,
            actual_kind,
        });
    }

    let members = resolve_sync_set(prior, target);
    if let Some(target_group) = target.clip.link_group
        && members.len() > 1
    {
        reject_semantics_mix(prior, &members, target_group)?;
    }

    for member in &members {
        if member.track_locked || member.clip.locked {
            return Err(ClipSetSpeedError::Locked {
                failed_clip: member.clip.id.to_string(),
            });
        }
    }

    let mut warnings = Vec::new();
    let plans = build_plans(&members, clip_id, args.factor, &mut warnings)?;
    check_planned_overlaps(prior, &plans)?;

    let Some(target_plan) = plans.iter().find(|plan| plan.is_target) else {
        return Err(ClipSetSpeedError::ClipNotFound {
            clip_id: args.clip.clone(),
        });
    };

    let data = ClipSetSpeedData {
        clip_id,
        speed: args.factor,
        duration_tk: target_plan.new_duration_tk,
        time_stretch_effect_id: None,
        removed_time_stretch_effect_id: None,
        linked_clip_ids: sorted_linked_clip_ids(&members, clip_id),
        linked_time_stretch_effect_ids: Vec::new(),
        removed_linked_time_stretch_effect_ids: Vec::new(),
    };

    let patch = build_patch(prior, &plans);
    Ok((patch, warnings, data))
}

fn validate_factor(factor: f64) -> Result<(), ClipSetSpeedError> {
    if factor.is_finite() && factor > 0.0 && factor <= 100.0 {
        return Ok(());
    }

    Err(ClipSetSpeedError::SchemaViolation {
        field: "factor",
        allowed: FACTOR_ALLOWED,
        value: factor,
    })
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
    members: &[LocatedClip<'_>],
    target_id: ClipId,
    factor: f64,
    warnings: &mut Vec<Value>,
) -> Result<Vec<SpeedPlan>, ClipSetSpeedError> {
    members
        .iter()
        .map(|member| {
            let new_duration_tk = checked_duration_tk(member.clip, factor)?;
            let mut clip = member.clip.clone();
            clip.speed = factor;
            clamp_fades(member.clip, &mut clip, new_duration_tk, warnings);
            remove_overflow_keyframes(member.clip, &mut clip, new_duration_tk, warnings);
            warn_extreme_speed(member.clip, factor, warnings);
            clamp_or_remove_effect_windows(member.clip, &mut clip, new_duration_tk, warnings);

            Ok(SpeedPlan {
                track_idx: member.track_idx,
                clip_idx: member.clip_idx,
                clip,
                new_duration_tk,
                is_target: member.clip.id == target_id,
            })
        })
        .collect()
}

#[allow(clippy::cast_precision_loss)]
fn checked_duration_tk(clip: &Clip, factor: f64) -> Result<i64, ClipSetSpeedError> {
    let diff = clip
        .source_out_tk
        .get()
        .saturating_sub(clip.source_in_tk.get());
    let scaled = (diff as f64) / factor;
    if !scaled.is_finite() || scaled > MAX_SAFE_INTEGER {
        return Err(ClipSetSpeedError::BadTime {
            field: "factor",
            clip_id: clip.id.to_string(),
        });
    }

    Ok(timeline_duration_tk(clip.source_in_tk, clip.source_out_tk, factor).get())
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
        "message": "clip fades clamped to fit speed-adjusted duration",
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
        "clip keyframes beyond the speed-adjusted duration were removed",
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
            continue;
        }

        if window.out_tk.get() > new_duration_tk {
            let from_out_tk = window.out_tk.get();
            window.out_tk = Tick::new(new_duration_tk);
            next.window = Some(window);
            changed = true;
            warnings.push(json!({
                "code": W_EFFECT_WINDOW_CLAMPED_CODE,
                "message": "effect window clamped to fit speed-adjusted duration",
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

fn warn_extreme_speed(clip: &Clip, factor: f64, warnings: &mut Vec<Value>) {
    if factor <= 16.0 || !clip.effects.iter().any(is_time_stretch) {
        return;
    }

    warnings.push(json!({
        "code": W_SPEED_EXTREME_CODE,
        "message": "extreme speed with time_stretch effect may produce artifacts",
        "details": {
            "clip_id": clip.id.to_string(),
            "factor": factor,
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

fn check_planned_overlaps(prior: &Project, plans: &[SpeedPlan]) -> Result<(), ClipSetSpeedError> {
    for plan in plans {
        let start = plan.clip.track_position_tk.get();
        let end = start.saturating_add(plan.new_duration_tk);

        for other_plan in plans
            .iter()
            .filter(|other| other.track_idx == plan.track_idx && other.clip.id != plan.clip.id)
        {
            let other_start = other_plan.clip.track_position_tk.get();
            let other_end = other_start.saturating_add(other_plan.new_duration_tk);
            if intervals_overlap(start, end, other_start, other_end) {
                return Err(ClipSetSpeedError::ClipOverlap {
                    failed_clip: plan.clip.id.to_string(),
                });
            }
        }

        for (clip_idx, other) in prior.tracks[plan.track_idx].clips.iter().enumerate() {
            if plans
                .iter()
                .any(|speed| speed.track_idx == plan.track_idx && speed.clip_idx == clip_idx)
            {
                continue;
            }
            let other_start = other.track_position_tk.get();
            let other_end = other_start.saturating_add(
                timeline_duration_tk(other.source_in_tk, other.source_out_tk, other.speed).get(),
            );
            if intervals_overlap(start, end, other_start, other_end) {
                return Err(ClipSetSpeedError::ClipOverlap {
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

fn build_patch(prior: &Project, plans: &[SpeedPlan]) -> Value {
    let mut ops = Vec::new();

    for plan in plans {
        let old_clip = &prior.tracks[plan.track_idx].clips[plan.clip_idx];
        push_replace_f64_if_changed(
            &mut ops,
            &format!("/tracks/{}/clips/{}/speed", plan.track_idx, plan.clip_idx),
            old_clip.speed,
            plan.clip.speed,
        );
        push_replace_i64_if_changed(
            &mut ops,
            &format!(
                "/tracks/{}/clips/{}/fade_in_tk",
                plan.track_idx, plan.clip_idx
            ),
            old_clip.fade_in_tk.get(),
            plan.clip.fade_in_tk.get(),
        );
        push_replace_i64_if_changed(
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
        if old_clip.effects != plan.clip.effects {
            ops.push(json!({
                "op": "replace",
                "path": format!(
                    "/tracks/{}/clips/{}/effects",
                    plan.track_idx, plan.clip_idx
                ),
                "value": plan.clip.effects,
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

fn push_replace_i64_if_changed(ops: &mut Vec<Value>, path: &str, old_value: i64, new_value: i64) {
    if old_value != new_value {
        ops.push(json!({
            "op": "replace",
            "path": path,
            "value": new_value,
        }));
    }
}

fn push_replace_f64_if_changed(ops: &mut Vec<Value>, path: &str, old_value: f64, new_value: f64) {
    if old_value.to_bits() != new_value.to_bits() {
        ops.push(json!({
            "op": "replace",
            "path": path,
            "value": new_value,
        }));
    }
}

fn planned_project_duration_tk(prior: &Project, plans: &[SpeedPlan]) -> i64 {
    let mut computed = 0_i64;
    for (track_idx, track) in prior.tracks.iter().enumerate() {
        for (clip_idx, clip) in track.clips.iter().enumerate() {
            if let Some(planned) = plans
                .iter()
                .find(|plan| plan.track_idx == track_idx && plan.clip_idx == clip_idx)
            {
                computed = computed.max(
                    planned
                        .clip
                        .track_position_tk
                        .get()
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
) -> Result<(), ClipSetSpeedError> {
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
        return Err(ClipSetSpeedError::LinkGroupSemanticsMix {
            link_group,
            member_kinds,
            semantics_classes,
            hint: LINK_GROUP_SEMANTICS_MIX_HINT,
        });
    }

    Ok(())
}

fn display_duration_kind(
    project: &Project,
    track_kind: TrackKind,
    asset_id: &AssetRef,
) -> Option<&'static str> {
    match classify_member(project, track_kind, asset_id) {
        MemberKind::Image => Some("image"),
        MemberKind::Text => Some("text"),
        MemberKind::Video | MemberKind::Audio => None,
    }
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
/// a valid `UUIDv7`, or [`ReconstructError::PostStateMissing`] when
/// the post-state does not contain the target clip.
pub fn data_envelope_from_post_state(
    args: &ClipSetSpeedArgs,
    post_state: &Project,
) -> Result<ClipSetSpeedData, ReconstructError> {
    let clip_id = args
        .clip
        .parse::<ClipId>()
        .map_err(|_| ReconstructError::TypeMismatch {
            name: "args.clip",
            expected: "UUIDv7 ClipId string",
        })?;

    let target =
        locate_clip(post_state, clip_id).ok_or_else(|| ReconstructError::PostStateMissing {
            detail: format!("clip.set_speed: clip id {clip_id} not found in post_state"),
        })?;
    let members = resolve_sync_set(post_state, target);
    let duration_tk = timeline_duration_tk(
        target.clip.source_in_tk,
        target.clip.source_out_tk,
        target.clip.speed,
    )
    .get();

    Ok(ClipSetSpeedData {
        clip_id,
        speed: target.clip.speed,
        duration_tk,
        time_stretch_effect_id: None,
        removed_time_stretch_effect_id: None,
        linked_clip_ids: sorted_linked_clip_ids(&members, clip_id),
        linked_time_stretch_effect_ids: Vec::new(),
        removed_linked_time_stretch_effect_ids: Vec::new(),
    })
}

impl From<ClipSetSpeedError> for VerbError {
    fn from(value: ClipSetSpeedError) -> Self {
        VerbError::BadArgs {
            detail: value.to_string(),
        }
    }
}

/// `clip.set_speed` verb registration entry.
#[derive(Debug, Default)]
pub struct ClipSetSpeedVerb;

impl Verb for ClipSetSpeedVerb {
    fn verb(&self) -> &'static str {
        "clip.set_speed"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: ClipSetSpeedArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("clip.set_speed: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, _data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!("clip.set_speed: patch construction failed: {err}"))
            })?;

        let post_state = prior
            .apply(&patch)
            .map_err(|err| VerbError::InvariantViolation {
                detail: format!("clip.set_speed: post-state validation failed: {err}"),
            })?;

        let envelope = data_envelope_from_post_state(&typed, &post_state).map_err(|err| {
            VerbError::Custom(format!(
                "clip.set_speed: data envelope reconstruction failed: {err}"
            ))
        })?;
        let data = serde_json::to_value(&envelope).map_err(|err| {
            VerbError::Custom(format!("clip.set_speed: data serialize failed: {err}"))
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
        let typed: ClipSetSpeedArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "ClipSetSpeedArgs",
            })?;

        let envelope = data_envelope_from_post_state(&typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
