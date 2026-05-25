//! `track.solo` (§4.5) — thirteenth production verb in the engine.
//!
//! ## Spec quote (`spec/commands/track.md` §4.5, verbatim)
//!
//! > Toggles `Track.solo`. The schema permits the field on any track kind;
//! > the verb does **not** reject non-audio tracks (no
//! > `E_TRACK_KIND_MISMATCH` guard, unlike `track.set_volume` §4.8 which is
//! > audio-only by construction).
//! > CLI: `verbreel track solo [--project <id>] --track <selector> [--solo <bool>]`
//! > MCP: `track.solo`
//! > Args: `project_id: string`, `track: string`, `solo?: boolean` (default `true`)
//! > Returns (`data`): `{ track_id: string; solo: boolean }`
//! > Errors: `E_TRACK_NOT_FOUND`, `E_BAD_SELECTOR`, `E_SELECTOR_KIND_MISMATCH`,
//! > `E_LOCKED`
//!
//! ## Render-time visibility rule (out of scope here)
//!
//! §4.5 documents a consolidated render-time visibility rule:
//! `!hidden && !muted && (no_sibling_of_same_kind_is_soloed || self.solo)`.
//! That rule belongs to `verbreel-render`; this state-layer verb only
//! flips the single `solo: bool` flag. State remains valid in every
//! combination of solo flags across tracks — there is no §0.13
//! invariant enforcing "at most one solo per kind" or similar.
//!
//! ## Selector handling (v1)
//!
//! `track` is accepted as **bare `UUIDv7` only** in this slice. Structural
//! selectors (for example `video[0]` or `video[name="main"]`) are deferred
//! to a future verb-slice. Bare selector parse failures map to
//! [`TrackSoloError::BadSelector`].
//!
//! ## Idempotent solo behavior (§0.6)
//!
//! Calling `track.solo` with the current solo state is a successful no-op:
//! - empty patch (`[]`)
//! - single [`W_NOOP_CODE`] warning (`message` = `track solo state unchanged`)
//! - data envelope still returns `{ track_id, solo }` from post-state.
//!
//! ## Lock behavior
//!
//! Locked tracks (`track.locked == true`) are rejected with
//! [`TrackSoloError::Locked`].

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::{ProjectId, TrackId};

/// Warning code emitted when the incoming solo state equals the current solo state.
pub const W_NOOP_CODE: &str = "W_NOOP";

/// Default `solo` value when omitted from `track.solo` args.
pub const DEFAULT_SOLO: bool = true;

/// Args for `track.solo`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackSoloArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Target track id as bare `UUIDv7`.
    pub track: String,

    /// Desired solo state. Omitted values default to `DEFAULT_SOLO`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solo: Option<bool>,
}

/// Envelope `data` returned by `track.solo`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackSoloData {
    /// Target track id.
    pub track_id: TrackId,

    /// New solo state in post-state.
    pub solo: bool,
}

/// Verb-level validation failures for `track.solo`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TrackSoloError {
    /// `args.track` is not parseable as `UUIDv7`.
    #[error("track.solo: `track` selector parse failed: {detail}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
    },

    /// No track exists for `track`.
    #[error("track.solo: track `{track_id}` not found")]
    TrackNotFound {
        /// Missing track id string.
        track_id: String,
    },

    /// The target track is locked.
    #[error("track.solo: track `{track_id}` is locked")]
    Locked {
        /// Locked track id.
        track_id: String,
    },
}

/// Build the RFC-6902 patch for `track.solo`.
///
/// # Errors
///
/// - [`TrackSoloError::BadSelector`] for non-UUIDv7 `args.track`.
/// - [`TrackSoloError::TrackNotFound`] if `args.track` resolves to no track.
/// - [`TrackSoloError::Locked`] if the target track is locked.
/// - idempotent no-op path: empty patch + [`W_NOOP_CODE`] warning when
///   `args.solo` equals the current track solo state.
pub fn compute_patch(
    prior: &Project,
    args: &TrackSoloArgs,
) -> Result<(Value, Vec<Value>, TrackSoloData), TrackSoloError> {
    let track_id = args
        .track
        .parse::<TrackId>()
        .map_err(|err| TrackSoloError::BadSelector {
            detail: err.to_string(),
        })?;

    let (global_idx, track) = prior
        .tracks
        .iter()
        .enumerate()
        .find(|(_, track)| track.id == track_id)
        .ok_or(TrackSoloError::TrackNotFound {
            track_id: args.track.clone(),
        })?;

    if track.locked {
        return Err(TrackSoloError::Locked {
            track_id: args.track.clone(),
        });
    }

    let current_solo = track.solo;
    let target_solo = args.solo.unwrap_or(DEFAULT_SOLO);

    if target_solo == current_solo {
        return Ok((
            json!([]),
            vec![json!({
                "code": W_NOOP_CODE,
                "message": "track solo state unchanged",
                "details": {
                    "track_id": track_id.to_string(),
                    "solo": current_solo,
                }
            })],
            TrackSoloData {
                track_id,
                solo: current_solo,
            },
        ));
    }

    let patch = json!([{
        "op": "replace",
        "path": format!("/tracks/{global_idx}/solo"),
        "value": target_solo,
    }]);

    Ok((
        patch,
        Vec::new(),
        TrackSoloData {
            track_id,
            solo: target_solo,
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
    args: &TrackSoloArgs,
    post_state: &Project,
) -> Result<TrackSoloData, ReconstructError> {
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
            detail: format!("track solo: track id {track_id} not found in post_state.tracks"),
        })?;

    Ok(TrackSoloData {
        track_id,
        solo: track.solo,
    })
}

/// `track.solo` verb registration entry.
#[derive(Debug, Default)]
pub struct TrackSoloVerb;

impl From<TrackSoloError> for VerbError {
    fn from(value: TrackSoloError) -> Self {
        match value {
            TrackSoloError::BadSelector { .. }
            | TrackSoloError::TrackNotFound { .. }
            | TrackSoloError::Locked { .. } => VerbError::BadArgs {
                detail: value.to_string(),
            },
        }
    }
}

impl Verb for TrackSoloVerb {
    fn verb(&self) -> &'static str {
        "track.solo"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: TrackSoloArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("track.solo: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, _data) = compute_patch(prior, &typed)?;

        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!("track.solo: patch construction failed: {err}"))
            })?;

        let post_state = prior
            .apply(&patch)
            .map_err(|err| VerbError::InvariantViolation {
                detail: format!("track.solo: post-state validation failed: {err}"),
            })?;

        let envelope = data_envelope_from_post_state(&typed, &post_state).map_err(|err| {
            VerbError::Custom(format!(
                "track.solo: data envelope reconstruction failed: {err}"
            ))
        })?;

        let data = serde_json::to_value(&envelope).map_err(|err| {
            VerbError::Custom(format!("track.solo: data serialize failed: {err}"))
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
        let typed: TrackSoloArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "TrackSoloArgs",
            })?;

        let envelope = data_envelope_from_post_state(&typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
