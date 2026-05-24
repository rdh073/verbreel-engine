//! `track.lock` (§4.6) — eleventh production verb in the engine.
//!
//! ## Spec quote (`spec/commands/track.md` §4.6, verbatim)
//!
//! > Sets `Track.locked`. The lock-state mutation itself is **not** blocked by
//! > `Track.locked` (an agent must be able to unlock a locked track) — parallel
//! > to `clip.lock`'s carve-out in §5.13. Idempotent — calling with the same
//! > value as the current state returns `Ok` with `patch: []` and one `W_NOOP`
//! > warning (consistent with §0.6's no-op convention). Other `E_LOCKED` returns
//! > elsewhere in this file refer to verbs that mutate track *content* (clips,
//! > effects, name, volume, pan), not the lock flag.
//! > CLI: `verbreel track lock [--project <id>] --track <selector> [--locked <bool>]`
//! > MCP: `track.lock`
//! > Args: `project_id: string`, `track: string`, `locked?: boolean` (default `true`)
//! > Returns (`data`): `{ track_id: string; locked: boolean }`
//! > Errors: `E_TRACK_NOT_FOUND`, `E_BAD_SELECTOR`, `E_SELECTOR_KIND_MISMATCH`.
//!
//! ## Selector handling (v1)
//!
//! `track` is accepted as **bare `UUIDv7` only** in this slice. Structural
//! selectors (for example `video[0]` or `video[name="main"]`) are deferred
//! to a future verb-slice. Bare selector parse failures map to
//! [`TrackLockError::BadSelector`].
//!
//! ## Lock behavior
//!
//! Locking/unlocking a track is not blocked by `Track.locked`. This
//! carve-out is explicit in §4.6 and allows unlocking a locked track.
//!
//! ## Idempotent behavior (§0.6)
//!
//! Calling `track.lock` with the current lock state is a successful no-op:
//! - empty patch (`[]`)
//! - single [`W_NOOP_CODE`] warning (`message` = `track lock state unchanged`)
//! - data envelope returns `{ track_id, locked }` from post-state.

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::{ProjectId, TrackId};

/// Warning code emitted when the requested lock state equals the current lock state.
pub const W_NOOP_CODE: &str = "W_NOOP";

/// Default `locked` value when omitted from `track.lock` args.
pub const DEFAULT_LOCKED: bool = true;

/// Args for `track.lock`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackLockArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Target track id as bare `UUIDv7`.
    pub track: String,

    /// Desired lock state. Omitted values default to `DEFAULT_LOCKED`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locked: Option<bool>,
}

/// Envelope `data` returned by `track.lock`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackLockData {
    /// Target track id.
    pub track_id: TrackId,

    /// New lock state in post-state.
    pub locked: bool,
}

/// Verb-level validation failures for `track.lock`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TrackLockError {
    /// `args.track` is not parseable as `UUIDv7`.
    #[error("track.lock: `track` selector parse failed: {detail}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
    },

    /// No track exists for `track`.
    #[error("track.lock: track `{track_id}` not found")]
    TrackNotFound {
        /// Missing track id string.
        track_id: String,
    },
}

/// Build the RFC-6902 patch for `track.lock`.
///
/// # Errors
///
/// - [`TrackLockError::BadSelector`] for non-UUIDv7 `args.track`.
/// - [`TrackLockError::TrackNotFound`] if `args.track` resolves to no track.
/// - idempotent no-op path: empty patch + [`W_NOOP_CODE`] warning when
///   `args.locked` equals the current track lock state.
pub fn compute_patch(
    prior: &Project,
    args: &TrackLockArgs,
) -> Result<(Value, Vec<Value>, TrackLockData), TrackLockError> {
    let track_id = args
        .track
        .parse::<TrackId>()
        .map_err(|err| TrackLockError::BadSelector {
            detail: err.to_string(),
        })?;

    let (global_idx, track) = prior
        .tracks
        .iter()
        .enumerate()
        .find(|(_, track)| track.id == track_id)
        .ok_or(TrackLockError::TrackNotFound {
            track_id: args.track.clone(),
        })?;

    let current_locked = track.locked;
    let target_locked = args.locked.unwrap_or(DEFAULT_LOCKED);

    if target_locked == current_locked {
        return Ok((
            json!([]),
            vec![json!({
                "code": W_NOOP_CODE,
                "message": "track lock state unchanged",
                "details": {
                    "track_id": track_id.to_string(),
                    "locked": current_locked,
                }
            })],
            TrackLockData {
                track_id,
                locked: current_locked,
            },
        ));
    }

    let patch = json!([{
        "op": "replace",
        "path": format!("/tracks/{global_idx}/locked"),
        "value": target_locked,
    }]);

    Ok((
        patch,
        Vec::new(),
        TrackLockData {
            track_id,
            locked: target_locked,
        },
    ))
}

/// Rebuilds the envelope from `(args, post_state)`.
///
/// # Errors
///
/// Returns [`ReconstructError::TypeMismatch`] when `args.track` is not
/// a valid `UUIDv7`, or [`ReconstructError::PostStateMissing`] when the
/// post-state does not contain the target track.
pub fn data_envelope_from_post_state(
    args: &TrackLockArgs,
    post_state: &Project,
) -> Result<TrackLockData, ReconstructError> {
    let track_id = args
        .track
        .parse::<TrackId>()
        .map_err(|_| ReconstructError::TypeMismatch {
            name: "args.track",
            expected: "UUIDv7 TrackId string",
        })?;

    let track = post_state
        .tracks
        .iter()
        .find(|track| track.id == track_id)
        .ok_or_else(|| ReconstructError::PostStateMissing {
            detail: format!("track lock: track id {track_id} not found in post_state.tracks"),
        })?;

    Ok(TrackLockData {
        track_id,
        locked: track.locked,
    })
}

/// `track.lock` verb registration entry.
#[derive(Debug, Default)]
pub struct TrackLockVerb;

impl From<TrackLockError> for VerbError {
    fn from(value: TrackLockError) -> Self {
        match value {
            TrackLockError::BadSelector { .. } | TrackLockError::TrackNotFound { .. } => {
                VerbError::BadArgs {
                    detail: value.to_string(),
                }
            }
        }
    }
}

impl Verb for TrackLockVerb {
    fn verb(&self) -> &'static str {
        "track.lock"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: TrackLockArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("track.lock: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, _data) = compute_patch(prior, &typed)?;

        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!("track.lock: patch construction failed: {err}"))
            })?;

        let post_state = prior
            .apply(&patch)
            .map_err(|err| VerbError::InvariantViolation {
                detail: format!("track.lock: post-state validation failed: {err}"),
            })?;

        let envelope = data_envelope_from_post_state(&typed, &post_state).map_err(|err| {
            VerbError::Custom(format!(
                "track.lock: data envelope reconstruction failed: {err}"
            ))
        })?;

        let data = serde_json::to_value(&envelope).map_err(|err| {
            VerbError::Custom(format!("track.lock: data serialize failed: {err}"))
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
        let typed: TrackLockArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "TrackLockArgs",
            })?;

        let envelope = data_envelope_from_post_state(&typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
