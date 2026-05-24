//! `track.rename` (§4.7) — tenth production verb in the engine.
//!
//! ## Spec quote (`spec/commands/track.md` §4.7, verbatim)
//!
//! > CLI: `verbreel track rename [--project <id>] --track <selector> --name <str>`
//! > MCP: `track.rename`
//! > Args: `project_id: string`, `track: string`, `name: string`
//! > Returns (`data`): `{ track_id: string; name: string }`
//! > Errors: `E_TRACK_NOT_FOUND`, `E_BAD_SELECTOR`, `E_SELECTOR_KIND_MISMATCH`,
//! > `E_TRACK_NAME_CONFLICT`, `E_LOCKED`,
//! > `E_SCHEMA_VIOLATION` (empty or >128-char name).
//!
//! ## Selector handling (v1)
//!
//! `track` is accepted as **bare UUIDv7 only** in this slice. Structural
//! selectors (for example `video[0]` or `video[name="main"]`) are deferred
//! to a future verb-slice. Bare selector parse failures map to
//! [`TrackRenameError::BadSelector`].
//!
//! ## Idempotent rename behavior (§0.6)
//!
//! Renaming to the same name is a successful no-op:
//! - empty patch (`[]`)
//! - single [`W_NOOP_CODE`] warning (`message` = `track name unchanged`)
//! - data envelope still returns `{ track_id, name }`
//!
//! ## Lock guard
//!
//! Locked tracks (`track.locked == true`) are rejected with
//! [`TrackRenameError::Locked`].

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use crate::track::TrackKind;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::{ProjectId, TrackId};

pub use crate::verbs::track_add::TRACK_NAME_MAX;

/// Warning code emitted when the incoming name equals the current name.
pub const W_NOOP_CODE: &str = "W_NOOP";

/// Args for `track.rename`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackRenameArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Target track id as bare UUIDv7.
    pub track: String,

    /// New track name.
    pub name: String,
}

/// Envelope `data` returned by `track.rename`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackRenameData {
    /// Renamed track id.
    pub track_id: TrackId,

    /// New track name in post-state.
    pub name: String,
}

/// Verb-level validation failures for `track.rename`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TrackRenameError {
    /// `args.track` is not parseable as UUIDv7.
    #[error("track.rename: `track` selector parse failed: {detail}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
    },

    /// No track exists for `track`.
    #[error("track.rename: track `{track_id}` not found")]
    TrackNotFound {
        /// Missing track id string.
        track_id: String,
    },

    /// The target track is locked.
    #[error("track.rename: track `{track_id}` (`{track_name}`) is locked")]
    Locked {
        /// Locked track id.
        track_id: String,

        /// Locked track name.
        track_name: String,
    },

    /// Empty track name.
    #[error("track.rename: `name` must be non-empty")]
    NameEmpty,

    /// Name too long by char count.
    #[error("track.rename: name has {actual} chars, exceeds maximum {max}")]
    NameTooLong {
        /// Measured length in chars.
        actual: usize,

        /// Maximum allowed chars.
        max: usize,
    },

    /// In-kind duplicate name.
    #[error("track.rename: track name `{name}` already exists for kind {kind:?}")]
    NameConflict {
        /// Name that conflicts.
        name: String,

        /// Track kind with the conflicting name.
        kind: TrackKind,
    },
}

/// Returns `Err` when another track in the same kind (excluding
/// `exclude_track_id`) already uses `name`.
pub fn check_name_conflict(
    prior: &Project,
    kind: TrackKind,
    name: &str,
    exclude_track_id: TrackId,
) -> Result<(), TrackRenameError> {
    if prior
        .tracks
        .iter()
        .any(|track| track.kind == kind && track.id != exclude_track_id && track.name == name)
    {
        return Err(TrackRenameError::NameConflict {
            name: name.to_string(),
            kind,
        });
    }

    Ok(())
}

/// Build the RFC-6902 patch for `track.rename`.
///
/// # Errors
///
/// - [`TrackRenameError::BadSelector`] for non-UUIDv7 `args.track`.
/// - [`TrackRenameError::TrackNotFound`] if `args.track` resolves to no track.
/// - [`TrackRenameError::Locked`] if the target track is locked.
/// - [`TrackRenameError::NameEmpty`] for empty `args.name`.
/// - [`TrackRenameError::NameTooLong`] for names longer than
///   [`TRACK_NAME_MAX`] chars.
/// - [`TrackRenameError::NameConflict`] for same-kind duplicate names (excluding
///   self).
/// - idempotent no-op path: empty patch + [`W_NOOP_CODE`] warning when names
///   are already equal.
pub fn compute_patch(
    prior: &Project,
    args: &TrackRenameArgs,
) -> Result<(Value, Vec<Value>, TrackRenameData), TrackRenameError> {
    let track_id = TrackId::try_from(args.track.clone())
        .map_err(|err| TrackRenameError::BadSelector { detail: err.to_string() })?;

    let (global_idx, track) = prior
        .tracks
        .iter()
        .enumerate()
        .find(|(_, track)| track.id == track_id)
        .ok_or(TrackRenameError::TrackNotFound {
            track_id: args.track.clone(),
        })?;

    if track.locked {
        return Err(TrackRenameError::Locked {
            track_id: args.track.clone(),
            track_name: track.name.clone(),
        });
    }

    if args.name.is_empty() {
        return Err(TrackRenameError::NameEmpty);
    }

    let actual = args.name.chars().count();
    if actual > TRACK_NAME_MAX {
        return Err(TrackRenameError::NameTooLong {
            actual,
            max: TRACK_NAME_MAX,
        });
    }

    if args.name == track.name {
        return Ok((
            json!([]),
            vec![json!({
                "code": W_NOOP_CODE,
                "message": "track name unchanged",
                "details": {
                    "track_id": track_id.to_string(),
                }
            })],
            TrackRenameData {
                track_id,
                name: args.name.clone(),
            },
        ));
    }

    check_name_conflict(prior, track.kind, &args.name, track_id)?;

    let patch = json!([{
        "op": "replace",
        "path": format!("/tracks/{global_idx}/name"),
        "value": args.name,
    }]);

    Ok((
        patch,
        Vec::new(),
        TrackRenameData {
            track_id,
            name: args.name.clone(),
        },
    ))
}

/// Rebuilds the envelope from `(args, post_state)`.
pub fn data_envelope_from_post_state(
    args: &TrackRenameArgs,
    post_state: &Project,
) -> Result<TrackRenameData, ReconstructError> {
    let track_id = TrackId::try_from(args.track.clone())
        .map_err(|_| ReconstructError::TypeMismatch {
            name: "args.track",
            expected: "UUIDv7 TrackId string",
        })?;

    let track = post_state
        .tracks
        .iter()
        .find(|track| track.id == track_id)
        .ok_or_else(|| ReconstructError::PostStateMissing {
            detail: format!("track rename: track id {track_id} not found in post_state.tracks"),
        })?;

    Ok(TrackRenameData {
        track_id,
        name: track.name.clone(),
    })
}

/// `track.rename` verb registration entry.
#[derive(Debug, Default)]
pub struct TrackRenameVerb;

impl From<TrackRenameError> for VerbError {
    fn from(value: TrackRenameError) -> Self {
        match value {
            TrackRenameError::BadSelector { .. }
            | TrackRenameError::TrackNotFound { .. }
            | TrackRenameError::Locked { .. }
            | TrackRenameError::NameEmpty
            | TrackRenameError::NameTooLong { .. }
            | TrackRenameError::NameConflict { .. } => VerbError::BadArgs {
                detail: value.to_string(),
            },
        }
    }
}

impl Verb for TrackRenameVerb {
    fn verb(&self) -> &'static str {
        "track.rename"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: TrackRenameArgs = serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
            detail: format!("track.rename: args deserialize failed: {err}"),
        })?;

        let (_patch_value, warnings, _data) = compute_patch(prior, &typed)?;

        let patch: json_patch::Patch = serde_json::from_value(_patch_value.clone())
            .map_err(|err| VerbError::Custom(format!("track.rename: patch construction failed: {err}")))?;

        let post_state = prior
            .apply(&patch)
            .map_err(|err| VerbError::InvariantViolation {
                detail: format!("track.rename: post-state validation failed: {err}"),
            })?;

        let envelope =
            data_envelope_from_post_state(&typed, &post_state).map_err(|err| VerbError::Custom(
                format!("track.rename: data envelope reconstruction failed: {err}"),
            ))?;

        let data = serde_json::to_value(&envelope)
            .map_err(|err| VerbError::Custom(format!("track.rename: data serialize failed: {err}")))?;

        Ok((patch, data, warnings))
    }

    fn reconstruct(
        &self,
        args: &Value,
        _patch: &Value,
        _warnings: &[Value],
        post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let typed: TrackRenameArgs = serde_json::from_value(args.clone())
            .map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "TrackRenameArgs",
            })?;

        let envelope = data_envelope_from_post_state(&typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
