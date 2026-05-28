//! `render.queue.list` (§21.2) — seventy-second production verb in the engine.
//!
//! ## Spec quote (`spec/commands/render-queue.md` §21.2, abbreviated)
//!
//! > CLI: `verbreel render queue list [--project <id>] [--state_filter <state>]`
//! > MCP: `render.queue.list`
//! > Args: `project_id?: string`, `state_filter?: "queued" |
//! >   "running" | "completed" | "failed" | "canceled" | "all"`
//! >   (default `"all"`).
//! > Returns (`data`): `{ items: QueueEntry[] }` ordered by
//! >   `(project_id, -priority, added_at)`.
//!
//! ## v1 floor — empty queue.
//!
//! Per §21.7, the queue's source of truth is the
//! `~/.verbreel/render-queue.json` persistence file shared across all
//! Verbreel processes on the host. The `Verb` trait's purity contract
//! forbids file I/O in `compute_patch`, so v1 always reports an empty
//! queue. This is also semantically correct in v1: `render.queue.add`
//! is a queue-enqueue floor that always errors with `E_QUEUE_FULL`, so
//! no successful enqueue is recorded and the persistence file is never
//! written. Reading it therefore returns empty regardless. Wiring
//! the file read needs a `VerbContext` / storage facade threaded through
//! `ProjectStore::mutate_via_verb` — same architectural gap that
//! `project.info` defers `event_count` for, `stock.list_providers`
//! defers config providers for, `list_capabilities` defers v1.1+ fields
//! for, `font.list` defers system fonts for, `tracker.list` defers
//! `cache_status` for, `tracker.remove` defers cache unlink for, and
//! `tracker.create` defers `cache_hash` population for. A future slice
//! introduces `VerbContext` and wires several deferred features at
//! once.
//!
//! ## Bundle metadata, not project state.
//!
//! `render.queue.list` is read-only and does not read or mutate project
//! state; both `project_id` and `state_filter` are accepted for shape
//! compatibility with the spec but are not consulted in v1 (the queue
//! is always empty).

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::ProjectId;

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// State of a single queue job — the closed set from §21.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QueueJobState {
    /// Job is waiting for the worker.
    Queued,
    /// Job is currently being rendered.
    Running,
    /// Job finished successfully.
    Completed,
    /// Job finished with an error.
    Failed,
    /// Job was canceled before completion.
    Canceled,
}

/// `state_filter` arg — the five `QueueJobState` variants plus `All`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QueueStateFilter {
    /// Match `QueueJobState::Queued` entries only.
    Queued,
    /// Match `QueueJobState::Running` entries only.
    Running,
    /// Match `QueueJobState::Completed` entries only.
    Completed,
    /// Match `QueueJobState::Failed` entries only.
    Failed,
    /// Match `QueueJobState::Canceled` entries only.
    Canceled,
    /// No filtering — return every entry.
    All,
}

/// Single queue entry returned by `render.queue.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueEntry {
    /// Globally unique queue-job identifier.
    pub queue_job_id: String,
    /// Project this queue entry belongs to.
    pub project_id: String,
    /// Current job state.
    pub state: QueueJobState,
    /// Preset arg that was queued.
    pub preset: String,
    /// Output path arg that was queued.
    pub out_path: String,
    /// Scheduler priority (higher runs first).
    pub priority: i32,
    /// RFC 3339 timestamp the entry was added.
    pub added_at: String,
    /// RFC 3339 timestamp the worker started the job — present iff
    /// `state ∈ {running, completed, failed}` and the worker reached it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
}

/// Arguments for `render.queue.list`.
///
/// `project_id` is required by the `Verb` trait shape; the spec
/// nominally allows it to be absent (in which case the verb enumerates
/// every open project), but the engine's verb dispatcher always carries
/// a `project_id` per §0.8, so we accept it and ignore it in v1.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderQueueListArgs {
    /// Required by the `Verb` trait shape; not read by the v1 impl.
    pub project_id: ProjectId,
    /// Optional state filter; defaults to `All` per §21.2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_filter: Option<QueueStateFilter>,
}

/// Envelope returned by `render.queue.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderQueueListData {
    /// Queue entries in `(project_id, -priority, added_at)` order.
    pub items: Vec<QueueEntry>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Error)]
/// Verb-level error type for `render.queue.list`.
pub enum RenderQueueListError {
    /// No verb-level runtime errors in v1 (empty queue always succeeds).
    #[error("render.queue.list: unreachable (no error variants)")]
    Unreachable,
}

/// Build the canonical `render.queue.list` data envelope.
fn build_data() -> RenderQueueListData {
    RenderQueueListData { items: Vec::new() }
}

/// Build the RFC 6902 patch for `render.queue.list`.
///
/// # Errors
///
/// No runtime errors are produced by this verb; the returned `Result`
/// exists for parity with the broader compute-patch API.
pub fn compute_patch(
    _prior: &Project,
    _args: &RenderQueueListArgs,
) -> Result<(Value, Vec<Value>, RenderQueueListData), RenderQueueListError> {
    Ok((json!([]), Vec::new(), build_data()))
}

/// Build the data envelope from `(args, post_state)`.
///
/// # Errors
///
/// Reuses [`compute_patch`], so this can only return reconstruction
/// errors introduced while rebuilding the deterministic envelope.
pub fn data_envelope_from_args(
    args: &RenderQueueListArgs,
    post_state: &Project,
) -> Result<RenderQueueListData, ReconstructError> {
    let (_, _, data) =
        compute_patch(post_state, args).map_err(|e| ReconstructError::Custom(e.to_string()))?;
    Ok(data)
}

impl From<RenderQueueListError> for VerbError {
    fn from(value: RenderQueueListError) -> Self {
        match value {
            RenderQueueListError::Unreachable => VerbError::BadArgs {
                detail: value.to_string(),
            },
        }
    }
}

/// The §0.8 verb for `render.queue.list`.
#[derive(Debug, Default)]
pub struct RenderQueueListVerb;

impl Verb for RenderQueueListVerb {
    fn verb(&self) -> &'static str {
        "render.queue.list"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: RenderQueueListArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("render.queue.list: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch = serde_json::from_value(patch_value).map_err(|err| {
            VerbError::Custom(format!(
                "render.queue.list: patch construction failed: {err}"
            ))
        })?;

        let data = serde_json::to_value(&data).map_err(|err| {
            VerbError::Custom(format!("render.queue.list: data envelope failed: {err}"))
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
        let typed: RenderQueueListArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "RenderQueueListArgs",
            })?;

        let envelope = data_envelope_from_args(&typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
