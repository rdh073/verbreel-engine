//! `stock.describe` (§17.4) — v1 local stock-not-found floor.
//!
//! Real `stock.describe` requires provider/runtime/catalog context to
//! resolve metadata. The pure `Verb` surface cannot do that in this
//! slice, so v1 implements only:
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
use crate::verbs::stock_list_providers::StockMediaKind;
use crate::verbs::stock_search::{StockSearchDimensions, StockSearchLicense};

/// Arguments for `stock.describe`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StockDescribeArgs {
    /// Required by the current `Verb` dispatch shape.
    pub project_id: ProjectId,
    /// Provider id from `stock.list_providers`.
    pub provider_id: String,
    /// Opaque provider stock identifier.
    pub stock_id: String,
}

/// Future success envelope for `stock.describe`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StockDescribeData {
    /// Echoed input stock id.
    pub stock_id: String,
    /// Echoed input provider id.
    pub provider_id: String,
    /// Resolved media kind.
    pub kind: StockMediaKind,
    /// Display title.
    pub title: String,
    /// Optional duration in ticks for temporal media.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_tk: Option<i64>,
    /// Optional dimensions for visual media.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dimensions: Option<StockSearchDimensions>,
    /// Preview thumbnail / low-res URL.
    pub preview_url: String,
    /// Optional provider fetch URL (informational only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_url: Option<String>,
    /// Optional declared file size in bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    /// Optional free-form author detail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Structured license metadata.
    pub license: StockSearchLicense,
}

/// Verb-level errors for `stock.describe`.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum StockDescribeError {
    /// Provider id is not registered.
    #[error(
        "stock.describe: E_STOCK_PROVIDER_UNKNOWN — provider_id `{provider_id}` is not \
         registered"
    )]
    ProviderUnknown {
        /// Provider id supplied by caller.
        provider_id: String,
    },
    /// Stock id does not resolve at the provider.
    #[error(
        "stock.describe: E_STOCK_NOT_FOUND — provider_id `{provider_id}` stock_id `{stock_id}` \
         does not resolve"
    )]
    StockNotFound {
        /// Provider id supplied by caller.
        provider_id: String,
        /// Stock id supplied by caller.
        stock_id: String,
    },
    /// Reserved: provider-side rate limit.
    #[error(
        "stock.describe: E_STOCK_RATE_LIMITED — provider_id `{provider_id}` retry_after_s \
         {retry_after_s}"
    )]
    RateLimited {
        /// Provider id that returned rate limit.
        provider_id: String,
        /// Retry-after in seconds.
        retry_after_s: u64,
    },
    /// Reserved: provider-side auth failure.
    #[error("stock.describe: E_STOCK_AUTH_REQUIRED — provider_id `{provider_id}` hint `{hint}`")]
    AuthRequired {
        /// Provider id requiring credentials.
        provider_id: String,
        /// Operator-facing credential hint.
        hint: String,
    },
}

/// Build the RFC 6902 patch for `stock.describe`.
///
/// v1 floor:
/// - non-`local` provider -> [`StockDescribeError::ProviderUnknown`]
/// - `local` provider -> [`StockDescribeError::StockNotFound`]
///
/// # Errors
///
/// Returns [`StockDescribeError::ProviderUnknown`] or
/// [`StockDescribeError::StockNotFound`] in this slice.
pub fn compute_patch(
    _prior: &Project,
    args: &StockDescribeArgs,
) -> Result<(Value, Vec<Value>, StockDescribeData), StockDescribeError> {
    if args.provider_id != "local" {
        return Err(StockDescribeError::ProviderUnknown {
            provider_id: args.provider_id.clone(),
        });
    }

    Err(StockDescribeError::StockNotFound {
        provider_id: args.provider_id.clone(),
        stock_id: args.stock_id.clone(),
    })
}

impl From<StockDescribeError> for VerbError {
    fn from(value: StockDescribeError) -> Self {
        match value {
            StockDescribeError::ProviderUnknown { .. }
            | StockDescribeError::StockNotFound { .. }
            | StockDescribeError::RateLimited { .. }
            | StockDescribeError::AuthRequired { .. } => VerbError::Custom(value.to_string()),
        }
    }
}

/// The §0.8 verb for `stock.describe`.
#[derive(Debug, Default)]
pub struct StockDescribeVerb;

impl Verb for StockDescribeVerb {
    fn verb(&self) -> &'static str {
        "stock.describe"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: StockDescribeArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("stock.describe: args deserialize failed: {err}"),
            })?;

        match compute_patch(prior, &typed) {
            Ok((patch_value, warnings, data)) => {
                let patch: json_patch::Patch =
                    serde_json::from_value(patch_value).map_err(|err| {
                        VerbError::Custom(format!(
                            "stock.describe: patch construction failed: {err}"
                        ))
                    })?;

                let data = serde_json::to_value(&data).map_err(|err| {
                    VerbError::Custom(format!("stock.describe: data envelope failed: {err}"))
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
        let _typed: StockDescribeArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "StockDescribeArgs",
            })?;

        Ok(Value::Null)
    }
}
