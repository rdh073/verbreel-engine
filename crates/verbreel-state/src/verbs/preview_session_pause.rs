//! `preview.session.pause` (§15.4) — v1 session not-found floor.
//!
//! ## Spec quote (`spec/commands/preview-session.md` §15.4, abbreviated)
//!
//! > CLI: `verbreel preview session pause --session_id <id>`
//! > MCP: `preview.session.pause`
//! > Args: `project_id: string`, `session_id: string`.
//! > Returns (`data`): `{ was_playing, at_tk }`.
//! > Errors: `E_PREVIEW_SESSION_NOT_FOUND`, `E_PROJECT_NOT_FOUND`.
//! > Warnings: `W_NOOP`.
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

/// Arguments for `preview.session.pause`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewSessionPauseArgs {
    /// Target project id.
    pub project_id: ProjectId,
    /// The preview session to pause.
    pub session_id: String,
}

/// Response envelope for a future successful `preview.session.pause`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviewSessionPauseData {
    /// Whether the session was actively playing before pause.
    pub was_playing: bool,
    /// Playhead tick at pause resolution.
    pub at_tk: i64,
}

/// Verb-level failures for `preview.session.pause`.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PreviewSessionPauseError {
    /// Runtime session-state miss for a well-formed request.
    #[error(
        "preview.session.pause: E_PREVIEW_SESSION_NOT_FOUND — session_id `{session_id}` does not \
         resolve to an active preview session for this project"
    )]
    SessionNotFound {
        /// Session id supplied by caller.
        session_id: String,
    },
    /// Reserved for runtime project resolution in follow-up slices.
    #[error("preview.session.pause: E_PROJECT_NOT_FOUND — project_id `{project_id}` not found")]
    ProjectNotFound {
        /// Missing project id.
        project_id: String,
    },
}

/// Build the RFC 6902 patch for `preview.session.pause`.
///
/// v1 floor: always returns [`PreviewSessionPauseError::SessionNotFound`]
/// for well-formed arguments.
///
/// # Errors
///
/// Returns [`PreviewSessionPauseError::SessionNotFound`] for every
/// well-formed request.
pub fn compute_patch(
    _prior: &Project,
    args: &PreviewSessionPauseArgs,
) -> Result<(Value, Vec<Value>, Value), PreviewSessionPauseError> {
    Err(PreviewSessionPauseError::SessionNotFound {
        session_id: args.session_id.clone(),
    })
}

impl From<PreviewSessionPauseError> for VerbError {
    fn from(value: PreviewSessionPauseError) -> Self {
        match value {
            PreviewSessionPauseError::SessionNotFound { .. }
            | PreviewSessionPauseError::ProjectNotFound { .. } => {
                VerbError::Custom(value.to_string())
            }
        }
    }
}

/// The §0.8 verb for `preview.session.pause`.
#[derive(Debug, Default)]
pub struct PreviewSessionPauseVerb;

impl Verb for PreviewSessionPauseVerb {
    fn verb(&self) -> &'static str {
        "preview.session.pause"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: PreviewSessionPauseArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("preview.session.pause: args deserialize failed: {err}"),
            })?;

        match compute_patch(prior, &typed) {
            Ok((patch_value, warnings, data)) => {
                let patch: json_patch::Patch =
                    serde_json::from_value(patch_value).map_err(|err| {
                        VerbError::Custom(format!(
                            "preview.session.pause: patch construction failed: {err}"
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
        let _typed: PreviewSessionPauseArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "PreviewSessionPauseArgs",
            })?;

        Ok(Value::Null)
    }
}
