//! `render.queue.clear` (§21.5) — seventy-third production verb in the engine.
//!
//! ## Spec quote (`spec/commands/render-queue.md` §21.5, abbreviated)
//!
//! > CLI: `verbreel render queue clear [--project <id>]
//! >   [--state_filter <state>...] [--confirm]`
//! > MCP: `render.queue.clear`
//! > Args: `project_id: string`, `state_filter?: ("queued" | "running" |
//! >   "completed" | "failed" | "canceled")[]` (default `["completed",
//! >   "canceled", "failed"]`), `confirm?: boolean` (default `false`).
//! > Returns (`data`): `{ removed_queue_job_ids: string[],
//! >   canceled_running_job_ids: string[] }`.
//! > Errors: `E_ARGS_INCOMPATIBLE` when `state_filter` includes
//! >   `"queued"` or `"running"` without `confirm: true`
//! >   (`details.field: "confirm"`, `details.hint`).
//!
//! ## v1 floor — queue persistence deferred.
//!
//! Per §21.7, the queue's source of truth is the
//! `~/.verbreel/render-queue.json` persistence file shared across all
//! Verbreel processes on the host. The `Verb` trait's purity contract
//! forbids file I/O in `compute_patch`, so v1 always reports empty
//! `removed_queue_job_ids` and `canceled_running_job_ids` arrays. This
//! is also semantically correct in v1: no `render.start` or
//! `render.queue.add` verb exists yet, so the persistence file is never
//! written and the queue is empty regardless. Wiring the file mutation
//! needs a `VerbContext` / storage facade threaded through
//! `ProjectStore::mutate_via_verb` — same architectural gap that
//! `render.queue.list` (§21.2, just shipped) defers queue reads for, and
//! every other v1-floor verb in the seventy-verb arc defers its
//! corresponding side effect for. A future slice introduces
//! `VerbContext` and wires several deferred features at once.
//!
//! What this verb DOES implement in v1 is the **confirm-gate
//! validation**: when `state_filter` includes a non-terminal state
//! (`"queued"` or `"running"`) without `confirm: true`, the verb refuses
//! with `E_ARGS_INCOMPATIBLE`. That arg-shape check is pure and ships
//! today.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::ProjectId;

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// Recovery hint emitted with `E_ARGS_INCOMPATIBLE` for the
/// non-terminal-state confirm-gate failure.
pub const CONFIRM_HINT: &str = "state_filter includes a non-terminal state; pass confirm: true \
                                to acknowledge that running jobs will be canceled";

/// `state_filter` array element — the five clear-eligible queue states.
///
/// Distinct from [`crate::verbs::render_queue_list::QueueStateFilter`]
/// because that enum carries a sixth `All` variant for the list verb's
/// single-string filter; clear's `state_filter` is an explicit array
/// without an "all" sentinel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QueueClearStateFilter {
    /// Clear entries currently waiting for the worker.
    Queued,
    /// Clear entries currently being rendered.
    Running,
    /// Clear entries that finished successfully.
    Completed,
    /// Clear entries that finished with an error.
    Failed,
    /// Clear entries that were canceled before completion.
    Canceled,
}

/// Default `state_filter` when the caller omits the field — only
/// terminal states are cleared by default.
const DEFAULT_FILTER: &[QueueClearStateFilter] = &[
    QueueClearStateFilter::Completed,
    QueueClearStateFilter::Canceled,
    QueueClearStateFilter::Failed,
];

/// Arguments for `render.queue.clear`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderQueueClearArgs {
    /// Target project id — queue clears are not cross-project.
    pub project_id: ProjectId,
    /// Optional filter; defaults to `["completed", "canceled", "failed"]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_filter: Option<Vec<QueueClearStateFilter>>,
    /// Required (`true`) when `state_filter` contains `"queued"` or
    /// `"running"`. Absent or `false` triggers `E_ARGS_INCOMPATIBLE` in
    /// that case.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm: Option<bool>,
}

/// Envelope returned by `render.queue.clear`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderQueueClearData {
    /// Queue job ids that were actually removed from the queue file.
    pub removed_queue_job_ids: Vec<String>,
    /// Subset of `removed_queue_job_ids` that were canceled mid-render
    /// as part of the clear.
    pub canceled_running_job_ids: Vec<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Error)]
/// Verb-level error type for `render.queue.clear`.
pub enum RenderQueueClearError {
    /// `state_filter` contains a non-terminal state without
    /// `confirm: true`. Maps to `E_ARGS_INCOMPATIBLE`.
    #[error(
        "render.queue.clear: state_filter includes non-terminal state(s) {non_terminal_states:?}; \
         confirm: true is required"
    )]
    ConfirmRequired {
        /// The non-terminal states found in `state_filter`, in input
        /// order — surfaced as `details.non_terminal_states` to the
        /// caller.
        non_terminal_states: Vec<String>,
        /// Recovery hint.
        hint: &'static str,
    },
    /// Reserved placeholder so the error enum is not vacuously
    /// inhabited; not reachable in v1 (queue persistence deferred).
    #[error("render.queue.clear: unreachable")]
    Unreachable,
}

/// Build the canonical `render.queue.clear` data envelope.
fn build_data() -> RenderQueueClearData {
    RenderQueueClearData {
        removed_queue_job_ids: Vec::new(),
        canceled_running_job_ids: Vec::new(),
    }
}

/// Validate the confirm-gate. Returns the non-terminal states found in
/// `filter` (in input order) — empty vec means the gate is satisfied.
fn collect_non_terminal(filter: &[QueueClearStateFilter]) -> Vec<&'static str> {
    filter
        .iter()
        .filter_map(|state| match state {
            QueueClearStateFilter::Queued => Some("queued"),
            QueueClearStateFilter::Running => Some("running"),
            QueueClearStateFilter::Completed
            | QueueClearStateFilter::Failed
            | QueueClearStateFilter::Canceled => None,
        })
        .collect()
}

/// Build the RFC 6902 patch for `render.queue.clear`.
///
/// # Errors
///
/// Returns [`RenderQueueClearError::ConfirmRequired`] when
/// `state_filter` contains `"queued"` or `"running"` and `confirm` is
/// not `true`.
pub fn compute_patch(
    _prior: &Project,
    args: &RenderQueueClearArgs,
) -> Result<(Value, Vec<Value>, RenderQueueClearData), RenderQueueClearError> {
    let filter: &[QueueClearStateFilter] = args.state_filter.as_deref().unwrap_or(DEFAULT_FILTER);
    let confirm = args.confirm.unwrap_or(false);

    let non_terminal = collect_non_terminal(filter);
    if !non_terminal.is_empty() && !confirm {
        return Err(RenderQueueClearError::ConfirmRequired {
            non_terminal_states: non_terminal.iter().map(|s| (*s).to_string()).collect(),
            hint: CONFIRM_HINT,
        });
    }

    Ok((json!([]), Vec::new(), build_data()))
}

/// Build the data envelope from `(args, post_state)`.
///
/// # Errors
///
/// Reuses [`compute_patch`], so this can only return reconstruction
/// errors introduced while rebuilding the deterministic envelope.
pub fn data_envelope_from_args(
    args: &RenderQueueClearArgs,
    post_state: &Project,
) -> Result<RenderQueueClearData, ReconstructError> {
    let (_, _, data) =
        compute_patch(post_state, args).map_err(|e| ReconstructError::Custom(e.to_string()))?;
    Ok(data)
}

impl From<RenderQueueClearError> for VerbError {
    fn from(value: RenderQueueClearError) -> Self {
        match value {
            RenderQueueClearError::ConfirmRequired { .. } | RenderQueueClearError::Unreachable => {
                VerbError::BadArgs {
                    detail: value.to_string(),
                }
            }
        }
    }
}

/// The §0.8 verb for `render.queue.clear`.
#[derive(Debug, Default)]
pub struct RenderQueueClearVerb;

impl Verb for RenderQueueClearVerb {
    fn verb(&self) -> &'static str {
        "render.queue.clear"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: RenderQueueClearArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("render.queue.clear: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch = serde_json::from_value(patch_value).map_err(|err| {
            VerbError::Custom(format!(
                "render.queue.clear: patch construction failed: {err}"
            ))
        })?;

        let data = serde_json::to_value(&data).map_err(|err| {
            VerbError::Custom(format!("render.queue.clear: data envelope failed: {err}"))
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
        let typed: RenderQueueClearArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "RenderQueueClearArgs",
            })?;

        let envelope = data_envelope_from_args(&typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
