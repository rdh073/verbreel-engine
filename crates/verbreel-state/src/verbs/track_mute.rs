//! `track.mute` (§4.4) — twelfth production verb in the engine.
//!
//! ## Spec quote (`spec/commands/track.md` §4.4, verbatim)
//!
//! > Toggles `Track.muted`. The schema permits the field on any track kind;
//! > the verb does **not** reject non-audio tracks (no
//! > `E_TRACK_KIND_MISMATCH` guard, unlike `track.set_volume` §4.8 which is
//! > audio-only by construction).
//! > CLI: `verbreel track mute [--project <id>] --track <selector> [--muted <bool>]`
//! > MCP: `track.mute`
//! > Args: `project_id: string`, `track: string`, `muted?: boolean` (default `true`)
//! > Returns (`data`): `{ track_id: string; muted: boolean }`
//! > Errors: `E_TRACK_NOT_FOUND`, `E_BAD_SELECTOR`, `E_SELECTOR_KIND_MISMATCH`,
//! > `E_LOCKED`
//!
//! ## Selector handling (v1)
//!
//! `track` is accepted as **bare `UUIDv7` only** in this slice. Structural
//! selectors (for example `video[0]` or `video[name="main"]`) are deferred
//! to a future verb-slice. Bare selector parse failures map to
//! [`TrackMuteError::BadSelector`].
//!
//! ## Idempotent mute behavior (§0.6)
//!
//! Calling `track.mute` with the current mute state is a successful no-op:
//! - empty patch (`[]`)
//! - single [`W_NOOP_CODE`] warning (`message` = `track mute state unchanged`)
//! - data envelope still returns `{ track_id, muted }` from post-state.
//!
//! ## Lock behavior
//!
//! Locked tracks (`track.locked == true`) are rejected with
//! [`TrackMuteError::Locked`].

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::{ProjectId, TrackId};

/// Warning code emitted when the incoming mute state equals the current mute state.
pub const W_NOOP_CODE: &str = "W_NOOP";

/// Default `muted` value when omitted from `track.mute` args.
pub const DEFAULT_MUTED: bool = true;

/// Args for `track.mute`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackMuteArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Target track id as bare `UUIDv7`.
    pub track: String,

    /// Desired mute state. Omitted values default to `DEFAULT_MUTED`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub muted: Option<bool>,
}

/// Envelope `data` returned by `track.mute`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackMuteData {
    /// Target track id.
    pub track_id: TrackId,

    /// New muted state in post-state.
    pub muted: bool,
}

/// Verb-level validation failures for `track.mute`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TrackMuteError {
    /// `args.track` is not parseable as `UUIDv7`.
    #[error("track.mute: `track` selector parse failed: {detail}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
    },

    /// No track exists for `track`.
    #[error("track.mute: track `{track_id}` not found")]
    TrackNotFound {
        /// Missing track id string.
        track_id: String,
    },

    /// The target track is locked.
    #[error("track.mute: track `{track_id}` is locked")]
    Locked {
        /// Locked track id.
        track_id: String,
    },
}

/// Build the RFC-6902 patch for `track.mute`.
///
/// # Errors
///
/// - [`TrackMuteError::BadSelector`] for non-UUIDv7 `args.track`.
/// - [`TrackMuteError::TrackNotFound`] if `args.track` resolves to no track.
/// - [`TrackMuteError::Locked`] if the target track is locked.
/// - idempotent no-op path: empty patch + [`W_NOOP_CODE`] warning when
///   `args.muted` equals the current track mute state.
pub fn compute_patch(
    prior: &Project,
    args: &TrackMuteArgs,
) -> Result<(Value, Vec<Value>, TrackMuteData), TrackMuteError> {
    let track_id = args
        .track
        .parse::<TrackId>()
        .map_err(|err| TrackMuteError::BadSelector {
            detail: err.to_string(),
        })?;

    let (global_idx, track) = prior
        .tracks
        .iter()
        .enumerate()
        .find(|(_, track)| track.id == track_id)
        .ok_or(TrackMuteError::TrackNotFound {
            track_id: args.track.clone(),
        })?;

    if track.locked {
        return Err(TrackMuteError::Locked {
            track_id: args.track.clone(),
        });
    }

    let current_muted = track.muted;
    let target_muted = args.muted.unwrap_or(DEFAULT_MUTED);

    if target_muted == current_muted {
        return Ok((
            json!([]),
            vec![json!({
                "code": W_NOOP_CODE,
                "message": "track mute state unchanged",
                "details": {
                    "track_id": track_id.to_string(),
                    "muted": current_muted,
                }
            })],
            TrackMuteData {
                track_id,
                muted: current_muted,
            },
        ));
    }

    let patch = json!([{
        "op": "replace",
        "path": format!("/tracks/{global_idx}/muted"),
        "value": target_muted,
    }]);

    Ok((
        patch,
        Vec::new(),
        TrackMuteData {
            track_id,
            muted: target_muted,
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
    args: &TrackMuteArgs,
    post_state: &Project,
) -> Result<TrackMuteData, ReconstructError> {
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
            detail: format!("track mute: track id {track_id} not found in post_state.tracks"),
        })?;

    Ok(TrackMuteData {
        track_id,
        muted: track.muted,
    })
}

/// `track.mute` verb registration entry.
#[derive(Debug, Default)]
pub struct TrackMuteVerb;

impl From<TrackMuteError> for VerbError {
    fn from(value: TrackMuteError) -> Self {
        match value {
            TrackMuteError::BadSelector { .. }
            | TrackMuteError::TrackNotFound { .. }
            | TrackMuteError::Locked { .. } => VerbError::BadArgs {
                detail: value.to_string(),
            },
        }
    }
}

impl Verb for TrackMuteVerb {
    fn verb(&self) -> &'static str {
        "track.mute"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: TrackMuteArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("track.mute: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, _data) = compute_patch(prior, &typed)?;

        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!("track.mute: patch construction failed: {err}"))
            })?;

        let post_state = prior
            .apply(&patch)
            .map_err(|err| VerbError::InvariantViolation {
                detail: format!("track.mute: post-state validation failed: {err}"),
            })?;

        let envelope = data_envelope_from_post_state(&typed, &post_state).map_err(|err| {
            VerbError::Custom(format!(
                "track.mute: data envelope reconstruction failed: {err}"
            ))
        })?;

        let data = serde_json::to_value(&envelope).map_err(|err| {
            VerbError::Custom(format!("track.mute: data serialize failed: {err}"))
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
        let typed: TrackMuteArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "TrackMuteArgs",
            })?;

        let envelope = data_envelope_from_post_state(&typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
