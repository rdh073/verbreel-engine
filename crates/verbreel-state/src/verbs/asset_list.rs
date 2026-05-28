//! `asset.list` (§3.2) — twenty-eighth production verb in the engine.
//!
//! ## Spec quote (`spec/commands/asset.md` §3.2, verbatim)
//!
//! > Returns `Project.assets[]` (the tagged-union `Asset` enum),
//! > optionally filtered by kind, sorted by `imported_at` ascending,
//! > with `id` ascending as tiebreaker.
//!
//! ## Read-only verb
//!
//! `asset.list` does not mutate project state; the patch is always
//! `[]`, no warnings are returned, and `data` carries the deterministic
//! sorted list of assets.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_events::Timestamp;
use verbreel_types::ProjectId;

use crate::asset::Asset;
use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// Asset kind filter for `asset.list`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AssetKindFilter {
    /// Video assets.
    Video,
    /// Audio assets.
    Audio,
    /// Image assets.
    Image,
    /// Subtitle assets.
    Subtitle,
}

/// Arguments for `asset.list`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetListArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Optional kind filter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<AssetKindFilter>,
}

/// Envelope returned by `asset.list`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetListData {
    /// Sorted and optionally filtered assets.
    pub assets: Vec<Asset>,
}

/// Verb-level error type for `asset.list`.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AssetListError {
    /// `asset.list` has no runtime error variants.
    #[error("asset.list: unreachable (no error variants)")]
    Unreachable,
}

fn asset_kind_str(asset: &Asset) -> &'static str {
    match asset {
        Asset::Video(_) => "video",
        Asset::Audio(_) => "audio",
        Asset::Image(_) => "image",
        Asset::Subtitle(_) => "subtitle",
    }
}

fn asset_imported_at(asset: &Asset) -> &Timestamp {
    match asset {
        Asset::Video(a) => &a.imported_at,
        Asset::Audio(a) => &a.imported_at,
        Asset::Image(a) => &a.imported_at,
        Asset::Subtitle(a) => &a.imported_at,
    }
}

fn asset_id_str(asset: &Asset) -> String {
    match asset {
        Asset::Video(a) => a.id.to_string(),
        Asset::Audio(a) => a.id.to_string(),
        Asset::Image(a) => a.id.to_string(),
        Asset::Subtitle(a) => a.id.to_string(),
    }
}

/// Build the RFC-6902 patch for `asset.list`.
///
/// # Errors
///
/// No runtime errors are expected from this verb itself.
pub fn compute_patch(
    prior: &Project,
    args: &AssetListArgs,
) -> Result<(Value, Vec<Value>, AssetListData), AssetListError> {
    let kind_filter: Option<&str> = args.kind.map(|k| match k {
        AssetKindFilter::Video => "video",
        AssetKindFilter::Audio => "audio",
        AssetKindFilter::Image => "image",
        AssetKindFilter::Subtitle => "subtitle",
    });

    let mut assets: Vec<Asset> = prior
        .assets
        .iter()
        .filter(|a| kind_filter.is_none_or(|k| asset_kind_str(a) == k))
        .cloned()
        .collect();

    assets.sort_by(|a, b| {
        asset_imported_at(a)
            .cmp(asset_imported_at(b))
            .then_with(|| asset_id_str(a).cmp(&asset_id_str(b)))
    });

    Ok((json!([]), Vec::new(), AssetListData { assets }))
}

/// Build the data envelope from `(args, post_state)`.
///
/// # Errors
///
/// Reuses [`compute_patch`] logic so the only possible error is from
/// impossible `Asset` parsing during construction.
pub fn data_envelope_from_post_state(
    args: &AssetListArgs,
    post_state: &Project,
) -> Result<AssetListData, ReconstructError> {
    let (_, _, data) =
        compute_patch(post_state, args).map_err(|e| ReconstructError::Custom(e.to_string()))?;
    Ok(data)
}

impl From<AssetListError> for VerbError {
    fn from(value: AssetListError) -> Self {
        match value {
            AssetListError::Unreachable => VerbError::BadArgs {
                detail: value.to_string(),
            },
        }
    }
}

/// The §0.8 verb for `asset.list`.
#[derive(Debug, Default)]
pub struct AssetListVerb;

impl Verb for AssetListVerb {
    fn verb(&self) -> &'static str {
        "asset.list"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: AssetListArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("asset.list: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch = serde_json::from_value(patch_value).map_err(|err| {
            VerbError::Custom(format!("asset.list: patch construction failed: {err}"))
        })?;
        let data = serde_json::to_value(&data)
            .map_err(|err| VerbError::Custom(format!("asset.list: data envelope failed: {err}")))?;

        Ok((patch, data, warnings))
    }

    fn reconstruct(
        &self,
        args: &Value,
        _patch: &Value,
        _warnings: &[Value],
        post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let typed: AssetListArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "AssetListArgs",
            })?;
        let envelope = data_envelope_from_post_state(&typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
