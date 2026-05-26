//! `tracker.remove` (§18.5) — seventieth production verb in the engine.
//!
//! ## Spec quote (`spec/commands/tracker.md` §18.5, summarized)
//!
//! Removes a tracker resource from `Project.trackers[]`. Already-applied
//! keyframes on target clips are NOT removed — they belong to the
//! clip's keyframe history per a prior `tracker.apply` event. By default
//! (`purge_cache: true`) the cache file at
//! `cache/trackers/<cache_hash>.json` is unlinked; pass
//! `purge_cache: false` to leave it on disk.
//!
//! ## Reconstructor compatibility
//!
//! The removed tracker is absent from post-state, so `reconstruct()`
//! cannot derive the data envelope from post-state alone. The forward
//! path emits one internal warning (`W_TRACKER_REMOVE_ENVELOPE`)
//! carrying `removed_tracker_id`, optional `cache_path`, and
//! `cache_purged`. The reconstructor reads that warning back into
//! [`TrackerRemoveData`], mirroring the destructive-verb envelope
//! pattern used by `asset.remove`, `clip.delete`, and `track.remove`.
//!
//! ## v1 floor — `cache_purged` is ALWAYS `false`.
//!
//! Per §18.5, `purge_cache: true` should unlink
//! `cache/trackers/<cache_hash>.json`. v1 always returns
//! `cache_purged: false` (even when `purge_cache: true` was passed)
//! because:
//!
//! 1. No `tracker.run` exists in v1.x yet → no tracker on a v1 project
//!    can carry a populated `cache_hash` → no cache file exists to
//!    unlink. The trivial reading (`false` for all calls) is
//!    structurally correct for the v1 ecosystem.
//! 2. Even if a tracker record had `cache_hash` set by hand (e.g. via
//!    direct `project.json` edit), the `unlink(2)` call would violate
//!    the `Verb` trait's pure-`compute_patch` contract.
//!
//! Same architectural deferral pattern as `project.info` (`event_count`),
//! `timeline.snapshot` (head `event_id`), `stock.list_providers` (config
//! providers), `list_capabilities` (v1.1+ fields), `font.list` (system
//! fonts), and `tracker.list` (`cache_status`). When `tracker.run` ships
//! AND a storage / file-I/O facade lands on the `Verb` trait, this verb
//! upgrades to actually honor `purge_cache: true`.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::ProjectId;

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// Internal warning code carrying the destructive data envelope.
pub const W_TRACKER_REMOVE_ENVELOPE_CODE: &str = "W_TRACKER_REMOVE_ENVELOPE";

/// Default for `purge_cache` when omitted from args. Returns
/// `Some(true)` (not `true`) because the field is `Option<bool>` —
/// callers can still distinguish "omitted" (becomes `Some(true)` via
/// this default) from "explicit `null`" (becomes `None`). The
/// `unnecessary_wraps` clippy lint is silenced here because serde's
/// `#[serde(default = "...")]` calling convention REQUIRES the
/// default function's return type match the field type exactly.
#[allow(clippy::unnecessary_wraps)]
const fn default_purge_cache() -> Option<bool> {
    Some(true)
}

/// Arguments for `tracker.remove`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackerRemoveArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Target tracker id, as a bare `UUIDv7` string (per Appendix A
    /// taxonomy). No strict UUID parse here — the placeholder
    /// `Project.trackers[]` shape carries `tracker_id` as a raw string
    /// and the verb matches via string equality.
    pub tracker_id: String,

    /// When `true` (default), the spec mandates unlinking the cache
    /// file at `cache/trackers/<cache_hash>.json`. v1 ignores this
    /// flag (see module note); `cache_purged` is always `false`.
    #[serde(default = "default_purge_cache")]
    pub purge_cache: Option<bool>,
}

/// Envelope returned by `tracker.remove`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackerRemoveData {
    /// The tracker id that was removed.
    pub tracker_id: String,

    /// Cache file path that existed at the time of removal. `None`
    /// when the tracker had no `cache_path` set (never run) or the
    /// field was empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_path: Option<String>,

    /// Whether the cache file was actually unlinked. Always `false`
    /// at this slice — see module docs.
    pub cache_purged: bool,
}

/// Verb-level validation failures for `tracker.remove`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TrackerRemoveError {
    /// `args.tracker_id` fails the basic selector contract (e.g.
    /// empty string). Kept for parity with §0.18 selector errors even
    /// though v1 does not strictly UUID-parse the placeholder
    /// `tracker_id` field.
    #[error("tracker.remove: `tracker_id` selector parse failed: {detail}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
    },

    /// No tracker matches `tracker_id` on the project.
    #[error("tracker.remove: tracker `{tracker_id}` not found")]
    TrackerNotFound {
        /// Missing tracker id string.
        tracker_id: String,
    },
}

impl From<TrackerRemoveError> for VerbError {
    fn from(value: TrackerRemoveError) -> Self {
        match value {
            TrackerRemoveError::BadSelector { .. } | TrackerRemoveError::TrackerNotFound { .. } => {
                VerbError::BadArgs {
                    detail: value.to_string(),
                }
            }
        }
    }
}

/// Build the RFC-6902 patch for `tracker.remove`.
///
/// # Errors
///
/// Returns [`TrackerRemoveError`] for bad selector or missing tracker.
pub fn compute_patch(
    prior: &Project,
    args: &TrackerRemoveArgs,
) -> Result<(Value, Vec<Value>, TrackerRemoveData), TrackerRemoveError> {
    if args.tracker_id.is_empty() {
        return Err(TrackerRemoveError::BadSelector {
            detail: "tracker_id is empty".to_string(),
        });
    }

    let index = prior
        .trackers
        .iter()
        .position(|tracker| {
            tracker.0.get("tracker_id").and_then(Value::as_str) == Some(args.tracker_id.as_str())
        })
        .ok_or_else(|| TrackerRemoveError::TrackerNotFound {
            tracker_id: args.tracker_id.clone(),
        })?;

    let cache_path = prior.trackers[index]
        .0
        .get("cache_path")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(String::from);

    let data = TrackerRemoveData {
        tracker_id: args.tracker_id.clone(),
        cache_path: cache_path.clone(),
        cache_purged: false,
    };

    let ops = vec![json!({
        "op": "remove",
        "path": format!("/trackers/{index}"),
    })];

    let warnings = vec![envelope_warning(&data)];
    Ok((Value::Array(ops), warnings, data))
}

fn envelope_warning(data: &TrackerRemoveData) -> Value {
    json!({
        "code": W_TRACKER_REMOVE_ENVELOPE_CODE,
        "message": format!("tracker.remove envelope for {}", data.tracker_id),
        "details": {
            "removed_tracker_id": data.tracker_id,
            "cache_path": data.cache_path,
            "cache_purged": data.cache_purged,
        }
    })
}

/// Rebuild [`TrackerRemoveData`] from recorded args and warnings.
///
/// # Errors
///
/// Returns [`ReconstructError`] if the internal envelope warning is
/// missing or malformed.
pub fn data_envelope_from_args_warnings(
    _args: &TrackerRemoveArgs,
    warnings: &[Value],
) -> Result<TrackerRemoveData, ReconstructError> {
    let details = envelope_details_from_warnings(warnings)?;
    Ok(TrackerRemoveData {
        tracker_id: required_string(details, "removed_tracker_id")?,
        cache_path: optional_string(details, "cache_path")?,
        cache_purged: required_bool(details, "cache_purged")?,
    })
}

fn envelope_details_from_warnings(warnings: &[Value]) -> Result<&Value, ReconstructError> {
    for warning in warnings {
        if warning.get("code").and_then(Value::as_str) != Some(W_TRACKER_REMOVE_ENVELOPE_CODE) {
            continue;
        }
        return warning
            .get("details")
            .ok_or(ReconstructError::MissingField {
                name: "warnings[].W_TRACKER_REMOVE_ENVELOPE.details",
            });
    }

    Err(ReconstructError::MissingField {
        name: "warnings[].W_TRACKER_REMOVE_ENVELOPE",
    })
}

fn required_string(details: &Value, name: &'static str) -> Result<String, ReconstructError> {
    details
        .get(name)
        .ok_or(ReconstructError::MissingField { name })?
        .as_str()
        .map(String::from)
        .ok_or(ReconstructError::TypeMismatch {
            name,
            expected: "string",
        })
}

fn optional_string(
    details: &Value,
    name: &'static str,
) -> Result<Option<String>, ReconstructError> {
    match details.get(name) {
        None => Ok(None),
        Some(value) if value.is_null() => Ok(None),
        Some(value) => {
            value
                .as_str()
                .map(|s| Some(s.to_string()))
                .ok_or(ReconstructError::TypeMismatch {
                    name,
                    expected: "string or null",
                })
        }
    }
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

/// The §0.8 verb for `tracker.remove`.
#[derive(Debug, Default)]
pub struct TrackerRemoveVerb;

impl Verb for TrackerRemoveVerb {
    fn verb(&self) -> &'static str {
        "tracker.remove"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: TrackerRemoveArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("tracker.remove: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!("tracker.remove: patch construction failed: {err}"))
            })?;
        let data = serde_json::to_value(&data)
            .map_err(|err| VerbError::Custom(format!("tracker.remove: data failed: {err}")))?;

        Ok((patch, data, warnings))
    }

    fn reconstruct(
        &self,
        args: &Value,
        _patch: &Value,
        warnings: &[Value],
        _post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let typed: TrackerRemoveArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "TrackerRemoveArgs",
            })?;

        let envelope = data_envelope_from_args_warnings(&typed, warnings)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
