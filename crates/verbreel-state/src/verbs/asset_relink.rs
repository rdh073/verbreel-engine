//! `asset.relink` (§3.5) — eighty-sixth production verb in the engine.
//!
//! ## Spec quote (`spec/commands/asset.md` §3.5, verbatim)
//!
//! > CLI: `verbreel asset relink [--project <id>] --asset_id <id>
//! >   --source_path <path> [--mode <copy|link>]`
//! > MCP: `asset.relink`
//! > Args: `project_id: string`, `asset_id: string`, `source_path:
//! >   string`, `mode?: "copy"|"link"` (default `"copy"`).
//! > Returns (`data`): `{ asset_id, old_resolved_path,
//! >   new_resolved_path, new_fingerprint: { mtime_ms, size_bytes },
//! >   mode_used, fallback_reason? }`.
//! > Errors: `E_ASSET_NOT_FOUND`, `E_ASSET_HASH_MISMATCH`,
//! >   `E_ASSET_PATH_NOT_FOUND`, `E_ASSET_UNREADABLE`, `E_IO`.
//! > Warnings: `W_ASSET_MODE_FALLBACK`.
//!
//! ## v1 floor — always errors with `E_ASSET_PATH_NOT_FOUND`.
//!
//! Repointing an asset requires reading the supplied `source_path`
//! from disk (computing SHA-256 to verify it matches the recorded
//! `Asset.hash`, stat-ing for the new fingerprint, optionally
//! hard-linking into the content-addressed store). All four are file
//! I/O — forbidden in the pure `Verb::compute_patch`. The same
//! `VerbContext` / storage facade plumbing that `asset.import` (§3.1)
//! and `asset.probe` (§3.3) defer is required here. Until that lands,
//! the verb's `compute_patch` cannot read the path, so every
//! well-formed call returns `E_ASSET_PATH_NOT_FOUND` carrying the
//! supplied `source_path` as `details.path`. The remaining four
//! spec'd error codes (`E_ASSET_NOT_FOUND`, `E_ASSET_HASH_MISMATCH`,
//! `E_ASSET_UNREADABLE`, `E_IO`) and the `W_ASSET_MODE_FALLBACK`
//! warning are declared in the spec surface but unreachable in v1 —
//! they light up alongside the file I/O work.
//!
//! ## `project_id` accommodation (spec lists it as a required arg).
//!
//! The spec quote above lists `project_id` as a required arg, so
//! `AssetRelinkArgs::project_id` is a required `ProjectId` (not an
//! `Option`). This also matches the kernel dispatch shape: the
//! `ProjectStore::mutate_via_verb` path resolves a `prior` project
//! from the args envelope before calling `Verb::compute_patch`. The
//! lighter `Option<ProjectId>` accommodation used by truly
//! project-less read-only metadata verbs (`render.list_presets`,
//! `font.list`, `list_capabilities`, `stock.list_providers`) is
//! reserved for verbs whose returned shape is purely engine-bundled
//! metadata; `asset.relink` is asset-shaped, not metadata-shaped, so
//! it follows the asset.probe precedent and keeps `project_id`
//! required.
//!
//! ## Reconstructor framing for an always-errors verb.
//!
//! `compute_patch` always returns `Err`, which means no successful
//! event is ever appended to `events.jsonl` (the §0.8 write-ordering
//! rule requires a successful patch before an event is written). The
//! reconstruct path is therefore unreachable in production v1. It
//! still has to clear the §0.8 startup gate against the fixture in
//! `default_fixtures()`, so the implementation deserializes the args
//! (the only round-trip the recorded tuple can support) and returns
//! `Value::Null` — the truthful "no data was ever recorded for this
//! verb in v1" envelope. The matching fixture records
//! `expected_data: null` so the gate's canonical-SHA equality holds
//! by construction.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use verbreel_types::ProjectId;

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// Storage mode for `asset.relink` — the `mode` argument's two
/// values. Spec §3.5 says `mode?: "copy" | "link"` with default
/// `"copy"`.
///
/// The default is resolved by the (deferred) real file-I/O slice; the
/// args struct keeps it as `Option<AssetMode>` so the wire-shape
/// exactly matches the spec (omitted → null at v1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AssetMode {
    /// Read the source through any symlink chain and write the bytes
    /// to the project's content-addressed store.
    Copy,
    /// Hard-link the resolved source inode into the project's
    /// content-addressed store; silently falls back to `Copy` when
    /// the OS rejects the link (cross-filesystem, Windows ACL, etc.).
    Link,
}

/// Arguments for `asset.relink`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetRelinkArgs {
    /// Target project id. Required by the spec § 3.5 args row and by
    /// the `Verb` trait shape (kernel dispatch resolves `prior` from
    /// this); ignored by the v1 floor impl which errors before
    /// reading project state.
    pub project_id: ProjectId,
    /// Id of the asset whose source is being repointed. v1 floor:
    /// never looked up — the verb errors before consulting
    /// `Project.assets`.
    pub asset_id: String,
    /// The new on-disk path the asset should be repointed at. v1
    /// floor: never read; surfaced back to the caller as
    /// `details.path` in the `E_ASSET_PATH_NOT_FOUND` error.
    pub source_path: String,
    /// Storage mode. Omitted → `None` at args layer; default `Copy`
    /// is resolved by the deferred real file-I/O slice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<AssetMode>,
}

/// Fingerprint envelope returned in [`AssetRelinkData`]. Spec §3.5
/// says `new_fingerprint: { mtime_ms: integer; size_bytes: integer }`.
///
/// Unreachable in v1 (every call errors), but the type is defined
/// here so downstream consumers (CLI, MCP) can pin against the spec'd
/// shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetRelinkFingerprint {
    /// Last-modified-time of the new source file in milliseconds
    /// since the Unix epoch (matches `FileFingerprint.mtime_ms`).
    pub mtime_ms: i64,
    /// Size of the new source file in bytes (matches
    /// `FileFingerprint.size_bytes`).
    pub size_bytes: i64,
}

/// Response envelope for a successful `asset.relink`.
///
/// v1 floor never constructs this shape (every call errors), but the
/// type is defined here so downstream consumers (CLI, MCP) can pin
/// against the spec'd response shape from §3.5.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetRelinkData {
    /// The asset id that was repointed — echoes the input arg.
    pub asset_id: String,
    /// The prior `Asset.path` (project-relative content-addressed
    /// `AssetPath` form `assets/<aa>/<sha256>.<ext>`).
    pub old_resolved_path: String,
    /// The new `Asset.path` (project-relative content-addressed
    /// `AssetPath` form). May equal `old_resolved_path` when the
    /// relink does not change the storage location (same hash, same
    /// prefix, same extension).
    pub new_resolved_path: String,
    /// Stat-derived fingerprint of the new source file. Replaces the
    /// prior `Asset.metadata.fingerprint`.
    pub new_fingerprint: AssetRelinkFingerprint,
    /// Resolved storage mode — includes silent fallback from `Link`
    /// to `Copy` (same fallback rule as §3.1).
    pub mode_used: AssetMode,
    /// Present iff a `Link` → `Copy` fallback fired. Mirrors the
    /// optional `fallback_reason?` field in the spec's return row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_reason: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Error)]
/// Verb-level error type for `asset.relink`.
pub enum AssetRelinkError {
    /// `source_path` does not resolve to a readable file on disk.
    /// Maps to `E_ASSET_PATH_NOT_FOUND`. In v1 floor this is returned
    /// for every well-formed call regardless of the path supplied.
    #[error(
        "asset.relink: E_ASSET_PATH_NOT_FOUND — source_path `{source_path}` does not resolve to a readable file"
    )]
    PathNotFound {
        /// The path the caller supplied — surfaced as `details.path`.
        source_path: String,
    },
}

/// Build the RFC 6902 patch for `asset.relink`.
///
/// v1 floor: always returns [`AssetRelinkError::PathNotFound`].
///
/// # Errors
///
/// Always errors with [`AssetRelinkError::PathNotFound`] in v1 — no
/// file I/O is performed so no path resolves.
pub fn compute_patch(
    _prior: &Project,
    args: &AssetRelinkArgs,
) -> Result<(Value, Vec<Value>, Value), AssetRelinkError> {
    Err(AssetRelinkError::PathNotFound {
        source_path: args.source_path.clone(),
    })
}

impl From<AssetRelinkError> for VerbError {
    fn from(value: AssetRelinkError) -> Self {
        match value {
            // PathNotFound is a runtime-state error (the file doesn't exist
            // on disk), not an arg-shape failure. Mapping to Custom keeps
            // validate_command (§1.4) honest: BadArgs there means
            // "args malformed" and would mis-report well-formed
            // {project_id, asset_id, source_path, mode?} as invalid.
            AssetRelinkError::PathNotFound { .. } => VerbError::Custom(value.to_string()),
        }
    }
}

/// The §0.8 verb for `asset.relink`.
#[derive(Debug, Default)]
pub struct AssetRelinkVerb;

impl Verb for AssetRelinkVerb {
    fn verb(&self) -> &'static str {
        "asset.relink"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: AssetRelinkArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("asset.relink: args deserialize failed: {err}"),
            })?;

        // v1 floor: compute_patch always returns Err with
        // E_ASSET_PATH_NOT_FOUND, so the `Ok` branch below is
        // structurally unreachable and only exists to keep the trait
        // shape consistent with other verbs.
        match compute_patch(prior, &typed) {
            Ok((patch_value, warnings, data)) => {
                let patch: json_patch::Patch =
                    serde_json::from_value(patch_value).map_err(|err| {
                        VerbError::Custom(format!("asset.relink: patch construction failed: {err}"))
                    })?;
                Ok((patch, data, warnings))
            }
            Err(e) => Err(e.into()),
        }
    }

    fn reconstruct(
        &self,
        args: &Value,
        _patch: &Value,
        _warnings: &[Value],
        _post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let _typed: AssetRelinkArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "AssetRelinkArgs",
            })?;

        // v1 floor: no successful event is ever recorded for this verb,
        // so the reconstructed envelope is null. See module doc.
        Ok(Value::Null)
    }
}
