//! `preview.session.frame_at` (§15.6) — v1 session not-found floor.
//!
//! ## Spec quote (`spec/commands/preview-session.md` §15.6, abbreviated)
//!
//! > CLI: `verbreel preview session frame_at --session_id <id> --at_tk <int> ...`
//! > MCP: `preview.session.frame_at`
//! > Args: `project_id: string`, `session_id: string`, `at_tk: integer`,
//! >   `out_path?: string`, `width_px?: integer`.
//! > Returns (`data`): `{ path, sha256, width, height, cache_hit }`.
//! > Errors: `E_PREVIEW_SESSION_NOT_FOUND`, `E_PROJECT_NOT_FOUND`,
//! >   `E_BAD_TIME`, `E_BAD_RANGE`, `E_BUSY`, `E_PREVIEW_SESSION_DECODER_FAILED`,
//! >   `E_PATH_ESCAPE`.
//!
//! ## v1 floor
//!
//! This pure verb slice validates local `at_tk >= 0` and optional
//! `width_px ∈ [1, 8192]`, then always returns
//! `E_PREVIEW_SESSION_NOT_FOUND` because session-manager/runtime lookup is
//! intentionally unavailable at this layer.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use verbreel_types::ProjectId;

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// Arguments for `preview.session.frame_at`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewSessionFrameAtArgs {
    /// Target project id.
    pub project_id: ProjectId,
    /// The preview session to query.
    pub session_id: String,
    /// Requested timeline tick.
    ///
    /// Signed by design so negative values deserialize and map to
    /// `E_BAD_TIME` (`VerbError::BadArgs`) rather than serde-level
    /// type errors.
    pub at_tk: i64,
    /// Optional caller-supplied output path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out_path: Option<String>,
    /// Optional output width override.
    ///
    /// Signed by design so range failures map to `E_BAD_RANGE`
    /// (`VerbError::BadArgs`) instead of serde-level type errors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width_px: Option<i64>,
}

/// Response envelope for a future successful `preview.session.frame_at`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviewSessionFrameAtData {
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
}

/// Verb-level failures for `preview.session.frame_at`.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PreviewSessionFrameAtError {
    /// `at_tk` must be greater than or equal to 0.
    #[error("preview.session.frame_at: E_BAD_TIME — at_tk {at_tk} must be >= 0")]
    BadTime {
        /// Offending `at_tk`.
        at_tk: i64,
    },
    /// `width_px` must be within `[1, 8192]`.
    #[error("preview.session.frame_at: E_BAD_RANGE — {field} {value} out of range {allowed}")]
    BadRange {
        /// Field name.
        field: String,
        /// Offending value.
        value: i64,
        /// Allowed range string.
        allowed: String,
    },
    /// Runtime session-state miss for a well-formed request.
    #[error(
        "preview.session.frame_at: E_PREVIEW_SESSION_NOT_FOUND — session_id `{session_id}` \
         does not resolve to an active preview session for this project"
    )]
    SessionNotFound {
        /// Session id supplied by caller.
        session_id: String,
    },
    /// Reserved for runtime project resolution in follow-up slices.
    #[error("preview.session.frame_at: E_PROJECT_NOT_FOUND — project_id `{project_id}` not found")]
    ProjectNotFound {
        /// Missing project id.
        project_id: String,
    },
    /// Reserved for runtime parallel frame-at saturation in follow-up slices.
    #[error(
        "preview.session.frame_at: E_BUSY — session_id `{session_id}` cannot accept more \
         concurrent frame_at requests: {reason}"
    )]
    Busy {
        /// Session id that is saturated.
        session_id: String,
        /// Runtime saturation reason detail.
        reason: String,
    },
    /// Reserved for warm-decoder failure in follow-up slices.
    #[error(
        "preview.session.frame_at: E_PREVIEW_SESSION_DECODER_FAILED — session_id `{session_id}` \
         decoder failed: {decoder_error}"
    )]
    DecoderFailed {
        /// Session id that hit decoder failure.
        session_id: String,
        /// Runtime decoder error detail.
        decoder_error: String,
    },
    /// Reserved for §0.11 path-safety checks in follow-up slices.
    #[error("preview.session.frame_at: E_PATH_ESCAPE — path `{path}` escapes project root")]
    PathEscape {
        /// Offending path.
        path: String,
    },
}

/// Build the RFC 6902 patch for `preview.session.frame_at`.
///
/// v1 floor: validates local argument bounds and then always returns
/// [`PreviewSessionFrameAtError::SessionNotFound`].
///
/// # Errors
///
/// Returns [`PreviewSessionFrameAtError::BadTime`] when `at_tk < 0`.
/// Returns [`PreviewSessionFrameAtError::BadRange`] when `width_px` is outside
/// `[1, 8192]`.
/// Returns [`PreviewSessionFrameAtError::SessionNotFound`] for every
/// otherwise well-formed request.
pub fn compute_patch(
    _prior: &Project,
    args: &PreviewSessionFrameAtArgs,
) -> Result<(Value, Vec<Value>, Value), PreviewSessionFrameAtError> {
    if args.at_tk < 0 {
        return Err(PreviewSessionFrameAtError::BadTime { at_tk: args.at_tk });
    }

    if let Some(width_px) = args.width_px
        && !(1..=8192).contains(&width_px)
    {
        return Err(PreviewSessionFrameAtError::BadRange {
            field: "width_px".to_string(),
            value: width_px,
            allowed: "[1, 8192]".to_string(),
        });
    }

    Err(PreviewSessionFrameAtError::SessionNotFound {
        session_id: args.session_id.clone(),
    })
}

impl From<PreviewSessionFrameAtError> for VerbError {
    fn from(value: PreviewSessionFrameAtError) -> Self {
        match value {
            PreviewSessionFrameAtError::BadTime { .. }
            | PreviewSessionFrameAtError::BadRange { .. } => VerbError::BadArgs {
                detail: value.to_string(),
            },
            PreviewSessionFrameAtError::SessionNotFound { .. }
            | PreviewSessionFrameAtError::ProjectNotFound { .. }
            | PreviewSessionFrameAtError::Busy { .. }
            | PreviewSessionFrameAtError::DecoderFailed { .. }
            | PreviewSessionFrameAtError::PathEscape { .. } => VerbError::Custom(value.to_string()),
        }
    }
}

/// The §0.8 verb for `preview.session.frame_at`.
#[derive(Debug, Default)]
pub struct PreviewSessionFrameAtVerb;

impl Verb for PreviewSessionFrameAtVerb {
    fn verb(&self) -> &'static str {
        "preview.session.frame_at"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: PreviewSessionFrameAtArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("preview.session.frame_at: args deserialize failed: {err}"),
            })?;

        match compute_patch(prior, &typed) {
            Ok((patch_value, warnings, data)) => {
                let patch: json_patch::Patch =
                    serde_json::from_value(patch_value).map_err(|err| {
                        VerbError::Custom(format!(
                            "preview.session.frame_at: patch construction failed: {err}"
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
        let _typed: PreviewSessionFrameAtArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "PreviewSessionFrameAtArgs",
            })?;

        Ok(Value::Null)
    }
}
