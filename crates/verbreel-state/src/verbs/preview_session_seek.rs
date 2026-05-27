//! `preview.session.seek` (§15.2) — v1 session not-found floor.
//!
//! ## Spec quote (`spec/commands/preview-session.md` §15.2, abbreviated)
//!
//! > CLI: `verbreel preview session seek --session_id <id> --at_tk <int>`
//! > MCP: `preview.session.seek`
//! > Args: `project_id: string`, `session_id: string`, `at_tk: integer`.
//! > Returns (`data`): `{ path, sha256, width, height, cache_hit, at_tk }`.
//! > Errors: `E_PREVIEW_SESSION_NOT_FOUND`, `E_PROJECT_NOT_FOUND`,
//! > `E_BAD_TIME`, `E_PREVIEW_SESSION_DECODER_FAILED`.
//!
//! ## v1 floor
//!
//! This pure verb slice validates only local `at_tk >= 0`, then always
//! returns `E_PREVIEW_SESSION_NOT_FOUND` because session-manager/runtime
//! lookup is intentionally unavailable at this layer.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use verbreel_types::ProjectId;

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// Arguments for `preview.session.seek`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewSessionSeekArgs {
    /// Target project id.
    pub project_id: ProjectId,
    /// The preview session to seek.
    pub session_id: String,
    /// Requested timeline tick to seek to.
    ///
    /// Signed by design so negative values deserialize and map to
    /// `E_BAD_TIME` (`VerbError::BadArgs`) rather than serde-level
    /// type errors.
    pub at_tk: i64,
}

/// Response envelope for a future successful `preview.session.seek`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviewSessionSeekData {
    /// Resolved output path.
    pub path: String,
    /// Hex SHA-256 of output bytes.
    pub sha256: String,
    /// Resolved output width.
    pub width: u32,
    /// Resolved output height.
    pub height: u32,
    /// Whether output came from cache.
    pub cache_hit: bool,
    /// Resolved tick (post-snap or post-clamp in future runtime slices).
    pub at_tk: i64,
}

/// Verb-level failures for `preview.session.seek`.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PreviewSessionSeekError {
    /// `at_tk` must be greater than or equal to 0.
    #[error("preview.session.seek: E_BAD_TIME — at_tk {at_tk} must be >= 0")]
    BadTime {
        /// Offending `at_tk`.
        at_tk: i64,
    },
    /// Runtime session-state miss for a well-formed request.
    #[error(
        "preview.session.seek: E_PREVIEW_SESSION_NOT_FOUND — session_id `{session_id}` does not \
         resolve to an active preview session for this project"
    )]
    SessionNotFound {
        /// Session id supplied by caller.
        session_id: String,
    },
    /// Reserved for runtime project resolution in follow-up slices.
    #[error("preview.session.seek: E_PROJECT_NOT_FOUND — project_id `{project_id}` not found")]
    ProjectNotFound {
        /// Missing project id.
        project_id: String,
    },
    /// Reserved for warm-decoder render failure in follow-up slices.
    #[error(
        "preview.session.seek: E_PREVIEW_SESSION_DECODER_FAILED — session_id `{session_id}` \
         decoder failed: {decoder_error}"
    )]
    DecoderFailed {
        /// Session id that hit decoder failure.
        session_id: String,
        /// Runtime decoder error detail.
        decoder_error: String,
    },
}

/// Build the RFC 6902 patch for `preview.session.seek`.
///
/// v1 floor: validates `at_tk >= 0` and then always returns
/// [`PreviewSessionSeekError::SessionNotFound`].
///
/// # Errors
///
/// Returns [`PreviewSessionSeekError::BadTime`] when `at_tk < 0`.
/// Returns [`PreviewSessionSeekError::SessionNotFound`] for every
/// otherwise well-formed request.
pub fn compute_patch(
    _prior: &Project,
    args: &PreviewSessionSeekArgs,
) -> Result<(Value, Vec<Value>, Value), PreviewSessionSeekError> {
    if args.at_tk < 0 {
        return Err(PreviewSessionSeekError::BadTime { at_tk: args.at_tk });
    }

    Err(PreviewSessionSeekError::SessionNotFound {
        session_id: args.session_id.clone(),
    })
}

impl From<PreviewSessionSeekError> for VerbError {
    fn from(value: PreviewSessionSeekError) -> Self {
        match value {
            PreviewSessionSeekError::BadTime { .. } => VerbError::BadArgs {
                detail: value.to_string(),
            },
            PreviewSessionSeekError::SessionNotFound { .. }
            | PreviewSessionSeekError::ProjectNotFound { .. }
            | PreviewSessionSeekError::DecoderFailed { .. } => VerbError::Custom(value.to_string()),
        }
    }
}

/// The §0.8 verb for `preview.session.seek`.
#[derive(Debug, Default)]
pub struct PreviewSessionSeekVerb;

impl Verb for PreviewSessionSeekVerb {
    fn verb(&self) -> &'static str {
        "preview.session.seek"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: PreviewSessionSeekArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("preview.session.seek: args deserialize failed: {err}"),
            })?;

        match compute_patch(prior, &typed) {
            Ok((patch_value, warnings, data)) => {
                let patch: json_patch::Patch =
                    serde_json::from_value(patch_value).map_err(|err| {
                        VerbError::Custom(format!(
                            "preview.session.seek: patch construction failed: {err}"
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
        let _typed: PreviewSessionSeekArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "PreviewSessionSeekArgs",
            })?;

        Ok(Value::Null)
    }
}
