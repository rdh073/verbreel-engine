//! `font.list` (§7.5) — sixty-eighth production verb in the engine.
//!
//! ## Spec quote (`spec/commands/text.md` §7.5)
//!
//! > CLI: `verbreel font list`
//! > MCP: `font.list`
//! > Args: none
//! > Returns (`data`): `{ fonts: { family: string;
//! >   styles: { weight: integer; italic: boolean }[];
//! >   source: "bundled" | "system" }[] }`
//!
//! ## v1 floor — empty list.
//!
//! Per §7.5, the full fonts list is bundled engine fonts plus
//! system-discovered fonts. v1 ships zero bundled fonts and defers
//! system enumeration: walking cross-platform font directories
//! (`/usr/share/fonts/`, `/System/Library/Fonts/`, `C:\Windows\Fonts\`)
//! and parsing family/style metadata with a crate such as `fontdb`
//! or `font-kit` is file I/O — forbidden in the `Verb` trait's pure
//! `compute_patch`. Both sources need a `VerbContext` / storage facade
//! threaded through `ProjectStore::mutate_via_verb` — the same
//! architectural gap that `stock.list_providers` defers config-file
//! providers for and `list_capabilities` defers v1.1+ subsystem fields
//! for. A future slice introduces `VerbContext` and wires several
//! deferred features at once.
//!
//! ## Bundle metadata, not project state.
//!
//! `font.list` is read-only and does not read or mutate project state;
//! it only exposes the engine's compile-time fonts list (currently
//! empty).

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::ProjectId;

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

/// A single weight + italic combination for a font family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FontStyle {
    /// CSS-style font weight (`100`..=`900`).
    pub weight: u32,
    /// Whether this style is italic.
    pub italic: bool,
}

/// Where the font was discovered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FontSource {
    /// Bundled by the engine.
    Bundled,
    /// Discovered on the host system.
    System,
}

/// Single font family entry returned by `font.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FontEntry {
    /// Font family name (e.g. `"Inter"`).
    pub family: String,
    /// Available styles for this family.
    pub styles: Vec<FontStyle>,
    /// Discovery source.
    pub source: FontSource,
}

/// Envelope returned by `font.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FontListData {
    /// Available fonts in deterministic order.
    pub fonts: Vec<FontEntry>,
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
    FontListData { fonts: Vec::new() }
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
