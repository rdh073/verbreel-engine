//! `track.reorder` (§4.3) — seventeenth production verb in the engine.
//!
//! ## Spec quote (`spec/commands/track.md` §4.3, verbatim)
//!
//! > Moves a track to a new **kind-relative** index. `to_index: 0` head;
//! > `to_index: <count-1>` tail. Cross-kind impossible.
//! > CLI: `verbreel track reorder [--project <id>] --track <selector> --to_index <i>`
//! > MCP: `track.reorder`
//! > Args: `project_id: string`, `track: string`, `to_index: integer`
//! > Returns (`data`): `{ track_id: string; kind: string; from_index: integer;
//! > to_index: integer }`
//! > Errors: `E_TRACK_NOT_FOUND`, `E_BAD_SELECTOR`,
//! > `E_SELECTOR_KIND_MISMATCH`, `E_TRACK_BAD_INDEX`
//!
//! ## No-op behavior
//!
//! If `to_index` already equals the target track's current kind-relative
//! position, the verb emits no patch and one [`W_NOOP_CODE`] warning.

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use crate::track::TrackKind;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::{ProjectId, TrackId};

/// Warning code emitted when no actual reorder is performed.
pub const W_NOOP_CODE: &str = "W_NOOP";

/// Args for `track.reorder`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackReorderArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Target track id as bare `UUIDv7`.
    pub track: String,

    /// Target kind-relative destination index.
    pub to_index: i64,
}

/// Envelope `data` returned by `track.reorder`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackReorderData {
    /// Target track id.
    pub track_id: TrackId,

    /// Target track kind as lower-case string (`video` / `audio` /
    /// `text` / `effect`).
    pub kind: String,

    /// Previous kind-relative index.
    pub from_index: u32,

    /// New kind-relative index.
    pub to_index: u32,
}

/// Verb-level validation failures for `track.reorder`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TrackReorderError {
    /// `args.track` is not parseable as `UUIDv7`.
    #[error("track.reorder: `track` selector parse failed: {detail}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
    },

    /// No track exists for `track`.
    #[error("track.reorder: track `{track_id}` not found")]
    TrackNotFound {
        /// Missing track id string.
        track_id: String,
    },

    /// Target kind-relative `to_index` is out of bounds.
    #[error("track.reorder: to_index {to_index} out of range for kind with {kind_count} tracks")]
    BadIndex {
        /// Rejected index.
        to_index: i64,

        /// Current number of tracks for that kind.
        kind_count: u32,
    },
}

fn kind_label(kind: TrackKind) -> &'static str {
    match kind {
        TrackKind::Video => "video",
        TrackKind::Audio => "audio",
        TrackKind::Text => "text",
        TrackKind::Effect => "effect",
    }
}

fn kind_indices(prior: &Project, kind: TrackKind) -> Vec<usize> {
    prior
        .tracks
        .iter()
        .enumerate()
        .filter(|(_, track)| track.kind == kind)
        .map(|(idx, _)| idx)
        .collect()
}

fn kind_relative_index(
    tracks: &[crate::track::Track],
    kind: TrackKind,
    global_idx: usize,
) -> usize {
    tracks
        .iter()
        .take(global_idx)
        .filter(|track| track.kind == kind)
        .count()
}

fn parse_tracks_index(path: &str, field: &'static str) -> Result<usize, ReconstructError> {
    let Some(suffix) = path.strip_prefix("/tracks/") else {
        return Err(ReconstructError::TypeMismatch {
            name: field,
            expected: "RFC6902 path in the form /tracks/<index>",
        });
    };

    suffix
        .parse::<usize>()
        .map_err(|_| ReconstructError::TypeMismatch {
            name: field,
            expected: "RFC6902 path in the form /tracks/<index>",
        })
}

/// Build the RFC-6902 patch for `track.reorder`.
///
/// # Errors
///
/// - [`TrackReorderError::BadSelector`] for malformed `args.track`.
/// - [`TrackReorderError::TrackNotFound`] when the track id is missing.
/// - [`TrackReorderError::BadIndex`] when `to_index` is out of bounds.
pub fn compute_patch(
    prior: &Project,
    args: &TrackReorderArgs,
) -> Result<(Value, Vec<Value>, TrackReorderData), TrackReorderError> {
    let track_id = args
        .track
        .parse::<TrackId>()
        .map_err(|err| TrackReorderError::BadSelector {
            detail: err.to_string(),
        })?;

    let (current_global_idx, track) = prior
        .tracks
        .iter()
        .enumerate()
        .find(|(_, track)| track.id == track_id)
        .ok_or_else(|| TrackReorderError::TrackNotFound {
            track_id: args.track.clone(),
        })?;

    let by_kind = kind_indices(prior, track.kind);
    let kind_count = by_kind
        .len()
        .try_into()
        .map_err(|_| TrackReorderError::BadIndex {
            to_index: args.to_index,
            kind_count: u32::MAX,
        })?;

    let Some(from_index) = by_kind
        .iter()
        .position(|&global| global == current_global_idx)
    else {
        return Err(TrackReorderError::TrackNotFound {
            track_id: args.track.clone(),
        });
    };

    let kind_count_i64 = i64::from(kind_count);
    if args.to_index < 0 || args.to_index >= kind_count_i64 {
        return Err(TrackReorderError::BadIndex {
            to_index: args.to_index,
            kind_count,
        });
    }

    let to_index = usize::try_from(args.to_index).map_err(|_| TrackReorderError::BadIndex {
        to_index: args.to_index,
        kind_count,
    })?;
    let to_index_u32 = u32::try_from(to_index).map_err(|_| TrackReorderError::BadIndex {
        to_index: args.to_index,
        kind_count,
    })?;
    let from_index_u32 = u32::try_from(from_index).map_err(|_| TrackReorderError::BadIndex {
        to_index: args.to_index,
        kind_count,
    })?;

    if from_index == to_index {
        return Ok((
            json!([]),
            vec![json!({
                "code": W_NOOP_CODE,
                "message": "track position unchanged",
                "details": {
                    "track_id": track_id.to_string(),
                    "kind": kind_label(track.kind),
                    "index": from_index_u32,
                }
            })],
            TrackReorderData {
                track_id,
                kind: kind_label(track.kind).to_string(),
                from_index: from_index_u32,
                to_index: to_index_u32,
            },
        ));
    }

    let to_global = by_kind[to_index];

    let patch = json!([{
        "op": "move",
        "from": format!("/tracks/{current_global_idx}"),
        "path": format!("/tracks/{to_global}"),
    }]);

    Ok((
        patch,
        Vec::new(),
        TrackReorderData {
            track_id,
            kind: kind_label(track.kind).to_string(),
            from_index: from_index_u32,
            to_index: to_index_u32,
        },
    ))
}

#[allow(clippy::too_many_lines)]
/// Rebuilds the envelope from `(args, patch, post_state)`.
///
/// # Errors
///
/// - [`ReconstructError::TypeMismatch`] when `patch`, `args`, or `warnings`
///   shape does not match replay requirements.
/// - [`ReconstructError::PostStateMissing`] if the reconstructed `track_id`
///   is not present in `post_state`.
pub fn data_envelope_from_patch_and_post_state(
    patch: &Value,
    args: &TrackReorderArgs,
    post_state: &Project,
) -> Result<TrackReorderData, ReconstructError> {
    let track_id = args
        .track
        .parse::<TrackId>()
        .map_err(|_| ReconstructError::TypeMismatch {
            name: "args.track",
            expected: "UUIDv7 TrackId string",
        })?;

    let (post_global_idx, track) = post_state
        .tracks
        .iter()
        .enumerate()
        .find(|(_, track)| track.id == track_id)
        .ok_or_else(|| ReconstructError::PostStateMissing {
            detail: format!("track.reorder: track id {track_id} not found in post_state.tracks"),
        })?;

    let to_index = kind_relative_index(&post_state.tracks, track.kind, post_global_idx);
    let to_index = u32::try_from(to_index).map_err(|_| {
        ReconstructError::Custom(
            "track.reorder: post-state kind-relative index exceeds u32".to_string(),
        )
    })?;

    let patch_ops = patch.as_array().ok_or(ReconstructError::TypeMismatch {
        name: "patch",
        expected: "array",
    })?;

    if patch_ops.is_empty() {
        return Ok(TrackReorderData {
            track_id,
            kind: kind_label(track.kind).to_string(),
            from_index: to_index,
            to_index,
        });
    }

    if patch_ops.len() != 1 {
        return Err(ReconstructError::Custom(
            "track.reorder: expected empty or single move op in patch".to_string(),
        ));
    }

    let op = patch_ops.first().ok_or(ReconstructError::Custom(
        "track.reorder: empty patch op list".to_string(),
    ))?;

    let op_obj = op.as_object().ok_or(ReconstructError::TypeMismatch {
        name: "patch[0]",
        expected: "object",
    })?;

    let op_kind =
        op_obj
            .get("op")
            .and_then(Value::as_str)
            .ok_or(ReconstructError::TypeMismatch {
                name: "patch[0].op",
                expected: "string",
            })?;

    if op_kind != "move" {
        return Err(ReconstructError::Custom(format!(
            "track.reorder: unsupported operation `{op_kind}`, expected `move`"
        )));
    }

    let from =
        op_obj
            .get("from")
            .and_then(Value::as_str)
            .ok_or(ReconstructError::TypeMismatch {
                name: "patch[0].from",
                expected: "string",
            })?;

    let path_ =
        op_obj
            .get("path")
            .and_then(Value::as_str)
            .ok_or(ReconstructError::TypeMismatch {
                name: "patch[0].path",
                expected: "string",
            })?;

    let from_global = parse_tracks_index(from, "patch[0].from")?;
    let to_global = parse_tracks_index(path_, "patch[0].path")?;

    let kind = track.kind;

    if to_global >= post_state.tracks.len() || from_global >= post_state.tracks.len() {
        return Err(ReconstructError::TypeMismatch {
            name: "patch[0]",
            expected: "index within post-state /tracks array",
        });
    }

    let mut pre_tracks = post_state.tracks.clone();
    let moved = pre_tracks.remove(to_global);
    if moved.id != track_id {
        return Err(ReconstructError::PostStateMissing {
            detail: format!("track.reorder: post-state reconstruction mismatch for {track_id}"),
        });
    }

    if from_global > pre_tracks.len() {
        return Err(ReconstructError::TypeMismatch {
            name: "patch[0].from",
            expected: "index within prior /tracks array",
        });
    }

    pre_tracks.insert(from_global, moved);

    let from_index = kind_relative_index(&pre_tracks, kind, from_global);
    let from_index = u32::try_from(from_index).map_err(|_| {
        ReconstructError::Custom(
            "track.reorder: pre-state kind-relative index exceeds u32".to_string(),
        )
    })?;

    Ok(TrackReorderData {
        track_id,
        kind: kind_label(kind).to_string(),
        from_index,
        to_index,
    })
}

/// `track.reorder` verb registration entry.
#[derive(Debug, Default)]
pub struct TrackReorderVerb;

impl From<TrackReorderError> for VerbError {
    fn from(value: TrackReorderError) -> Self {
        match value {
            TrackReorderError::BadSelector { .. }
            | TrackReorderError::TrackNotFound { .. }
            | TrackReorderError::BadIndex { .. } => VerbError::BadArgs {
                detail: value.to_string(),
            },
        }
    }
}

impl Verb for TrackReorderVerb {
    fn verb(&self) -> &'static str {
        "track.reorder"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: TrackReorderArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("track.reorder: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, _data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!("track.reorder: patch construction failed: {err}"))
            })?;

        let post_state = prior
            .apply(&patch)
            .map_err(|err| VerbError::InvariantViolation {
                detail: format!("track.reorder: post-state validation failed: {err}"),
            })?;

        let envelope = data_envelope_from_patch_and_post_state(&patch_value, &typed, &post_state)
            .map_err(|err| {
            VerbError::Custom(format!(
                "track.reorder: data envelope reconstruction failed: {err}"
            ))
        })?;

        let data = serde_json::to_value(&envelope).map_err(|err| {
            VerbError::Custom(format!("track.reorder: data serialize failed: {err}"))
        })?;

        Ok((patch, data, warnings))
    }

    fn reconstruct(
        &self,
        args: &Value,
        patch: &Value,
        _warnings: &[Value],
        post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let typed: TrackReorderArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "TrackReorderArgs",
            })?;

        let envelope = data_envelope_from_patch_and_post_state(patch, &typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
