//! `asset.import` (§3.1) — eighty-fourth production verb. Third slice
//! of the asset arc; v1 floor implements arg-shape validation and the
//! spec's documented empty-batch no-op path.
//!
//! ## Spec quote (`spec/commands/asset.md` §3.1, abbreviated)
//!
//! > Copies (or links) a file into the project's content-addressed
//! > `assets/` store and probes it with ffprobe.
//! >
//! > CLI: `verbreel asset import [--project <id>] --paths <path>...
//! >   [--mode <copy|link>] [--soft]`
//! > MCP: `asset.import`
//! > Args: `project_id: string`, `paths: string[]` (`maxItems: 1000`),
//! >   `mode?: "copy" | "link"` (default `"copy"`),
//! >   `soft?: boolean` (default `false`).
//! > Returns (`data`): `{ assets: Asset[], modes_used: ImportMode[],
//! >   missing_paths: string[], skipped_input_indices: integer[] }`.
//! > Errors: `E_ASSET_PATH_NOT_FOUND`, `E_ASSET_UNSUPPORTED_KIND`,
//! >   `E_ASSET_UNREADABLE`, `E_ASSET_PROBE_TIMEOUT`, `E_IO`,
//! >   `E_SCHEMA_VIOLATION` (when `paths` exceeds `maxItems: 1000`).
//! > Warnings: `W_ASSET_DUPLICATE_HASH`, `W_ASSET_MODE_FALLBACK`.
//! >
//! > An empty array (`[]`) is a successful no-op (`patch: []`,
//! > `assets: []`, `warnings: []`).
//!
//! ## v1 floor — real file I/O deferred
//!
//! The content-addressed importer (ffprobe + hash + write into
//! `assets/<aa>/<sha256>.<ext>`) is the same architectural gap the
//! render.queue arc and `project.forget` (§2.8) defer behind their own
//! v1 floors: the [`crate::reconstructor::Verb`] trait's purity
//! contract forbids file I/O in `compute_patch`. Wiring real I/O needs
//! a `VerbContext` / storage facade threaded through
//! `ProjectStore::mutate_via_verb`. A future slice introduces that
//! plumbing and replaces the v1 floor here in one go.
//!
//! What this verb DOES implement in v1:
//!
//! 1. The **`maxItems: 1000` schema check** — `paths.len() > 1000`
//!    surfaces as [`AssetImportError::SchemaViolation`] (mapped to
//!    `E_SCHEMA_VIOLATION` per §0.8's per-verb upper-bound convention).
//! 2. The **empty-batch no-op** — `paths.is_empty()` returns the spec's
//!    documented `data` envelope with every array empty. This path is
//!    a successful call: no patch, no warnings, no error.
//! 3. The **non-empty rejection** — any `paths.len() >= 1` returns
//!    [`AssetImportError::AssetPathNotFound`] naming the first input
//!    path. v1 has no way to read the file, so the spec-listed
//!    `E_ASSET_PATH_NOT_FOUND` is the truthful answer (the path is, by
//!    construction, not reachable through this engine build).

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::ProjectId;

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// Per-verb upper bound on the `paths` array, per §0.8's per-verb
/// `maxItems` convention echoed in §3.1.
pub const PATHS_MAX_BATCH: usize = 1000;

/// `E_SCHEMA_VIOLATION` recovery hint surfaced when `paths.len() >
/// PATHS_MAX_BATCH`. Verbatim from §3.1's `details.hint`.
pub const SCHEMA_VIOLATION_HINT: &str = "split the batch into smaller calls";

/// Asset import storage mode — the `mode` argument's two values.
///
/// Spec says `mode?: "copy" | "link"` with default `"copy"`. At v1
/// the default is resolved by the (deferred) real importer; the args
/// struct keeps it as `Option<ImportMode>` so the wire-shape exactly
/// matches the spec (omitted → null).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImportMode {
    /// Read the source through any symlink chain and write the bytes
    /// to the project's content-addressed store.
    Copy,
    /// Hard-link the resolved source inode into the project's
    /// content-addressed store; silently falls back to `Copy` when the
    /// OS rejects the link (cross-filesystem, Windows ACL, etc.).
    Link,
}

/// Arguments for `asset.import`.
///
/// `deny_unknown_fields` enforces the §3.1 published surface — stray
/// MCP/HTTP payload keys surface as arg-shape errors via the `Verb`
/// trait's `BadArgs` route rather than being silently accepted.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetImportArgs {
    /// Target project id.
    pub project_id: ProjectId,
    /// Per-call list of source paths to import. Empty array is a
    /// successful no-op (spec); >1000 entries triggers
    /// [`AssetImportError::SchemaViolation`].
    pub paths: Vec<String>,
    /// Storage mode. Omitted → `None` (default `Copy` is resolved by
    /// the real importer, which is deferred at v1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<ImportMode>,
    /// Per §0.10: when `Some(true)`, missing paths downgrade from
    /// error to `W_NOOP` warnings instead of aborting the batch.
    /// Default `None` (treated as `false` by the deferred real
    /// importer).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub soft: Option<bool>,
}

/// Envelope returned by a successful `asset.import` call.
///
/// At v1 every successful call is the empty-paths no-op, so all four
/// arrays are empty in the only success branch. The field types
/// (`Vec<Value>` for `assets` and `modes_used`) keep the wire shape
/// open for the slice that ships the real importer without forcing
/// downstream rebuilds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetImportData {
    /// One [`crate::Asset`] record per successful input path, in input
    /// order. Empty at v1 — the empty-paths case is the only success.
    pub assets: Vec<Value>,
    /// One `{ asset_id, mode_used, fallback_reason?, input_path }`
    /// entry per successful input path, in input order. Empty at v1.
    pub modes_used: Vec<Value>,
    /// Under `--soft`, paths that failed with
    /// `E_ASSET_PATH_NOT_FOUND` / `E_ASSET_UNREADABLE` /
    /// `E_ASSET_PROBE_TIMEOUT`. Empty at v1.
    pub missing_paths: Vec<String>,
    /// Parallel to `missing_paths` — the zero-based input indices that
    /// soft-skipped. Empty at v1.
    pub skipped_input_indices: Vec<i64>,
}

/// Verb-level error type for `asset.import`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AssetImportError {
    /// `paths.len() > PATHS_MAX_BATCH`. Maps to `E_SCHEMA_VIOLATION`
    /// with `details.field: "paths"`, `details.hint: "split the batch
    /// into smaller calls"`, and the actual/max counts for telemetry.
    #[error(
        "asset.import: schema violation: field `{field}` exceeds maxItems \
         (actual: {actual_count}, max: {max_count}); {hint}"
    )]
    SchemaViolation {
        /// JSON Pointer name of the violating field (always `"paths"`
        /// in v1).
        field: &'static str,
        /// Static recovery hint surfaced as `details.hint`.
        hint: &'static str,
        /// Caller-supplied count (so the agent sees how far past the
        /// cap the batch was).
        actual_count: usize,
        /// The cap itself (constant, but echoed for hint-rendering).
        max_count: usize,
    },
    /// v1 floor: any non-empty `paths` resolves here because no file
    /// I/O happens at the verb layer. Maps to `E_ASSET_PATH_NOT_FOUND`.
    #[error("asset.import: path not found: {path}")]
    AssetPathNotFound {
        /// The input path (verbatim from `paths[0]`).
        path: String,
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

/// Import zero or more source paths into the project's
/// content-addressed `assets/` store.
///
/// v1 floor — see the module-level doc. Validation order:
///
/// 1. `paths.len() > PATHS_MAX_BATCH` → [`AssetImportError::SchemaViolation`].
/// 2. `paths.is_empty()` → spec's documented no-op success.
/// 3. `paths.len() >= 1` → [`AssetImportError::AssetPathNotFound`] for
///    the first input path.
///
/// # Errors
///
/// See the variants of [`AssetImportError`].
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
/// At v1 the only success path is the empty-paths no-op: patch `[]`,
/// no warnings, the empty-batch envelope. Non-empty calls return
/// `Err` and the patch is moot.
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

/// Reconstruct the data envelope from `(args, post_state)`.
///
/// Reuses [`compute_patch`], so reconstruction can only fail for the
/// same reasons the forward call would — which means: if the recorded
/// event is the empty-paths no-op (the only success branch in v1), the
/// envelope rebuilds; if the recorded event is anything else, the
/// forward call didn't succeed in the first place and the recorded
/// tuple is invalid.
///
/// # Errors
///
/// Reuses [`compute_patch`], so this can only return reconstruction
/// errors introduced while rebuilding the deterministic envelope.
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
            // Arg-shape rejection (per-verb `maxItems` is part of the
            // args schema) → BadArgs, mirroring the
            // render_queue_clear confirm-gate mapping.
            AssetImportError::SchemaViolation { .. } => VerbError::BadArgs {
                detail: value.to_string(),
            },
            // Runtime-classed rejection (the path "exists" in the
            // caller's view but the engine cannot resolve it) →
            // Custom, mirroring how queue/render verbs surface their
            // not-found cases.
            AssetImportError::AssetPathNotFound { .. } => VerbError::Custom(value.to_string()),
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
        _patch: &Value,
        _warnings: &[Value],
        post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let typed: AssetImportArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "AssetImportArgs",
            })?;

        let envelope = data_envelope_from_args(&typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
