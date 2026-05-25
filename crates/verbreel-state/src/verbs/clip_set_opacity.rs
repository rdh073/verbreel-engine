//! `clip.set_opacity` (§5.10) — twentieth production verb in the engine.
//!
//! ## Spec quote (`spec/commands/clip.md` §5.10, verbatim)
//!
//! > CLI: `verbreel clip set_opacity [--project <id>] --clip <id> --opacity <0..1>`
//! > MCP: `clip.set_opacity`
//! > Args: `project_id: string`, `clip: string`, `opacity: number`
//! > Returns (`data`): `{ clip_id: string; opacity: number }`
//! > Errors: `E_NOT_FOUND`, `E_NO_MATCH`, `E_BAD_SELECTOR`,
//! >         `E_SELECTOR_KIND_MISMATCH`, `E_BAD_RANGE`, `E_LOCKED`
//!
//! ## Behavior
//! `clip` is accepted as **bare `UUIDv7` only** in this slice.
//! Opacity is permissive across all clip kinds (no kind guard).

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::{ClipId, ProjectId};

/// Warning code emitted when the incoming opacity equals current.
pub const W_NOOP_CODE: &str = "W_NOOP";

/// Args for `clip.set_opacity`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipSetOpacityArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Target clip id as bare `UUIDv7`.
    pub clip: String,

    /// New opacity in range `[0.0, 1.0]`.
    pub opacity: f64,
}

/// Envelope `data` returned by `clip.set_opacity`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipSetOpacityData {
    /// Target clip id.
    pub clip_id: ClipId,

    /// New opacity in post-state.
    pub opacity: f64,
}

/// Verb-level validation failures for `clip.set_opacity`.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum ClipSetOpacityError {
    /// `args.clip` is not parseable as `UUIDv7`.
    #[error("clip.set_opacity: `clip` selector parse failed: {detail}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
    },

    /// No clip exists for `clip`.
    #[error("clip.set_opacity: clip `{clip_id}` not found")]
    ClipNotFound {
        /// Missing clip id string.
        clip_id: String,
    },

    /// The target clip is locked.
    #[error("clip.set_opacity: clip `{clip_id}` is locked")]
    Locked {
        /// Locked clip id.
        clip_id: String,
    },

    /// The provided opacity is out of range or non-finite.
    #[error("clip.set_opacity: opacity {value} out of range [0.0, 1.0]")]
    BadRange {
        /// Invalid opacity value.
        value: f64,
    },
}

/// Build the RFC-6902 patch for `clip.set_opacity`.
///
/// # Errors
///
/// - [`ClipSetOpacityError::BadSelector`] for non-UUIDv7 `args.clip`.
/// - [`ClipSetOpacityError::ClipNotFound`] if `args.clip` resolves to no clip.
/// - [`ClipSetOpacityError::Locked`] if target clip is locked.
/// - [`ClipSetOpacityError::BadRange`] if opacity is not finite in
///   `[0.0, 1.0]`.
/// - idempotent no-op path: empty patch + [`W_NOOP_CODE`] warning when
///   opacities are already equal.
pub fn compute_patch(
    prior: &Project,
    args: &ClipSetOpacityArgs,
) -> Result<(Value, Vec<Value>, ClipSetOpacityData), ClipSetOpacityError> {
    let clip_id = args
        .clip
        .parse::<ClipId>()
        .map_err(|err| ClipSetOpacityError::BadSelector {
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

    let (t_idx, c_idx, clip) = location.ok_or_else(|| ClipSetOpacityError::ClipNotFound {
        clip_id: args.clip.clone(),
    })?;

    if clip.locked {
        return Err(ClipSetOpacityError::Locked {
            clip_id: args.clip.clone(),
        });
    }

    if !(0.0..=1.0).contains(&args.opacity) || !args.opacity.is_finite() {
        return Err(ClipSetOpacityError::BadRange {
            value: args.opacity,
        });
    }

    let current_opacity = clip.opacity;
    let target_opacity = args.opacity;

    #[allow(clippy::float_cmp)]
    if target_opacity == current_opacity {
        return Ok((
            json!([]),
            vec![json!({
                "code": W_NOOP_CODE,
                "message": "clip opacity unchanged",
                "details": {
                    "clip_id": clip_id.to_string(),
                    "opacity": current_opacity,
                }
            })],
            ClipSetOpacityData {
                clip_id,
                opacity: current_opacity,
            },
        ));
    }

    let patch = json!([{
        "op": "replace",
        "path": format!("/tracks/{t_idx}/clips/{c_idx}/opacity"),
        "value": target_opacity,
    }]);

    Ok((
        patch,
        Vec::new(),
        ClipSetOpacityData {
            clip_id,
            opacity: target_opacity,
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
    args: &ClipSetOpacityArgs,
    post_state: &Project,
) -> Result<ClipSetOpacityData, ReconstructError> {
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
                return Ok(ClipSetOpacityData {
                    clip_id,
                    opacity: clip.opacity,
                });
            }
        }
    }

    Err(ReconstructError::PostStateMissing {
        detail: format!("clip set_opacity: clip id {clip_id} not found in post_state"),
    })
}

/// `clip.set_opacity` verb registration entry.
#[derive(Debug, Default)]
pub struct ClipSetOpacityVerb;

impl From<ClipSetOpacityError> for VerbError {
    fn from(value: ClipSetOpacityError) -> Self {
        match value {
            ClipSetOpacityError::BadSelector { .. }
            | ClipSetOpacityError::ClipNotFound { .. }
            | ClipSetOpacityError::Locked { .. }
            | ClipSetOpacityError::BadRange { .. } => VerbError::BadArgs {
                detail: value.to_string(),
            },
        }
    }
}

impl Verb for ClipSetOpacityVerb {
    fn verb(&self) -> &'static str {
        "clip.set_opacity"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: ClipSetOpacityArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("clip.set_opacity: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, _data) = compute_patch(prior, &typed)?;

        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!(
                    "clip.set_opacity: patch construction failed: {err}"
                ))
            })?;

        let post_state = prior
            .apply(&patch)
            .map_err(|err| VerbError::InvariantViolation {
                detail: format!("clip.set_opacity: post-state validation failed: {err}"),
            })?;

        let envelope = data_envelope_from_post_state(&typed, &post_state).map_err(|err| {
            VerbError::Custom(format!(
                "clip.set_opacity: data envelope reconstruction failed: {err}"
            ))
        })?;

        let data = serde_json::to_value(&envelope).map_err(|err| {
            VerbError::Custom(format!("clip.set_opacity: data serialize failed: {err}"))
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
        let typed: ClipSetOpacityArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "ClipSetOpacityArgs",
            })?;

        let envelope = data_envelope_from_post_state(&typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
