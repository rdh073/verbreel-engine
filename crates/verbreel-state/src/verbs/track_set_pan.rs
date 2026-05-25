//! `track.set_pan` (§4.9) — sixteenth production verb in the engine.
//!
//! ## Spec quote (`spec/commands/track.md` §4.9, verbatim)
//!
//! > Audio tracks only.
//! > CLI: `verbreel track set_pan [--project <id>] --track <selector> --pan <-1..1>`
//! > MCP: `track.set_pan`
//! > Args: `project_id: string`, `track: string`, `pan: number`
//! > Returns (`data`): `{ track_id: string; pan: number }`
//! > Errors: `E_TRACK_NOT_FOUND`, `E_BAD_SELECTOR`, `E_SELECTOR_KIND_MISMATCH`, `E_TRACK_KIND_MISMATCH`, `E_BAD_RANGE`, `E_LOCKED`
//!
//! ## Audio-only and kind mismatch guard
//!
//! This verb is restricted to audio tracks. When a non-audio track is
//! targeted, the verb fails with
//! [`TrackSetPanError::KindMismatch`] before evaluating lock/range
//! guards.
//!
//! ## Pan range guard
//!
//! Accepted values are `-1.0 <= pan <= 1.0` and finite. Values
//! outside that range, or non-finite values like `NaN` and infinities,
//! fail as [`TrackSetPanError::BadRange`].
//!
//! ## Selector handling (v1)
//!
//! `track` is accepted as **bare `UUIDv7` only** in this slice.
//! Structural selectors (for example `video[0]` or `video[name="main"]`)
//! are deferred to a future verb-slice. Bare selector parse failures
//! map to [`TrackSetPanError::BadSelector`].
//!
//! ## Idempotent pan behavior (§0.6)
//!
//! Calling `track.set_pan` with the current pan is a successful no-op:
//! - empty patch (`[]`)
//! - single [`W_NOOP_CODE`] warning (`message` = `track pan unchanged`)
//! - data envelope still returns `{ track_id, pan }` from post-state.
//!
//! ## Lock behavior
//!
//! Locked tracks (`track.locked == true`) are rejected with
//! [`TrackSetPanError::Locked`].

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use crate::track::TrackKind;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::{ProjectId, TrackId};

/// Warning code emitted when the incoming pan equals the current pan.
pub const W_NOOP_CODE: &str = "W_NOOP";

/// Args for `track.set_pan`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackSetPanArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Target track id as bare `UUIDv7`.
    pub track: String,

    /// Desired pan (must be within `[-1.0, 1.0]` and finite).
    pub pan: f64,
}

/// Envelope `data` returned by `track.set_pan`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackSetPanData {
    /// Target track id.
    pub track_id: TrackId,

    /// New pan in post-state.
    pub pan: f64,
}

/// Verb-level validation failures for `track.set_pan`.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum TrackSetPanError {
    /// `args.track` is not parseable as `UUIDv7`.
    #[error("track.set_pan: `track` selector parse failed: {detail}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
    },

    /// No track exists for `track`.
    #[error("track.set_pan: track `{track_id}` not found")]
    TrackNotFound {
        /// Missing track id string.
        track_id: String,
    },

    /// Target track kind is not audio.
    #[error(
        "track.set_pan: track `{track_id}` is kind {found_kind:?}, but only audio tracks support pan"
    )]
    KindMismatch {
        /// Target track id.
        track_id: String,

        /// Target track kind.
        found_kind: TrackKind,
    },

    /// The target track is locked.
    #[error("track.set_pan: track `{track_id}` is locked")]
    Locked {
        /// Locked track id.
        track_id: String,
    },

    /// The provided pan is out of bounds or non-finite.
    #[error("track.set_pan: pan {value} out of range [-1.0, 1.0]")]
    BadRange {
        /// Invalid pan value.
        value: f64,
    },
}

/// Build the RFC-6902 patch for `track.set_pan`.
///
/// # Errors
///
/// - [`TrackSetPanError::BadSelector`] for non-UUIDv7 `args.track`.
/// - [`TrackSetPanError::TrackNotFound`] if `args.track` resolves to no track.
/// - [`TrackSetPanError::KindMismatch`] if the target track is not audio.
/// - [`TrackSetPanError::Locked`] if the target track is locked.
/// - [`TrackSetPanError::BadRange`] if `args.pan` is not finite in `[-1.0, 1.0]`.
/// - idempotent no-op path: empty patch + [`W_NOOP_CODE`] warning when
///   `args.pan` equals the current track pan.
pub fn compute_patch(
    prior: &Project,
    args: &TrackSetPanArgs,
) -> Result<(Value, Vec<Value>, TrackSetPanData), TrackSetPanError> {
    let track_id = args
        .track
        .parse::<TrackId>()
        .map_err(|err| TrackSetPanError::BadSelector {
            detail: err.to_string(),
        })?;

    let (global_idx, track) = prior
        .tracks
        .iter()
        .enumerate()
        .find(|(_, track)| track.id == track_id)
        .ok_or(TrackSetPanError::TrackNotFound {
            track_id: args.track.clone(),
        })?;

    if track.kind != TrackKind::Audio {
        return Err(TrackSetPanError::KindMismatch {
            track_id: args.track.clone(),
            found_kind: track.kind,
        });
    }

    if track.locked {
        return Err(TrackSetPanError::Locked {
            track_id: args.track.clone(),
        });
    }

    if !(-1.0..=1.0).contains(&args.pan) || !args.pan.is_finite() {
        return Err(TrackSetPanError::BadRange { value: args.pan });
    }

    let current_pan = track.pan;
    let target_pan = args.pan;

    #[allow(clippy::float_cmp)]
    if target_pan == current_pan {
        return Ok((
            json!([]),
            vec![json!({
                "code": W_NOOP_CODE,
                "message": "track pan unchanged",
                "details": {
                    "track_id": track_id.to_string(),
                    "pan": current_pan,
                }
            })],
            TrackSetPanData {
                track_id,
                pan: current_pan,
            },
        ));
    }

    let patch = json!([{
        "op": "replace",
        "path": format!("/tracks/{global_idx}/pan"),
        "value": target_pan,
    }]);

    Ok((
        patch,
        Vec::new(),
        TrackSetPanData {
            track_id,
            pan: target_pan,
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
    args: &TrackSetPanArgs,
    post_state: &Project,
) -> Result<TrackSetPanData, ReconstructError> {
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
            detail: format!("track pan: track id {track_id} not found in post_state.tracks"),
        })?;

    Ok(TrackSetPanData {
        track_id,
        pan: track.pan,
    })
}

/// `track.set_pan` verb registration entry.
#[derive(Debug, Default)]
pub struct TrackSetPanVerb;

impl From<TrackSetPanError> for VerbError {
    fn from(value: TrackSetPanError) -> Self {
        match value {
            TrackSetPanError::BadSelector { .. }
            | TrackSetPanError::TrackNotFound { .. }
            | TrackSetPanError::KindMismatch { .. }
            | TrackSetPanError::Locked { .. }
            | TrackSetPanError::BadRange { .. } => VerbError::BadArgs {
                detail: value.to_string(),
            },
        }
    }
}

impl Verb for TrackSetPanVerb {
    fn verb(&self) -> &'static str {
        "track.set_pan"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: TrackSetPanArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("track.set_pan: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, _data) = compute_patch(prior, &typed)?;

        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!("track.set_pan: patch construction failed: {err}"))
            })?;

        let post_state = prior
            .apply(&patch)
            .map_err(|err| VerbError::InvariantViolation {
                detail: format!("track.set_pan: post-state validation failed: {err}"),
            })?;

        let envelope = data_envelope_from_post_state(&typed, &post_state).map_err(|err| {
            VerbError::Custom(format!(
                "track.set_pan: data envelope reconstruction failed: {err}"
            ))
        })?;

        let data = serde_json::to_value(&envelope).map_err(|err| {
            VerbError::Custom(format!("track.set_pan: data serialize failed: {err}"))
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
        let typed: TrackSetPanArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "TrackSetPanArgs",
            })?;

        let envelope = data_envelope_from_post_state(&typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
