//! `clip.reverse` (§5.8) — forty-fifth production verb in the engine.
//!
//! Sets `Clip.reversed` and propagates the same value across every
//! member of the target clip's link group. Omitted `reversed` defaults
//! to `true`; the verb is a setter, not a toggle.

use crate::asset::Asset;
use crate::clip::Clip;
use crate::newtypes::AssetRef;
use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use crate::track::TrackKind;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::{ClipId, LinkGroupId, ProjectId};

/// Warning code emitted when every sync-set member already has the
/// requested reverse state.
pub const W_NOOP_CODE: &str = "W_NOOP";

/// Default `reversed` value when omitted from `clip.reverse` args.
pub const DEFAULT_REVERSED: bool = true;

/// Recovery hint returned with `E_LINK_GROUP_SEMANTICS_MIX`.
pub const LINK_GROUP_SEMANTICS_MIX_HINT: &str =
    "call clip.unlink first, then mutate each clip independently";

/// Args for `clip.reverse`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipReverseArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Target clip id as bare `UUIDv7`.
    pub clip: String,

    /// Desired reverse state. Omitted values default to
    /// [`DEFAULT_REVERSED`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reversed: Option<bool>,
}

/// Envelope `data` returned by `clip.reverse`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipReverseData {
    /// Target clip id.
    pub clip_id: ClipId,

    /// New reverse state in post-state.
    pub reversed: bool,

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

/// Verb-level validation failures for `clip.reverse`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ClipReverseError {
    /// `args.clip` is not parseable as `UUIDv7`.
    #[error("E_BAD_SELECTOR: clip.reverse: `clip` selector parse failed: {detail}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
    },

    /// No clip exists for `clip`.
    #[error("E_NOT_FOUND: clip.reverse: clip `{clip_id}` not found")]
    ClipNotFound {
        /// Missing clip id string.
        clip_id: String,
    },

    /// A link group mixes source-slice and display-duration classes.
    #[error(
        "E_LINK_GROUP_SEMANTICS_MIX: clip.reverse: link group `{link_group}` mixes source-time semantics classes"
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
    #[error("E_LOCKED: clip.reverse: clip `{failed_clip}` or its parent track is locked")]
    Locked {
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

/// Build the RFC-6902 patch for `clip.reverse`.
///
/// # Errors
///
/// Returns [`ClipReverseError`] for selector parse failure, missing
/// target clip, cross-semantics link groups, or any locked sync-set
/// member.
pub fn compute_patch(
    prior: &Project,
    args: &ClipReverseArgs,
) -> Result<(Value, Vec<Value>, ClipReverseData), ClipReverseError> {
    let clip_id = args
        .clip
        .parse::<ClipId>()
        .map_err(|err| ClipReverseError::BadSelector {
            detail: err.to_string(),
        })?;

    let target = locate_clip(prior, clip_id).ok_or_else(|| ClipReverseError::ClipNotFound {
        clip_id: args.clip.clone(),
    })?;

    let members = resolve_sync_set(prior, target);
    if let Some(target_group) = target.clip.link_group
        && members.len() > 1
    {
        reject_semantics_mix(prior, &members, target_group)?;
    }

    for member in &members {
        if member.track_locked || member.clip.locked {
            return Err(ClipReverseError::Locked {
                failed_clip: member.clip.id.to_string(),
            });
        }
    }

    let new_reversed = args.reversed.unwrap_or(DEFAULT_REVERSED);
    let data = ClipReverseData {
        clip_id,
        reversed: new_reversed,
        linked_clip_ids: sorted_linked_clip_ids(&members, clip_id),
    };

    if members
        .iter()
        .all(|member| member.clip.reversed == new_reversed)
    {
        return Ok((
            json!([]),
            vec![json!({
                "code": W_NOOP_CODE,
                "message": "clip reversed state unchanged",
                "details": {
                    "clip_id": clip_id.to_string(),
                    "reversed": new_reversed,
                }
            })],
            data,
        ));
    }

    let ops = members
        .iter()
        .filter(|member| member.clip.reversed != new_reversed)
        .map(|member| {
            json!({
                "op": "replace",
                "path": format!(
                    "/tracks/{}/clips/{}/reversed",
                    member.track_idx, member.clip_idx
                ),
                "value": new_reversed,
            })
        })
        .collect::<Vec<_>>();

    Ok((Value::Array(ops), Vec::new(), data))
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
) -> Result<(), ClipReverseError> {
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
        return Err(ClipReverseError::LinkGroupSemanticsMix {
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
/// a valid `UUIDv7`, or [`ReconstructError::PostStateMissing`] when
/// the post-state does not contain the target clip.
pub fn data_envelope_from_post_state(
    args: &ClipReverseArgs,
    post_state: &Project,
) -> Result<ClipReverseData, ReconstructError> {
    let clip_id = args
        .clip
        .parse::<ClipId>()
        .map_err(|_| ReconstructError::TypeMismatch {
            name: "args.clip",
            expected: "UUIDv7 ClipId string",
        })?;

    let target =
        locate_clip(post_state, clip_id).ok_or_else(|| ReconstructError::PostStateMissing {
            detail: format!("clip.reverse: clip id {clip_id} not found in post_state"),
        })?;
    let members = resolve_sync_set(post_state, target);

    Ok(ClipReverseData {
        clip_id,
        reversed: target.clip.reversed,
        linked_clip_ids: sorted_linked_clip_ids(&members, clip_id),
    })
}

/// `clip.reverse` verb registration entry.
#[derive(Debug, Default)]
pub struct ClipReverseVerb;

impl From<ClipReverseError> for VerbError {
    fn from(value: ClipReverseError) -> Self {
        VerbError::BadArgs {
            detail: value.to_string(),
        }
    }
}

impl Verb for ClipReverseVerb {
    fn verb(&self) -> &'static str {
        "clip.reverse"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: ClipReverseArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("clip.reverse: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, _data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!("clip.reverse: patch construction failed: {err}"))
            })?;

        let post_state = prior
            .apply(&patch)
            .map_err(|err| VerbError::InvariantViolation {
                detail: format!("clip.reverse: post-state validation failed: {err}"),
            })?;

        let envelope = data_envelope_from_post_state(&typed, &post_state).map_err(|err| {
            VerbError::Custom(format!(
                "clip.reverse: data envelope reconstruction failed: {err}"
            ))
        })?;
        let data = serde_json::to_value(&envelope).map_err(|err| {
            VerbError::Custom(format!("clip.reverse: data serialize failed: {err}"))
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
        let typed: ClipReverseArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "ClipReverseArgs",
            })?;

        let envelope = data_envelope_from_post_state(&typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
