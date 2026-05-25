//! `keyframe.list` (§8.4) — twenty-ninth production verb in the engine.
//!
//! ## Spec quote (`spec/commands/keyframe.md` §8.4, verbatim)
//!
//! > Ordering: sorted by (`property`, `time_tk`) ascending.
//! > CLI: `verbreel keyframe list [--project <id>] --clip <id> [--property <path>]`
//! > MCP: `keyframe.list`
//! > Args: `project_id: string`, `clip: string`, `property?: string`
//! > Returns (`data`): `{ keyframes: Keyframe[] }`
//! > Errors: `E_NOT_FOUND`, `E_NO_MATCH`, `E_BAD_SELECTOR`,
//! > `E_SELECTOR_KIND_MISMATCH`
//!
//! ## Read-only verb
//!
//! `keyframe.list` returns one clip’s `keyframes[]`, optionally filtered
//! by `property`, in a deterministic order.

use crate::keyframe::Keyframe;
use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::{ClipId, ProjectId};

/// Arguments for `keyframe.list`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyframeListArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Target clip id as bare `UUIDv7`.
    pub clip: String,

    /// Optional property selector. Returns only keyframes whose `property`
    /// exactly matches this dotted path when provided.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub property: Option<String>,
}

/// Envelope returned by `keyframe.list`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeyframeListData {
    /// Sorted, and optionally filtered, keyframes.
    pub keyframes: Vec<Keyframe>,
}

/// Verb-level validation failures for `keyframe.list`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum KeyframeListError {
    /// `args.clip` is not parseable as a `ClipId`.
    #[error("keyframe.list: `clip` selector parse failed: {detail}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
    },

    /// No clip exists for `args.clip`.
    #[error("keyframe.list: clip `{clip_id}` not found")]
    ClipNotFound {
        /// Missing clip id string.
        clip_id: String,
    },
}

/// Build the RFC-6902 patch for `keyframe.list`.
///
/// # Errors
/// Returns [`KeyframeListError`] for selector parse failures or missing clips.
pub fn compute_patch(
    prior: &Project,
    args: &KeyframeListArgs,
) -> Result<(Value, Vec<Value>, KeyframeListData), KeyframeListError> {
    let clip_id = args
        .clip
        .parse::<ClipId>()
        .map_err(|err| KeyframeListError::BadSelector {
            detail: err.to_string(),
        })?;

    let clip = prior
        .tracks
        .iter()
        .flat_map(|t| t.clips.iter())
        .find(|c| c.id == clip_id)
        .ok_or_else(|| KeyframeListError::ClipNotFound {
            clip_id: args.clip.clone(),
        })?;

    let mut keyframes: Vec<Keyframe> = clip
        .keyframes
        .iter()
        .filter(|kf| {
            args.property
                .as_deref()
                .is_none_or(|p| kf.property.as_str() == p)
        })
        .cloned()
        .collect();

    keyframes.sort_by(|a, b| {
        a.property
            .as_str()
            .cmp(b.property.as_str())
            .then_with(|| a.time_tk.cmp(&b.time_tk))
    });

    Ok((json!([]), Vec::new(), KeyframeListData { keyframes }))
}

/// Build the data envelope from `(args, post_state)`.
///
/// # Errors
/// Returns a [`ReconstructError`] for type mismatches or missing clip in post state.
pub fn data_envelope_from_post_state(
    args: &KeyframeListArgs,
    post_state: &Project,
) -> Result<KeyframeListData, ReconstructError> {
    let clip_id = args
        .clip
        .parse::<ClipId>()
        .map_err(|_| ReconstructError::TypeMismatch {
            name: "args.clip",
            expected: "UUIDv7 ClipId string",
        })?;

    let clip = post_state
        .tracks
        .iter()
        .flat_map(|t| t.clips.iter())
        .find(|c| c.id == clip_id)
        .ok_or_else(|| ReconstructError::PostStateMissing {
            detail: format!("keyframe.list: clip {clip_id} not found in post_state"),
        })?;

    let mut keyframes: Vec<Keyframe> = clip
        .keyframes
        .iter()
        .filter(|kf| {
            args.property
                .as_deref()
                .is_none_or(|p| kf.property.as_str() == p)
        })
        .cloned()
        .collect();

    keyframes.sort_by(|a, b| {
        a.property
            .as_str()
            .cmp(b.property.as_str())
            .then_with(|| a.time_tk.cmp(&b.time_tk))
    });

    Ok(KeyframeListData { keyframes })
}

impl From<KeyframeListError> for VerbError {
    fn from(value: KeyframeListError) -> Self {
        match value {
            KeyframeListError::BadSelector { .. } | KeyframeListError::ClipNotFound { .. } => {
                VerbError::BadArgs {
                    detail: value.to_string(),
                }
            }
        }
    }
}

/// The §0.8 verb for `keyframe.list`.
#[derive(Debug, Default)]
pub struct KeyframeListVerb;

impl Verb for KeyframeListVerb {
    fn verb(&self) -> &'static str {
        "keyframe.list"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: KeyframeListArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("keyframe.list: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, data) = compute_patch(prior, &typed)?;

        let patch: json_patch::Patch = serde_json::from_value(patch_value).map_err(|err| {
            VerbError::Custom(format!("keyframe.list: patch construction failed: {err}"))
        })?;

        let data = serde_json::to_value(&data).map_err(|err| {
            VerbError::Custom(format!("keyframe.list: data envelope failed: {err}"))
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
        let typed: KeyframeListArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "KeyframeListArgs",
            })?;

        let envelope = data_envelope_from_post_state(&typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|e| ReconstructError::Custom(e.to_string()))
    }
}
