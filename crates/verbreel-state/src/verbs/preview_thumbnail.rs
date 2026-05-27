//! `preview.thumbnail` (§14.3) — v1 thumbnail/cache floor.
//!
//! ## Spec quote (`spec/commands/preview.md` §14.3, abbreviated)
//!
//! > CLI: `verbreel preview thumbnail [--project <id>] --target <selector> --count <int> ...`
//! > MCP: `preview.thumbnail`
//! > Args: `project_id`, `target`, `count`, optional `out_dir`, `width_px`.
//! > Returns (`data`): `{ paths, sha256s }`.
//! > Errors: `E_BAD_SELECTOR`, `E_NOT_FOUND`, `E_NO_MATCH`,
//! > `E_SELECTOR_KIND_MISMATCH`, `E_CLIP_KIND_MISMATCH`,
//! > `E_ASSET_UNSUPPORTED_KIND`, `E_BAD_RANGE`, `E_PATH_ESCAPE`, `E_IO`.
//!
//! ## v1 floor
//!
//! This pure verb slice validates only local argument shape/range
//! pinned by the spec (`target` must be qualified `asset:` or `clip:`,
//! `count ∈ [1, 1000]`, optional `width_px ∈ [1, 8192]`) and then
//! always returns `E_IO` because thumbnail rendering/cache/file-writing
//! runtime is intentionally unavailable at this layer.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use verbreel_types::ProjectId;

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

const COUNT_ALLOWED: &str = "[1, 1000]";
const WIDTH_ALLOWED: &str = "[1, 8192]";
const TARGET_HINT: &str = "target must be qualified `asset:<...>` or `clip:<...>`";

/// Arguments for `preview.thumbnail`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewThumbnailArgs {
    /// Target project id.
    pub project_id: ProjectId,
    /// Qualified multi-noun selector.
    pub target: String,
    /// Requested thumbnail count.
    ///
    /// Signed by design so out-of-range values map to `E_BAD_RANGE`
    /// (`VerbError::BadArgs`) instead of serde-level type errors.
    pub count: i64,
    /// Optional caller-supplied output directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out_dir: Option<String>,
    /// Optional output width override.
    ///
    /// Signed by design so out-of-range values map to `E_BAD_RANGE`
    /// (`VerbError::BadArgs`) instead of serde-level type errors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width_px: Option<i64>,
}

/// Envelope `data` for a future successful `preview.thumbnail`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviewThumbnailData {
    /// Resolved thumbnail output paths.
    pub paths: Vec<String>,
    /// Hex SHA-256 values for each emitted thumbnail.
    pub sha256s: Vec<String>,
}

/// Verb-level failures for `preview.thumbnail`.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PreviewThumbnailError {
    /// `target` is empty, bare, malformed, or has an unsupported prefix.
    #[error("preview.thumbnail: E_BAD_SELECTOR — {detail}; hint: {hint}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
        /// Recovery hint.
        hint: String,
    },
    /// Selector has a known but unsupported noun prefix.
    #[error(
        "preview.thumbnail: E_SELECTOR_KIND_MISMATCH — selector prefix `{actual_prefix}` is not thumbnail-compatible"
    )]
    SelectorKindMismatch {
        /// Offending prefix token.
        actual_prefix: String,
    },
    /// `count` or `width_px` is outside its allowed range.
    #[error("preview.thumbnail: E_BAD_RANGE — {field} {value} out of range {allowed}")]
    BadRange {
        /// Field name.
        field: String,
        /// Offending value.
        value: i64,
        /// Allowed range string.
        allowed: String,
    },
    /// Runtime thumbnail/cache/file-writing unavailable in v1 floor.
    #[error("preview.thumbnail: E_IO — {detail}")]
    Io {
        /// Runtime detail for operators.
        detail: String,
    },
    /// Reserved for selector resolution against project graph.
    #[error("preview.thumbnail: E_NOT_FOUND — target `{target}` not found")]
    NotFound {
        /// Resolved target the caller supplied.
        target: String,
    },
    /// Reserved for selector forms that match no entity.
    #[error("preview.thumbnail: E_NO_MATCH — selector `{selector}` matched no target")]
    NoMatch {
        /// Original selector string.
        selector: String,
    },
    /// Reserved for clip selectors resolving to non-video/image clips.
    #[error(
        "preview.thumbnail: E_CLIP_KIND_MISMATCH — clip `{target}` is not thumbnail-compatible (actual kind `{actual_kind}`)"
    )]
    ClipKindMismatch {
        /// Target clip selector/id.
        target: String,
        /// Actual clip kind.
        actual_kind: String,
    },
    /// Reserved for asset selectors resolving to non-video/image assets.
    #[error(
        "preview.thumbnail: E_ASSET_UNSUPPORTED_KIND — asset `{target}` kind `{actual_kind}` is unsupported for thumbnails"
    )]
    AssetUnsupportedKind {
        /// Target asset selector/id.
        target: String,
        /// Actual asset kind.
        actual_kind: String,
    },
    /// Reserved for §0.11 path-safety checks in a follow-up slice.
    #[error("preview.thumbnail: E_PATH_ESCAPE — path `{path}` escapes project root")]
    PathEscape {
        /// Offending path.
        path: String,
    },
}

fn validate_target_shape(target: &str) -> Result<(), PreviewThumbnailError> {
    if target.is_empty() {
        return Err(PreviewThumbnailError::BadSelector {
            detail: "selector is empty".to_string(),
            hint: TARGET_HINT.to_string(),
        });
    }

    let (prefix, _body) =
        target
            .split_once(':')
            .ok_or_else(|| PreviewThumbnailError::BadSelector {
                detail: format!("selector `{target}` is unqualified"),
                hint: TARGET_HINT.to_string(),
            })?;

    match prefix {
        "asset" | "clip" => Ok(()),
        "track" | "effect" | "keyframe" | "marker" => {
            Err(PreviewThumbnailError::SelectorKindMismatch {
                actual_prefix: prefix.to_string(),
            })
        }
        _ => Err(PreviewThumbnailError::BadSelector {
            detail: format!("unknown selector prefix `{prefix}`"),
            hint: TARGET_HINT.to_string(),
        }),
    }
}

/// Build the RFC 6902 patch for `preview.thumbnail`.
///
/// v1 floor: validates local selector/range constraints and then
/// always returns [`PreviewThumbnailError::Io`].
///
/// # Errors
///
/// Returns [`PreviewThumbnailError::BadSelector`] when `target` is
/// empty, unqualified, or has an unsupported prefix.
/// Returns [`PreviewThumbnailError::SelectorKindMismatch`] for known
/// but unsupported selector prefixes (e.g. `track:`).
/// Returns [`PreviewThumbnailError::BadRange`] when `count` or
/// `width_px` is outside its allowed bounds.
/// Returns [`PreviewThumbnailError::Io`] for every otherwise
/// well-formed request.
pub fn compute_patch(
    _prior: &Project,
    args: &PreviewThumbnailArgs,
) -> Result<(Value, Vec<Value>, Value), PreviewThumbnailError> {
    validate_target_shape(&args.target)?;

    if !(1..=1000).contains(&args.count) {
        return Err(PreviewThumbnailError::BadRange {
            field: "count".to_string(),
            value: args.count,
            allowed: COUNT_ALLOWED.to_string(),
        });
    }

    if let Some(width_px) = args.width_px
        && !(1..=8192).contains(&width_px)
    {
        return Err(PreviewThumbnailError::BadRange {
            field: "width_px".to_string(),
            value: width_px,
            allowed: WIDTH_ALLOWED.to_string(),
        });
    }

    let out_dir = args.out_dir.as_deref().unwrap_or("<cache/thumbnails>");
    Err(PreviewThumbnailError::Io {
        detail: format!(
            "preview thumbnail renderer/cache unavailable in v1 floor (target `{}`, count {}, out_dir `{out_dir}`, width_px {:?})",
            args.target, args.count, args.width_px
        ),
    })
}

impl From<PreviewThumbnailError> for VerbError {
    fn from(value: PreviewThumbnailError) -> Self {
        match value {
            PreviewThumbnailError::BadSelector { .. }
            | PreviewThumbnailError::SelectorKindMismatch { .. }
            | PreviewThumbnailError::BadRange { .. } => VerbError::BadArgs {
                detail: value.to_string(),
            },
            PreviewThumbnailError::Io { .. }
            | PreviewThumbnailError::NotFound { .. }
            | PreviewThumbnailError::NoMatch { .. }
            | PreviewThumbnailError::ClipKindMismatch { .. }
            | PreviewThumbnailError::AssetUnsupportedKind { .. }
            | PreviewThumbnailError::PathEscape { .. } => VerbError::Custom(value.to_string()),
        }
    }
}

/// The §0.8 verb for `preview.thumbnail`.
#[derive(Debug, Default)]
pub struct PreviewThumbnailVerb;

impl Verb for PreviewThumbnailVerb {
    fn verb(&self) -> &'static str {
        "preview.thumbnail"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: PreviewThumbnailArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("preview.thumbnail: args deserialize failed: {err}"),
            })?;

        match compute_patch(prior, &typed) {
            Ok((patch_value, warnings, data)) => {
                let patch: json_patch::Patch =
                    serde_json::from_value(patch_value).map_err(|err| {
                        VerbError::Custom(format!(
                            "preview.thumbnail: patch construction failed: {err}"
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
        let _typed: PreviewThumbnailArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "PreviewThumbnailArgs",
            })?;

        Ok(Value::Null)
    }
}
