//! `stock.search` (§17.2) — v1 local empty-results floor.
//!
//! ## v1 floor
//!
//! Real `stock.search` delegates to registered providers and validates
//! provider responses. The state-layer `Verb` implementation has no
//! provider runtime/context, so v1 accepts only the built-in `local`
//! provider and returns an empty item list. Unknown provider ids map to
//! `E_STOCK_PROVIDER_UNKNOWN`.

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::ProjectId;

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use crate::verbs::stock_list_providers::StockMediaKind;

fn default_limit() -> i64 {
    25
}

fn default_any_filter() -> String {
    "any".to_string()
}

/// `filters` object for `stock.search`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StockSearchFilters {
    /// Minimum duration in ticks (inclusive), when provided.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_min_tk: Option<i64>,
    /// Maximum duration in ticks (exclusive relative to `duration_min_tk`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_max_tk: Option<i64>,
    /// Aspect filter (`16:9`, `9:16`, `1:1`, `any`).
    #[serde(default = "default_any_filter")]
    pub aspect: String,
    /// License family filter (`cc0`, `cc-by`, `royalty-free`, `any`).
    #[serde(default = "default_any_filter")]
    pub license: String,
}

impl Default for StockSearchFilters {
    fn default() -> Self {
        Self {
            duration_min_tk: None,
            duration_max_tk: None,
            aspect: default_any_filter(),
            license: default_any_filter(),
        }
    }
}

/// Arguments for `stock.search`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StockSearchArgs {
    /// Required by the current `Verb` dispatch shape.
    pub project_id: ProjectId,
    /// Provider id from `stock.list_providers`.
    pub provider_id: String,
    /// Free-text query (1..=512 chars).
    pub query: String,
    /// Requested stock media kind literal.
    pub kind: String,
    /// Maximum items to return (default `25`, allowed `[1, 100]`).
    #[serde(default = "default_limit")]
    pub limit: i64,
    /// Structured filters (default `{}`).
    #[serde(default, deserialize_with = "deserialize_filters_object")]
    pub filters: StockSearchFilters,
}

fn deserialize_filters_object<'de, D>(deserializer: D) -> Result<StockSearchFilters, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    if !value.is_object() {
        return Err(D::Error::custom(
            "stock.search: filters must be an object when provided",
        ));
    }

    serde_json::from_value(value).map_err(D::Error::custom)
}

/// Dimensions in pixels for stock items that carry image/video geometry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StockSearchDimensions {
    /// Width in pixels.
    pub width: u64,
    /// Height in pixels.
    pub height: u64,
}

/// License object surfaced in each `stock.search` item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StockSearchLicense {
    /// SPDX id or stock sentinel (`royalty-free`, `unknown`).
    pub spdx: String,
    /// Whether attribution is required by license terms.
    pub attribution_required: bool,
    /// Provider-required attribution text when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attribution_text: Option<String>,
    /// Canonical source URL for the item on the provider side.
    pub source_url: String,
}

/// One result row returned by `stock.search`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StockSearchItem {
    /// Provider-namespaced opaque id.
    pub stock_id: String,
    /// Provider id for round-trip clarity.
    pub provider_id: String,
    /// Resolved media kind.
    pub kind: StockMediaKind,
    /// Display title.
    pub title: String,
    /// Duration in ticks for temporal media.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_tk: Option<i64>,
    /// Dimensions for visual media.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dimensions: Option<StockSearchDimensions>,
    /// Preview thumbnail / low-res URL.
    pub preview_url: String,
    /// Structured license metadata.
    pub license: StockSearchLicense,
}

/// Envelope returned by `stock.search`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StockSearchData {
    /// First-page stock search results.
    pub items: Vec<StockSearchItem>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Error)]
/// Verb-level errors for `stock.search`.
pub enum StockSearchError {
    /// Provider id is not registered.
    #[error(
        "stock.search: E_STOCK_PROVIDER_UNKNOWN — provider_id `{provider_id}` is not \
         registered; registered_ids={registered_ids:?}"
    )]
    ProviderUnknown {
        /// Provider id supplied by caller.
        provider_id: String,
        /// Provider ids currently registered.
        registered_ids: Vec<String>,
    },
    /// Reserved for upstream provider rate-limit failures.
    #[error(
        "stock.search: E_STOCK_RATE_LIMITED — provider_id `{provider_id}` retry_after_s \
         {retry_after_s}"
    )]
    RateLimited {
        /// Provider id that returned rate limit.
        provider_id: String,
        /// Retry-after window in seconds.
        retry_after_s: u64,
    },
    /// Reserved for upstream provider auth failures.
    #[error("stock.search: E_STOCK_AUTH_REQUIRED — provider_id `{provider_id}` hint `{hint}`")]
    AuthRequired {
        /// Provider id that requires credentials.
        provider_id: String,
        /// Operator-facing credential hint.
        hint: String,
    },
    /// Range failure from `limit` or duration filters.
    #[error(
        "stock.search: E_BAD_RANGE — field `{field}` requested `{requested}` allowed `{allowed}`"
    )]
    BadRange {
        /// Field that violated range constraints.
        field: String,
        /// Requested value as string.
        requested: String,
        /// Allowed range or invariant description.
        allowed: String,
    },
    /// Schema/enum-literal failure for kind/query/filter literals.
    #[error("stock.search: E_SCHEMA_VIOLATION — field `{field}` detail `{detail}`")]
    SchemaViolation {
        /// Field that failed validation.
        field: String,
        /// Human-readable failure detail.
        detail: String,
    },
}

/// Build the RFC 6902 patch for `stock.search`.
///
/// # Errors
///
/// Returns [`StockSearchError::SchemaViolation`] for invalid `query`,
/// `kind`, `filters.aspect`, or `filters.license`.
/// Returns [`StockSearchError::BadRange`] for invalid `limit` or
/// duration filter ranges.
/// Returns [`StockSearchError::ProviderUnknown`] for non-`local`
/// provider ids in the v1 floor.
pub fn compute_patch(
    _prior: &Project,
    args: &StockSearchArgs,
) -> Result<(Value, Vec<Value>, StockSearchData), StockSearchError> {
    let query_len = args.query.chars().count();
    if !(1..=512).contains(&query_len) {
        return Err(StockSearchError::SchemaViolation {
            field: "query".to_string(),
            detail: format!("expected length in [1, 512], got {query_len}"),
        });
    }

    if !matches!(
        args.kind.as_str(),
        "video" | "audio" | "image" | "sticker" | "music"
    ) {
        return Err(StockSearchError::SchemaViolation {
            field: "kind".to_string(),
            detail: format!(
                "expected one of [video, audio, image, sticker, music], got `{}`",
                args.kind
            ),
        });
    }

    if !(1..=100).contains(&args.limit) {
        return Err(StockSearchError::BadRange {
            field: "limit".to_string(),
            requested: args.limit.to_string(),
            allowed: "[1, 100]".to_string(),
        });
    }

    if let Some(duration_min_tk) = args.filters.duration_min_tk
        && duration_min_tk < 0
    {
        return Err(StockSearchError::BadRange {
            field: "filters.duration_min_tk".to_string(),
            requested: duration_min_tk.to_string(),
            allowed: ">= 0".to_string(),
        });
    }

    if let Some(duration_max_tk) = args.filters.duration_max_tk {
        if duration_max_tk < 0 {
            return Err(StockSearchError::BadRange {
                field: "filters.duration_max_tk".to_string(),
                requested: duration_max_tk.to_string(),
                allowed: ">= 0".to_string(),
            });
        }
        if let Some(duration_min_tk) = args.filters.duration_min_tk
            && duration_max_tk <= duration_min_tk
        {
            return Err(StockSearchError::BadRange {
                field: "filters.duration_max_tk".to_string(),
                requested: duration_max_tk.to_string(),
                allowed: format!("> filters.duration_min_tk ({duration_min_tk})"),
            });
        }
    }

    if !matches!(
        args.filters.aspect.as_str(),
        "16:9" | "9:16" | "1:1" | "any"
    ) {
        return Err(StockSearchError::SchemaViolation {
            field: "filters.aspect".to_string(),
            detail: format!(
                "expected one of [16:9, 9:16, 1:1, any], got `{}`",
                args.filters.aspect
            ),
        });
    }

    if !matches!(
        args.filters.license.as_str(),
        "cc0" | "cc-by" | "royalty-free" | "any"
    ) {
        return Err(StockSearchError::SchemaViolation {
            field: "filters.license".to_string(),
            detail: format!(
                "expected one of [cc0, cc-by, royalty-free, any], got `{}`",
                args.filters.license
            ),
        });
    }

    if args.provider_id != "local" {
        return Err(StockSearchError::ProviderUnknown {
            provider_id: args.provider_id.clone(),
            registered_ids: vec!["local".to_string()],
        });
    }

    Ok((json!([]), vec![], StockSearchData { items: vec![] }))
}

/// Build the data envelope from `(args, post_state)`.
///
/// # Errors
///
/// Reuses [`compute_patch`], so this can only return reconstruction
/// errors from deterministic v1-floor validation.
pub fn data_envelope_from_args(
    args: &StockSearchArgs,
    post_state: &Project,
) -> Result<StockSearchData, ReconstructError> {
    let (_, _, data) =
        compute_patch(post_state, args).map_err(|e| ReconstructError::Custom(e.to_string()))?;
    Ok(data)
}

impl From<StockSearchError> for VerbError {
    fn from(value: StockSearchError) -> Self {
        match value {
            StockSearchError::ProviderUnknown { .. }
            | StockSearchError::RateLimited { .. }
            | StockSearchError::AuthRequired { .. }
            | StockSearchError::BadRange { .. }
            | StockSearchError::SchemaViolation { .. } => VerbError::Custom(value.to_string()),
        }
    }
}

/// The §0.8 verb for `stock.search`.
#[derive(Debug, Default)]
pub struct StockSearchVerb;

impl Verb for StockSearchVerb {
    fn verb(&self) -> &'static str {
        "stock.search"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: StockSearchArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("stock.search: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch = serde_json::from_value(patch_value).map_err(|err| {
            VerbError::Custom(format!("stock.search: patch construction failed: {err}"))
        })?;
        let data = serde_json::to_value(&data).map_err(|err| {
            VerbError::Custom(format!("stock.search: data envelope failed: {err}"))
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
        let typed: StockSearchArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "StockSearchArgs",
            })?;

        let envelope = data_envelope_from_args(&typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
