//! `template.describe` (§16.2) — v1 template not-found floor.
//!
//! ## Spec quote (`spec/commands/template.md` §16.2, abbreviated)
//!
//! > CLI: `verbreel template describe --template_id <id>`
//! > MCP: `template.describe`
//! > Args: `template_id: string`.
//! > Returns (`data`): full template descriptor including slot
//! > constraints/defaults and embedded asset count.
//! > Errors: `E_TEMPLATE_NOT_FOUND`.
//!
//! ## v1 floor
//!
//! This pure verb slice validates only local argument shape, then always
//! returns `E_TEMPLATE_NOT_FOUND` because template catalog/runtime lookup
//! is intentionally unavailable at this layer.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use verbreel_types::ProjectId;

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use crate::verbs::template_list::{
    TemplateCanvasHint, TemplateFpsHint, TemplateSlotKind, TemplateSource,
};

/// Arguments for `template.describe`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateDescribeArgs {
    /// Target project id required by the `Verb` trait shape.
    pub project_id: ProjectId,
    /// Opaque template id to describe.
    pub template_id: String,
}

/// Full slot constraints for `template.describe`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateSlotConstraints {
    /// Optional minimum duration (ticks) for media slots.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_duration_tk: Option<i64>,
    /// Optional maximum duration (ticks) for media slots.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_duration_tk: Option<i64>,
    /// Optional aspect ratio hint for image/video slots.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aspect_ratio_hint: Option<String>,
    /// Optional hard max chars for text slots.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_chars: Option<i64>,
}

/// Full slot descriptor returned by `template.describe`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateSlotDescriptor {
    /// Slot identifier.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Human-readable slot description.
    pub description: String,
    /// Slot kind.
    pub kind: TemplateSlotKind,
    /// Whether this slot is required.
    pub required: bool,
    /// Optional default value for optional slots.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_value: Option<String>,
    /// Optional full slot constraints.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub constraints: Option<TemplateSlotConstraints>,
}

/// Future success envelope for `template.describe`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateDescribeData {
    /// Template id.
    pub id: String,
    /// Template name.
    pub name: String,
    /// Template description.
    pub description: String,
    /// Template author attribution.
    pub author: String,
    /// Absolute path to preview thumbnail.
    pub preview_thumbnail_path: String,
    /// Template source.
    pub source: TemplateSource,
    /// Absolute install path.
    pub install_path: String,
    /// Template file schema version.
    pub template_schema_version: String,
    /// Embedded project graph schema version.
    pub project_graph_schema_version: String,
    /// Duration hint in ticks.
    pub duration_hint_tk: i64,
    /// Canvas hint.
    pub canvas_hint: TemplateCanvasHint,
    /// FPS hint.
    pub fps_hint: TemplateFpsHint,
    /// Opaque template tags.
    pub tags: Vec<String>,
    /// Full slot descriptors.
    pub slots: Vec<TemplateSlotDescriptor>,
    /// Count of embedded template assets.
    pub embedded_asset_count: u64,
    /// Creation time (RFC 3339).
    pub created_at: String,
    /// Engine version hint.
    pub engine_version_hint: String,
}

/// Verb-level failures for `template.describe`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TemplateDescribeError {
    /// Runtime template-catalog miss for a well-formed request.
    #[error(
        "template.describe: E_TEMPLATE_NOT_FOUND — template_id `{template_id}` does not resolve \
         to an installed template"
    )]
    TemplateNotFound {
        /// Template id supplied by caller.
        template_id: String,
    },
}

/// Build the RFC 6902 patch for `template.describe`.
///
/// v1 floor: always returns [`TemplateDescribeError::TemplateNotFound`]
/// for well-formed arguments.
///
/// # Errors
///
/// Returns [`TemplateDescribeError::TemplateNotFound`] for every
/// well-formed request.
pub fn compute_patch(
    _prior: &Project,
    args: &TemplateDescribeArgs,
) -> Result<(Value, Vec<Value>, Value), TemplateDescribeError> {
    Err(TemplateDescribeError::TemplateNotFound {
        template_id: args.template_id.clone(),
    })
}

impl From<TemplateDescribeError> for VerbError {
    fn from(value: TemplateDescribeError) -> Self {
        match value {
            TemplateDescribeError::TemplateNotFound { .. } => VerbError::Custom(value.to_string()),
        }
    }
}

/// The §0.8 verb for `template.describe`.
#[derive(Debug, Default)]
pub struct TemplateDescribeVerb;

impl Verb for TemplateDescribeVerb {
    fn verb(&self) -> &'static str {
        "template.describe"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: TemplateDescribeArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("template.describe: args deserialize failed: {err}"),
            })?;

        match compute_patch(prior, &typed) {
            Ok((patch_value, warnings, data)) => {
                let patch: json_patch::Patch =
                    serde_json::from_value(patch_value).map_err(|err| {
                        VerbError::Custom(format!(
                            "template.describe: patch construction failed: {err}"
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
        let _typed: TemplateDescribeArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "TemplateDescribeArgs",
            })?;

        Ok(Value::Null)
    }
}
