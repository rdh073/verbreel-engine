//! `asset.gc` (§3.6) — eighty-seventh production verb in the engine.
//!
//! ## Spec quote (`spec/commands/asset.md` §3.6, abbreviated)
//!
//! > Deletes orphaned files under `assets/` — files on disk that no
//! > `Asset` record (in any in-scope project) refers to.
//! > CLI: `verbreel asset gc [--project <id>] [--global]
//! >   [--suppress_orphan_risk]`
//! > MCP: `asset.gc`
//! > Args: `project_id?: string, global?: boolean,
//! >   suppress_orphan_risk?: boolean` (default `false`).
//! > Returns (`data`): `{ removed_paths: string[]; freed_bytes: integer }`.
//! > Errors: `E_IO`, `E_GC_NOT_ALLOWED`, `E_ARGS_INCOMPATIBLE`
//! >   (both `project_id` and `global: true` supplied),
//! >   `E_PROJECT_NOT_FOUND` (no scope resolvable).
//! > Warnings: `W_GC_ORPHAN_RISK` (only with `--global`; suppressible).
//!
//! ## v1 floor — filesystem traversal deferred.
//!
//! Real GC requires walking `<project>/assets/<aa>/` and computing
//! `set-difference(file_hashes, Project.assets[].hash)`, then unlinking
//! orphans. That work is filesystem-bound and forbidden in
//! `Verb::compute_patch` (the §0.8 purity contract). v1 always reports
//! `{ removed_paths: [], freed_bytes: 0 }`. Same architectural deferral
//! pattern that `render.queue.list` (§21.2), `render.queue.clear`
//! (§21.5), `tracker.list` cache-stat, `font.list` system-fonts, and
//! every other v1-floor verb in the eighty-seven-verb arc applies for
//! its corresponding side effect. A future slice introduces
//! `VerbContext` (storage facade threaded through
//! `ProjectStore::mutate_via_verb`) and wires the actual traversal
//! here.
//!
//! What this verb DOES implement in v1 is the **arg-shape validation
//! layer** per spec:
//!
//! - both `project_id` and `global: true` → `ArgsIncompatible`
//!   (mutually exclusive scopes).
//! - neither scope (MCP/HTTP path, where the active-project inference
//!   from §0.12 does not apply) → `ProjectNotFound` with the spec hint.
//! - `global: true` only → `GcNotAllowed` (v1 assumes engine config
//!   blocks global GC; config-reading I/O happens nowhere in
//!   `compute_patch`).
//! - `project_id: Some(_)` only → empty data envelope, no warnings.
//!
//! ## `W_GC_ORPHAN_RISK` not emitted in v1.
//!
//! The spec emits `W_GC_ORPHAN_RISK` only on the `--global` path; v1
//! refuses that path with `GcNotAllowed` before reaching warning-emit
//! territory, so the warning code is not constructed here. It will land
//! together with the global-GC wiring slice.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::ProjectId;

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// Recovery hint emitted on the neither-scope path. Verbatim from §3.6.
pub const NEITHER_HINT: &str = "supply project_id for single-project gc, or global: true for \
                                cross-project gc (requires engine config allow_global_gc)";

/// Arguments for `asset.gc`.
///
/// `deny_unknown_fields` enforces the §3.6 published surface: stray
/// MCP/HTTP payload keys surface as arg-shape errors rather than being
/// silently accepted.
///
/// Per §3.6 both `project_id` and `global` are optional, but exactly
/// one must resolve a scope; the cross-validation lives in
/// [`compute_patch`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetGcArgs {
    /// Single-project scope. Mutually exclusive with `global: true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<ProjectId>,
    /// Cross-project scope. Requires engine config `allow_global_gc` —
    /// v1 always refuses with `GcNotAllowed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub global: Option<bool>,
    /// Suppress `W_GC_ORPHAN_RISK` on the `--global` path. v1 never
    /// emits the warning (the `--global` path errors before warning
    /// emit), so this field is currently a no-op preserved for
    /// forward-compatibility with the global-GC wiring slice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suppress_orphan_risk: Option<bool>,
}

/// Envelope returned by `asset.gc`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetGcData {
    /// Paths of files actually unlinked, relative to the project root
    /// (e.g. `"assets/aa/aa…ee.mp4"`). Always empty in v1.
    pub removed_paths: Vec<String>,
    /// Total bytes reclaimed across `removed_paths`. Always 0 in v1.
    pub freed_bytes: u64,
}

#[allow(dead_code)]
#[derive(Debug, Error)]
/// Verb-level error type for `asset.gc`.
pub enum AssetGcError {
    /// Both `project_id` and `global: true` were supplied. Maps to
    /// `E_ARGS_INCOMPATIBLE`.
    #[error("asset.gc: args incompatible: {detail}")]
    ArgsIncompatible {
        /// Static hint identifying the incompatibility.
        detail: &'static str,
    },

    /// Neither `project_id` nor `global: true` resolved a scope. Maps
    /// to `E_PROJECT_NOT_FOUND` per §3.6's "neither flag supplied"
    /// rule for the MCP/HTTP transports (CLI active-project inference
    /// is layered above this verb per §0.12).
    #[error("asset.gc: no scope resolvable; {hint}")]
    ProjectNotFound {
        /// Recovery hint surfaced verbatim from §3.6.
        hint: &'static str,
    },

    /// `global: true` supplied without engine config `allow_global_gc`
    /// being true. Maps to `E_GC_NOT_ALLOWED`. v1 always refuses
    /// global GC (the config read is deferred together with the
    /// traversal itself).
    #[error("asset.gc: global gc refused; engine config does not set allow_global_gc")]
    GcNotAllowed,

    /// Filesystem failure during the (deferred) GC traversal. Maps to
    /// `E_IO`. Not constructed in v1; declared so the slice that wires
    /// real GC has stable surface.
    #[allow(dead_code)]
    #[error("asset.gc: I/O failure: {0}")]
    Io(#[from] std::io::Error),
}

/// Build the canonical `asset.gc` data envelope (always empty in v1).
fn build_data() -> AssetGcData {
    AssetGcData {
        removed_paths: Vec::new(),
        freed_bytes: 0,
    }
}

/// Build the RFC 6902 patch for `asset.gc`.
///
/// The patch is always empty: `asset.gc` deletes filesystem entries,
/// not `project.json` records. Validation is performed here against
/// the (`project_id`, `global`) scope pair.
///
/// # Errors
///
/// - [`AssetGcError::ArgsIncompatible`] — both `project_id` and
///   `global: true` were supplied.
/// - [`AssetGcError::ProjectNotFound`] — no scope resolved.
/// - [`AssetGcError::GcNotAllowed`] — `global: true` only (v1
///   refuses).
pub fn compute_patch(
    _prior: &Project,
    args: &AssetGcArgs,
) -> Result<(Value, Vec<Value>, AssetGcData), AssetGcError> {
    let global = args.global.unwrap_or(false);
    match (args.project_id.is_some(), global) {
        (true, true) => Err(AssetGcError::ArgsIncompatible {
            detail: "`project_id` and `global: true` are mutually exclusive",
        }),
        (false, false) => Err(AssetGcError::ProjectNotFound { hint: NEITHER_HINT }),
        (false, true) => Err(AssetGcError::GcNotAllowed),
        (true, false) => Ok((json!([]), Vec::new(), build_data())),
    }
}

/// Build the data envelope from `(args, post_state)`.
///
/// # Errors
///
/// Reuses [`compute_patch`], so this can only return reconstruction
/// errors introduced while rebuilding the deterministic envelope.
pub fn data_envelope_from_args(
    args: &AssetGcArgs,
    post_state: &Project,
) -> Result<AssetGcData, ReconstructError> {
    let (_, _, data) =
        compute_patch(post_state, args).map_err(|e| ReconstructError::Custom(e.to_string()))?;
    Ok(data)
}

impl From<AssetGcError> for VerbError {
    fn from(value: AssetGcError) -> Self {
        match value {
            AssetGcError::ArgsIncompatible { .. }
            | AssetGcError::ProjectNotFound { .. }
            | AssetGcError::GcNotAllowed => VerbError::BadArgs {
                detail: value.to_string(),
            },
            AssetGcError::Io(_) => VerbError::Custom(value.to_string()),
        }
    }
}

/// The §0.8 verb for `asset.gc`.
#[derive(Debug, Default)]
pub struct AssetGcVerb;

impl Verb for AssetGcVerb {
    fn verb(&self) -> &'static str {
        "asset.gc"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: AssetGcArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("asset.gc: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch = serde_json::from_value(patch_value).map_err(|err| {
            VerbError::Custom(format!("asset.gc: patch construction failed: {err}"))
        })?;

        let data = serde_json::to_value(&data)
            .map_err(|err| VerbError::Custom(format!("asset.gc: data envelope failed: {err}")))?;

        Ok((patch, data, warnings))
    }

    fn reconstruct(
        &self,
        args: &Value,
        _patch: &Value,
        _warnings: &[Value],
        post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let typed: AssetGcArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "AssetGcArgs",
            })?;

        let envelope = data_envelope_from_args(&typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
