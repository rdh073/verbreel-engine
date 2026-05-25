//! `clip.unlink` (§5.16) — twenty-fifth production verb in the engine.
//!
//! ## Spec quote (`spec/commands/clip.md` §5.16, verbatim)
//!
//! > Breaks a link group so each member can be edited independently.
//! > Clears `Clip.link_group` on every member.
//!
//! ## Lock behavior
//!
//! `clip.unlink` is blocked by any locked member in the target link group.
//! Unlike `clip.lock`, this is a strict lock check for the entire group.
//!
//! ## Patch structure
//!
//! The verb emits a pair of RFC-6902 ops for each member:
//! 1. `test` on `/tracks/{t}/clips/{c}/link_group`
//! 2. `replace` on the same path to `null`
//!
//! The `test` op is required so reconstructor can recover the
//! pre-clear `link_group` from the recorded patch.

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::{ClipId, LinkGroupId, ProjectId};

/// Args for `clip.unlink`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipUnlinkArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Target clip id as bare `UUIDv7`.
    pub clip: String,
}

/// Envelope data returned by `clip.unlink`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipUnlinkData {
    /// Group id that was cleared from each member.
    pub link_group: LinkGroupId,

    /// Every clip id whose link was cleared, sorted lexicographically.
    pub cleared_clip_ids: Vec<ClipId>,
}

/// Verb-level validation failures for `clip.unlink`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ClipUnlinkError {
    /// `args.clip` is not parseable as `UUIDv7`.
    #[error("clip.unlink: `clip` selector parse failed: {detail}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
    },

    /// Target clip exists but has no link group.
    #[error("clip.unlink: clip `{clip_id}` not found")]
    ClipNotFound {
        /// Missing clip id string.
        clip_id: String,
    },

    /// Target clip is not linked.
    #[error("clip.unlink: clip `{clip_id}` is not in any link group")]
    NotLinked {
        /// Target clip id string.
        clip_id: String,
    },

    /// A member of the target link group is locked.
    #[error("clip.unlink: link-group member `{clip_id}` is locked")]
    MemberLocked {
        /// First locked member id encountered while scanning.
        clip_id: String,
    },
}

/// Build the RFC-6902 patch for `clip.unlink`.
///
/// # Errors
///
/// - [`ClipUnlinkError::BadSelector`] when `args.clip` is not `UUIDv7`.
/// - [`ClipUnlinkError::ClipNotFound`] when target clip does not exist.
/// - [`ClipUnlinkError::NotLinked`] when target has no link group.
/// - [`ClipUnlinkError::MemberLocked`] when any member in the group is
///   locked.
pub fn compute_patch(
    prior: &Project,
    args: &ClipUnlinkArgs,
) -> Result<(Value, Vec<Value>, ClipUnlinkData), ClipUnlinkError> {
    let clip_id = args
        .clip
        .parse::<ClipId>()
        .map_err(|err| ClipUnlinkError::BadSelector {
            detail: err.to_string(),
        })?;

    let mut target_link_group: Option<Option<LinkGroupId>> = None;
    for track in &prior.tracks {
        for clip in &track.clips {
            if clip.id == clip_id {
                target_link_group = Some(clip.link_group);
                break;
            }
        }
        if target_link_group.is_some() {
            break;
        }
    }

    let target_link_group = target_link_group.ok_or_else(|| ClipUnlinkError::ClipNotFound {
        clip_id: args.clip.clone(),
    })?;

    let Some(target_group) = target_link_group else {
        return Err(ClipUnlinkError::NotLinked {
            clip_id: args.clip.clone(),
        });
    };

    let members: Vec<(usize, usize, ClipId, bool)> = prior
        .tracks
        .iter()
        .enumerate()
        .flat_map(|(t_idx, track)| {
            track
                .clips
                .iter()
                .enumerate()
                .filter_map(move |(c_idx, clip)| {
                    (clip.link_group == Some(target_group)).then_some((
                        t_idx,
                        c_idx,
                        clip.id,
                        clip.locked,
                    ))
                })
        })
        .collect::<Vec<(usize, usize, ClipId, bool)>>();

    if let Some((_, _, locked_id, _)) = members.iter().find(|(_, _, _, locked)| *locked) {
        return Err(ClipUnlinkError::MemberLocked {
            clip_id: locked_id.to_string(),
        });
    }

    let mut ops = Vec::with_capacity(members.len() * 2);
    let mut sorted_members = members;
    sorted_members.sort_by(|(t_left, c_left, _, _), (t_right, c_right, _, _)| {
        t_left.cmp(t_right).then(c_left.cmp(c_right))
    });

    let target_group_string = target_group.to_string();
    for (track_idx, clip_idx, _, _) in &sorted_members {
        ops.push(json!({
            "op": "test",
            "path": format!("/tracks/{track_idx}/clips/{clip_idx}/link_group"),
            "value": target_group_string,
        }));
        ops.push(json!({
            "op": "replace",
            "path": format!("/tracks/{track_idx}/clips/{clip_idx}/link_group"),
            "value": null,
        }));
    }

    let mut cleared_clip_ids = sorted_members
        .iter()
        .map(|(_, _, id, _)| *id)
        .collect::<Vec<_>>();
    cleared_clip_ids.sort_by_key(ToString::to_string);

    Ok((
        Value::Array(ops),
        Vec::new(),
        ClipUnlinkData {
            link_group: target_group,
            cleared_clip_ids,
        },
    ))
}

fn parse_link_group_path(path: &str) -> Result<(usize, usize), ReconstructError> {
    const EXPECTED_NAME: &str = "patch[n].path";
    const EXPECTED_PATH: &str =
        "RFC6902 path in the form /tracks/<track-index>/clips/<clip-index>/link_group";

    let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    if parts.len() != 5 || parts[0] != "tracks" || parts[2] != "clips" || parts[4] != "link_group" {
        return Err(ReconstructError::TypeMismatch {
            name: EXPECTED_NAME,
            expected: EXPECTED_PATH,
        });
    }

    let track_index = parts[1]
        .parse::<usize>()
        .map_err(|_| ReconstructError::TypeMismatch {
            name: EXPECTED_NAME,
            expected: EXPECTED_PATH,
        })?;
    let clip_index = parts[3]
        .parse::<usize>()
        .map_err(|_| ReconstructError::TypeMismatch {
            name: EXPECTED_NAME,
            expected: EXPECTED_PATH,
        })?;

    Ok((track_index, clip_index))
}

fn extract_link_group_from_patch(patch: &Value) -> Result<LinkGroupId, ReconstructError> {
    let ops = patch.as_array().ok_or(ReconstructError::TypeMismatch {
        name: "patch",
        expected: "JSON array",
    })?;

    for op in ops {
        let op_obj = op.as_object().ok_or(ReconstructError::TypeMismatch {
            name: "patch[n]",
            expected: "RFC6902 op object",
        })?;
        if op_obj.get("op").and_then(Value::as_str) == Some("test") {
            let value = op_obj.get("value").and_then(Value::as_str).ok_or(
                ReconstructError::TypeMismatch {
                    name: "patch[n].value",
                    expected: "UUIDv7 LinkGroupId string",
                },
            )?;
            return value
                .parse::<LinkGroupId>()
                .map_err(|_| ReconstructError::TypeMismatch {
                    name: "patch[n].value",
                    expected: "UUIDv7 LinkGroupId string",
                });
        }
    }

    Err(ReconstructError::PostStateMissing {
        detail: "clip.unlink: no test op in patch — cannot recover link_group".to_string(),
    })
}

/// Rebuilds the data envelope from `(patch, post_state)`.
///
/// # Errors
///
/// Returns [`ReconstructError`] when patch JSON shape is wrong or any
/// referenced clip cannot be found in post-state.
pub fn data_envelope_from_patch_and_post_state(
    patch: &Value,
    post_state: &Project,
) -> Result<ClipUnlinkData, ReconstructError> {
    let link_group = extract_link_group_from_patch(patch)?;

    let patch_ops = patch.as_array().ok_or(ReconstructError::TypeMismatch {
        name: "patch",
        expected: "JSON array",
    })?;

    let mut cleared_clip_ids = Vec::new();
    for op in patch_ops {
        let Some(op_obj) = op.as_object() else {
            return Err(ReconstructError::TypeMismatch {
                name: "patch[n]",
                expected: "RFC6902 op object",
            });
        };

        if op_obj.get("op").and_then(Value::as_str) != Some("test") {
            continue;
        }

        let op_path =
            op_obj
                .get("path")
                .and_then(Value::as_str)
                .ok_or(ReconstructError::TypeMismatch {
                    name: "patch[n].path",
                    expected: "RFC6902 path string",
                })?;
        let (track_idx, clip_idx) = parse_link_group_path(op_path)?;
        let clip = post_state
            .tracks
            .get(track_idx)
            .and_then(|track| track.clips.get(clip_idx))
            .ok_or_else(|| ReconstructError::PostStateMissing {
                detail: format!("clip.unlink: post-state path missing {op_path}"),
            })?;
        cleared_clip_ids.push(clip.id);
    }

    cleared_clip_ids.sort_by_key(ToString::to_string);
    Ok(ClipUnlinkData {
        link_group,
        cleared_clip_ids,
    })
}

/// `clip.unlink` verb registration entry.
#[derive(Debug, Default)]
pub struct ClipUnlinkVerb;

impl From<ClipUnlinkError> for VerbError {
    fn from(value: ClipUnlinkError) -> Self {
        match value {
            ClipUnlinkError::BadSelector { .. }
            | ClipUnlinkError::ClipNotFound { .. }
            | ClipUnlinkError::NotLinked { .. }
            | ClipUnlinkError::MemberLocked { .. } => VerbError::BadArgs {
                detail: value.to_string(),
            },
        }
    }
}

impl Verb for ClipUnlinkVerb {
    fn verb(&self) -> &'static str {
        "clip.unlink"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: ClipUnlinkArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("clip.unlink: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!("clip.unlink: patch construction failed: {err}"))
            })?;

        let _post_state = prior
            .apply(&patch)
            .map_err(|err| VerbError::InvariantViolation {
                detail: format!("clip.unlink: post-state validation failed: {err}"),
            })?;

        let data = serde_json::to_value(&data).map_err(|err| {
            VerbError::Custom(format!("clip.unlink: data serialize failed: {err}"))
        })?;

        Ok((patch, data, warnings))
    }

    fn reconstruct(
        &self,
        _args: &Value,
        patch: &Value,
        _warnings: &[Value],
        post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let envelope = data_envelope_from_patch_and_post_state(patch, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
