//! `clip.set_volume` (§5.11) — twenty-first production verb.
//!
//! ## Spec quote (`spec/commands/clip.md` §5.11, verbatim)
//!
//! > Sets `Clip.volume`, the per-clip linear gain on the audio path.
//! > The target clip MUST be on a `kind: "audio"` track.
//! > Calling against a video / image / text clip returns `E_CLIP_KIND_MISMATCH`.
//! > CLI: `verbreel clip set_volume [--project <id>] --clip <id> --volume <0..4>`
//! > MCP: `clip.set_volume`
//! > Args: `project_id: string`, `clip: string`, `volume: number`
//! > Returns (`data`): `{ clip_id: string; volume: number }`
//! > Errors: `E_NOT_FOUND`, `E_NO_MATCH`, `E_BAD_SELECTOR`,
//! >         `E_CLIP_KIND_MISMATCH`, `E_BAD_RANGE`, `E_LOCKED`
//!
//! ## Audio-only check and idempotency
//!
//! `clip.set_volume` is constrained to clips on an audio track.
//! The target clip must be found first; if its parent track is not audio,
//! the verb returns [`ClipSetVolumeError::KindMismatch`] before lock/range
//! checks.
//! Idempotent no-op behavior is the same as other setter verbs: if incoming
//! `volume` equals current clip volume, the verb returns:
//! - empty patch (`[]`)
//! - single [`W_NOOP_CODE`] warning (`message` = `clip volume unchanged`)
//! - data envelope from post-state.

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use crate::track::TrackKind;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::{ClipId, ProjectId};

/// Warning code emitted when the incoming volume equals current.
pub const W_NOOP_CODE: &str = "W_NOOP";

/// Args for `clip.set_volume`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipSetVolumeArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Target clip id as bare `UUIDv7`.
    pub clip: String,

    /// New volume in range `[0.0, 4.0]`.
    pub volume: f64,
}

/// Envelope `data` returned by `clip.set_volume`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipSetVolumeData {
    /// Target clip id.
    pub clip_id: ClipId,

    /// New clip volume in post-state.
    pub volume: f64,
}

/// Verb-level validation failures for `clip.set_volume`.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum ClipSetVolumeError {
    /// `args.clip` is not parseable as `UUIDv7`.
    #[error("clip.set_volume: `clip` selector parse failed: {detail}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
    },

    /// No clip exists for `clip`.
    #[error("clip.set_volume: clip `{clip_id}` not found")]
    ClipNotFound {
        /// Missing clip id string.
        clip_id: String,
    },

    /// Target clip is on a non-audio track.
    #[error("clip.set_volume: clip `{clip_id}` is on a {found_kind:?} track, not audio")]
    KindMismatch {
        /// Missing clip id string.
        clip_id: String,

        /// Actual track kind for this clip.
        found_kind: TrackKind,
    },

    /// The target clip is locked.
    #[error("clip.set_volume: clip `{clip_id}` is locked")]
    Locked {
        /// Locked clip id.
        clip_id: String,
    },

    /// The provided volume is out of bounds or non-finite.
    #[error("clip.set_volume: volume {value} out of range [0.0, 4.0]")]
    BadRange {
        /// Invalid volume value.
        value: f64,
    },
}

/// Build the RFC-6902 patch for `clip.set_volume`.
///
/// # Errors
///
/// - [`ClipSetVolumeError::BadSelector`] for non-UUIDv7 `args.clip`.
/// - [`ClipSetVolumeError::ClipNotFound`] if `args.clip` resolves to no clip.
/// - [`ClipSetVolumeError::KindMismatch`] if clip parent track is not audio.
/// - [`ClipSetVolumeError::Locked`] if target clip is locked.
/// - [`ClipSetVolumeError::BadRange`] if `args.volume` is not finite in
///   `[0.0, 4.0]`.
/// - idempotent no-op path: empty patch + [`W_NOOP_CODE`] warning when
///   `args.volume` equals the current clip volume.
pub fn compute_patch(
    prior: &Project,
    args: &ClipSetVolumeArgs,
) -> Result<(Value, Vec<Value>, ClipSetVolumeData), ClipSetVolumeError> {
    let clip_id = args
        .clip
        .parse::<ClipId>()
        .map_err(|err| ClipSetVolumeError::BadSelector {
            detail: err.to_string(),
        })?;

    let mut location: Option<(usize, usize, &crate::track::Track, &crate::clip::Clip)> = None;
    for (t_idx, track) in prior.tracks.iter().enumerate() {
        for (c_idx, clip) in track.clips.iter().enumerate() {
            if clip.id == clip_id {
                location = Some((t_idx, c_idx, track, clip));
                break;
            }
        }
        if location.is_some() {
            break;
        }
    }

    let (t_idx, c_idx, track, clip) = location.ok_or_else(|| ClipSetVolumeError::ClipNotFound {
        clip_id: args.clip.clone(),
    })?;

    if track.kind != TrackKind::Audio {
        return Err(ClipSetVolumeError::KindMismatch {
            clip_id: args.clip.clone(),
            found_kind: track.kind,
        });
    }

    if clip.locked {
        return Err(ClipSetVolumeError::Locked {
            clip_id: args.clip.clone(),
        });
    }

    if !(0.0..=4.0).contains(&args.volume) || !args.volume.is_finite() {
        return Err(ClipSetVolumeError::BadRange { value: args.volume });
    }

    let current_volume = clip.volume;
    let target_volume = args.volume;

    #[allow(clippy::float_cmp)]
    if target_volume == current_volume {
        return Ok((
            json!([]),
            vec![json!({
                "code": W_NOOP_CODE,
                "message": "clip volume unchanged",
                "details": {
                    "clip_id": clip_id.to_string(),
                    "volume": current_volume,
                }
            })],
            ClipSetVolumeData {
                clip_id,
                volume: current_volume,
            },
        ));
    }

    let patch = json!([{
        "op": "replace",
        "path": format!("/tracks/{t_idx}/clips/{c_idx}/volume"),
        "value": target_volume,
    }]);

    Ok((
        patch,
        Vec::new(),
        ClipSetVolumeData {
            clip_id,
            volume: target_volume,
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
    args: &ClipSetVolumeArgs,
    post_state: &Project,
) -> Result<ClipSetVolumeData, ReconstructError> {
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
                return Ok(ClipSetVolumeData {
                    clip_id,
                    volume: clip.volume,
                });
            }
        }
    }

    Err(ReconstructError::PostStateMissing {
        detail: format!("clip set_volume: clip id {clip_id} not found in post_state"),
    })
}

/// `clip.set_volume` verb registration entry.
#[derive(Debug, Default)]
pub struct ClipSetVolumeVerb;

impl From<ClipSetVolumeError> for VerbError {
    fn from(value: ClipSetVolumeError) -> Self {
        match value {
            ClipSetVolumeError::BadSelector { .. }
            | ClipSetVolumeError::ClipNotFound { .. }
            | ClipSetVolumeError::KindMismatch { .. }
            | ClipSetVolumeError::Locked { .. }
            | ClipSetVolumeError::BadRange { .. } => VerbError::BadArgs {
                detail: value.to_string(),
            },
        }
    }
}

impl Verb for ClipSetVolumeVerb {
    fn verb(&self) -> &'static str {
        "clip.set_volume"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: ClipSetVolumeArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("clip.set_volume: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, _data) = compute_patch(prior, &typed)?;

        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!("clip.set_volume: patch construction failed: {err}"))
            })?;

        let post_state = prior
            .apply(&patch)
            .map_err(|err| VerbError::InvariantViolation {
                detail: format!("clip.set_volume: post-state validation failed: {err}"),
            })?;

        let envelope = data_envelope_from_post_state(&typed, &post_state).map_err(|err| {
            VerbError::Custom(format!(
                "clip.set_volume: data envelope reconstruction failed: {err}"
            ))
        })?;

        let data = serde_json::to_value(&envelope).map_err(|err| {
            VerbError::Custom(format!("clip.set_volume: data serialize failed: {err}"))
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
        let typed: ClipSetVolumeArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "ClipSetVolumeArgs",
            })?;

        let envelope = data_envelope_from_post_state(&typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
