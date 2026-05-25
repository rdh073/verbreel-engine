//! `clip.set_blend_mode` (§5.18) — twenty-second production verb.
//!
//! ## Spec quote (`spec/commands/clip.md` §5.18, verbatim)
//!
//! > Sets `Clip.blend_mode`.
//! > Full-write setter — omitting `blend_mode` rejected at args-schema validator.
//! > To restore v1.0 default, pass `blend_mode: "normal"`. Idempotent W_NOOP.
//! > Render-time scope: video/image/text — observable. Audio — inert. Schema permits on every kind (forward compat); call against audio succeeds with `W_BLEND_MODE_INERT_ON_AUDIO`.
//! > Linked clips: not propagated.
//! > CLI: `verbreel clip set_blend_mode [--project <id>] --clip <id> --blend_mode <name>`
//! > MCP: `clip.set_blend_mode`
//! > Args: `project_id: string`, `clip: string`, `blend_mode: "normal"|"multiply"|"screen"|"overlay"|"soft-light"|"hard-light"|"darken"|"lighten"|"difference"|"color-dodge"|"color-burn"`
//! > Returns (`data`): `{ clip_id: string; blend_mode: string }`
//! > Errors: `E_NOT_FOUND`, `E_NO_MATCH`, `E_BAD_SELECTOR`, `E_SELECTOR_KIND_MISMATCH`, `E_SCHEMA_VIOLATION`, `E_LOCKED`
//! > Warnings: `W_NOOP`, `W_BLEND_MODE_INERT_ON_AUDIO`
//!
//! `W_BLEND_MODE_INERT_ON_AUDIO` is emitted when the target clip resolves
//! on an audio track. The field is stored and serialized but ignored at
//! render time per `spec/commands/clip.md` §5.18.
//! Audio targets still pass `BadKindMismatch` checks because this is a
//! first enum-setter verb and is scope-forward-compatible by spec.

use crate::clip::BlendMode;
use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use crate::track::TrackKind;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::{ClipId, ProjectId};

/// Warning code emitted when the incoming blend mode equals current.
pub const W_NOOP_CODE: &str = "W_NOOP";

/// Warning code emitted when the target clip lives on an audio track.
pub const W_BLEND_MODE_INERT_ON_AUDIO_CODE: &str = "W_BLEND_MODE_INERT_ON_AUDIO";

/// Args for `clip.set_blend_mode`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipSetBlendModeArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Target clip id as bare `UUIDv7`.
    pub clip: String,

    /// New blend mode.
    pub blend_mode: BlendMode,
}

/// Envelope `data` returned by `clip.set_blend_mode`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipSetBlendModeData {
    /// Target clip id.
    pub clip_id: ClipId,

    /// New blend mode in post-state.
    pub blend_mode: BlendMode,
}

/// Verb-level validation failures for `clip.set_blend_mode`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ClipSetBlendModeError {
    /// `args.clip` is not parseable as `UUIDv7`.
    #[error("clip.set_blend_mode: `clip` selector parse failed: {detail}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
    },

    /// No clip exists for `clip`.
    #[error("clip.set_blend_mode: clip `{clip_id}` not found")]
    ClipNotFound {
        /// Missing clip id.
        clip_id: String,
    },

    /// The target clip is locked.
    #[error("clip.set_blend_mode: clip `{clip_id}` is locked")]
    Locked {
        /// Locked clip id.
        clip_id: String,
    },
}

/// Build the RFC-6902 patch for `clip.set_blend_mode`.
///
/// # Errors
///
/// - [`ClipSetBlendModeError::BadSelector`] for non-UUIDv7 `args.clip`.
/// - [`ClipSetBlendModeError::ClipNotFound`] if `args.clip` resolves to no clip.
/// - [`ClipSetBlendModeError::Locked`] if the target clip is locked.
/// - Idempotent no-op path: empty patch + [`W_NOOP_CODE`] warning when
///   `args.blend_mode` equals current clip blend mode.
/// - Audio-track warning: [`W_BLEND_MODE_INERT_ON_AUDIO_CODE`] when a changed
///   value is stored for an audio clip and render ignores it.
///
/// # Panics
///
/// This function may panic if `serde_json::to_value` fails while
/// serializing a valid `BlendMode` to JSON. Given `BlendMode` is a
/// fixed string enum, this path is unreachable under current schema.
pub fn compute_patch(
    prior: &Project,
    args: &ClipSetBlendModeArgs,
) -> Result<(Value, Vec<Value>, ClipSetBlendModeData), ClipSetBlendModeError> {
    let clip_id =
        args.clip
            .parse::<ClipId>()
            .map_err(|err| ClipSetBlendModeError::BadSelector {
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

    let (t_idx, c_idx, track, clip) =
        location.ok_or_else(|| ClipSetBlendModeError::ClipNotFound {
            clip_id: args.clip.clone(),
        })?;

    if clip.locked {
        return Err(ClipSetBlendModeError::Locked {
            clip_id: args.clip.clone(),
        });
    }

    let target_blend_mode = args.blend_mode;
    let current_blend_mode = clip.blend_mode;

    if target_blend_mode == current_blend_mode {
        return Ok((
            json!([]),
            vec![json!({
                "code": W_NOOP_CODE,
                "message": "clip blend_mode unchanged",
                "details": {
                    "clip_id": clip_id.to_string(),
                    "blend_mode": serde_json::to_value(current_blend_mode)
                        .expect("BlendMode serializes"),
                }
            })],
            ClipSetBlendModeData {
                clip_id,
                blend_mode: current_blend_mode,
            },
        ));
    }

    let patch = json!([{
        "op": "replace",
        "path": format!("/tracks/{t_idx}/clips/{c_idx}/blend_mode"),
        "value": target_blend_mode,
    }]);

    let mut warnings = Vec::new();
    if track.kind == TrackKind::Audio {
        warnings.push(json!({
            "code": W_BLEND_MODE_INERT_ON_AUDIO_CODE,
            "message": "blend_mode is inert on audio tracks (stored but ignored at render time)",
            "details": {
                "clip_id": clip_id.to_string(),
                "blend_mode": serde_json::to_value(target_blend_mode)
                    .expect("BlendMode serializes"),
            }
        }));
    }

    Ok((
        patch,
        warnings,
        ClipSetBlendModeData {
            clip_id,
            blend_mode: target_blend_mode,
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
    args: &ClipSetBlendModeArgs,
    post_state: &Project,
) -> Result<ClipSetBlendModeData, ReconstructError> {
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
                return Ok(ClipSetBlendModeData {
                    clip_id,
                    blend_mode: clip.blend_mode,
                });
            }
        }
    }

    Err(ReconstructError::PostStateMissing {
        detail: format!("clip set_blend_mode: clip id {clip_id} not found in post_state"),
    })
}

/// `clip.set_blend_mode` verb registration entry.
#[derive(Debug, Default)]
pub struct ClipSetBlendModeVerb;

impl From<ClipSetBlendModeError> for VerbError {
    fn from(value: ClipSetBlendModeError) -> Self {
        match value {
            ClipSetBlendModeError::BadSelector { .. }
            | ClipSetBlendModeError::ClipNotFound { .. }
            | ClipSetBlendModeError::Locked { .. } => VerbError::BadArgs {
                detail: value.to_string(),
            },
        }
    }
}

impl Verb for ClipSetBlendModeVerb {
    fn verb(&self) -> &'static str {
        "clip.set_blend_mode"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: ClipSetBlendModeArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("clip.set_blend_mode: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, _data) = compute_patch(prior, &typed)?;

        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!(
                    "clip.set_blend_mode: patch construction failed: {err}"
                ))
            })?;

        let post_state = prior
            .apply(&patch)
            .map_err(|err| VerbError::InvariantViolation {
                detail: format!("clip.set_blend_mode: post-state validation failed: {err}"),
            })?;

        let envelope = data_envelope_from_post_state(&typed, &post_state).map_err(|err| {
            VerbError::Custom(format!(
                "clip.set_blend_mode: data envelope reconstruction failed: {err}"
            ))
        })?;

        let data = serde_json::to_value(&envelope).map_err(|err| {
            VerbError::Custom(format!("clip.set_blend_mode: data serialize failed: {err}"))
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
        let typed: ClipSetBlendModeArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "ClipSetBlendModeArgs",
            })?;

        let envelope = data_envelope_from_post_state(&typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
