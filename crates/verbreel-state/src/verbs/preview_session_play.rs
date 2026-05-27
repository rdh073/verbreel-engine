//! `preview.session.play` (§15.3) — v1 session not-found floor.
//!
//! ## Spec quote (`spec/commands/preview-session.md` §15.3, abbreviated)
//!
//! > CLI: `verbreel preview session play --session_id <id>`
//! > MCP: `preview.session.play`
//! > Args: `project_id: string`, `session_id: string`.
//! > Returns (`data`): `{ stream_started, channel_kind }`.
//! > Errors: `E_PREVIEW_SESSION_NOT_FOUND`, `E_PROJECT_NOT_FOUND`,
//! > `E_PREVIEW_SESSION_DECODER_FAILED`.
//!
//! ## v1 floor
//!
//! This pure verb slice validates only local argument shape, then always
//! returns `E_PREVIEW_SESSION_NOT_FOUND` because session-manager/runtime
//! lookup is intentionally unavailable at this layer.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use verbreel_types::ProjectId;

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use crate::verbs::preview_session_create::PreviewSessionChannelKind;

/// Arguments for `preview.session.play`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewSessionPlayArgs {
    /// Target project id.
    pub project_id: ProjectId,
    /// The preview session to start.
    pub session_id: String,
}

/// Response envelope for a future successful `preview.session.play`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviewSessionPlayData {
    /// Whether the stream channel was started.
    pub stream_started: bool,
    /// Effective stream channel kind.
    pub channel_kind: PreviewSessionChannelKind,
}

/// Verb-level failures for `preview.session.play`.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PreviewSessionPlayError {
    /// Runtime session-state miss for a well-formed request.
    #[error(
        "preview.session.play: E_PREVIEW_SESSION_NOT_FOUND — session_id `{session_id}` does not \
         resolve to an active preview session for this project"
    )]
    SessionNotFound {
        /// Session id supplied by caller.
        session_id: String,
    },
    /// Reserved for runtime project resolution in follow-up slices.
    #[error("preview.session.play: E_PROJECT_NOT_FOUND — project_id `{project_id}` not found")]
    ProjectNotFound {
        /// Missing project id.
        project_id: String,
    },
    /// Reserved for warm-decoder playback failure in follow-up slices.
    #[error(
        "preview.session.play: E_PREVIEW_SESSION_DECODER_FAILED — session_id `{session_id}` \
         decoder failed: {decoder_error}"
    )]
    DecoderFailed {
        /// Session id that hit decoder failure.
        session_id: String,
        /// Runtime decoder error detail.
        decoder_error: String,
    },
}

/// Build the RFC 6902 patch for `preview.session.play`.
///
/// v1 floor: always returns [`PreviewSessionPlayError::SessionNotFound`]
/// for well-formed arguments.
///
/// # Errors
///
/// Returns [`PreviewSessionPlayError::SessionNotFound`] for every
/// well-formed request.
pub fn compute_patch(
    _prior: &Project,
    args: &PreviewSessionPlayArgs,
) -> Result<(Value, Vec<Value>, Value), PreviewSessionPlayError> {
    Err(PreviewSessionPlayError::SessionNotFound {
        session_id: args.session_id.clone(),
    })
}

impl From<PreviewSessionPlayError> for VerbError {
    fn from(value: PreviewSessionPlayError) -> Self {
        match value {
            PreviewSessionPlayError::SessionNotFound { .. }
            | PreviewSessionPlayError::ProjectNotFound { .. }
            | PreviewSessionPlayError::DecoderFailed { .. } => VerbError::Custom(value.to_string()),
        }
    }
}

/// The §0.8 verb for `preview.session.play`.
#[derive(Debug, Default)]
pub struct PreviewSessionPlayVerb;

impl Verb for PreviewSessionPlayVerb {
    fn verb(&self) -> &'static str {
        "preview.session.play"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: PreviewSessionPlayArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("preview.session.play: args deserialize failed: {err}"),
            })?;

        match compute_patch(prior, &typed) {
            Ok((patch_value, warnings, data)) => {
                let patch: json_patch::Patch =
                    serde_json::from_value(patch_value).map_err(|err| {
                        VerbError::Custom(format!(
                            "preview.session.play: patch construction failed: {err}"
                        ))
                    })?;
                Ok((patch, data, warnings))
            }
            Err(err) => Err(err.into()),
        }
    }

    fn reconstruct(
        &self,
        args: &Value,
        _patch: &Value,
        _warnings: &[Value],
        _post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let _typed: PreviewSessionPlayArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "PreviewSessionPlayArgs",
            })?;

        Ok(Value::Null)
    }
}
