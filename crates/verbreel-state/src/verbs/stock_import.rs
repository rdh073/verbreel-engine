//! `stock.import` (§17.3) — v1 local stock-not-found floor.
//!
//! Real `stock.import` requires provider/runtime/filesystem context
//! (provider `describe`/`fetch`, hashing, probing, asset-store writes,
//! and metadata stamping). The pure `Verb` surface cannot do that in
//! this slice, so v1 implements only the lookup floor:
//!
//! - `provider_id != "local"` -> `E_STOCK_PROVIDER_UNKNOWN`
//! - `provider_id == "local"` -> `E_STOCK_NOT_FOUND` for every
//!   well-formed `stock_id`.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use verbreel_types::ProjectId;

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

fn default_mode() -> StockImportMode {
    StockImportMode::Copy
}

/// `mode` for `stock.import`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StockImportMode {
    /// Copy bytes into content-addressed store.
    Copy,
    /// Link source path when provider/runtime supports it.
    Link,
}

/// Arguments for `stock.import`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StockImportArgs {
    /// Target project id.
    pub project_id: ProjectId,
    /// Provider id from `stock.list_providers`.
    pub provider_id: String,
    /// Opaque provider stock identifier.
    pub stock_id: String,
    /// Import mode. Omitted -> `copy`.
    #[serde(default = "default_mode")]
    pub mode: StockImportMode,
    /// Allow `"unknown"` license in future full implementation.
    #[serde(default)]
    pub accept_license_unknown: bool,
}

/// License metadata that would be recorded on successful imports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StockImportLicenseRecorded {
    /// SPDX id or `"unknown"` sentinel.
    pub spdx: String,
    /// Attribution text when required.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attribution_text: Option<String>,
}

/// Future success envelope for `stock.import`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StockImportData {
    /// Resulting asset id.
    pub asset_id: String,
    /// Echoed input stock id.
    pub stock_id: String,
    /// Echoed input provider id.
    pub provider_id: String,
    /// Recorded license metadata.
    pub license_recorded: StockImportLicenseRecorded,
    /// Bytes downloaded from provider.
    pub bytes_downloaded: u64,
    /// Whether the import resolved to an existing asset hash.
    pub dedup_hit: bool,
    /// Mode used after any fallback.
    pub mode_used: StockImportMode,
}

/// Verb-level errors for `stock.import`.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum StockImportError {
    /// Provider id is not registered.
    #[error(
        "stock.import: E_STOCK_PROVIDER_UNKNOWN — provider_id `{provider_id}` is not registered"
    )]
    ProviderUnknown {
        /// Provider id supplied by caller.
        provider_id: String,
    },
    /// Stock id does not resolve at the provider.
    #[error(
        "stock.import: E_STOCK_NOT_FOUND — provider_id `{provider_id}` stock_id `{stock_id}` does not resolve"
    )]
    StockNotFound {
        /// Provider id supplied by caller.
        provider_id: String,
        /// Stock id supplied by caller.
        stock_id: String,
    },
    /// Reserved: provider-side rate limit.
    #[error(
        "stock.import: E_STOCK_RATE_LIMITED — provider_id `{provider_id}` retry_after_s {retry_after_s}"
    )]
    RateLimited {
        /// Provider id that returned rate limit.
        provider_id: String,
        /// Retry-after in seconds.
        retry_after_s: u64,
    },
    /// Reserved: provider-side auth failure.
    #[error("stock.import: E_STOCK_AUTH_REQUIRED — provider_id `{provider_id}` hint `{hint}`")]
    AuthRequired {
        /// Provider id requiring credentials.
        provider_id: String,
        /// Operator-facing credential hint.
        hint: String,
    },
    /// Reserved: unknown/missing license metadata.
    #[error(
        "stock.import: E_STOCK_LICENSE_UNKNOWN — provider_id `{provider_id}` stock_id `{stock_id}` hint `{hint}`"
    )]
    LicenseUnknown {
        /// Provider id for failing stock item.
        provider_id: String,
        /// Stock id with unknown license.
        stock_id: String,
        /// Recovery hint.
        hint: String,
    },
    /// Reserved: fetch failed.
    #[error(
        "stock.import: E_STOCK_FETCH_FAILED — provider_id `{provider_id}` stock_id `{stock_id}` upstream_status `{upstream_status}` elapsed_s {elapsed_s}"
    )]
    FetchFailed {
        /// Provider id for failing transfer.
        provider_id: String,
        /// Stock id for failing transfer.
        stock_id: String,
        /// Upstream status string.
        upstream_status: String,
        /// Elapsed seconds.
        elapsed_s: u64,
    },
    /// Reserved: incompatible args (e.g. link + non-local provider).
    #[error("stock.import: E_ARGS_INCOMPATIBLE — hint `{hint}`")]
    ArgsIncompatible {
        /// Recovery hint.
        hint: String,
    },
    /// Reserved: delegated from asset-import flow.
    #[error("stock.import: E_ASSET_UNSUPPORTED_KIND — {detail}")]
    AssetUnsupportedKind {
        /// Runtime detail.
        detail: String,
    },
    /// Reserved: delegated from asset-import flow.
    #[error("stock.import: E_ASSET_UNREADABLE — {detail}")]
    AssetUnreadable {
        /// Runtime detail.
        detail: String,
    },
    /// Reserved: delegated from asset-import flow.
    #[error("stock.import: E_ASSET_PROBE_TIMEOUT — {detail}")]
    AssetProbeTimeout {
        /// Runtime detail.
        detail: String,
    },
    /// Reserved: delegated/runtime IO failure.
    #[error("stock.import: E_IO — {detail}")]
    Io {
        /// Runtime detail.
        detail: String,
    },
    /// Reserved: delegated/runtime schema validation failure.
    #[error("stock.import: E_SCHEMA_VIOLATION — field `{field}` detail `{detail}`")]
    SchemaViolation {
        /// Field name that failed validation.
        field: String,
        /// Validation detail.
        detail: String,
    },
}

/// Build the RFC 6902 patch for `stock.import`.
///
/// v1 floor:
/// - non-`local` provider -> [`StockImportError::ProviderUnknown`]
/// - `local` provider -> [`StockImportError::StockNotFound`]
///
/// # Errors
///
/// Returns [`StockImportError::ProviderUnknown`] or
/// [`StockImportError::StockNotFound`] in this slice.
pub fn compute_patch(
    _prior: &Project,
    args: &StockImportArgs,
) -> Result<(Value, Vec<Value>, StockImportData), StockImportError> {
    if args.provider_id != "local" {
        return Err(StockImportError::ProviderUnknown {
            provider_id: args.provider_id.clone(),
        });
    }

    Err(StockImportError::StockNotFound {
        provider_id: args.provider_id.clone(),
        stock_id: args.stock_id.clone(),
    })
}

impl From<StockImportError> for VerbError {
    fn from(value: StockImportError) -> Self {
        match value {
            StockImportError::ProviderUnknown { .. }
            | StockImportError::StockNotFound { .. }
            | StockImportError::RateLimited { .. }
            | StockImportError::AuthRequired { .. }
            | StockImportError::LicenseUnknown { .. }
            | StockImportError::FetchFailed { .. }
            | StockImportError::ArgsIncompatible { .. }
            | StockImportError::AssetUnsupportedKind { .. }
            | StockImportError::AssetUnreadable { .. }
            | StockImportError::AssetProbeTimeout { .. }
            | StockImportError::Io { .. }
            | StockImportError::SchemaViolation { .. } => VerbError::Custom(value.to_string()),
        }
    }
}

/// The §0.8 verb for `stock.import`.
#[derive(Debug, Default)]
pub struct StockImportVerb;

impl Verb for StockImportVerb {
    fn verb(&self) -> &'static str {
        "stock.import"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: StockImportArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("stock.import: args deserialize failed: {err}"),
            })?;

        match compute_patch(prior, &typed) {
            Ok((patch_value, warnings, data)) => {
                let patch: json_patch::Patch =
                    serde_json::from_value(patch_value).map_err(|err| {
                        VerbError::Custom(format!("stock.import: patch construction failed: {err}"))
                    })?;

                let data = serde_json::to_value(&data).map_err(|err| {
                    VerbError::Custom(format!("stock.import: data envelope failed: {err}"))
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
        let _typed: StockImportArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "StockImportArgs",
            })?;

        Ok(Value::Null)
    }
}
