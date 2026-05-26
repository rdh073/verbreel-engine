//! `clip.duplicate` (§5.6) — forty-ninth production verb in the engine.
//!
//! Clones a clip onto its source track. Linked clips are structurally
//! duplicated as a new linked unit without source-semantics-mix
//! rejection per §5.15.

use std::collections::{BTreeMap, HashMap};

use crate::clip::Clip;
use crate::invariants::timeline_duration_tk;
use crate::keyframe::KeyframeProperty;
use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::{ClipId, EffectId, KeyframeId, LinkGroupId, ProjectId, Tick, TrackId};

/// Internal warning code carrying minted IDs for reconstructor replay.
pub const W_CLIP_DUPLICATE_ENVELOPE_CODE: &str = "W_CLIP_DUPLICATE_ENVELOPE";

/// Recovery hint returned when `auto_gap` and a positive `gap_tk` are
/// both supplied.
pub const AUTO_GAP_INCOMPATIBLE_HINT: &str = "auto_gap is mutually exclusive with non-zero gap_tk";

/// Args for `clip.duplicate`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipDuplicateArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Target clip id as bare `UUIDv7`.
    pub clip: String,

    /// Optional gap after each source clip's end tick. Omitted values
    /// default to `0`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gap_tk: Option<i64>,

    /// Whether the engine should resolve the next free target-track
    /// slot and apply that same resolved gap delta to every linked
    /// sibling.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_gap: Option<bool>,
}

/// One linked sibling duplicate returned in the `clip.duplicate`
/// envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SiblingDuplicate {
    /// Source sibling clip id.
    pub source_clip_id: ClipId,
    /// Freshly minted sibling duplicate id.
    pub new_clip_id: ClipId,
    /// Sibling duplicate timeline position on its own track.
    pub track_position_tk: i64,
}

/// Envelope `data` returned by `clip.duplicate`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipDuplicateData {
    /// Target source clip id.
    pub source_clip_id: ClipId,
    /// Freshly minted target duplicate id.
    pub new_clip_id: ClipId,
    /// Target duplicate timeline position.
    pub track_position_tk: i64,
    /// Resolved gap after the source clip's end tick.
    pub resolved_gap_tk: i64,
    /// Freshly minted link group shared by duplicates, or `null` for
    /// singleton sources.
    pub new_link_group: Option<LinkGroupId>,
    /// Linked siblings duplicated by the same operation, excluding
    /// the target.
    pub sibling_duplicates: Vec<SiblingDuplicate>,
}

/// Verb-level validation failures for `clip.duplicate`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ClipDuplicateError {
    /// `args.clip` is not parseable as `UUIDv7`.
    #[error("E_BAD_SELECTOR: clip.duplicate: `clip` selector parse failed: {detail}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
    },

    /// No clip exists for `clip`.
    #[error("E_NOT_FOUND: clip.duplicate: clip `{clip_id}` not found")]
    ClipNotFound {
        /// Missing clip id string.
        clip_id: String,
    },

    /// Mutually exclusive args were supplied together.
    #[error("E_ARGS_INCOMPATIBLE: clip.duplicate: {hint}")]
    ArgsIncompatible {
        /// Recovery hint.
        hint: &'static str,
    },

    /// A time argument is negative.
    #[error("E_BAD_TIME: clip.duplicate: `{field}` value {value} must be >= 0")]
    BadTime {
        /// Invalid field name.
        field: &'static str,
        /// Invalid value.
        value: i64,
    },

    /// A destination track is locked.
    #[error("E_LOCKED: clip.duplicate: destination track `{failed_target}` is locked")]
    Locked {
        /// Locked destination track id.
        failed_target: String,
    },

    /// A planned duplicate would overlap another clip on its track.
    #[error("E_CLIP_OVERLAP: clip.duplicate: duplicate of `{failed_clip}` would overlap")]
    ClipOverlap {
        /// Source clip whose planned duplicate failed.
        failed_clip: String,
    },
}

#[derive(Debug, Clone, Copy)]
struct LocatedClip<'a> {
    track_idx: usize,
    clip_idx: usize,
    track_id: TrackId,
    track_locked: bool,
    clip: &'a Clip,
}

#[derive(Debug, Clone)]
struct DuplicatePlan {
    track_idx: usize,
    source_clip_idx: usize,
    source_clip_id: ClipId,
    new_position_tk: i64,
    duration_tk: i64,
    duplicate: Clip,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EnvelopeWarningDetails {
    new_clip_id: ClipId,
    new_link_group: Option<LinkGroupId>,
    resolved_gap_tk: i64,
    sibling_new_clip_ids: BTreeMap<String, ClipId>,
}

/// Build the RFC-6902 patch for `clip.duplicate`.
///
/// # Errors
///
/// Returns [`ClipDuplicateError`] for selector parse failure, missing
/// target clip, incompatible args, negative gaps, locked destination
/// tracks, or overlap conflicts.
pub fn compute_patch(
    prior: &Project,
    args: &ClipDuplicateArgs,
) -> Result<(Value, Vec<Value>, ClipDuplicateData), ClipDuplicateError> {
    let clip_id = args
        .clip
        .parse::<ClipId>()
        .map_err(|err| ClipDuplicateError::BadSelector {
            detail: err.to_string(),
        })?;

    let target = locate_clip(prior, clip_id).ok_or_else(|| ClipDuplicateError::ClipNotFound {
        clip_id: args.clip.clone(),
    })?;

    let gap_tk = args.gap_tk.unwrap_or(0);
    let auto_gap = args.auto_gap.unwrap_or(false);
    if auto_gap && gap_tk > 0 {
        return Err(ClipDuplicateError::ArgsIncompatible {
            hint: AUTO_GAP_INCOMPATIBLE_HINT,
        });
    }
    if gap_tk < 0 {
        return Err(ClipDuplicateError::BadTime {
            field: "gap_tk",
            value: gap_tk,
        });
    }

    let members = resolve_structural_set(prior, target);
    for member in &members {
        if member.track_locked {
            return Err(ClipDuplicateError::Locked {
                failed_target: member.track_id.to_string(),
            });
        }
    }

    let target_end_tk = clip_end_tk(target.clip);
    let resolved_gap_tk = if auto_gap {
        let target_duration_tk = clip_duration_tk(target.clip);
        find_next_free_slot(
            &prior.tracks[target.track_idx].clips,
            target_end_tk,
            target_duration_tk,
        )
        .saturating_sub(target_end_tk)
    } else {
        gap_tk
    };

    let new_link_group = target.clip.link_group.map(|_| LinkGroupId::now());
    let new_clip_ids = members
        .iter()
        .map(|member| (member.clip.id, ClipId::now()))
        .collect::<HashMap<_, _>>();

    let plans = members
        .iter()
        .map(|member| {
            build_duplicate_plan(
                member,
                new_clip_ids[&member.clip.id],
                new_link_group,
                resolved_gap_tk,
            )
        })
        .collect::<Vec<_>>();

    check_planned_overlaps(prior, &plans)?;

    let data = build_data(&members, clip_id, &plans, resolved_gap_tk, new_link_group);
    let warnings = vec![envelope_warning(&data)];
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
                        track_id: track.id,
                        track_locked: track.locked,
                        clip,
                    })
                })
        })
        .collect()
}

fn build_duplicate_plan(
    member: &LocatedClip<'_>,
    new_clip_id: ClipId,
    new_link_group: Option<LinkGroupId>,
    resolved_gap_tk: i64,
) -> DuplicatePlan {
    let duration_tk = clip_duration_tk(member.clip);
    let new_position_tk = clip_end_tk(member.clip).saturating_add(resolved_gap_tk);
    let duplicate = duplicate_clip(member.clip, new_clip_id, new_position_tk, new_link_group);
    DuplicatePlan {
        track_idx: member.track_idx,
        source_clip_idx: member.clip_idx,
        source_clip_id: member.clip.id,
        new_position_tk,
        duration_tk,
        duplicate,
    }
}

fn duplicate_clip(
    source: &Clip,
    new_clip_id: ClipId,
    new_position_tk: i64,
    new_link_group: Option<LinkGroupId>,
) -> Clip {
    let mut duplicate = source.clone();
    duplicate.id = new_clip_id;
    duplicate.track_position_tk = Tick::new(new_position_tk);
    duplicate.link_group = new_link_group;

    let effect_id_map = duplicate
        .effects
        .iter_mut()
        .map(|effect| {
            let old_id = effect.id;
            let new_id = EffectId::now();
            effect.id = new_id;
            (old_id.to_string(), new_id.to_string())
        })
        .collect::<BTreeMap<_, _>>();

    duplicate.keyframes = duplicate
        .keyframes
        .into_iter()
        .map(|mut keyframe| {
            keyframe.id = KeyframeId::now();
            let property = remap_keyframe_property(keyframe.property.as_str(), &effect_id_map);
            if property != keyframe.property.as_str() {
                keyframe.property = KeyframeProperty::new(property)
                    .expect("effect-id remapped keyframe property remains valid");
            }
            keyframe
        })
        .collect();

    duplicate
}

fn remap_keyframe_property(property: &str, effect_id_map: &BTreeMap<String, String>) -> String {
    let mut remapped = property.to_string();
    for (old_id, new_id) in effect_id_map {
        remapped = remapped.replace(&format!("effects[{old_id}]"), &format!("effects[{new_id}]"));
    }
    remapped
}

fn clip_duration_tk(clip: &Clip) -> i64 {
    timeline_duration_tk(clip.source_in_tk, clip.source_out_tk, clip.speed).get()
}

fn clip_end_tk(clip: &Clip) -> i64 {
    clip.track_position_tk
        .get()
        .saturating_add(clip_duration_tk(clip))
}

fn find_next_free_slot(clips: &[Clip], start_tk: i64, width_tk: i64) -> i64 {
    let mut intervals = clips
        .iter()
        .map(|clip| (clip.track_position_tk.get(), clip_end_tk(clip)))
        .collect::<Vec<_>>();
    intervals.sort_by_key(|(start, end)| (*start, *end));

    let mut candidate = start_tk;
    for (other_start, other_end) in intervals {
        if other_end <= candidate {
            continue;
        }
        if candidate.saturating_add(width_tk) <= other_start {
            return candidate;
        }
        candidate = candidate.max(other_end);
    }
    candidate
}

fn check_planned_overlaps(
    prior: &Project,
    plans: &[DuplicatePlan],
) -> Result<(), ClipDuplicateError> {
    for plan in plans {
        let start = plan.new_position_tk;
        let end = start.saturating_add(plan.duration_tk);

        for other_plan in plans.iter().filter(|other| {
            other.track_idx == plan.track_idx && other.source_clip_id != plan.source_clip_id
        }) {
            let other_start = other_plan.new_position_tk;
            let other_end = other_start.saturating_add(other_plan.duration_tk);
            if intervals_overlap(start, end, other_start, other_end) {
                return Err(ClipDuplicateError::ClipOverlap {
                    failed_clip: plan.source_clip_id.to_string(),
                });
            }
        }

        for other in &prior.tracks[plan.track_idx].clips {
            let other_start = other.track_position_tk.get();
            let other_end = clip_end_tk(other);
            if intervals_overlap(start, end, other_start, other_end) {
                return Err(ClipDuplicateError::ClipOverlap {
                    failed_clip: plan.source_clip_id.to_string(),
                });
            }
        }
    }
    Ok(())
}

fn intervals_overlap(a_start: i64, a_end: i64, b_start: i64, b_end: i64) -> bool {
    a_start < b_end && b_start < a_end
}

fn build_data(
    members: &[LocatedClip<'_>],
    target_id: ClipId,
    plans: &[DuplicatePlan],
    resolved_gap_tk: i64,
    new_link_group: Option<LinkGroupId>,
) -> ClipDuplicateData {
    let target_plan = plans
        .iter()
        .find(|plan| plan.source_clip_id == target_id)
        .expect("target plan exists for target member");

    ClipDuplicateData {
        source_clip_id: target_id,
        new_clip_id: target_plan.duplicate.id,
        track_position_tk: target_plan.new_position_tk,
        resolved_gap_tk,
        new_link_group,
        sibling_duplicates: sibling_duplicates(members, target_id, plans),
    }
}

fn sibling_duplicates(
    members: &[LocatedClip<'_>],
    target_id: ClipId,
    plans: &[DuplicatePlan],
) -> Vec<SiblingDuplicate> {
    members
        .iter()
        .filter(|member| member.clip.id != target_id)
        .map(|member| {
            let plan = plans
                .iter()
                .find(|plan| plan.source_clip_id == member.clip.id)
                .expect("member has a matching duplicate plan");
            SiblingDuplicate {
                source_clip_id: member.clip.id,
                new_clip_id: plan.duplicate.id,
                track_position_tk: plan.new_position_tk,
            }
        })
        .collect()
}

fn build_patch(prior: &Project, plans: &[DuplicatePlan]) -> Value {
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
            clips.push(clip.clone());
            for plan in plans_for_track
                .iter()
                .filter(|plan| plan.source_clip_idx == clip_idx)
            {
                clips.push(plan.duplicate.clone());
            }
        }
        clips.sort_by_key(|clip| (clip.track_position_tk.get(), clip.id.to_string()));
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

fn planned_project_duration_tk(prior: &Project, plans: &[DuplicatePlan]) -> i64 {
    let mut computed = 0_i64;
    for (track_idx, track) in prior.tracks.iter().enumerate() {
        for clip in &track.clips {
            computed = computed.max(clip_end_tk(clip));
        }
        for plan in plans.iter().filter(|plan| plan.track_idx == track_idx) {
            computed = computed.max(plan.new_position_tk.saturating_add(plan.duration_tk));
        }
    }
    computed
}

fn envelope_warning(data: &ClipDuplicateData) -> Value {
    let sibling_new_clip_ids = data
        .sibling_duplicates
        .iter()
        .map(|sibling| (sibling.source_clip_id.to_string(), sibling.new_clip_id))
        .collect::<BTreeMap<_, _>>();
    json!({
        "code": W_CLIP_DUPLICATE_ENVELOPE_CODE,
        "message": "clip.duplicate envelope",
        "details": {
            "new_clip_id": data.new_clip_id,
            "new_link_group": data.new_link_group,
            "resolved_gap_tk": data.resolved_gap_tk,
            "sibling_new_clip_ids": sibling_new_clip_ids,
        },
    })
}

/// Rebuild `ClipDuplicateData` from recorded warnings and post-state.
///
/// # Errors
///
/// Returns [`ReconstructError`] if the internal envelope warning is
/// missing or malformed, if `args.clip` is invalid, or if a minted
/// duplicate is absent from `post_state`.
pub fn data_envelope_from_args_warnings(
    args: &ClipDuplicateArgs,
    warnings: &[Value],
    post_state: &Project,
) -> Result<ClipDuplicateData, ReconstructError> {
    let source_clip_id =
        args.clip
            .parse::<ClipId>()
            .map_err(|_| ReconstructError::TypeMismatch {
                name: "args.clip",
                expected: "UUIDv7 ClipId string",
            })?;
    let details = envelope_details_from_warnings(warnings)?;

    let target_duplicate = locate_clip(post_state, details.new_clip_id).ok_or_else(|| {
        ReconstructError::PostStateMissing {
            detail: format!(
                "clip.duplicate: new target clip id {} not found in post_state",
                details.new_clip_id
            ),
        }
    })?;

    let mut sibling_duplicates = details
        .sibling_new_clip_ids
        .iter()
        .map(|(source_id, new_id)| {
            let source_clip_id =
                source_id
                    .parse::<ClipId>()
                    .map_err(|_| ReconstructError::TypeMismatch {
                        name: "warnings[].W_CLIP_DUPLICATE_ENVELOPE.details.sibling_new_clip_ids",
                        expected: "map of UUIDv7 ClipId strings",
                    })?;
            let duplicate = locate_clip(post_state, *new_id).ok_or_else(|| {
                ReconstructError::PostStateMissing {
                    detail: format!(
                        "clip.duplicate: sibling clip id {new_id} not found in post_state"
                    ),
                }
            })?;
            Ok(SiblingDuplicate {
                source_clip_id,
                new_clip_id: *new_id,
                track_position_tk: duplicate.clip.track_position_tk.get(),
            })
        })
        .collect::<Result<Vec<_>, ReconstructError>>()?;
    sibling_duplicates.sort_by_key(|sibling| sibling.source_clip_id.to_string());

    Ok(ClipDuplicateData {
        source_clip_id,
        new_clip_id: details.new_clip_id,
        track_position_tk: target_duplicate.clip.track_position_tk.get(),
        resolved_gap_tk: details.resolved_gap_tk,
        new_link_group: details.new_link_group,
        sibling_duplicates,
    })
}

fn envelope_details_from_warnings(
    warnings: &[Value],
) -> Result<EnvelopeWarningDetails, ReconstructError> {
    for warning in warnings {
        if warning.get("code").and_then(Value::as_str) != Some(W_CLIP_DUPLICATE_ENVELOPE_CODE) {
            continue;
        }
        let details = warning
            .get("details")
            .ok_or(ReconstructError::MissingField {
                name: "warnings[].W_CLIP_DUPLICATE_ENVELOPE.details",
            })?;
        return serde_json::from_value(details.clone()).map_err(|_| {
            ReconstructError::TypeMismatch {
                name: "warnings[].W_CLIP_DUPLICATE_ENVELOPE.details",
                expected: "ClipDuplicate envelope details",
            }
        });
    }

    Err(ReconstructError::MissingField {
        name: "warnings[].W_CLIP_DUPLICATE_ENVELOPE",
    })
}

/// `clip.duplicate` verb registration entry.
#[derive(Debug, Default)]
pub struct ClipDuplicateVerb;

impl From<ClipDuplicateError> for VerbError {
    fn from(value: ClipDuplicateError) -> Self {
        VerbError::BadArgs {
            detail: value.to_string(),
        }
    }
}

impl Verb for ClipDuplicateVerb {
    fn verb(&self) -> &'static str {
        "clip.duplicate"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: ClipDuplicateArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("clip.duplicate: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!("clip.duplicate: patch construction failed: {err}"))
            })?;

        let post_state = prior
            .apply(&patch)
            .map_err(|err| VerbError::InvariantViolation {
                detail: format!("clip.duplicate: post-state validation failed: {err}"),
            })?;
        drop(post_state);

        let data = serde_json::to_value(&data).map_err(|err| {
            VerbError::Custom(format!("clip.duplicate: data serialize failed: {err}"))
        })?;

        Ok((patch, data, warnings))
    }

    fn reconstruct(
        &self,
        args: &Value,
        _patch: &Value,
        warnings: &[Value],
        post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let typed: ClipDuplicateArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "ClipDuplicateArgs",
            })?;

        let envelope = data_envelope_from_args_warnings(&typed, warnings, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
