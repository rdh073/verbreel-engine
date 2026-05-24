//! `marker.remove` (§13.3) — seventh production verb in the engine.
//!
//! ## Spec quote (`spec/commands/marker.md` §13.3, verbatim)
//!
//! > CLI: `verbreel marker remove [--project <id>] [--marker <id>]... [--soft]`
//! > MCP: `marker.remove`
//! > Args: `project_id: string`, `markers: string[]` (`maxItems: 1000` per
//! > §0.8 — batch larger than this rejected at args-schema validation with
//! > `E_SCHEMA_VIOLATION` carrying `details.field: "markers"`,
//! > `details.hint: "split the batch into smaller calls"`)
//! > Returns (`data`): `{ removed_marker_ids: string[]; missing_marker_ids: string[] }`
//! > Errors: `E_NOT_FOUND` (only when `soft=false` and any marker is missing;
//! > `details.failed_index`, `details.failed_target` per §0.10),
//! > `E_SCHEMA_VIOLATION` (`markers` exceeds `maxItems: 1000`).
//!
//! ## Empty-array no-op rule (§0.6)
//!
//! `marker.remove` follows the empty-array no-op contract shared by all
//! plural-target verbs: `markers: []` is a successful no-op that returns
//! an empty patch, no warnings, and an all-empty data envelope.
//!
//! The kernel is currently responsible for deciding whether to skip the event
//! line for an empty patch; if it does write one (as it does today in
//! `apply_write_ordering`), this implementation still returns the no-op patch and
//! this task should record that as a deviation.
//!
//! ## Soft mode (`soft=true`) and `W_NOOP` warnings (§0.10)
//!
//! Missing ids are downgraded to warnings when `soft=true`:
//!
//! - warning code: `W_NOOP`
//! - message: `marker not found (soft skip)`
//! - details: `marker_id`, `input_index`
//!
//! The operation proceeds for ids that are present.
//!
//! ## Patch index ordering
//!
//! RFC-6902 remove operations are emitted in *descending index* order to
//! avoid index drift while applying the patch.
//!
//! This keeps each `/markers/<idx>` path valid as earlier removals
//! shift lower indices left in the array.
//!
//! ## Duplicate-id-in-args decision
//!
//! Duplicate ids in `markers` are treated as a single successful removal
//! followed by a soft-skip for subsequent duplicates in the same call.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::{MarkerId, ProjectId};

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// Maximum allowed batch length for `markers` (§0.8).
pub const MARKERS_MAX_BATCH: usize = 1000;

/// Soft-mode warning code when a marker is missing and skipped.
pub const W_NOOP_CODE: &str = "W_NOOP";

/// Args for `marker.remove`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarkerRemoveArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Marker ids to remove, by order.
    pub markers: Vec<String>,

    /// `true` downgrades missing ids from error to `W_NOOP` warnings.
    #[serde(default)]
    pub soft: bool,
}

/// Envelope `data` returned by `marker.remove`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkerRemoveData {
    /// IDs successfully removed.
    pub removed_marker_ids: Vec<String>,

    /// Missing IDs skipped under `soft=true`.
    pub missing_marker_ids: Vec<String>,
}

/// Verb-level validation failures surfaced by [`compute_patch`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MarkerRemoveError {
    /// `args.markers.len() > MARKERS_MAX_BATCH`.
    #[error("marker.remove: too many markers ({actual}) > max ({max})")]
    BatchTooLarge {
        /// Actual number of marker ids supplied.
        actual: usize,
        /// Hard maximum.
        max: usize,
    },

    /// Marker id failed UUID parsing.
    #[error("marker.remove: marker {detail} is invalid UUIDv7")]
    MarkerIdInvalid {
        /// Raw parse failure detail.
        detail: String,
        /// Input index for this marker string in `args.markers`.
        marker_index: usize,
    },

    /// Missing marker in strict mode.
    #[error("marker.remove: marker `{marker_id}` not found at index {failed_index}")]
    MarkerNotFound {
        /// Missing marker id.
        marker_id: String,
        /// Input index where the missing marker was first requested.
        failed_index: usize,
    },
}

/// Build the RFC 6902 patch for `marker.remove`.
///
/// Steps:
/// 1. Fast-path empty array: returns no-op patch.
/// 2. Batch-size cap check.
/// 3. Parse each marker id from `args.markers` as `MarkerId`.
/// 4. Build `marker_id -> prior index` map.
/// 5. Walk input ids in order:
///    - present marker ids remove once (and mark them consumed)
///    - missing ids emit `W_NOOP` in soft mode
///    - missing ids return `MarkerNotFound` in strict mode
/// 6. Sort remaining removed indices descending and emit remove ops.
/// 7. Return `(patch, warnings, data)`.
///
/// # Errors
///
/// Returns [`MarkerRemoveError`] for all validation failures:
///
/// - `BatchTooLarge` when the `markers` input exceeds `MARKERS_MAX_BATCH`.
/// - `MarkerIdInvalid` when any provided marker id is not a valid `UUIDv7`.
/// - `MarkerNotFound` when `soft == false` and a requested marker id is absent.
pub fn compute_patch(
    prior: &Project,
    args: &MarkerRemoveArgs,
) -> Result<(Value, Vec<Value>, MarkerRemoveData), MarkerRemoveError> {
    if args.markers.is_empty() {
        return Ok((
            json!([]),
            Vec::new(),
            MarkerRemoveData {
                removed_marker_ids: Vec::new(),
                missing_marker_ids: Vec::new(),
            },
        ));
    }

    if args.markers.len() > MARKERS_MAX_BATCH {
        return Err(MarkerRemoveError::BatchTooLarge {
            actual: args.markers.len(),
            max: MARKERS_MAX_BATCH,
        });
    }

    let parsed: Vec<(usize, MarkerId, String)> = args
        .markers
        .iter()
        .enumerate()
        .map(|(idx, marker)| {
            marker
                .parse::<MarkerId>()
                .map(|parsed| (idx, parsed, marker.clone()))
                .map_err(|err| MarkerRemoveError::MarkerIdInvalid {
                    detail: err.to_string(),
                    marker_index: idx,
                })
        })
        .collect::<Result<_, _>>()?;

    let prior_by_id: HashMap<MarkerId, usize> = prior
        .markers
        .iter()
        .enumerate()
        .map(|(index, marker)| (marker.id, index))
        .collect();

    let mut still_present: HashSet<MarkerId> = prior_by_id.keys().copied().collect();
    let mut removed: Vec<(usize, String)> = Vec::new();
    let mut data = MarkerRemoveData {
        removed_marker_ids: Vec::new(),
        missing_marker_ids: Vec::new(),
    };
    let mut warnings = Vec::new();

    for (input_index, marker_id, marker_string) in parsed
        .iter()
        .map(|(input_index, marker_id, marker_string)| (*input_index, marker_id, marker_string))
    {
        if let Some(&marker_idx) = prior_by_id.get(marker_id)
            && still_present.remove(marker_id)
        {
            removed.push((marker_idx, marker_string.clone()));
            data.removed_marker_ids.push(marker_string.clone());
            continue;
        }

        if args.soft {
            data.missing_marker_ids.push(marker_string.clone());
            warnings.push(json!({
                "code": W_NOOP_CODE,
                "message": "marker not found (soft skip)",
                "details": {
                    "marker_id": marker_string,
                    "input_index": input_index,
                }
            }));
            continue;
        }

        return Err(MarkerRemoveError::MarkerNotFound {
            marker_id: marker_string.clone(),
            failed_index: input_index,
        });
    }

    removed.sort_by(|(left, _), (right, _)| right.cmp(left));
    let patch = Value::Array(
        removed
            .iter()
            .map(|(idx, _)| {
                json!({
                    "op": "remove",
                    "path": format!("/markers/{idx}"),
                })
            })
            .collect(),
    );

    Ok((patch, warnings, data))
}

/// Rebuild `MarkerRemoveData` from `(args, warnings)` alone.
///
/// This function is used both in `compute_patch` (by-value pass-through)
/// and on replay:
/// `reconstruct` reads `args` and `warnings` so it can recover exactly the same
/// envelope as the original call.
#[must_use]
pub fn data_envelope_from_args_warnings(
    args: &MarkerRemoveArgs,
    warnings: &[Value],
) -> MarkerRemoveData {
    let missing: Vec<String> = warnings
        .iter()
        .filter(|warning| warning.get("code").and_then(Value::as_str) == Some(W_NOOP_CODE))
        .filter_map(|warning| {
            warning
                .get("details")
                .and_then(|details| details.get("marker_id"))
                .and_then(Value::as_str)
                .map(ToString::to_string)
        })
        .collect();

    let mut missing_counts: HashMap<&str, usize> = HashMap::new();
    for marker_id in &missing {
        *missing_counts.entry(marker_id.as_str()).or_insert(0) += 1;
    }

    let mut removed_marker_ids = Vec::new();
    let mut missing_marker_ids = Vec::new();
    for marker in &args.markers {
        if let Some(count) = missing_counts.get_mut(marker.as_str())
            && *count > 0
        {
            *count -= 1;
            missing_marker_ids.push(marker.clone());
            continue;
        }

        removed_marker_ids.push(marker.clone());
    }

    MarkerRemoveData {
        removed_marker_ids,
        missing_marker_ids,
    }
}

/// Funnel verb-level errors into the kernel error tax.
impl From<MarkerRemoveError> for VerbError {
    fn from(value: MarkerRemoveError) -> Self {
        match value {
            MarkerRemoveError::BatchTooLarge { .. }
            | MarkerRemoveError::MarkerIdInvalid { .. }
            | MarkerRemoveError::MarkerNotFound { .. } => VerbError::BadArgs {
                detail: value.to_string(),
            },
        }
    }
}

/// The §0.8 verb for `marker.remove`.
#[derive(Debug, Default)]
pub struct MarkerRemoveVerb;

impl Verb for MarkerRemoveVerb {
    fn verb(&self) -> &'static str {
        "marker.remove"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: MarkerRemoveArgs =
            serde_json::from_value(args.clone()).map_err(|e| VerbError::BadArgs {
                detail: format!("marker.remove: args deserialize failed: {e}"),
            })?;

        let (patch_value, warnings, data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|e| {
                VerbError::Custom(format!("marker.remove: patch construction failed: {e}"))
            })?;

        let data = serde_json::to_value(&data)
            .map_err(|e| VerbError::Custom(format!("marker.remove: data envelope failed: {e}")))?;

        Ok((patch, data, warnings))
    }

    fn reconstruct(
        &self,
        args: &Value,
        _patch: &Value,
        warnings: &[Value],
        _post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let args: MarkerRemoveArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "MarkerRemoveArgs",
            })?;

        let envelope = data_envelope_from_args_warnings(&args, warnings);
        serde_json::to_value(&envelope).map_err(|e| ReconstructError::Custom(e.to_string()))
    }
}
