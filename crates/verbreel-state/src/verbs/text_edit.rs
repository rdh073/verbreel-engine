//! `text.edit` (§7.2) — thirtieth production verb in the engine.
//!
//! ## Spec quote (`spec/commands/text.md` §7.2, verbatim)
//!
//! > Writes `Clip.text.content` on a text clip.
//! > Non-text clips lack the `text` field; calling against video/audio/image
//! > returns `E_CLIP_KIND_MISMATCH`.
//! > CLI: `verbreel text edit [--project <id>] --clip <id> --content <str>`
//! > MCP: `text.edit`
//! > Args: `project_id: string`, `clip: string`, `content: string`
//! > Returns (`data`): `{ clip_id: string; content: string }`
//! > Errors: `E_NOT_FOUND`, `E_NO_MATCH`, `E_BAD_SELECTOR`,
//! >         `E_CLIP_KIND_MISMATCH`, `E_SCHEMA_VIOLATION`, `E_LOCKED`.
//!
//! ## Text-only check and idempotency
//!
//! `text.edit` updates the `text.content` field on the target clip.
//! The target clip must be on a text track; non-text tracks return
//! [`TextEditError::ClipKindMismatch`].
//! Idempotent no-op behavior is the same as other setter verbs: if incoming
//! `content` equals current `text.content`, the verb returns:
//! - empty patch (`[]`)
//! - single [`W_NOOP_CODE`] warning (`message` = `text content unchanged`)
//! - data envelope from post-state
use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use crate::track::TrackKind;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::{ClipId, ProjectId};

/// Warning code emitted when incoming content equals current content.
pub const W_NOOP_CODE: &str = "W_NOOP";

/// Maximum allowed text content length.
pub const MAX_CONTENT_LEN: usize = 8192;

/// Args for `text.edit`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextEditArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Target clip id as bare `UUIDv7`.
    pub clip: String,

    /// New content value.
    pub content: String,
}

/// Envelope `data` returned by `text.edit`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextEditData {
    /// Target clip id.
    pub clip_id: ClipId,

    /// Updated text content in post-state.
    pub content: String,
}

/// Verb-level validation failures for `text.edit`.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum TextEditError {
    /// `args.clip` is not parseable as `UUIDv7`.
    #[error("text.edit: `clip` selector parse failed: {detail}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
    },

    /// No clip exists for `clip`.
    #[error("text.edit: clip `{clip_id}` not found")]
    ClipNotFound {
        /// Missing clip id string.
        clip_id: String,
    },

    /// Target clip is on a non-text track.
    #[error("text.edit: clip `{clip_id}` is on a {found_kind:?} track, not text")]
    ClipKindMismatch {
        /// Missing clip id string.
        clip_id: String,

        /// Actual track kind.
        found_kind: TrackKind,
    },

    /// The target clip is locked.
    #[error("text.edit: clip `{clip_id}` is locked")]
    Locked {
        /// Locked clip id.
        clip_id: String,
    },

    /// Content length exceeds the schema limit.
    #[error("text.edit: content schema violation: {detail}")]
    SchemaViolation {
        /// Human-readable detail.
        detail: String,
    },
}

/// Build the RFC-6902 patch for `text.edit`.
///
/// # Errors
///
/// - [`TextEditError::BadSelector`] for non-UUIDv7 `args.clip`.
/// - [`TextEditError::ClipNotFound`] if `args.clip` resolves to no clip.
/// - [`TextEditError::ClipKindMismatch`] if clip parent track is not text.
/// - [`TextEditError::Locked`] if target clip is locked.
/// - [`TextEditError::SchemaViolation`] if `content` is longer than
///   [`MAX_CONTENT_LEN`] characters.
/// - idempotent no-op path: empty patch + [`W_NOOP_CODE`] warning when
///   contents are already equal.
pub fn compute_patch(
    prior: &Project,
    args: &TextEditArgs,
) -> Result<(Value, Vec<Value>, TextEditData), TextEditError> {
    let clip_id = args
        .clip
        .parse::<ClipId>()
        .map_err(|err| TextEditError::BadSelector {
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

    let (t_idx, c_idx, track, clip) = location.ok_or_else(|| TextEditError::ClipNotFound {
        clip_id: args.clip.clone(),
    })?;

    if track.kind != TrackKind::Text {
        return Err(TextEditError::ClipKindMismatch {
            clip_id: args.clip.clone(),
            found_kind: track.kind,
        });
    }

    if clip.locked {
        return Err(TextEditError::Locked {
            clip_id: args.clip.clone(),
        });
    }

    let content_len = args.content.chars().count();
    if content_len > MAX_CONTENT_LEN {
        return Err(TextEditError::SchemaViolation {
            detail: format!("content length {content_len} exceeds max {MAX_CONTENT_LEN}"),
        });
    }

    let current_content = clip.text.as_ref().map(|text| text.content.as_str());
    if current_content == Some(args.content.as_str()) {
        return Ok((
            json!([]),
            vec![json!({
                "code": W_NOOP_CODE,
                "message": "text content unchanged",
                "details": {
                    "clip_id": clip_id.to_string(),
                    "content": args.content,
                }
            })],
            TextEditData {
                clip_id,
                content: args.content.clone(),
            },
        ));
    }

    let patch = json!([{
        "op": "replace",
        "path": format!("/tracks/{t_idx}/clips/{c_idx}/text/content"),
        "value": args.content.clone(),
    }]);

    Ok((
        patch,
        Vec::new(),
        TextEditData {
            clip_id,
            content: args.content.clone(),
        },
    ))
}

/// Rebuilds the envelope from `(args, post_state)`.
///
/// # Errors
///
/// Returns [`ReconstructError::TypeMismatch`] when `args.clip` is not a
/// valid `UUIDv7`, or [`ReconstructError::PostStateMissing`] when the
/// post-state does not contain the target clip or clip text.
pub fn data_envelope_from_post_state(
    args: &TextEditArgs,
    post_state: &Project,
) -> Result<TextEditData, ReconstructError> {
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
                let content = clip
                    .text
                    .as_ref()
                    .map(|text| text.content.clone())
                    .ok_or_else(|| ReconstructError::PostStateMissing {
                        detail: format!("text.edit: clip {clip_id} has no text element"),
                    })?;
                return Ok(TextEditData { clip_id, content });
            }
        }
    }

    Err(ReconstructError::PostStateMissing {
        detail: format!("text.edit: clip {clip_id} not found in post_state"),
    })
}

/// `text.edit` verb registration entry.
#[derive(Debug, Default)]
pub struct TextEditVerb;

impl From<TextEditError> for VerbError {
    fn from(value: TextEditError) -> Self {
        match value {
            TextEditError::BadSelector { .. }
            | TextEditError::ClipNotFound { .. }
            | TextEditError::ClipKindMismatch { .. }
            | TextEditError::Locked { .. }
            | TextEditError::SchemaViolation { .. } => VerbError::BadArgs {
                detail: value.to_string(),
            },
        }
    }
}

impl Verb for TextEditVerb {
    fn verb(&self) -> &'static str {
        "text.edit"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: TextEditArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("text.edit: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, _data) = compute_patch(prior, &typed)?;

        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!("text.edit: patch construction failed: {err}"))
            })?;

        let post_state = prior
            .apply(&patch)
            .map_err(|err| VerbError::InvariantViolation {
                detail: format!("text.edit: post-state validation failed: {err}"),
            })?;

        let envelope = data_envelope_from_post_state(&typed, &post_state).map_err(|err| {
            VerbError::Custom(format!(
                "text.edit: data envelope reconstruction failed: {err}"
            ))
        })?;

        Ok((
            patch,
            serde_json::to_value(&envelope).map_err(|err| {
                VerbError::Custom(format!(
                    "text.edit: data envelope serialization failed: {err}"
                ))
            })?,
            warnings,
        ))
    }

    fn reconstruct(
        &self,
        args: &Value,
        _patch: &Value,
        _warnings: &[Value],
        post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let typed: TextEditArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "TextEditArgs",
            })?;

        let envelope = data_envelope_from_post_state(&typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
