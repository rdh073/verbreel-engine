//! `template.from_project` (§16.4) — v1 file-writer unavailable floor.
//!
//! ## v1 floor
//!
//! Real `template.from_project` exports a source project (or sub-range)
//! into a portable `.verbreel-template` file, rewrites caller-marked
//! clips/text clips as slots, embeds assets, and writes sidecar bytes.
//! Those operations require filesystem/runtime context outside pure
//! [`Verb::compute_patch`]. This v1 state-layer floor validates only
//! argument shape and returns `E_IO` for every well-formed request.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use verbreel_types::ProjectId;

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// Slot marker for media clips in `template.from_project`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateSlotClipArg {
    /// Source project clip id (opaque v1 string).
    pub clip_id: String,
    /// Slot id to assign in exported template metadata.
    pub slot_id: String,
    /// Human-readable slot name.
    pub slot_name: String,
}

/// Slot marker for text clips in `template.from_project`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateSlotTextArg {
    /// Source project clip id (opaque v1 string).
    pub clip_id: String,
    /// Slot id to assign in exported template metadata.
    pub slot_id: String,
    /// Human-readable slot name.
    pub slot_name: String,
}

/// Arguments for `template.from_project`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateFromProjectArgs {
    /// Source project id.
    pub project_id: ProjectId,
    /// Output template file path (opaque v1 string).
    pub out_path: String,
    /// Template display name.
    pub name: String,
    /// Optional template description (defaults to empty string).
    #[serde(default)]
    pub description: String,
    /// Optional author attribution (defaults to empty string).
    #[serde(default)]
    pub author: String,
    /// Media slot markers.
    #[serde(default)]
    pub slot_clips: Vec<TemplateSlotClipArg>,
    /// Text slot markers.
    #[serde(default)]
    pub slot_texts: Vec<TemplateSlotTextArg>,
    /// Whether to include slot default values in the exported template.
    #[serde(default)]
    pub include_slot_defaults: bool,
    /// Optional source-range start tick.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_tk: Option<i64>,
    /// Optional source-range end tick.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_tk: Option<i64>,
    /// Optional preview PNG path for thumbnail embedding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview_png: Option<String>,
    /// Opaque template tags.
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Future success envelope for `template.from_project`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateFromProjectData {
    /// Freshly minted template id.
    pub template_id: String,
    /// Resolved output path.
    pub out_path: String,
    /// Total slot count (media + text).
    pub slot_count: u64,
    /// Total embedded asset records.
    pub embedded_asset_count: u64,
    /// Total embedded asset bytes (pre-base64).
    pub embedded_asset_bytes: u64,
    /// Written `.verbreel-template` file size in bytes.
    pub bytes_written: u64,
}

/// Verb-level failures for `template.from_project`.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TemplateFromProjectError {
    /// Reserved for path-safety escapes per §0.11.
    #[error("template.from_project: E_PATH_ESCAPE — {detail}")]
    PathEscape {
        /// Human-readable path-safety detail.
        detail: String,
    },
    /// Reserved for pre-existing output path failures.
    #[error("template.from_project: E_OUT_PATH_EXISTS — out_path `{out_path}` already exists")]
    OutPathExists {
        /// Existing output path.
        out_path: String,
    },
    /// Reserved for template-shape and slot-binding validation failures.
    #[error("template.from_project: E_TEMPLATE_SCHEMA_VIOLATION — {detail}")]
    TemplateSchemaViolation {
        /// Human-readable schema/slot validation detail.
        detail: String,
    },
    /// Reserved for source-project lookup misses (for example slot clip ids).
    #[error("template.from_project: E_NOT_FOUND — {detail}")]
    NotFound {
        /// Human-readable missing-entity detail.
        detail: String,
    },
    /// Reserved for source-range validation failures.
    #[error("template.from_project: E_BAD_TIME — {detail}")]
    BadTime {
        /// Human-readable bad-time detail.
        detail: String,
    },
    /// Runtime writer/runtime context is unavailable in this pure v1 floor.
    #[error("template.from_project: E_IO — {detail}")]
    Io {
        /// Human-readable runtime detail.
        detail: String,
    },
}

/// Build the RFC 6902 patch for `template.from_project`.
///
/// v1 floor: always returns [`TemplateFromProjectError::Io`].
///
/// # Errors
///
/// Always errors with [`TemplateFromProjectError::Io`] in v1 because
/// template-file writing is intentionally deferred.
pub fn compute_patch(
    _prior: &Project,
    args: &TemplateFromProjectArgs,
) -> Result<(Value, Vec<Value>, TemplateFromProjectData), TemplateFromProjectError> {
    Err(TemplateFromProjectError::Io {
        detail: format!(
            "template file writer unavailable in the v1 floor (out_path `{}`)",
            args.out_path
        ),
    })
}

impl From<TemplateFromProjectError> for VerbError {
    fn from(value: TemplateFromProjectError) -> Self {
        match value {
            TemplateFromProjectError::PathEscape { .. }
            | TemplateFromProjectError::OutPathExists { .. }
            | TemplateFromProjectError::TemplateSchemaViolation { .. }
            | TemplateFromProjectError::NotFound { .. }
            | TemplateFromProjectError::BadTime { .. }
            | TemplateFromProjectError::Io { .. } => VerbError::Custom(value.to_string()),
        }
    }
}

/// The §0.8 verb for `template.from_project`.
#[derive(Debug, Default)]
pub struct TemplateFromProjectVerb;

impl Verb for TemplateFromProjectVerb {
    fn verb(&self) -> &'static str {
        "template.from_project"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: TemplateFromProjectArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("template.from_project: args deserialize failed: {err}"),
            })?;

        match compute_patch(prior, &typed) {
            Ok((patch_value, warnings, data)) => {
                let patch: json_patch::Patch =
                    serde_json::from_value(patch_value).map_err(|err| {
                        VerbError::Custom(format!(
                            "template.from_project: patch construction failed: {err}"
                        ))
                    })?;
                Ok((
                    patch,
                    serde_json::to_value(data).map_err(|err| {
                        VerbError::Custom(format!(
                            "template.from_project: data serialization failed: {err}"
                        ))
                    })?,
                    warnings,
                ))
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
        let _typed: TemplateFromProjectArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "TemplateFromProjectArgs",
            })?;

        Ok(Value::Null)
    }
}
