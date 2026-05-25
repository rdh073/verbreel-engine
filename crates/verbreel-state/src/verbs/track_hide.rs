//! `track.hide` (§4.10) — fourteenth production verb in the engine.
//!
//! ## Spec quote (`spec/commands/track.md` §4.10, verbatim)
//!
//! > Toggles `Track.hidden`. Hidden tracks are excluded from the GUI
//! > preview and from `render.start` output, but the project's clips,
//! > effects, and keyframes are preserved. Audio tracks: a hidden audio
//! > track contributes no samples to the mix.
//! > CLI: `verbreel track hide [--project <id>] --track <selector> [--hidden <bool>]`
//! > MCP: `track.hide`
//! > Args: `project_id: string`, `track: string`, `hidden?: boolean` (default `true`)
//! > Returns (`data`): `{ track_id: string; hidden: boolean }`
//! > Errors: `E_TRACK_NOT_FOUND`, `E_BAD_SELECTOR`, `E_SELECTOR_KIND_MISMATCH`,
//! > `E_LOCKED`
//!
//! ## Render-time visibility (out of scope here)
//!
//! §4.10 says hidden tracks are excluded from GUI preview and render
//! output. That filtering belongs to `verbreel-render`; this
//! state-layer verb only flips the single `hidden: bool` flag. State
//! remains valid in every combination of hidden flags across tracks —
//! no §0.13 invariant rejects any pattern.
//!
//! ## Selector handling (v1)
//!
//! `track` is accepted as **bare `UUIDv7` only** in this slice. Structural
//! selectors (for example `video[0]` or `video[name="main"]`) are deferred
//! to a future verb-slice. Bare selector parse failures map to
//! [`TrackHideError::BadSelector`].
//!
//! ## Idempotent hidden behavior (§0.6)
//!
//! Calling `track.hide` with the current hidden state is a successful no-op:
//! - empty patch (`[]`)
//! - single [`W_NOOP_CODE`] warning (`message` = `track hidden state unchanged`)
//! - data envelope still returns `{ track_id, hidden }` from post-state.
//!
//! ## Lock behavior
//!
//! Locked tracks (`track.locked == true`) are rejected with
//! [`TrackHideError::Locked`].

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::{ProjectId, TrackId};

/// Warning code emitted when the incoming hidden state equals the current hidden state.
pub const W_NOOP_CODE: &str = "W_NOOP";

/// Default `hidden` value when omitted from `track.hide` args.
pub const DEFAULT_HIDDEN: bool = true;

/// Args for `track.hide`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackHideArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Target track id as bare `UUIDv7`.
    pub track: String,

    /// Desired hidden state. Omitted values default to `DEFAULT_HIDDEN`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
}

/// Envelope `data` returned by `track.hide`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackHideData {
    /// Target track id.
    pub track_id: TrackId,

    /// New hidden state in post-state.
    pub hidden: bool,
}

/// Verb-level validation failures for `track.hide`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TrackHideError {
    /// `args.track` is not parseable as `UUIDv7`.
    #[error("track.hide: `track` selector parse failed: {detail}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
    },

    /// No track exists for `track`.
    #[error("track.hide: track `{track_id}` not found")]
    TrackNotFound {
        /// Missing track id string.
        track_id: String,
    },

    /// The target track is locked.
    #[error("track.hide: track `{track_id}` is locked")]
    Locked {
        /// Locked track id.
        track_id: String,
    },
}

/// Build the RFC-6902 patch for `track.hide`.
///
/// # Errors
///
/// - [`TrackHideError::BadSelector`] for non-UUIDv7 `args.track`.
/// - [`TrackHideError::TrackNotFound`] if `args.track` resolves to no track.
/// - [`TrackHideError::Locked`] if the target track is locked.
/// - idempotent no-op path: empty patch + [`W_NOOP_CODE`] warning when
///   `args.hidden` equals the current track hidden state.
pub fn compute_patch(
    prior: &Project,
    args: &TrackHideArgs,
) -> Result<(Value, Vec<Value>, TrackHideData), TrackHideError> {
    let track_id = args
        .track
        .parse::<TrackId>()
        .map_err(|err| TrackHideError::BadSelector {
            detail: err.to_string(),
        })?;

    let (global_idx, track) = prior
        .tracks
        .iter()
        .enumerate()
        .find(|(_, track)| track.id == track_id)
        .ok_or(TrackHideError::TrackNotFound {
            track_id: args.track.clone(),
        })?;

    if track.locked {
        return Err(TrackHideError::Locked {
            track_id: args.track.clone(),
        });
    }

    let current_hidden = track.hidden;
    let target_hidden = args.hidden.unwrap_or(DEFAULT_HIDDEN);

    if target_hidden == current_hidden {
        return Ok((
            json!([]),
            vec![json!({
                "code": W_NOOP_CODE,
                "message": "track hidden state unchanged",
                "details": {
                    "track_id": track_id.to_string(),
                    "hidden": current_hidden,
                }
            })],
            TrackHideData {
                track_id,
                hidden: current_hidden,
            },
        ));
    }

    let patch = json!([{
        "op": "replace",
        "path": format!("/tracks/{global_idx}/hidden"),
        "value": target_hidden,
    }]);

    Ok((
        patch,
        Vec::new(),
        TrackHideData {
            track_id,
            hidden: target_hidden,
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
    args: &TrackHideArgs,
    post_state: &Project,
) -> Result<TrackHideData, ReconstructError> {
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
            detail: format!("track hidden: track id {track_id} not found in post_state.tracks"),
        })?;

    Ok(TrackHideData {
        track_id,
        hidden: track.hidden,
    })
}

/// `track.hide` verb registration entry.
#[derive(Debug, Default)]
pub struct TrackHideVerb;

impl From<TrackHideError> for VerbError {
    fn from(value: TrackHideError) -> Self {
        match value {
            TrackHideError::BadSelector { .. }
            | TrackHideError::TrackNotFound { .. }
            | TrackHideError::Locked { .. } => VerbError::BadArgs {
                detail: value.to_string(),
            },
        }
    }
}

impl Verb for TrackHideVerb {
    fn verb(&self) -> &'static str {
        "track.hide"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: TrackHideArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("track.hide: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, _data) = compute_patch(prior, &typed)?;

        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!("track.hide: patch construction failed: {err}"))
            })?;

        let post_state = prior
            .apply(&patch)
            .map_err(|err| VerbError::InvariantViolation {
                detail: format!("track.hide: post-state validation failed: {err}"),
            })?;

        let envelope = data_envelope_from_post_state(&typed, &post_state).map_err(|err| {
            VerbError::Custom(format!(
                "track.hide: data envelope reconstruction failed: {err}"
            ))
        })?;

        let data = serde_json::to_value(&envelope).map_err(|err| {
            VerbError::Custom(format!("track.hide: data serialize failed: {err}"))
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
        let typed: TrackHideArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "TrackHideArgs",
            })?;

        let envelope = data_envelope_from_post_state(&typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
