//! `clip.rename` (§5.17) — nineteenth production verb in the engine.
//!
//! ## Spec quote (`spec/commands/clip.md` §5.17, verbatim)
//!
//! > Renames a clip. Per §5.15, `name` is a per-member property and is
//! > not propagated across link groups.
//! > Clip names are not required to be unique anywhere.
//! > CLI: `verbreel clip rename [--project <id>] --clip <id> --name <str>`
//! > MCP: `clip.rename`
//! > Args: `project_id: string`, `clip: string`, `name: string`
//! > (1–128 chars per schema)
//! > Returns (`data`): `{ clip_id: string; name: string }`
//! > Errors: `E_NOT_FOUND`, `E_LOCKED`, `E_SCHEMA_VIOLATION` (empty
//! > or >128-char name).
//!
//! ## Selector handling (v1)
//!
//! `clip` is accepted as **bare `UUIDv7` only** in this slice. Structural
//! selectors are deferred to a future verb-slice. Bare selector parse
//! failures map to [`ClipRenameError::BadSelector`].
//!
//! ## Idempotent rename behavior (§0.6)
//!
//! Renaming to the same name is a successful no-op:
//! - empty patch (`[]`)
//! - single [`W_NOOP_CODE`] warning (`message` = `clip name unchanged`)
//! - data envelope still returns `{ clip_id, name }`
//!
//! ## Lock guard
//!
//! Locked clips (`clip.locked == true`) are rejected with
//! [`ClipRenameError::Locked`].

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::{ClipId, ProjectId};

/// Maximum number of allowed `name` chars.
pub const CLIP_NAME_MAX: usize = 128;

/// Warning code emitted when the incoming name equals the current name.
pub const W_NOOP_CODE: &str = "W_NOOP";

/// Args for `clip.rename`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipRenameArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Target clip id as bare `UUIDv7`.
    pub clip: String,

    /// New clip name.
    pub name: String,
}

/// Envelope `data` returned by `clip.rename`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipRenameData {
    /// Target clip id.
    pub clip_id: ClipId,

    /// New clip name in post-state.
    pub name: String,
}

/// Verb-level validation failures for `clip.rename`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ClipRenameError {
    /// `args.clip` is not parseable as `UUIDv7`.
    #[error("clip.rename: `clip` selector parse failed: {detail}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
    },

    /// No clip exists for `clip`.
    #[error("clip.rename: clip `{clip_id}` not found")]
    ClipNotFound {
        /// Missing clip id string.
        clip_id: String,
    },

    /// The target clip is locked.
    #[error("clip.rename: clip `{clip_id}` is locked")]
    Locked {
        /// Locked clip id.
        clip_id: String,
    },

    /// Name out of `[1, 128]` chars.
    #[error("clip.rename: name schema violation: {detail}")]
    SchemaViolation {
        /// Human-readable schema violation detail.
        detail: String,
    },
}

/// Build the RFC-6902 patch for `clip.rename`.
///
/// # Errors
///
/// - [`ClipRenameError::BadSelector`] for non-UUIDv7 `args.clip`.
/// - [`ClipRenameError::SchemaViolation`] if `args.name` is empty or
///   over [`CLIP_NAME_MAX`] chars.
/// - [`ClipRenameError::ClipNotFound`] if `args.clip` resolves to no clip.
/// - [`ClipRenameError::Locked`] if the target clip is locked.
/// - idempotent no-op path: empty patch + [`W_NOOP_CODE`] warning when
///   names are already equal.
pub fn compute_patch(
    prior: &Project,
    args: &ClipRenameArgs,
) -> Result<(Value, Vec<Value>, ClipRenameData), ClipRenameError> {
    let clip_id = args
        .clip
        .parse::<ClipId>()
        .map_err(|err| ClipRenameError::BadSelector {
            detail: err.to_string(),
        })?;

    let actual = args.name.chars().count();
    if !(1..=CLIP_NAME_MAX).contains(&actual) {
        return Err(ClipRenameError::SchemaViolation {
            detail: "name length out of range [1, 128]".to_string(),
        });
    }

    let mut location: Option<(usize, usize, &crate::clip::Clip)> = None;
    for (t_idx, track) in prior.tracks.iter().enumerate() {
        for (c_idx, clip) in track.clips.iter().enumerate() {
            if clip.id == clip_id {
                location = Some((t_idx, c_idx, clip));
                break;
            }
        }
        if location.is_some() {
            break;
        }
    }

    let (t_idx, c_idx, clip) = location.ok_or_else(|| ClipRenameError::ClipNotFound {
        clip_id: args.clip.clone(),
    })?;

    if clip.locked {
        return Err(ClipRenameError::Locked {
            clip_id: args.clip.clone(),
        });
    }

    if args.name == clip.name {
        return Ok((
            json!([]),
            vec![json!({
                "code": W_NOOP_CODE,
                "message": "clip name unchanged",
                "details": {
                    "clip_id": clip_id.to_string(),
                    "name": &args.name,
                }
            })],
            ClipRenameData {
                clip_id,
                name: args.name.clone(),
            },
        ));
    }

    let patch = json!([{
        "op": "replace",
        "path": format!("/tracks/{t_idx}/clips/{c_idx}/name"),
        "value": args.name.clone(),
    }]);

    Ok((
        patch,
        Vec::new(),
        ClipRenameData {
            clip_id,
            name: args.name.clone(),
        },
    ))
}

/// Rebuilds the envelope from `(args, post_state)`.
///
/// # Errors
///
/// Returns [`ReconstructError::TypeMismatch`] when `args.clip` is not
/// a valid `UUIDv7`, or [`ReconstructError::PostStateMissing`] when the
/// post-state does not contain the target clip.
pub fn data_envelope_from_post_state(
    args: &ClipRenameArgs,
    post_state: &Project,
) -> Result<ClipRenameData, ReconstructError> {
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
                return Ok(ClipRenameData {
                    clip_id,
                    name: clip.name.clone(),
                });
            }
        }
    }

    Err(ReconstructError::PostStateMissing {
        detail: format!("clip rename: clip id {clip_id} not found in post_state"),
    })
}

/// `clip.rename` verb registration entry.
#[derive(Debug, Default)]
pub struct ClipRenameVerb;

impl From<ClipRenameError> for VerbError {
    fn from(value: ClipRenameError) -> Self {
        match value {
            ClipRenameError::BadSelector { .. }
            | ClipRenameError::ClipNotFound { .. }
            | ClipRenameError::Locked { .. }
            | ClipRenameError::SchemaViolation { .. } => VerbError::BadArgs {
                detail: value.to_string(),
            },
        }
    }
}

impl Verb for ClipRenameVerb {
    fn verb(&self) -> &'static str {
        "clip.rename"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: ClipRenameArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("clip.rename: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, _data) = compute_patch(prior, &typed)?;

        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!("clip.rename: patch construction failed: {err}"))
            })?;

        let post_state = prior
            .apply(&patch)
            .map_err(|err| VerbError::InvariantViolation {
                detail: format!("clip.rename: post-state validation failed: {err}"),
            })?;

        let envelope = data_envelope_from_post_state(&typed, &post_state).map_err(|err| {
            VerbError::Custom(format!(
                "clip.rename: data envelope reconstruction failed: {err}"
            ))
        })?;

        let data = serde_json::to_value(&envelope).map_err(|err| {
            VerbError::Custom(format!("clip.rename: data serialize failed: {err}"))
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
        let typed: ClipRenameArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "ClipRenameArgs",
            })?;

        let envelope = data_envelope_from_post_state(&typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
