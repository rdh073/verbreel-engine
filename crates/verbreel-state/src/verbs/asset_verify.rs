//! `asset.verify` (§3.7) — eighty-eighth production verb in the engine.
//!
//! ## Spec quote (`spec/commands/asset.md` §3.7)
//!
//! > CLI: `verbreel asset verify [--project <id>] [--strict]`
//! > MCP: `asset.verify`
//! > Args: `project_id: string`, `strict?: boolean` (default `false`).
//! > Returns (`data`): `{ checked_count: integer; unverified_asset_ids:
//! >   string[]; mode: "fast" | "strict" }`
//! > Errors: `E_IO`.
//!
//! ## v1 floor — count-only, no fingerprint stat.
//!
//! Per §3.7, the real integrity check `stat()`s every asset's on-disk
//! file (fast mode) or re-hashes every byte (strict mode). Both
//! variants are file I/O — forbidden in the `Verb` trait's pure
//! `compute_patch`. v1 ships:
//!
//! - `checked_count = prior.assets.len() as u64` — the real count of
//!   tracked assets, so the envelope's headline number is honest.
//! - `unverified_asset_ids = vec![]` — always empty until real
//!   fingerprint stat lands. An always-empty list means the v1 verb
//!   reports "every asset is fine" regardless of on-disk drift; that
//!   matches the §0.13 documentation gap that `asset.relink` is the
//!   only path that clears the unverified flag at v1.
//! - `mode` echoes the requested mode (`Fast` by default, `Strict` if
//!   `args.strict == Some(true)`).
//!
//! Same architectural gap as `font.list` system enumeration and
//! `stock.list_providers` config providers: a future slice that wires
//! `VerbContext` / storage facade through `ProjectStore::mutate_via_verb`
//! activates the real check here.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::ProjectId;

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// Arguments for `asset.verify`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetVerifyArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Strict mode flag. When `Some(true)`, the spec'd verb re-hashes
    /// every asset's bytes; when `None` or `Some(false)`, fast mode
    /// (mtime + size stat). v1 floor echoes the chosen mode but
    /// performs no I/O — see module docs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strict: Option<bool>,
}

/// Which integrity check ran. Mirrors the spec's `"fast" | "strict"`
/// string enum at the envelope level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AssetVerifyMode {
    /// `stat()` per asset — fingerprint (mtime + size) comparison.
    Fast,
    /// Re-hash every asset's bytes — SHA-256 comparison.
    Strict,
}

/// Envelope returned by `asset.verify`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetVerifyData {
    /// Number of assets considered by the check.
    pub checked_count: u64,
    /// Asset ids whose on-disk bytes drifted from their recorded
    /// fingerprint / hash. v1 floor: always empty until real
    /// fingerprint stat lands.
    pub unverified_asset_ids: Vec<String>,
    /// Which mode the check ran in.
    pub mode: AssetVerifyMode,
}

/// Verb-level error type for `asset.verify`.
///
/// `E_IO` from the spec is deferred until real fingerprint stat lands;
/// the v1 floor performs no I/O and has no reachable error.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AssetVerifyError {
    /// No verb-level runtime errors at v1.
    #[error("asset.verify: unreachable (no error variants at v1)")]
    Unreachable,
}

/// Build the RFC 6902 patch for `asset.verify`.
///
/// # Errors
///
/// No runtime errors are produced by this verb at v1; the returned
/// `Result` exists for parity with the broader compute-patch API and
/// for forward compatibility when `E_IO` becomes reachable.
pub fn compute_patch(
    prior: &Project,
    args: &AssetVerifyArgs,
) -> Result<(Value, Vec<Value>, AssetVerifyData), AssetVerifyError> {
    let mode = if args.strict == Some(true) {
        AssetVerifyMode::Strict
    } else {
        AssetVerifyMode::Fast
    };

    let data = AssetVerifyData {
        checked_count: prior.assets.len() as u64,
        unverified_asset_ids: Vec::new(),
        mode,
    };

    Ok((json!([]), Vec::new(), data))
}

/// Build the data envelope from `(args, post_state)`.
///
/// # Errors
///
/// Reuses [`compute_patch`], so this can only return reconstruction
/// errors introduced while rebuilding the deterministic envelope.
pub fn data_envelope_from_args(
    args: &AssetVerifyArgs,
    post_state: &Project,
) -> Result<AssetVerifyData, ReconstructError> {
    let (_, _, data) =
        compute_patch(post_state, args).map_err(|e| ReconstructError::Custom(e.to_string()))?;
    Ok(data)
}

impl From<AssetVerifyError> for VerbError {
    fn from(value: AssetVerifyError) -> Self {
        match value {
            AssetVerifyError::Unreachable => VerbError::BadArgs {
                detail: value.to_string(),
            },
        }
    }
}

/// The §0.8 verb for `asset.verify`.
#[derive(Debug, Default)]
pub struct AssetVerifyVerb;

impl Verb for AssetVerifyVerb {
    fn verb(&self) -> &'static str {
        "asset.verify"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: AssetVerifyArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("asset.verify: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch = serde_json::from_value(patch_value).map_err(|err| {
            VerbError::Custom(format!("asset.verify: patch construction failed: {err}"))
        })?;

        let data = serde_json::to_value(&data).map_err(|err| {
            VerbError::Custom(format!("asset.verify: data envelope failed: {err}"))
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
        let typed: AssetVerifyArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "AssetVerifyArgs",
            })?;

        let envelope = data_envelope_from_args(&typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
