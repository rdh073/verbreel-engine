//! `template.list` (§16.1) — eighty-fourth production verb in the engine.
//!
//! ## v1 floor — empty template catalog.
//!
//! Per §16.1, real template listing merges bundled templates and
//! user-installed templates under `~/.verbreel/templates/`, validates
//! each install, resolves preview paths, and emits skip warnings.
//! The `Verb` trait's purity contract forbids filesystem and runtime
//! context access in `compute_patch`, so v1 returns an empty list for
//! every well-formed request while preserving the published args/data
//! surface.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::ProjectId;

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// Template source filter for `template.list`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TemplateSource {
    /// Bundled read-only template.
    Bundled,
    /// User-installed template under `~/.verbreel/templates/`.
    User,
}

/// Slot kind summary in template list rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TemplateSlotKind {
    /// Video slot.
    Video,
    /// Audio slot.
    Audio,
    /// Image slot.
    Image,
    /// Text slot.
    Text,
}

/// Arguments for `template.list`.
///
/// `project_id` is required by the `Verb` trait shape and ignored by
/// this read-only v1 floor.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateListArgs {
    /// Required by the `Verb` trait shape; not read by the v1 impl.
    pub project_id: ProjectId,
    /// Optional source filter (`bundled` or `user`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<TemplateSource>,
    /// Optional opaque tag filter (for example `"vertical"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

/// Canvas hint exposed in template list rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateCanvasHint {
    /// Canvas width in pixels.
    pub width: u32,
    /// Canvas height in pixels.
    pub height: u32,
}

/// FPS hint exposed in template list rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateFpsHint {
    /// FPS numerator.
    pub num: u32,
    /// FPS denominator.
    pub den: u32,
}

/// Compact slot summary surfaced by `template.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateSlotSummary {
    /// Slot identifier.
    pub id: String,
    /// Display label.
    pub name: String,
    /// Slot media/text kind.
    pub kind: TemplateSlotKind,
    /// Whether this slot is required.
    pub required: bool,
}

/// A single template row in `template.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateListEntry {
    /// Template id (`UUIDv7` string).
    pub id: String,
    /// Template display name.
    pub name: String,
    /// Template description.
    pub description: String,
    /// Absolute path to preview thumbnail (or empty string).
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
    /// Compact slot summaries.
    pub slots: Vec<TemplateSlotSummary>,
}

/// Envelope returned by `template.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateListData {
    /// Matched templates in deterministic order.
    pub templates: Vec<TemplateListEntry>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Error)]
/// Verb-level error type for `template.list`.
pub enum TemplateListError {
    /// Template catalog I/O failed.
    #[error("template.list: E_IO — {detail}")]
    Io {
        /// Human-readable I/O failure detail.
        detail: String,
    },
}

/// Build the canonical `template.list` data envelope.
fn build_data() -> TemplateListData {
    TemplateListData {
        templates: Vec::new(),
    }
}

/// Build the RFC 6902 patch for `template.list`.
///
/// # Errors
///
/// No runtime errors are produced by this v1 floor; the returned
/// `Result` exists for parity with the broader compute-patch API.
pub fn compute_patch(
    _prior: &Project,
    _args: &TemplateListArgs,
) -> Result<(Value, Vec<Value>, TemplateListData), TemplateListError> {
    Ok((json!([]), Vec::new(), build_data()))
}

/// Build the data envelope from `(args, post_state)`.
///
/// # Errors
///
/// Reuses [`compute_patch`], so this can only return reconstruction
/// errors introduced while rebuilding the deterministic envelope.
pub fn data_envelope_from_args(
    args: &TemplateListArgs,
    post_state: &Project,
) -> Result<TemplateListData, ReconstructError> {
    let (_, _, data) =
        compute_patch(post_state, args).map_err(|e| ReconstructError::Custom(e.to_string()))?;
    Ok(data)
}

impl From<TemplateListError> for VerbError {
    fn from(value: TemplateListError) -> Self {
        match value {
            TemplateListError::Io { .. } => VerbError::Custom(value.to_string()),
        }
    }
}

/// The §0.8 verb for `template.list`.
#[derive(Debug, Default)]
pub struct TemplateListVerb;

impl Verb for TemplateListVerb {
    fn verb(&self) -> &'static str {
        "template.list"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: TemplateListArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("template.list: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch = serde_json::from_value(patch_value).map_err(|err| {
            VerbError::Custom(format!("template.list: patch construction failed: {err}"))
        })?;

        let data = serde_json::to_value(&data).map_err(|err| {
            VerbError::Custom(format!("template.list: data envelope failed: {err}"))
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
        let typed: TemplateListArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "TemplateListArgs",
            })?;

        let envelope = data_envelope_from_args(&typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
