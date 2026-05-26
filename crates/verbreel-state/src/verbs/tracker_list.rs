//! `tracker.list` (§18.4) — sixty-ninth production verb in the engine.
//!
//! ## Spec quote (`spec/commands/tracker.md` §18.4)
//!
//! > Read-only enumeration of every tracker on the current project.
//! > CLI: `verbreel tracker list [--project <id>]`
//! > MCP: `tracker.list`
//! > Args: `project_id: string`
//! > Returns (`data`): `{ trackers: TrackerEntry[] }`
//! > Errors: `E_PROJECT_NOT_FOUND` (§0.12 universal-implicit).
//! > Warnings: none.
//!
//! ## v1 floor — `cache_status` is always `Unrun`.
//!
//! Per §18.4, `cache_status` is resolved by `stat(2)` on `cache_path`
//! at call time: `"fresh"` when `cache_hash` is set AND the file
//! exists, `"dangling"` when set AND missing, `"unrun"` when empty.
//! v1 always returns `Unrun` because:
//!
//! 1. No `tracker.run` exists in v1.x yet → no tracker on a v1
//!    project can carry a populated `cache_hash`. The trivial reading
//!    (`Unrun` for all entries) is structurally correct for the v1
//!    ecosystem.
//! 2. Even if a tracker record had `cache_hash` set by hand (e.g. via
//!    direct `project.json` edit), the `stat(2)` call would violate
//!    the `Verb` trait's pure-`compute_patch` contract.
//!
//! Same architectural deferral pattern as `project.info` (`event_count`),
//! `timeline.snapshot` (head `event_id`), `stock.list_providers` (config
//! providers), `list_capabilities` (v1.1+ fields), and `font.list`
//! (system fonts). When `tracker.run` ships AND a storage / file-stat
//! facade lands on the `Verb` trait, this verb upgrades to compute
//! `Fresh` vs `Dangling` via context.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::ProjectId;

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// Arguments for `tracker.list`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackerListArgs {
    /// Target project id.
    pub project_id: ProjectId,
}

/// Cache freshness for a tracker's `cache_path`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrackerCacheStatus {
    /// `cache_hash` set and the cache file exists.
    Fresh,
    /// `cache_hash` set but the cache file is missing.
    Dangling,
    /// `cache_hash` is empty — no `tracker.run` has ever completed.
    Unrun,
}

/// Single tracker entry returned by `tracker.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackerEntry {
    /// Tracker id (`UUIDv7` string per schema).
    pub tracker_id: String,
    /// Source clip id (the analysis source).
    pub source_clip_id: String,
    /// `true` iff `source_clip_id` resolves to a live clip on any
    /// track in the current project graph.
    pub source_clip_exists: bool,
    /// Tracker algorithm name (free-form string at v1).
    pub algorithm: String,
    /// Sample count from the last successful `tracker.run`; `-1` when
    /// the tracker has never been run.
    pub sample_count: i64,
    /// Clip ids currently receiving keyframes from this tracker.
    pub applied_to_clip_ids: Vec<String>,
    /// SHA-256 hex of the cache key; empty string when never run.
    pub cache_hash: String,
    /// Cache file path; empty string when `cache_hash` is empty.
    pub cache_path: String,
    /// Cache freshness — v1 always returns `Unrun` (see module note).
    pub cache_status: TrackerCacheStatus,
    /// RFC-3339 timestamp of the last successful `tracker.run`; absent
    /// until first successful run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_at: Option<String>,
}

/// Envelope returned by `tracker.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackerListData {
    /// Tracker entries in `Project.trackers[]` insertion order.
    pub trackers: Vec<TrackerEntry>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Error)]
/// Verb-level error type for `tracker.list`.
pub enum TrackerListError {
    /// No verb-level runtime errors per spec.
    #[error("tracker.list: unreachable (no error variants)")]
    Unreachable,
}

impl From<TrackerListError> for VerbError {
    fn from(value: TrackerListError) -> Self {
        match value {
            TrackerListError::Unreachable => VerbError::BadArgs {
                detail: value.to_string(),
            },
        }
    }
}

fn build_trackers(prior: &Project) -> Vec<TrackerEntry> {
    prior
        .trackers
        .iter()
        .map(|tracker| {
            let map = &tracker.0;
            let tracker_id = map
                .get("tracker_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let source_clip_id = map
                .get("source_clip_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let algorithm = map
                .get("algorithm")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let sample_count = map
                .get("sample_count")
                .and_then(Value::as_i64)
                .unwrap_or(-1);
            let applied_to_clip_ids = map
                .get("applied_to_clip_ids")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let cache_hash = map
                .get("cache_hash")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let cache_path = map
                .get("cache_path")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let last_run_at = map
                .get("last_run_at")
                .and_then(Value::as_str)
                .map(String::from);

            let source_clip_exists = !source_clip_id.is_empty()
                && prior
                    .tracks
                    .iter()
                    .flat_map(|track| track.clips.iter())
                    .any(|clip| clip.id.to_string() == source_clip_id);

            TrackerEntry {
                tracker_id,
                source_clip_id,
                source_clip_exists,
                algorithm,
                sample_count,
                applied_to_clip_ids,
                cache_hash,
                cache_path,
                cache_status: TrackerCacheStatus::Unrun,
                last_run_at,
            }
        })
        .collect()
}

/// Build the RFC 6902 patch and data envelope for `tracker.list`.
///
/// # Errors
///
/// No runtime errors are produced by this verb; the returned `Result`
/// exists for parity with the broader compute-patch API.
pub fn compute_patch(
    prior: &Project,
    _args: &TrackerListArgs,
) -> Result<(Value, Vec<Value>, TrackerListData), TrackerListError> {
    Ok((
        json!([]),
        Vec::new(),
        TrackerListData {
            trackers: build_trackers(prior),
        },
    ))
}

/// Build the data envelope from `(args, post_state)`.
///
/// # Errors
///
/// Reuses [`compute_patch`], so this can only return reconstruction
/// errors introduced while rebuilding the deterministic envelope.
pub fn data_envelope_from_post_state(
    args: &TrackerListArgs,
    post_state: &Project,
) -> Result<TrackerListData, ReconstructError> {
    let (_, _, data) =
        compute_patch(post_state, args).map_err(|e| ReconstructError::Custom(e.to_string()))?;
    Ok(data)
}

/// The §0.8 verb for `tracker.list`.
#[derive(Debug, Default)]
pub struct TrackerListVerb;

impl Verb for TrackerListVerb {
    fn verb(&self) -> &'static str {
        "tracker.list"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: TrackerListArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("tracker.list: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch = serde_json::from_value(patch_value).map_err(|err| {
            VerbError::Custom(format!("tracker.list: patch construction failed: {err}"))
        })?;

        let data = serde_json::to_value(&data).map_err(|err| {
            VerbError::Custom(format!("tracker.list: data envelope failed: {err}"))
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
        let typed: TrackerListArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "TrackerListArgs",
            })?;

        let envelope = data_envelope_from_post_state(&typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
