//! `asset.probe` (§3.3) — eighty-fifth production verb in the engine.
//!
//! ## Spec quote (`spec/commands/asset.md` §3.3, verbatim)
//!
//! > CLI: `verbreel asset probe --path <path>`
//! > MCP: `asset.probe`
//! > Args: `path: string`.
//! > Returns (`data`): `{ kind: "video"|"audio"|"image"|"subtitle";
//! >   hash: string; metadata: object }`.
//! > Errors: `E_ASSET_PATH_NOT_FOUND`, `E_ASSET_UNREADABLE`,
//! >   `E_ASSET_UNSUPPORTED_KIND`.
//!
//! ## v1 floor — always errors with `E_ASSET_PATH_NOT_FOUND`.
//!
//! Probing a file requires reading bytes from disk (hashing with
//! SHA-256, deriving `kind` from magic bytes / extension, running an
//! ffprobe-equivalent to fill `metadata`). All three are file I/O —
//! forbidden in the pure `Verb::compute_patch`. The same `VerbContext`
//! / storage facade plumbing that `asset.import` (§3.1) and the rest of
//! the asset arc defer is required here. Until that lands, the verb's
//! `compute_patch` cannot read the path, so every well-formed call
//! returns `E_ASSET_PATH_NOT_FOUND` carrying the supplied `path` as
//! `details.path`. `E_ASSET_UNREADABLE` and `E_ASSET_UNSUPPORTED_KIND`
//! are declared in the spec error list but unreachable in v1 — they
//! light up alongside the file I/O work.
//!
//! ## `project_id` accommodation (spec says only `path`).
//!
//! The spec quote above declares `Args: path: string`. v1 ships with a
//! **required** `project_id: ProjectId` field alongside `path`. Reason:
//! the kernel's `ProjectStore::mutate_via_verb` path resolves a `prior`
//! project from the args envelope before calling `Verb::compute_patch`,
//! so a project hook has to land somewhere in the args struct.
//! `render.cancel` / `render.status` / `preview.session.close` /
//! `render.queue.cancel` carry a required `project_id` next to their
//! domain arg for the same reason and serve as the precedent. The
//! lighter `Option<ProjectId>` accommodation used by truly project-less
//! read-only metadata verbs (`render.list_presets`,
//! `stock.list_providers`, `font.list`, `list_capabilities`) is reserved
//! for verbs whose returned shape is purely engine-bundled metadata;
//! `asset.probe` is path-shaped, not metadata-shaped, so it follows the
//! always-errors precedent and keeps `project_id` required.
//!
//! ## Reconstructor framing for an always-errors verb.
//!
//! `compute_patch` always returns `Err`, which means no successful event
//! is ever appended to `events.jsonl` (the §0.8 write-ordering rule
//! requires a successful patch before an event is written). The
//! reconstruct path is therefore unreachable in production v1. It still
//! has to clear the §0.8 startup gate against the fixture in
//! `default_fixtures()`, so the implementation deserializes the args
//! (the only round-trip the recorded tuple can support) and returns
//! `Value::Null` — the truthful "no data was ever recorded for this
//! verb in v1" envelope. The matching fixture records
//! `expected_data: null` so the gate's canonical-SHA equality holds by
//! construction.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use verbreel_types::ProjectId;

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// Arguments for `asset.probe`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetProbeArgs {
    /// Target project id. Required by the `Verb` trait shape (kernel
    /// dispatch resolves `prior` from this); ignored by the v1 floor
    /// impl which errors before reading project state.
    pub project_id: ProjectId,
    /// The filesystem path to probe. v1 floor: never read; surfaced
    /// back to the caller as `details.path` in the `E_ASSET_PATH_NOT_FOUND`
    /// error.
    pub path: String,
}

/// Response envelope for a successful `asset.probe`.
///
/// v1 floor never constructs this shape (every call errors), but the
/// type is defined here so downstream consumers (CLI, MCP) can pin
/// against the spec'd response shape. `kind` is one of
/// `"video"|"audio"|"image"|"subtitle"` per §3.3; stored as `String`
/// because the data type is unreachable in v1 and validating the enum
/// at the type level would force callers to thread the variant through
/// before the file-I/O work that derives it has shipped. `metadata` is
/// a free-form `Value` because the per-kind metadata shapes
/// (`VideoAssetMetadata`, `AudioAssetMetadata`, `ImageAssetMetadata`,
/// `SubtitleAssetMetadata`) are derived by the ffprobe-equivalent
/// pipeline that this v1 floor defers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetProbeData {
    /// One of `"video"|"audio"|"image"|"subtitle"` per §3.3.
    pub kind: String,
    /// Hex-encoded SHA-256 of the probed file's bytes.
    pub hash: String,
    /// Per-kind metadata derived by the ffprobe-equivalent pipeline.
    /// Free-form `Value` at v1 — the typed variants land alongside the
    /// file-I/O work.
    pub metadata: Value,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Error)]
/// Verb-level error type for `asset.probe`.
pub enum AssetProbeError {
    /// `path` does not resolve to a readable file on disk. Maps to
    /// `E_ASSET_PATH_NOT_FOUND`. In v1 floor this is returned for every
    /// well-formed call regardless of the path supplied.
    #[error(
        "asset.probe: E_ASSET_PATH_NOT_FOUND — path `{path}` does not resolve to a readable file"
    )]
    PathNotFound {
        /// The path the caller supplied — surfaced as `details.path`.
        path: String,
    },
}

/// Build the RFC 6902 patch for `asset.probe`.
///
/// v1 floor: always returns [`AssetProbeError::PathNotFound`].
///
/// # Errors
///
/// Always errors with [`AssetProbeError::PathNotFound`] in v1 — no file
/// I/O is performed so no path resolves.
pub fn compute_patch(
    _prior: &Project,
    args: &AssetProbeArgs,
) -> Result<(Value, Vec<Value>, Value), AssetProbeError> {
    Err(AssetProbeError::PathNotFound {
        path: args.path.clone(),
    })
}

impl From<AssetProbeError> for VerbError {
    fn from(value: AssetProbeError) -> Self {
        match value {
            // PathNotFound is a runtime-state error (the file doesn't exist
            // on disk), not an arg-shape failure. Mapping to Custom keeps
            // validate_command (§1.4) honest: BadArgs there means
            // "args malformed" and would mis-report well-formed
            // {project_id, path} as invalid.
            AssetProbeError::PathNotFound { .. } => VerbError::Custom(value.to_string()),
        }
    }
}

/// The §0.8 verb for `asset.probe`.
#[derive(Debug, Default)]
pub struct AssetProbeVerb;

impl Verb for AssetProbeVerb {
    fn verb(&self) -> &'static str {
        "asset.probe"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: AssetProbeArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("asset.probe: args deserialize failed: {err}"),
            })?;

        // v1 floor: compute_patch always returns Err with
        // E_ASSET_PATH_NOT_FOUND, so the `Ok` branch below is
        // structurally unreachable and only exists to keep the trait
        // shape consistent with other verbs.
        match compute_patch(prior, &typed) {
            Ok((patch_value, warnings, data)) => {
                let patch: json_patch::Patch =
                    serde_json::from_value(patch_value).map_err(|err| {
                        VerbError::Custom(format!("asset.probe: patch construction failed: {err}"))
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
        let _typed: AssetProbeArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "AssetProbeArgs",
            })?;

        // v1 floor: no successful event is ever recorded for this verb,
        // so the reconstructed envelope is null. See module doc.
        Ok(Value::Null)
    }
}
