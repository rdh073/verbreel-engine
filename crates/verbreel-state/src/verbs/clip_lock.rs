//! `clip.lock` (§5.13) — eighteenth production verb in the engine.
//!
//! ## Spec quote (`spec/commands/clip.md` §5.13, verbatim)
//!
//! > Sets `Clip.locked`. Idempotent — calling with the same value as
//! > the current state returns `Ok` with `patch: []` and one `W_NOOP`
//! > warning.
//! > `track.locked` and `clip.locked` are **not** consulted in this
//! > verb (unlike most clip mutators).
//! > CLI: `verbreel clip lock [--project <id>] --clip <id> [--locked <bool>]`
//! > MCP: `clip.lock`
//! > Args: `project_id: string`, `clip: string`, `locked?: boolean`
//! > (default `true`)
//! > Returns (`data`): `{ clip_id: string; locked: boolean }`
//! > Errors: `E_NOT_FOUND`, `E_NO_MATCH`, `E_BAD_SELECTOR`, `E_SELECTOR_KIND_MISMATCH`.
//!
//! ## Selector handling (v1)
//!
//! `clip` is accepted as **bare `UUIDv7` only** in this slice. Structural
//! selectors are deferred to a future verb slice.
//!
//! ## Lock behavior
//!
//! `clip.lock` ignores lock checks entirely, so locked clips and tracks
//! can still be toggled (carve-out).
//!
//! ## Idempotent behavior (§0.6)
//!
//! Calling `clip.lock` with the current lock state is a successful
//! no-op:
//! - empty patch (`[]`)
//! - single [`W_NOOP_CODE`] warning (`message` = `clip lock state unchanged`)
//! - data envelope returns `{ clip_id, locked }` from post-state.

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::{ClipId, ProjectId};

/// Warning code emitted when the incoming lock state equals the
/// current lock state.
pub const W_NOOP_CODE: &str = "W_NOOP";

/// Default `locked` value when omitted from `clip.lock` args.
pub const DEFAULT_LOCKED: bool = true;

/// Args for `clip.lock`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipLockArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Target clip id as bare `UUIDv7`.
    pub clip: String,

    /// Desired lock state. Omitted values default to `DEFAULT_LOCKED`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locked: Option<bool>,
}

/// Envelope `data` returned by `clip.lock`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipLockData {
    /// Target clip id.
    pub clip_id: ClipId,

    /// New lock state in post-state.
    pub locked: bool,
}

/// Verb-level validation failures for `clip.lock`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ClipLockError {
    /// `args.clip` is not parseable as `UUIDv7`.
    #[error("clip.lock: `clip` selector parse failed: {detail}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
    },

    /// No clip exists for `clip`.
    #[error("clip.lock: clip `{clip_id}` not found")]
    ClipNotFound {
        /// Missing clip id string.
        clip_id: String,
    },
}

/// Build the RFC-6902 patch for `clip.lock`.
///
/// # Errors
///
/// - [`ClipLockError::BadSelector`] for non-UUIDv7 `args.clip`.
/// - [`ClipLockError::ClipNotFound`] if `args.clip` resolves to no clip.
/// - idempotent no-op path: empty patch + [`W_NOOP_CODE`] warning when
///   `args.locked` equals the current clip lock state.
pub fn compute_patch(
    prior: &Project,
    args: &ClipLockArgs,
) -> Result<(Value, Vec<Value>, ClipLockData), ClipLockError> {
    let clip_id = args
        .clip
        .parse::<ClipId>()
        .map_err(|err| ClipLockError::BadSelector {
            detail: err.to_string(),
        })?;

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

    let (t_idx, c_idx, clip) = location.ok_or_else(|| ClipLockError::ClipNotFound {
        clip_id: args.clip.clone(),
    })?;

    let current_locked = clip.locked;
    let target_locked = args.locked.unwrap_or(DEFAULT_LOCKED);

    if target_locked == current_locked {
        return Ok((
            json!([]),
            vec![json!({
                "code": W_NOOP_CODE,
                "message": "clip lock state unchanged",
                "details": {
                    "clip_id": clip_id.to_string(),
                    "locked": current_locked,
                }
            })],
            ClipLockData {
                clip_id,
                locked: current_locked,
            },
        ));
    }

    let patch = json!([{
        "op": "replace",
        "path": format!("/tracks/{t_idx}/clips/{c_idx}/locked"),
        "value": target_locked,
    }]);

    Ok((
        patch,
        Vec::new(),
        ClipLockData {
            clip_id,
            locked: target_locked,
        },
    ))
}

/// Rebuilds the envelope from `(args, post_state)`.
///
/// # Errors
///
/// Returns [`ReconstructError::TypeMismatch`] when `args.clip` is not
/// a valid `UUIDv7`, or [`ReconstructError::PostStateMissing`] when
/// the post-state does not contain the target clip.
pub fn data_envelope_from_post_state(
    args: &ClipLockArgs,
    post_state: &Project,
) -> Result<ClipLockData, ReconstructError> {
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
                return Ok(ClipLockData {
                    clip_id,
                    locked: clip.locked,
                });
            }
        }
    }

    Err(ReconstructError::PostStateMissing {
        detail: format!("clip lock: clip id {clip_id} not found in post_state"),
    })
}

/// `clip.lock` verb registration entry.
#[derive(Debug, Default)]
pub struct ClipLockVerb;

impl From<ClipLockError> for VerbError {
    fn from(value: ClipLockError) -> Self {
        match value {
            ClipLockError::BadSelector { .. } | ClipLockError::ClipNotFound { .. } => {
                VerbError::BadArgs {
                    detail: value.to_string(),
                }
            }
        }
    }
}

impl Verb for ClipLockVerb {
    fn verb(&self) -> &'static str {
        "clip.lock"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: ClipLockArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("clip.lock: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, _data) = compute_patch(prior, &typed)?;

        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!("clip.lock: patch construction failed: {err}"))
            })?;

        let post_state = prior
            .apply(&patch)
            .map_err(|err| VerbError::InvariantViolation {
                detail: format!("clip.lock: post-state validation failed: {err}"),
            })?;

        let envelope = data_envelope_from_post_state(&typed, &post_state).map_err(|err| {
            VerbError::Custom(format!(
                "clip.lock: data envelope reconstruction failed: {err}"
            ))
        })?;

        let data = serde_json::to_value(&envelope)
            .map_err(|err| VerbError::Custom(format!("clip.lock: data serialize failed: {err}")))?;

        Ok((patch, data, warnings))
    }

    fn reconstruct(
        &self,
        args: &Value,
        _patch: &Value,
        _warnings: &[Value],
        post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let typed: ClipLockArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "ClipLockArgs",
            })?;

        let envelope = data_envelope_from_post_state(&typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
