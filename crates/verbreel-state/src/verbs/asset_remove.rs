//! `asset.remove` (§3.4) — fifty-eighth production verb in the engine.
//!
//! ## Spec quote (`spec/commands/asset.md` §3.4, summarized)
//!
//! Removes an asset record from `Project.assets[]`. By default refuses
//! if any clip references the asset. With `cascade: true`, also deletes
//! every clip whose `asset_id` matches. The underlying content-addressed
//! file under `assets/<aa>/<sha256>.<ext>` is **left on disk** by spec
//! design — bytes removal is the responsibility of `asset.gc` (§3.6).
//!
//! ## Reconstructor compatibility
//!
//! The removed asset (and any cascaded clips) are absent from
//! post-state, so `reconstruct()` cannot derive the data envelope from
//! post-state alone. The forward path therefore emits one internal
//! warning (`W_ASSET_REMOVE_ENVELOPE`) carrying every removed id and
//! the `file_orphaned` flag. The reconstructor reads that warning back
//! into [`AssetRemoveData`], mirroring the destructive-verb envelope
//! pattern used by `clip.delete` and `track.remove`.
//!
//! ## Deferred from this slice
//!
//! - **Cross-project `file_orphaned` check.** The spec defines
//!   `file_orphaned: true` as a hint after walking
//!   `~/.verbreel/projects-index` to confirm no other project's assets
//!   reference this hash. That walk needs file I/O and a multimap that
//!   this slice does not yet wire. For now `file_orphaned` is always
//!   `false` — a safe over-approximation (the spec also documents that
//!   the flag is informational, not a deletion guarantee).
//! - **Asset bytes deletion.** Per §3.4 the bytes are deliberately
//!   left on disk; standalone bytes deletion is `asset.gc` (§3.6).
//! - **`dry_run` preview.** Not part of the §3.4 args; the universal
//!   `dry_run` envelope (§0.5) will route through a future kernel hook
//!   rather than re-implement preview here.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::{AssetId, ClipId, ProjectId};

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// Internal warning code carrying the destructive data envelope.
pub const W_ASSET_REMOVE_ENVELOPE_CODE: &str = "W_ASSET_REMOVE_ENVELOPE";

/// Arguments for `asset.remove`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetRemoveArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Target asset id, as a bare `UUIDv7` string.
    pub asset_id: String,

    /// `true` deletes every clip whose `asset_id` matches. Defaults to
    /// `false` (refuse if any clip references the asset).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cascade: Option<bool>,
}

impl AssetRemoveArgs {
    fn cascade(&self) -> bool {
        self.cascade.unwrap_or(false)
    }
}

/// Envelope returned by `asset.remove`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetRemoveData {
    /// The asset id that was removed.
    pub removed_asset_id: AssetId,

    /// Clip ids removed by cascade, sorted by UUID string. Empty when
    /// `cascade=false` (default) or when no clips referenced the asset.
    pub removed_clip_ids: Vec<ClipId>,

    /// Whether the underlying content-addressed file is now orphaned
    /// across `~/.verbreel/projects-index`. Always `false` at this
    /// slice — the cross-project check is deferred (see module docs).
    pub file_orphaned: bool,
}

/// Verb-level validation failures for `asset.remove`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AssetRemoveError {
    /// `args.asset_id` is not parseable as `UUIDv7`.
    #[error("asset.remove: `asset_id` selector parse failed: {detail}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
    },

    /// No asset exists for `asset_id`.
    #[error("asset.remove: asset `{asset_id}` not found")]
    AssetNotFound {
        /// Missing asset id string.
        asset_id: String,
    },

    /// At least one clip references the asset and `cascade=false`.
    #[error(
        "asset.remove: asset `{asset_id}` is in use by {referencing_count} clip(s); pass cascade: true to also remove them"
    )]
    AssetInUse {
        /// Asset id that has live references.
        asset_id: String,
        /// Number of clips referencing the asset.
        referencing_count: usize,
    },
}

#[derive(Debug, Clone)]
struct ClipLocation {
    track_idx: usize,
    clip_idx: usize,
    clip_id: ClipId,
}

/// Build the RFC-6902 patch for `asset.remove`.
///
/// # Errors
///
/// Returns [`AssetRemoveError`] for bad selector, missing asset, or
/// in-use asset (default `cascade=false`).
pub fn compute_patch(
    prior: &Project,
    args: &AssetRemoveArgs,
) -> Result<(Value, Vec<Value>, AssetRemoveData), AssetRemoveError> {
    let asset_id =
        args.asset_id
            .parse::<AssetId>()
            .map_err(|err| AssetRemoveError::BadSelector {
                detail: err.to_string(),
            })?;

    let asset_idx = prior
        .assets
        .iter()
        .position(|asset| asset.id() == &asset_id)
        .ok_or_else(|| AssetRemoveError::AssetNotFound {
            asset_id: args.asset_id.clone(),
        })?;

    let referencing = referencing_clip_locations(prior, asset_id);

    if !referencing.is_empty() && !args.cascade() {
        return Err(AssetRemoveError::AssetInUse {
            asset_id: args.asset_id.clone(),
            referencing_count: referencing.len(),
        });
    }

    let removed_clip_ids = sort_ids(referencing.iter().map(|location| location.clip_id));

    let data = AssetRemoveData {
        removed_asset_id: asset_id,
        removed_clip_ids,
        // Deferred — see module docs.
        file_orphaned: false,
    };

    let mut ops = Vec::new();

    // Remove cascaded clips first, descending by `(clip_idx, track_idx)`
    // so each index in the patch stays stable as predecessors are
    // dropped. Mirror the descending-index pattern from clip.delete.
    let mut clip_removals: Vec<(usize, usize)> = referencing
        .iter()
        .map(|location| (location.track_idx, location.clip_idx))
        .collect();
    clip_removals.sort_by(|left, right| right.cmp(left));
    for (track_idx, clip_idx) in &clip_removals {
        ops.push(json!({
            "op": "remove",
            "path": format!("/tracks/{track_idx}/clips/{clip_idx}"),
        }));
    }

    ops.push(json!({
        "op": "remove",
        "path": format!("/assets/{asset_idx}"),
    }));

    let warnings = vec![envelope_warning(&data)];
    Ok((Value::Array(ops), warnings, data))
}

fn referencing_clip_locations(prior: &Project, asset_id: AssetId) -> Vec<ClipLocation> {
    let mut locations = Vec::new();
    let mut seen = HashSet::new();
    for (track_idx, track) in prior.tracks.iter().enumerate() {
        for (clip_idx, clip) in track.clips.iter().enumerate() {
            if clip.asset_id.id() == Some(&asset_id) && seen.insert(clip.id) {
                locations.push(ClipLocation {
                    track_idx,
                    clip_idx,
                    clip_id: clip.id,
                });
            }
        }
    }
    locations
}

fn envelope_warning(data: &AssetRemoveData) -> Value {
    json!({
        "code": W_ASSET_REMOVE_ENVELOPE_CODE,
        "message": "asset.remove envelope",
        "details": {
            "removed_asset_id": data.removed_asset_id.to_string(),
            "removed_clip_ids": stringify_ids(&data.removed_clip_ids),
            "file_orphaned": data.file_orphaned,
        }
    })
}

fn stringify_ids<T: ToString>(ids: &[T]) -> Vec<String> {
    ids.iter().map(ToString::to_string).collect()
}

fn sort_ids<T>(ids: impl IntoIterator<Item = T>) -> Vec<T>
where
    T: ToString,
{
    let mut ids: Vec<T> = ids.into_iter().collect();
    ids.sort_by_key(ToString::to_string);
    ids
}

/// Rebuild [`AssetRemoveData`] from recorded args and warnings.
///
/// # Errors
///
/// Returns [`ReconstructError`] if the internal envelope warning is
/// missing or malformed.
pub fn data_envelope_from_args_warnings(
    _args: &AssetRemoveArgs,
    warnings: &[Value],
) -> Result<AssetRemoveData, ReconstructError> {
    let details = envelope_details_from_warnings(warnings)?;
    Ok(AssetRemoveData {
        removed_asset_id: required_id(details, "removed_asset_id")?,
        removed_clip_ids: required_id_list(details, "removed_clip_ids")?,
        file_orphaned: required_bool(details, "file_orphaned")?,
    })
}

fn envelope_details_from_warnings(warnings: &[Value]) -> Result<&Value, ReconstructError> {
    for warning in warnings {
        if warning.get("code").and_then(Value::as_str) != Some(W_ASSET_REMOVE_ENVELOPE_CODE) {
            continue;
        }
        return warning
            .get("details")
            .ok_or(ReconstructError::MissingField {
                name: "warnings[].W_ASSET_REMOVE_ENVELOPE.details",
            });
    }

    Err(ReconstructError::MissingField {
        name: "warnings[].W_ASSET_REMOVE_ENVELOPE",
    })
}

fn required_id<T>(details: &Value, name: &'static str) -> Result<T, ReconstructError>
where
    T: std::str::FromStr,
{
    let raw = details
        .get(name)
        .ok_or(ReconstructError::MissingField { name })?
        .as_str()
        .ok_or(ReconstructError::TypeMismatch {
            name,
            expected: "UUIDv7 string",
        })?;
    raw.parse::<T>()
        .map_err(|_| ReconstructError::TypeMismatch {
            name,
            expected: "UUIDv7 string",
        })
}

fn required_id_list<T>(details: &Value, name: &'static str) -> Result<Vec<T>, ReconstructError>
where
    T: std::str::FromStr + ToString,
{
    let values = details
        .get(name)
        .ok_or(ReconstructError::MissingField { name })?
        .as_array()
        .ok_or(ReconstructError::TypeMismatch {
            name,
            expected: "array of UUIDv7 strings",
        })?;
    let ids = values
        .iter()
        .map(|value| {
            let raw = value.as_str().ok_or(ReconstructError::TypeMismatch {
                name,
                expected: "array of UUIDv7 strings",
            })?;
            raw.parse::<T>()
                .map_err(|_| ReconstructError::TypeMismatch {
                    name,
                    expected: "array of UUIDv7 strings",
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(sort_ids(ids))
}

fn required_bool(details: &Value, name: &'static str) -> Result<bool, ReconstructError> {
    details
        .get(name)
        .ok_or(ReconstructError::MissingField { name })?
        .as_bool()
        .ok_or(ReconstructError::TypeMismatch {
            name,
            expected: "bool",
        })
}

impl From<AssetRemoveError> for VerbError {
    fn from(value: AssetRemoveError) -> Self {
        match value {
            AssetRemoveError::BadSelector { .. }
            | AssetRemoveError::AssetNotFound { .. }
            | AssetRemoveError::AssetInUse { .. } => VerbError::BadArgs {
                detail: value.to_string(),
            },
        }
    }
}

/// The §0.8 verb for `asset.remove`.
#[derive(Debug, Default)]
pub struct AssetRemoveVerb;

impl Verb for AssetRemoveVerb {
    fn verb(&self) -> &'static str {
        "asset.remove"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: AssetRemoveArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("asset.remove: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!("asset.remove: patch construction failed: {err}"))
            })?;
        let data = serde_json::to_value(&data)
            .map_err(|err| VerbError::Custom(format!("asset.remove: data failed: {err}")))?;

        Ok((patch, data, warnings))
    }

    fn reconstruct(
        &self,
        args: &Value,
        _patch: &Value,
        warnings: &[Value],
        _post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let typed: AssetRemoveArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "AssetRemoveArgs",
            })?;

        let envelope = data_envelope_from_args_warnings(&typed, warnings)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
