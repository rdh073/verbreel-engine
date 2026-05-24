//! `marker.list` (§13.4) — eighth production verb in the engine.
//!
//! ## Spec quote (`spec/commands/marker.md` §13.4, verbatim)
//!
//! > Ordering: returned `markers` are sorted by `time_tk` **ascending**, with
//! > `Marker.id` as a tiebreaker for markers at the same tick. The total
//! > ordering makes the list reproducible across calls and across replays.
//! >
//! > CLI: `verbreel marker list [--project <id>]`
//! > MCP: `marker.list`
//! > Args: `project_id: string`
//! > Returns (`data`): `{ markers: Marker[] }`
//!
//! ## First read-only verb in the registry
//!
//! `marker.list` does not mutate project state, emits no patch and no
//! warnings, but still emits a non-empty data envelope (the list). The
//! patch is always `[]` and is intentionally deterministic.
//!
//! ## Total ordering
//!
//! Sorting is total and stable because `time_tk` asc is the primary key and
//! stringified `Marker.id` is the tiebreaker.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::marker::Marker;
use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use verbreel_types::ProjectId;

/// Args for `marker.list`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarkerListArgs {
    /// Target project id.
    pub project_id: ProjectId,
}

/// Envelope `data` returned by `marker.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkerListData {
    /// Sorted marker list as the data envelope.
    pub markers: Vec<Marker>,
}

/// Build the RFC 6902 patch for `marker.list`.
///
/// Read-only verbs in this engine use an empty patch to signal no state
/// mutation.
#[must_use]
pub fn compute_patch(_prior: &Project, _args: &MarkerListArgs) -> (Value, Vec<Value>) {
    (json!([]), Vec::new())
}

/// Return markers from `post_state` in deterministic order:
///
/// 1) `time_tk` ascending
/// 2) `id` as lexicographic tie-breaker
#[must_use]
pub fn sorted_markers(post_state: &Project) -> Vec<Marker> {
    let mut markers = post_state.markers.clone();
    markers.sort_by_key(|marker| (marker.time_tk.get(), marker.id.to_string()));
    markers
}

/// Build the data envelope (`data`) from post-state.
#[must_use]
pub fn data_envelope(post_state: &Project) -> MarkerListData {
    MarkerListData {
        markers: sorted_markers(post_state),
    }
}

/// The §0.8 verb for `marker.list`.
#[derive(Debug, Default)]
pub struct MarkerListVerb;

impl Verb for MarkerListVerb {
    fn verb(&self) -> &'static str {
        "marker.list"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: MarkerListArgs =
            serde_json::from_value(args.clone()).map_err(|e| VerbError::BadArgs {
                detail: format!("marker.list: args deserialize failed: {e}"),
            })?;

        let (patch_value, warnings) = compute_patch(prior, &typed);
        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|e| {
                VerbError::Custom(format!("marker.list: patch construction failed: {e}"))
            })?;

        let data = data_envelope(prior);
        let data = serde_json::to_value(&data)
            .map_err(|e| VerbError::Custom(format!("marker.list: data envelope failed: {e}")))?;

        Ok((patch, data, warnings))
    }

    fn reconstruct(
        &self,
        args: &Value,
        _patch: &Value,
        _warnings: &[Value],
        post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let _typed: MarkerListArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "MarkerListArgs",
            })?;
        let envelope = data_envelope(post_state);
        serde_json::to_value(&envelope).map_err(|e| ReconstructError::Custom(e.to_string()))
    }
}
