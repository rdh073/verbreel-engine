//! `preview.frame` (§14.1) — v1 renderer/cache floor.
//!
//! ## Spec quote (`spec/commands/preview.md` §14.1, abbreviated)
//!
//! > CLI: `verbreel preview frame [--project <id>] --at_tk <int> ...`
//! > MCP: `preview.frame`
//! > Args: `project_id`, `at_tk`, optional `out_path`, `width_px`,
//! > `deterministic`.
//! > Returns (`data`): `{ path, sha256, width, height, cache_hit }`.
//! > Errors: `E_IO`, `E_BAD_TIME`, `E_BAD_RANGE`, `E_PATH_ESCAPE`.
//!
//! ## v1 floor
//!
//! This pure verb slice validates only local argument bounds pinned by
//! the spec (`at_tk >= 0`, `width_px ∈ [1, 8192]`) and then always
//! returns `E_IO` because renderer/cache/file-writing runtime is
//! intentionally unavailable at this layer.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use verbreel_types::ProjectId;

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// Arguments for `preview.frame`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewFrameArgs {
    /// Target project id.
    pub project_id: ProjectId,
    /// Requested timeline tick to preview.
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
    /// Determinism toggle (defaults false when omitted).
    #[serde(default)]
    pub deterministic: bool,
}

/// Envelope `data` for a future successful `preview.frame`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviewFrameData {
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

/// Verb-level failures for `preview.frame`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PreviewFrameError {
    /// `at_tk` must be greater than or equal to 0.
    #[error("preview.frame: E_BAD_TIME — at_tk {at_tk} must be >= 0")]
    BadTime {
        /// Offending `at_tk`.
        at_tk: i64,
    },
    /// `width_px` must be within `[1, 8192]`.
    #[error("preview.frame: E_BAD_RANGE — {field} {value} out of range {allowed}")]
    BadRange {
        /// Field name.
        field: String,
        /// Offending value.
        value: i64,
        /// Allowed range string.
        allowed: String,
    },
    /// Renderer/cache runtime unavailable in v1 floor.
    #[error("preview.frame: E_IO — {detail}")]
    Io {
        /// Runtime detail for operators.
        detail: String,
    },
    /// Reserved for §0.11 path-safety checks in a follow-up slice.
    #[error("preview.frame: E_PATH_ESCAPE — path `{path}` escapes project root")]
    PathEscape {
        /// Offending path.
        path: String,
    },
}

/// Build the RFC 6902 patch for `preview.frame`.
///
/// v1 floor: validates local argument bounds and then always returns
/// [`PreviewFrameError::Io`].
///
/// # Errors
///
/// Returns [`PreviewFrameError::BadTime`] when `at_tk < 0`.
/// Returns [`PreviewFrameError::BadRange`] when `width_px` is outside
/// `[1, 8192]`.
/// Returns [`PreviewFrameError::Io`] for every otherwise well-formed
/// request.
pub fn compute_patch(
    _prior: &Project,
    args: &PreviewFrameArgs,
) -> Result<(Value, Vec<Value>, Value), PreviewFrameError> {
    if args.at_tk < 0 {
        return Err(PreviewFrameError::BadTime { at_tk: args.at_tk });
    }

    if let Some(width_px) = args.width_px
        && !(1..=8192).contains(&width_px)
    {
        return Err(PreviewFrameError::BadRange {
            field: "width_px".to_string(),
            value: width_px,
            allowed: "[1, 8192]".to_string(),
        });
    }

    let out_path = args.out_path.as_deref().unwrap_or("<cache/frames>");
    Err(PreviewFrameError::Io {
        detail: format!(
            "preview renderer/cache unavailable in v1 floor (at_tk {}, out_path `{out_path}`, width_px {:?}, deterministic {})",
            args.at_tk, args.width_px, args.deterministic
        ),
    })
}

impl From<PreviewFrameError> for VerbError {
    fn from(value: PreviewFrameError) -> Self {
        match value {
            PreviewFrameError::BadTime { .. } | PreviewFrameError::BadRange { .. } => {
                VerbError::BadArgs {
                    detail: value.to_string(),
                }
            }
            PreviewFrameError::Io { .. } | PreviewFrameError::PathEscape { .. } => {
                VerbError::Custom(value.to_string())
            }
        }
    }
}

/// The §0.8 verb for `preview.frame`.
#[derive(Debug, Default)]
pub struct PreviewFrameVerb;

impl Verb for PreviewFrameVerb {
    fn verb(&self) -> &'static str {
        "preview.frame"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: PreviewFrameArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("preview.frame: args deserialize failed: {err}"),
            })?;

        match compute_patch(prior, &typed) {
            Ok((patch_value, warnings, data)) => {
                let patch: json_patch::Patch =
                    serde_json::from_value(patch_value).map_err(|err| {
                        VerbError::Custom(format!(
                            "preview.frame: patch construction failed: {err}"
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
        let _typed: PreviewFrameArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "PreviewFrameArgs",
            })?;

        Ok(Value::Null)
    }
}
