//! `asset.import` (§3.1) — content-addressed import surface.
//!
//! The pure verb path (`compute_patch`) stays as a v1 floor because it
//! has no filesystem context. The native kernel path
//! (`ProjectStore::mutate_via_verb`) calls
//! [`compute_patch_with_root`] to wire real CAS writes.

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::ProjectId;

#[cfg(feature = "native")]
use crate::asset::{Asset, SubtitleAsset};
#[cfg(feature = "native")]
use crate::asset_meta::{FileFingerprint, SubtitleAssetMetadata};
#[cfg(feature = "native")]
use crate::newtypes::{AssetPath, Sha256};
use std::collections::HashMap;
#[cfg(feature = "native")]
use std::fs;
#[cfg(feature = "native")]
use std::path::Path;
#[cfg(feature = "native")]
use std::time::UNIX_EPOCH;
#[cfg(feature = "native")]
use verbreel_events::Timestamp;
#[cfg(feature = "native")]
use verbreel_storage::cas::key_for_bytes;
#[cfg(feature = "native")]
use verbreel_storage::fs::atomic_write_bytes;
#[cfg(feature = "native")]
use verbreel_types::AssetId;

/// Per-verb upper bound on the `paths` array.
pub const PATHS_MAX_BATCH: usize = 1000;

/// `E_SCHEMA_VIOLATION` hint when `paths.len() > PATHS_MAX_BATCH`.
pub const SCHEMA_VIOLATION_HINT: &str = "split the batch into smaller calls";

/// Asset import storage mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImportMode {
    /// Byte-copy source into project CAS.
    Copy,
    /// Hard-link request (currently resolves as copy in this slice).
    Link,
}

/// Arguments for `asset.import`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetImportArgs {
    /// Target project id.
    pub project_id: ProjectId,
    /// Source paths to import.
    pub paths: Vec<String>,
    /// Requested storage mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<ImportMode>,
    /// Soft-skip toggle (reserved for fuller batch behavior).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub soft: Option<bool>,
}

/// Success envelope for `asset.import`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetImportData {
    /// Imported assets in input order.
    pub assets: Vec<Value>,
    /// Resolved mode evidence in input order.
    pub modes_used: Vec<Value>,
    /// Soft-skipped paths.
    pub missing_paths: Vec<String>,
    /// Soft-skipped input indices.
    pub skipped_input_indices: Vec<i64>,
}

/// Verb-level error type for `asset.import`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AssetImportError {
    /// `paths.len() > PATHS_MAX_BATCH`.
    #[error(
        "asset.import: schema violation: field `{field}` exceeds maxItems \
         (actual: {actual_count}, max: {max_count}); {hint}"
    )]
    SchemaViolation {
        /// Violating field name.
        field: &'static str,
        /// Recovery hint.
        hint: &'static str,
        /// Caller-supplied count.
        actual_count: usize,
        /// Limit count.
        max_count: usize,
    },
    /// Input path did not exist.
    #[error("asset.import: path not found: {path}")]
    AssetPathNotFound {
        /// Missing path.
        path: String,
    },
    /// Source read or destination write failed.
    #[error("asset.import: io failure on `{path}`: {detail}")]
    Io {
        /// Path involved in the failure.
        path: String,
        /// Underlying error text.
        detail: String,
    },
    /// Produced patch/data violated internal expectations.
    #[error("asset.import: inconsistent patch/data: {detail}")]
    InconsistentPatch {
        /// Internal mismatch detail.
        detail: String,
    },
}

/// Build the empty-batch success envelope.
fn empty_data() -> AssetImportData {
    AssetImportData {
        assets: Vec::new(),
        modes_used: Vec::new(),
        missing_paths: Vec::new(),
        skipped_input_indices: Vec::new(),
    }
}

/// Pure v1 floor for `asset.import`.
///
/// This helper keeps the legacy behavior for callsites that do not
/// provide project-root filesystem context.
///
/// # Errors
///
/// Returns [`AssetImportError::SchemaViolation`] when `paths` exceeds
/// the cap, and [`AssetImportError::AssetPathNotFound`] for non-empty
/// calls.
pub fn import(args: &AssetImportArgs) -> Result<AssetImportData, AssetImportError> {
    if args.paths.len() > PATHS_MAX_BATCH {
        return Err(AssetImportError::SchemaViolation {
            field: "paths",
            hint: SCHEMA_VIOLATION_HINT,
            actual_count: args.paths.len(),
            max_count: PATHS_MAX_BATCH,
        });
    }
    if args.paths.is_empty() {
        return Ok(empty_data());
    }
    Err(AssetImportError::AssetPathNotFound {
        path: args.paths[0].clone(),
    })
}

/// Build the RFC 6902 patch for `asset.import`.
///
/// Pure fallback path with no filesystem context.
///
/// # Errors
///
/// Forwards [`AssetImportError`] from [`import`].
pub fn compute_patch(
    _prior: &Project,
    args: &AssetImportArgs,
) -> Result<(Value, Vec<Value>, AssetImportData), AssetImportError> {
    let data = import(args)?;
    Ok((json!([]), Vec::new(), data))
}

#[cfg(feature = "native")]
fn canonical_ext(path: &Path) -> String {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .filter(|ext| {
            !ext.is_empty()
                && ext
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        })
        .unwrap_or_else(|| "bin".to_string())
}

#[cfg(feature = "native")]
fn fingerprint_for(path: &Path) -> Result<FileFingerprint, AssetImportError> {
    let md = fs::metadata(path).map_err(|e| AssetImportError::Io {
        path: path.display().to_string(),
        detail: e.to_string(),
    })?;
    let mtime_ms = md
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| {
            u64::try_from(d.as_millis().min(u128::from(u64::MAX)))
                .expect("value clamped to u64::MAX")
        });
    Ok(FileFingerprint {
        mtime_ms,
        size_bytes: md.len(),
    })
}

#[cfg(feature = "native")]
fn build_subtitle_asset(
    source_path: &Path,
    sha256_hex: &str,
    cas_rel_path: &str,
    ext: &str,
) -> Result<Asset, AssetImportError> {
    let hash =
        Sha256::new(sha256_hex.to_string()).map_err(|e| AssetImportError::InconsistentPatch {
            detail: e.to_string(),
        })?;
    let path = AssetPath::new(cas_rel_path.to_string()).map_err(|e| {
        AssetImportError::InconsistentPatch {
            detail: e.to_string(),
        }
    })?;
    let fingerprint = fingerprint_for(source_path)?;

    Ok(Asset::Subtitle(SubtitleAsset {
        id: AssetId::now(),
        hash,
        path,
        original_filename: source_path.file_name().map_or_else(
            || source_path.display().to_string(),
            |n| n.to_string_lossy().to_string(),
        ),
        // Placeholder epoch — real import time is wired in a follow-up.
        // Constructed through the typed boundary so even the placeholder
        // is validated RFC 3339, not a raw string literal.
        imported_at: Timestamp::parse("1970-01-01T00:00:00Z")
            .expect("epoch literal is valid RFC 3339"),
        metadata: SubtitleAssetMetadata {
            container: ext.to_string(),
            language: None,
            segment_count: None,
            fingerprint,
        },
    }))
}

fn mode_used_for(_args: &AssetImportArgs) -> &'static str {
    "copy"
}

/// Native import path with project-root filesystem context.
///
/// Invariant repaired here: non-empty `paths` now write source bytes to
/// CAS (`assets/<aa>/<sha256>.<ext>`) using
/// `verbreel_storage::cas::key_for_bytes`, and patch-add a
/// corresponding `Asset` record carrying canonical hash/path fields.
///
/// # Errors
///
/// Returns schema violations, missing-path errors, and I/O errors.
#[cfg(feature = "native")]
pub fn compute_patch_with_root(
    _prior: &Project,
    args: &AssetImportArgs,
    project_root: &Path,
) -> Result<(Value, Vec<Value>, AssetImportData), AssetImportError> {
    if args.paths.len() > PATHS_MAX_BATCH {
        return Err(AssetImportError::SchemaViolation {
            field: "paths",
            hint: SCHEMA_VIOLATION_HINT,
            actual_count: args.paths.len(),
            max_count: PATHS_MAX_BATCH,
        });
    }
    if args.paths.is_empty() {
        return Ok((json!([]), Vec::new(), empty_data()));
    }

    let mut patch_ops = Vec::with_capacity(args.paths.len());
    let mut assets = Vec::with_capacity(args.paths.len());
    let mut modes_used = Vec::with_capacity(args.paths.len());

    for input_path in &args.paths {
        let src = Path::new(input_path);
        let bytes = fs::read(src).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                AssetImportError::AssetPathNotFound {
                    path: input_path.clone(),
                }
            } else {
                AssetImportError::Io {
                    path: input_path.clone(),
                    detail: e.to_string(),
                }
            }
        })?;

        let ext = canonical_ext(src);
        let key = key_for_bytes(&bytes, &ext).map_err(|e| AssetImportError::InconsistentPatch {
            detail: e.to_string(),
        })?;
        let dst = project_root.join(&key.relative_path);
        if dst.exists() {
            let existing = fs::read(&dst).map_err(|e| AssetImportError::Io {
                path: dst.display().to_string(),
                detail: e.to_string(),
            })?;
            if existing != bytes {
                return Err(AssetImportError::Io {
                    path: dst.display().to_string(),
                    detail: "existing CAS object contents do not match expected hash".to_string(),
                });
            }
        } else {
            if let Some(parent) = dst.parent() {
                fs::create_dir_all(parent).map_err(|e| AssetImportError::Io {
                    path: parent.display().to_string(),
                    detail: e.to_string(),
                })?;
            }
            atomic_write_bytes(&dst, &bytes).map_err(|e| AssetImportError::Io {
                path: dst.display().to_string(),
                detail: e.to_string(),
            })?;
        }

        let asset = build_subtitle_asset(src, &key.sha256_hex, &key.relative_path, &ext)?;
        let asset_value =
            serde_json::to_value(&asset).map_err(|e| AssetImportError::InconsistentPatch {
                detail: e.to_string(),
            })?;
        patch_ops.push(json!({
            "op": "add",
            "path": "/assets/-",
            "value": asset_value.clone(),
        }));
        assets.push(asset_value);
        modes_used.push(json!({
            "asset_id": asset.id().to_string(),
            "mode_used": mode_used_for(args),
            "input_path": input_path,
        }));
    }

    Ok((
        Value::Array(patch_ops),
        Vec::new(),
        AssetImportData {
            assets,
            modes_used,
            missing_paths: Vec::new(),
            skipped_input_indices: Vec::new(),
        },
    ))
}

fn asset_ids_from_patch(patch: &Value) -> Result<Vec<String>, ReconstructError> {
    let ops = patch.as_array().ok_or_else(|| {
        ReconstructError::Custom("asset.import: patch must be an array".to_string())
    })?;
    let mut ids = Vec::with_capacity(ops.len());
    for op in ops {
        let op_path = op.get("path").and_then(Value::as_str).unwrap_or_default();
        let op_kind = op.get("op").and_then(Value::as_str).unwrap_or_default();
        if op_kind != "add" || op_path != "/assets/-" {
            return Err(ReconstructError::Custom(
                "asset.import: patch op must be `add /assets/-`".to_string(),
            ));
        }
        let id = op
            .get("value")
            .and_then(|v| v.get("id"))
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ReconstructError::Custom("asset.import: patch asset missing id".to_string())
            })?;
        ids.push(id.to_string());
    }
    Ok(ids)
}

fn replay_data_from_patch(
    args: &AssetImportArgs,
    patch: &Value,
    post_state: &Project,
) -> Result<AssetImportData, ReconstructError> {
    let ids = asset_ids_from_patch(patch)?;
    if ids.len() != args.paths.len() {
        return Err(ReconstructError::Custom(format!(
            "asset.import: patch assets count {} does not match args.paths count {}",
            ids.len(),
            args.paths.len()
        )));
    }

    let mut by_id: HashMap<String, Value> = HashMap::new();
    for asset in &post_state.assets {
        let value =
            serde_json::to_value(asset).map_err(|e| ReconstructError::Custom(e.to_string()))?;
        by_id.insert(asset.id().to_string(), value);
    }

    let mut assets = Vec::with_capacity(ids.len());
    let mut modes_used = Vec::with_capacity(ids.len());
    for (idx, id) in ids.iter().enumerate() {
        let asset = by_id.get(id).ok_or_else(|| {
            ReconstructError::Custom(format!(
                "asset.import: post_state missing asset id from patch: {id}"
            ))
        })?;
        assets.push(asset.clone());
        modes_used.push(json!({
            "asset_id": id,
            "mode_used": mode_used_for(args),
            "input_path": args.paths[idx],
        }));
    }

    Ok(AssetImportData {
        assets,
        modes_used,
        missing_paths: Vec::new(),
        skipped_input_indices: Vec::new(),
    })
}

/// Reconstruct the data envelope from `(args, post_state)`.
///
/// Pure fallback path uses [`compute_patch`] and is valid for the
/// no-root floor behavior.
///
/// # Errors
///
/// Reuses [`compute_patch`], mapped into [`ReconstructError`].
pub fn data_envelope_from_args(
    args: &AssetImportArgs,
    post_state: &Project,
) -> Result<AssetImportData, ReconstructError> {
    let (_, _, data) =
        compute_patch(post_state, args).map_err(|e| ReconstructError::Custom(e.to_string()))?;
    Ok(data)
}

impl From<AssetImportError> for VerbError {
    fn from(value: AssetImportError) -> Self {
        match value {
            AssetImportError::SchemaViolation { .. } => VerbError::BadArgs {
                detail: value.to_string(),
            },
            AssetImportError::AssetPathNotFound { .. }
            | AssetImportError::Io { .. }
            | AssetImportError::InconsistentPatch { .. } => VerbError::Custom(value.to_string()),
        }
    }
}

/// The §0.8 verb for `asset.import`.
#[derive(Debug, Default)]
pub struct AssetImportVerb;

impl Verb for AssetImportVerb {
    fn verb(&self) -> &'static str {
        "asset.import"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: AssetImportArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("asset.import: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch = serde_json::from_value(patch_value).map_err(|err| {
            VerbError::Custom(format!("asset.import: patch construction failed: {err}"))
        })?;

        let data = serde_json::to_value(&data).map_err(|err| {
            VerbError::Custom(format!("asset.import: data envelope failed: {err}"))
        })?;

        Ok((patch, data, warnings))
    }

    fn reconstruct(
        &self,
        args: &Value,
        patch: &Value,
        _warnings: &[Value],
        post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let typed: AssetImportArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "AssetImportArgs",
            })?;

        let envelope = replay_data_from_patch(&typed, patch, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
