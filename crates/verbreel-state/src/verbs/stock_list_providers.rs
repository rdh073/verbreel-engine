//! `stock.list_providers` (§17.1) — sixty-seventh production verb in the engine.
//!
//! ## Spec quote (`spec/commands/stock.md` §17.1, abbreviated)
//!
//! > CLI: `verbreel stock list_providers`
//! > MCP: `stock.list_providers`
//! > Args: none
//! > Returns (`data`): `{ providers: Provider[] }` where every
//! > `Provider` carries `id`, `name`, `kind`, `kinds_supported`,
//! > `requires_credentials`, and optional `base_url` /
//! > `rate_limit_per_minute`. The kind enum is the closed set
//! > `local | http_catalog | custom_command`; media kinds are the
//! > closed set `video | audio | image | sticker | music`.
//!
//! ## v1 floor — built-in `local` only.
//!
//! Per §17.1, the full provider list is the in-config
//! `~/.verbreel/config.json::stock_providers[]` array (§17.7) plus the
//! always-present built-in `local`. The `Verb` trait's purity contract
//! forbids file I/O in `compute_patch`, so v1 ships the built-in only.
//! Config-file-registered providers need a `VerbContext` / storage
//! facade threaded through `ProjectStore::mutate_via_verb` so the
//! dispatcher can parse the config at startup and pass providers via
//! context — same architectural gap that `project.info` defers
//! `event_count` for, `timeline.snapshot` defers head `event_id` for,
//! and `list_capabilities` defers v1.1+ subsystem fields for. A future
//! slice introduces `VerbContext` and wires several deferred features
//! at once.
//!
//! ## Bundle metadata, not project state.
//!
//! `stock.list_providers` is read-only and does not read or mutate
//! project state; it only exposes the engine's compile-time
//! provider list.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::ProjectId;

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// Arguments for `stock.list_providers`.
///
/// Args takes `project_id` for trait compatibility; the impl ignores
/// it — see `list_capabilities` / `effect.list_available` precedent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StockListProvidersArgs {
    /// Required by the `Verb` trait shape; not read by the impl.
    pub project_id: ProjectId,
}

/// Provider kind — the closed set from §17.1 / §17.7.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    /// Built-in bundled provider sourced from `~/.verbreel/stock/`.
    Local,
    /// HTTPS catalog provider per §17.7.
    HttpCatalog,
    /// Custom-command provider per §17.8.
    CustomCommand,
}

/// Stock media kind — the closed set a provider can serve.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StockMediaKind {
    /// Video clip.
    Video,
    /// Audio clip.
    Audio,
    /// Image / still.
    Image,
    /// Sticker (animated or static overlay).
    Sticker,
    /// Music track.
    Music,
}

/// Single provider entry returned by `stock.list_providers`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provider {
    /// Unique provider id (e.g. `"local"`).
    pub id: String,
    /// Human-readable display name.
    pub name: String,
    /// Provider kind.
    pub kind: ProviderKind,
    /// Media kinds this provider can serve.
    pub kinds_supported: Vec<StockMediaKind>,
    /// Whether the provider currently requires credentials.
    pub requires_credentials: bool,
    /// Base URL — present for `http_catalog` (and some `custom_command`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Provider-declared rate cap per minute, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limit_per_minute: Option<u32>,
}

/// Envelope returned by `stock.list_providers`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StockListProvidersData {
    /// Registered providers in deterministic order.
    pub providers: Vec<Provider>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Error)]
/// Verb-level error type for `stock.list_providers`.
pub enum StockListProvidersError {
    /// No verb-level runtime errors.
    #[error("stock.list_providers: unreachable (no error variants)")]
    Unreachable,
}

/// Build the always-present built-in `local` provider entry.
fn local_provider() -> Provider {
    Provider {
        id: "local".to_string(),
        name: "Local catalog".to_string(),
        kind: ProviderKind::Local,
        kinds_supported: vec![
            StockMediaKind::Video,
            StockMediaKind::Audio,
            StockMediaKind::Image,
            StockMediaKind::Sticker,
            StockMediaKind::Music,
        ],
        requires_credentials: false,
        base_url: None,
        rate_limit_per_minute: None,
    }
}

/// Build the canonical `stock.list_providers` data envelope.
fn build_data() -> StockListProvidersData {
    StockListProvidersData {
        providers: vec![local_provider()],
    }
}

/// Build the RFC 6902 patch for `stock.list_providers`.
///
/// # Errors
///
/// No runtime errors are produced by this verb; the returned `Result`
/// exists for parity with the broader compute-patch API.
pub fn compute_patch(
    _prior: &Project,
    _args: &StockListProvidersArgs,
) -> Result<(Value, Vec<Value>, StockListProvidersData), StockListProvidersError> {
    Ok((json!([]), Vec::new(), build_data()))
}

/// Build the data envelope from `(args, post_state)`.
///
/// # Errors
///
/// Reuses [`compute_patch`], so this can only return reconstruction
/// errors introduced while rebuilding the deterministic envelope.
pub fn data_envelope_from_args(
    args: &StockListProvidersArgs,
    post_state: &Project,
) -> Result<StockListProvidersData, ReconstructError> {
    let (_, _, data) =
        compute_patch(post_state, args).map_err(|e| ReconstructError::Custom(e.to_string()))?;
    Ok(data)
}

impl From<StockListProvidersError> for VerbError {
    fn from(value: StockListProvidersError) -> Self {
        match value {
            StockListProvidersError::Unreachable => VerbError::BadArgs {
                detail: value.to_string(),
            },
        }
    }
}

/// The §0.8 verb for `stock.list_providers`.
#[derive(Debug, Default)]
pub struct StockListProvidersVerb;

impl Verb for StockListProvidersVerb {
    fn verb(&self) -> &'static str {
        "stock.list_providers"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: StockListProvidersArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("stock.list_providers: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch = serde_json::from_value(patch_value).map_err(|err| {
            VerbError::Custom(format!(
                "stock.list_providers: patch construction failed: {err}"
            ))
        })?;

        let data = serde_json::to_value(&data).map_err(|err| {
            VerbError::Custom(format!("stock.list_providers: data envelope failed: {err}"))
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
        let typed: StockListProvidersArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "StockListProvidersArgs",
            })?;

        let envelope = data_envelope_from_args(&typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
