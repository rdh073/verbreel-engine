//! `list_capabilities` (§1.5) — sixty-third production verb in the engine.
//!
//! ## Spec quote (`spec/commands/meta.md` §1.5, abbreviated)
//!
//! > CLI: `verbreel capabilities`
//! > MCP: `list_capabilities`
//! > Args: none
//! > Returns (`data`): the v1.0 floor exposes thirteen mandatory
//! > fields — `engine_version`, `supported_schema_versions`,
//! > `tick_rate_hz`, `verbs[]`, `effects[]`, `render_presets[]`,
//! > `caption_languages[]`, `caption_engine`, `caption_models[]`,
//! > `caption_default_model`, `linked_ffmpeg_version`,
//! > `tested_ffmpeg_range`, and `bundled_fonts[]`. The seven v1.1+
//! > fields (`tracker_algorithms`, `tracker_model_versions`,
//! > `audio_analysis_algorithms`, `audio_analysis_features`,
//! > `stock_providers`, `template_count`, `preview_session_formats`)
//! > are omitted entirely — their presence would imply v1.1
//! > capability the engine does not have.
//!
//! ## Bundle metadata, not project state.
//!
//! `list_capabilities` is read-only and does not read or mutate
//! project state; it only exposes engine build metadata sourced from
//! compile-time constants and the registered-verb / bundled-effect
//! enumerations.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::ProjectId;

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use crate::verbs::effect_list_available::bundled_effects;

/// Arguments for `list_capabilities`.
///
/// Args takes `project_id` for trait compatibility; the impl ignores
/// it — see `effect.list_available` precedent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListCapabilitiesArgs {
    /// Required by the `Verb` trait shape; not read by the impl.
    pub project_id: ProjectId,
}

/// Single verb metadata entry in the returned `verbs[]` field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerbEntry {
    /// Verb id (e.g. `"project.set_metadata"`).
    pub name: String,
    /// Short verb summary. Empty in v1 — `Verb` trait does not yet
    /// expose `fn summary() -> &'static str`.
    pub summary: String,
    /// Args schema identifier. Empty in v1 — `Verb` trait does not yet
    /// expose schema ids.
    pub args_schema_id: String,
}

/// Single effect metadata entry in the returned `effects[]` field.
///
/// Three fields: a projection of [`crate::verbs::effect_list_available::AvailableEffect`]
/// with `summary` dropped to match the §1.5 spec shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectEntry {
    /// Effect kind identifier.
    pub kind: String,
    /// Effect category.
    pub category: String,
    /// Parameter schema identifier.
    pub params_schema_id: String,
}

/// Envelope returned by `list_capabilities`.
///
/// The thirteen v1.0-mandatory fields, in spec order. The seven v1.1+
/// fields are omitted entirely.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListCapabilitiesData {
    /// Engine `SemVer` string, sourced from `CARGO_PKG_VERSION`.
    pub engine_version: String,
    /// Supported schema range as a `SemVer` range expression.
    pub supported_schema_versions: String,
    /// Tick rate in Hz (§0.2 — always `240_000` in v1).
    pub tick_rate_hz: u64,
    /// Registered verbs, sorted by `name` ascending.
    pub verbs: Vec<VerbEntry>,
    /// Bundled effect kinds, sorted by `kind` ascending.
    pub effects: Vec<EffectEntry>,
    /// Render presets. Empty in v1 — render subsystem deferred.
    pub render_presets: Vec<String>,
    /// Caption languages. Empty in v1 — AI subsystem deferred.
    pub caption_languages: Vec<String>,
    /// Caption engine identifier. Empty in v1 — AI subsystem deferred.
    pub caption_engine: String,
    /// Caption models. Empty in v1 — AI subsystem deferred.
    pub caption_models: Vec<String>,
    /// Default caption model. Empty in v1 — AI subsystem deferred.
    pub caption_default_model: String,
    /// Linked ffmpeg version. Empty in v1 — codec wiring deferred.
    pub linked_ffmpeg_version: String,
    /// Tested ffmpeg range. Empty in v1 — codec wiring deferred.
    pub tested_ffmpeg_range: String,
    /// Bundled fonts. Empty in v1 — font enumeration deferred.
    pub bundled_fonts: Vec<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Error)]
/// Verb-level error type for `list_capabilities`.
pub enum ListCapabilitiesError {
    /// No verb-level runtime errors.
    #[error("list_capabilities: unreachable (no error variants)")]
    Unreachable,
}

/// Supported project schema range expressed as a `SemVer` range.
const SUPPORTED_SCHEMA_VERSIONS: &str = ">=1.0.0 <2.0.0";

/// Build the registered-verb entries for `verbs[]`, sorted by name.
fn collect_verb_entries() -> Vec<VerbEntry> {
    let registry = crate::verbs::default_registry();
    let mut entries: Vec<VerbEntry> = registry
        .iter()
        .map(|(name, _)| VerbEntry {
            name: name.to_string(),
            summary: String::new(),
            args_schema_id: String::new(),
        })
        .collect();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    entries
}

/// Build the bundled-effect entries for `effects[]`, projecting the
/// 4-field `AvailableEffect` shape down to the 3-field `EffectEntry`
/// shape required by §1.5 (drops `summary`).
fn collect_effect_entries() -> Vec<EffectEntry> {
    bundled_effects()
        .into_iter()
        .map(|e| EffectEntry {
            kind: e.kind,
            category: e.category,
            params_schema_id: e.params_schema_id,
        })
        .collect()
}

/// Build the canonical `list_capabilities` data envelope.
fn build_data() -> ListCapabilitiesData {
    ListCapabilitiesData {
        engine_version: env!("CARGO_PKG_VERSION").to_string(),
        supported_schema_versions: SUPPORTED_SCHEMA_VERSIONS.to_string(),
        tick_rate_hz: u64::from(verbreel_types::TICK_RATE_HZ),
        verbs: collect_verb_entries(),
        effects: collect_effect_entries(),
        render_presets: super::render_list_presets::bundled_presets()
            .into_iter()
            .map(|p| p.name)
            .collect(),
        caption_languages: Vec::new(),
        caption_engine: String::new(),
        caption_models: Vec::new(),
        caption_default_model: String::new(),
        linked_ffmpeg_version: String::new(),
        tested_ffmpeg_range: String::new(),
        bundled_fonts: Vec::new(),
    }
}

/// Build the RFC 6902 patch for `list_capabilities`.
///
/// # Errors
///
/// No runtime errors are produced by this verb; the returned `Result`
/// exists for parity with the broader compute-patch API.
pub fn compute_patch(
    _prior: &Project,
    _args: &ListCapabilitiesArgs,
) -> Result<(Value, Vec<Value>, ListCapabilitiesData), ListCapabilitiesError> {
    Ok((json!([]), Vec::new(), build_data()))
}

/// Build the data envelope from `(args, post_state)`.
///
/// # Errors
///
/// Reuses [`compute_patch`], so this can only return reconstruction
/// errors introduced while rebuilding the deterministic envelope.
pub fn data_envelope_from_args(
    args: &ListCapabilitiesArgs,
    post_state: &Project,
) -> Result<ListCapabilitiesData, ReconstructError> {
    let (_, _, data) =
        compute_patch(post_state, args).map_err(|e| ReconstructError::Custom(e.to_string()))?;
    Ok(data)
}

impl From<ListCapabilitiesError> for VerbError {
    fn from(value: ListCapabilitiesError) -> Self {
        match value {
            ListCapabilitiesError::Unreachable => VerbError::BadArgs {
                detail: value.to_string(),
            },
        }
    }
}

/// The §0.8 verb for `list_capabilities`.
#[derive(Debug, Default)]
pub struct ListCapabilitiesVerb;

impl Verb for ListCapabilitiesVerb {
    fn verb(&self) -> &'static str {
        "list_capabilities"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: ListCapabilitiesArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("list_capabilities: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch = serde_json::from_value(patch_value).map_err(|err| {
            VerbError::Custom(format!(
                "list_capabilities: patch construction failed: {err}"
            ))
        })?;

        let data = serde_json::to_value(&data).map_err(|err| {
            VerbError::Custom(format!("list_capabilities: data envelope failed: {err}"))
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
        let typed: ListCapabilitiesArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "ListCapabilitiesArgs",
            })?;

        let envelope = data_envelope_from_args(&typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
