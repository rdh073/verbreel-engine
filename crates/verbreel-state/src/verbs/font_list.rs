//! `font.list` (§7.5) — sixty-eighth production verb in the engine.
//!
//! ## Spec quote (`spec/commands/text.md` §7.5)
//!
//! > CLI: `verbreel font list`
//! > MCP: `font.list`
//! > Args: none
//! > Returns (`data`): `{ families: { name: string;
//! >   source: "bundled" | "system"; path?: string }[] }`

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::ProjectId;

use crate::font_registry;
use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// Arguments for `font.list`.
///
/// Args takes `project_id` for trait compatibility; the impl ignores
/// it — see `stock.list_providers` / `list_capabilities` /
/// `effect.list_available` precedent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FontListArgs {
    /// Required by the `Verb` trait shape; not read by the impl.
    pub project_id: ProjectId,
}

/// Single family entry returned by `font.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FontFamilyEntry {
    /// Font family name (e.g. `"Inter"`).
    pub name: String,
    /// Discovery source.
    pub source: font_registry::RegistrySource,
    /// Optional font-file path for filesystem-backed families.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// Envelope returned by `font.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FontListData {
    /// Available fonts in deterministic order.
    pub families: Vec<FontFamilyEntry>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Error)]
/// Verb-level error type for `font.list`.
pub enum FontListError {
    /// No verb-level runtime errors.
    #[error("font.list: unreachable (no error variants)")]
    Unreachable,
}

/// Build the canonical `font.list` data envelope.
fn build_data() -> FontListData {
    let families = font_registry::list()
        .into_iter()
        .map(|family| FontFamilyEntry {
            name: family.name,
            source: family.source,
            path: family.path,
        })
        .collect();
    FontListData { families }
}

/// Build the RFC 6902 patch for `font.list`.
///
/// # Errors
///
/// No runtime errors are produced by this verb; the returned `Result`
/// exists for parity with the broader compute-patch API.
pub fn compute_patch(
    _prior: &Project,
    _args: &FontListArgs,
) -> Result<(Value, Vec<Value>, FontListData), FontListError> {
    Ok((json!([]), Vec::new(), build_data()))
}

/// Build the data envelope from `(args, post_state)`.
///
/// # Errors
///
/// Reuses [`compute_patch`], so this can only return reconstruction
/// errors introduced while rebuilding the deterministic envelope.
pub fn data_envelope_from_args(
    args: &FontListArgs,
    post_state: &Project,
) -> Result<FontListData, ReconstructError> {
    let (_, _, data) =
        compute_patch(post_state, args).map_err(|e| ReconstructError::Custom(e.to_string()))?;
    Ok(data)
}

impl From<FontListError> for VerbError {
    fn from(value: FontListError) -> Self {
        match value {
            FontListError::Unreachable => VerbError::BadArgs {
                detail: value.to_string(),
            },
        }
    }
}

/// The §0.8 verb for `font.list`.
#[derive(Debug, Default)]
pub struct FontListVerb;

impl Verb for FontListVerb {
    fn verb(&self) -> &'static str {
        "font.list"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: FontListArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("font.list: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch = serde_json::from_value(patch_value).map_err(|err| {
            VerbError::Custom(format!("font.list: patch construction failed: {err}"))
        })?;

        let data = serde_json::to_value(&data)
            .map_err(|err| VerbError::Custom(format!("font.list: data envelope failed: {err}")))?;

        Ok((patch, data, warnings))
    }

    fn reconstruct(
        &self,
        args: &Value,
        _patch: &Value,
        _warnings: &[Value],
        post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let typed: FontListArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "FontListArgs",
            })?;

        let envelope = data_envelope_from_args(&typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
