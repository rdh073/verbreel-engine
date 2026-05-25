//! `track.set_volume` (§4.8) — fifteenth production verb in the engine.
//!
//! ## Spec quote (`spec/commands/track.md` §4.8, verbatim)
//!
//! > Audio tracks only.
//! > CLI: `verbreel track set_volume [--project <id>] --track <selector> --volume <0..4>`
//! > MCP: `track.set_volume`
//! > Args: `project_id: string`, `track: string`, `volume: number`
//! > Returns (`data`): `{ track_id: string; volume: number }`
//! > Errors: `E_TRACK_NOT_FOUND`, `E_BAD_SELECTOR`, `E_SELECTOR_KIND_MISMATCH`, `E_TRACK_KIND_MISMATCH`, `E_BAD_RANGE`, `E_LOCKED`
//!
//! ## Audio-only and kind mismatch guard
//!
//! This verb is restricted to audio tracks. When a non-audio track is
//! targeted, the verb fails with
//! [`TrackSetVolumeError::KindMismatch`] before evaluating lock/range
//! guards.
//!
//! ## Volume range guard
//!
//! Accepted values are `0.0 <= volume <= 4.0` and finite. Values
//! outside that range, or non-finite values like `NaN` and infinities,
//! fail as `TrackSetVolumeError::BadRange`.
//!
//! ## Selector handling (v1)
//!
//! `track` is accepted as **bare `UUIDv7` only** in this slice.
//! Structural selectors (for example `video[0]` or `video[name="main"]`)
//! are deferred to a future verb-slice. Bare selector parse failures
//! map to [`TrackSetVolumeError::BadSelector`].
//!
//! ## Idempotent volume behavior (§0.6)
//!
//! Calling `track.set_volume` with the current volume is a successful no-op:
//! - empty patch (`[]`)
//! - single [`W_NOOP_CODE`] warning (`message` = `track volume unchanged`)
//! - data envelope still returns `{ track_id, volume }` from post-state.
//!
//! ## Lock behavior
//!
//! Locked tracks (`track.locked == true`) are rejected with
//! [`TrackSetVolumeError::Locked`].

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use crate::track::TrackKind;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::{ProjectId, TrackId};

/// Warning code emitted when the incoming volume equals the current volume.
pub const W_NOOP_CODE: &str = "W_NOOP";

/// Args for `track.set_volume`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackSetVolumeArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Target track id as bare `UUIDv7`.
    pub track: String,

    /// Desired volume (must be within `[0.0, 4.0]` and finite).
    pub volume: f64,
}

/// Envelope `data` returned by `track.set_volume`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackSetVolumeData {
    /// Target track id.
    pub track_id: TrackId,

    /// New volume in post-state.
    pub volume: f64,
}

/// Verb-level validation failures for `track.set_volume`.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum TrackSetVolumeError {
    /// `args.track` is not parseable as `UUIDv7`.
    #[error("track.set_volume: `track` selector parse failed: {detail}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
    },

    /// No track exists for `track`.
    #[error("track.set_volume: track `{track_id}` not found")]
    TrackNotFound {
        /// Missing track id string.
        track_id: String,
    },

    /// Target track kind is not audio.
    #[error(
        "track.set_volume: track `{track_id}` is kind {found_kind:?}, but only audio tracks support volume"
    )]
    KindMismatch {
        /// Target track id.
        track_id: String,

        /// Target track kind.
        found_kind: TrackKind,
    },

    /// The target track is locked.
    #[error("track.set_volume: track `{track_id}` is locked")]
    Locked {
        /// Locked track id.
        track_id: String,
    },

    /// The provided volume is out of bounds or non-finite.
    #[error("track.set_volume: volume {value} out of range [0.0, 4.0]")]
    BadRange {
        /// Invalid volume value.
        value: f64,
    },
}

/// Build the RFC-6902 patch for `track.set_volume`.
///
/// # Errors
///
/// - [`TrackSetVolumeError::BadSelector`] for non-UUIDv7 `args.track`.
/// - [`TrackSetVolumeError::TrackNotFound`] if `args.track` resolves to no track.
/// - [`TrackSetVolumeError::KindMismatch`] if the target track is not audio.
/// - [`TrackSetVolumeError::Locked`] if the target track is locked.
/// - [`TrackSetVolumeError::BadRange`] if `args.volume` is not finite in `[0.0, 4.0]`.
/// - idempotent no-op path: empty patch + [`W_NOOP_CODE`] warning when
///   `args.volume` equals the current track volume.
pub fn compute_patch(
    prior: &Project,
    args: &TrackSetVolumeArgs,
) -> Result<(Value, Vec<Value>, TrackSetVolumeData), TrackSetVolumeError> {
    let track_id =
        args.track
            .parse::<TrackId>()
            .map_err(|err| TrackSetVolumeError::BadSelector {
                detail: err.to_string(),
            })?;

    let (global_idx, track) = prior
        .tracks
        .iter()
        .enumerate()
        .find(|(_, track)| track.id == track_id)
        .ok_or(TrackSetVolumeError::TrackNotFound {
            track_id: args.track.clone(),
        })?;

    if track.kind != TrackKind::Audio {
        return Err(TrackSetVolumeError::KindMismatch {
            track_id: args.track.clone(),
            found_kind: track.kind,
        });
    }

    if track.locked {
        return Err(TrackSetVolumeError::Locked {
            track_id: args.track.clone(),
        });
    }

    if !(0.0..=4.0).contains(&args.volume) || !args.volume.is_finite() {
        return Err(TrackSetVolumeError::BadRange { value: args.volume });
    }

    let current_volume = track.volume;
    let target_volume = args.volume;

    #[allow(clippy::float_cmp)]
    if target_volume == current_volume {
        return Ok((
            json!([]),
            vec![json!({
                "code": W_NOOP_CODE,
                "message": "track volume unchanged",
                "details": {
                    "track_id": track_id.to_string(),
                    "volume": current_volume,
                }
            })],
            TrackSetVolumeData {
                track_id,
                volume: current_volume,
            },
        ));
    }

    let patch = json!([{
        "op": "replace",
        "path": format!("/tracks/{global_idx}/volume"),
        "value": target_volume,
    }]);

    Ok((
        patch,
        Vec::new(),
        TrackSetVolumeData {
            track_id,
            volume: target_volume,
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
    args: &TrackSetVolumeArgs,
    post_state: &Project,
) -> Result<TrackSetVolumeData, ReconstructError> {
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
            detail: format!("track volume: track id {track_id} not found in post_state.tracks"),
        })?;

    Ok(TrackSetVolumeData {
        track_id,
        volume: track.volume,
    })
}

/// `track.set_volume` verb registration entry.
#[derive(Debug, Default)]
pub struct TrackSetVolumeVerb;

impl From<TrackSetVolumeError> for VerbError {
    fn from(value: TrackSetVolumeError) -> Self {
        match value {
            TrackSetVolumeError::BadSelector { .. }
            | TrackSetVolumeError::TrackNotFound { .. }
            | TrackSetVolumeError::KindMismatch { .. }
            | TrackSetVolumeError::Locked { .. }
            | TrackSetVolumeError::BadRange { .. } => VerbError::BadArgs {
                detail: value.to_string(),
            },
        }
    }
}

impl Verb for TrackSetVolumeVerb {
    fn verb(&self) -> &'static str {
        "track.set_volume"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: TrackSetVolumeArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("track.set_volume: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, _data) = compute_patch(prior, &typed)?;

        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!(
                    "track.set_volume: patch construction failed: {err}"
                ))
            })?;

        let post_state = prior
            .apply(&patch)
            .map_err(|err| VerbError::InvariantViolation {
                detail: format!("track.set_volume: post-state validation failed: {err}"),
            })?;

        let envelope = data_envelope_from_post_state(&typed, &post_state).map_err(|err| {
            VerbError::Custom(format!(
                "track.set_volume: data envelope reconstruction failed: {err}"
            ))
        })?;

        let data = serde_json::to_value(&envelope).map_err(|err| {
            VerbError::Custom(format!("track.set_volume: data serialize failed: {err}"))
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
        let typed: TrackSetVolumeArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "TrackSetVolumeArgs",
            })?;

        let envelope = data_envelope_from_post_state(&typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
