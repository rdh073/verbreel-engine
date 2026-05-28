//! `render.queue.cancel` (§21.4) — seventy-fifth production verb in the engine.
//!
//! ## Spec quote (`spec/commands/render-queue.md` §21.4, abbreviated)
//!
//! > CLI: `verbreel render queue cancel [--project <id>] --queue_job_id <id>`
//! > MCP: `render.queue.cancel`
//! > Args: `project_id: string`, `queue_job_id: string`.
//! > Returns (`data`): `{ queue_job_id, state: "canceled", partial_path? }`.
//! > Errors: `E_QUEUE_JOB_NOT_FOUND` — `queue_job_id` does not resolve
//! >   in the project's queue. `details.queue_job_id`.
//!
//! ## v1 floor — always errors with `E_QUEUE_JOB_NOT_FOUND`.
//!
//! Per §21.7, the queue's source of truth is the
//! `~/.verbreel/render-queue.json` persistence file shared across all
//! Verbreel processes on the host. The `Verb` trait's purity contract
//! forbids file I/O in `compute_patch`, so v1 cannot consult the
//! persistence file. The v1 floor is also semantically correct:
//! `render.queue.add` is a queue-enqueue floor that always errors with
//! `E_QUEUE_FULL`, so no successful enqueue is recorded and the queue is
//! genuinely empty — every queried `queue_job_id` truly does not
//! resolve. Wiring the file
//! read/write needs a `VerbContext` / storage facade threaded through
//! `ProjectStore::mutate_via_verb` — same architectural gap that
//! `render.queue.list` (§21.2), `render.queue.clear` (§21.5), and
//! `render.queue.status` (§21.3) defer queue persistence for. A future
//! slice introduces `VerbContext` and wires several deferred features
//! at once.
//!
//! ## Reconstructor framing for an always-errors verb.
//!
//! `compute_patch` always returns `Err`, which means no successful event
//! is ever appended to `events.jsonl` (the §0.8 write-ordering rule
//! requires a successful patch before an event is written). The
//! reconstruct path is therefore unreachable in production v1.
//! It still has to clear the §0.8 startup gate against the fixture in
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

/// Arguments for `render.queue.cancel`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderQueueCancelArgs {
    /// Target project id.
    pub project_id: ProjectId,
    /// The queue job to cancel. v1 floor: never resolves.
    pub queue_job_id: String,
}

/// Response envelope for a successful `render.queue.cancel`.
///
/// v1 floor never constructs this shape (every call errors), but the
/// type is defined here so downstream consumers (CLI, MCP) can pin
/// against the spec'd response shape. `state` is always `"canceled"`
/// on the success path; `partial_path` is `Some(...)` only when the
/// canceled job was in a `running` state with persisted partial
/// output (queue persistence is deferred).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderQueueCancelData {
    /// Echo of the canceled job's id.
    pub queue_job_id: String,
    /// Always `"canceled"` on a successful cancel.
    pub state: String,
    /// Path to any persisted partial output left behind by a canceled
    /// `running` job. `None` for canceled `queued` jobs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partial_path: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Error)]
/// Verb-level error type for `render.queue.cancel`.
pub enum RenderQueueCancelError {
    /// `queue_job_id` does not resolve in the project's queue. Maps to
    /// `E_QUEUE_JOB_NOT_FOUND`. In v1 floor this is returned for every
    /// well-formed call regardless of the id supplied.
    #[error(
        "render.queue.cancel: E_QUEUE_JOB_NOT_FOUND — queue_job_id `{queue_job_id}` does not \
         resolve in the project's queue"
    )]
    QueueJobNotFound {
        /// The id the caller supplied — surfaced as `details.queue_job_id`.
        queue_job_id: String,
    },
}

/// Build the RFC 6902 patch for `render.queue.cancel`.
///
/// v1 floor: always returns
/// [`RenderQueueCancelError::QueueJobNotFound`].
///
/// # Errors
///
/// Always errors with [`RenderQueueCancelError::QueueJobNotFound`] in
/// v1 — the queue is empty so no id resolves.
pub fn compute_patch(
    _prior: &Project,
    args: &RenderQueueCancelArgs,
) -> Result<(Value, Vec<Value>, Value), RenderQueueCancelError> {
    Err(RenderQueueCancelError::QueueJobNotFound {
        queue_job_id: args.queue_job_id.clone(),
    })
}

impl From<RenderQueueCancelError> for VerbError {
    fn from(value: RenderQueueCancelError) -> Self {
        match value {
            // QueueJobNotFound is a runtime-state error (queue miss), not an
            // arg-shape failure. Mapping to Custom keeps validate_command (§1.4)
            // honest: BadArgs there means "args malformed" and would mis-report
            // well-formed {project_id, queue_job_id} as invalid.
            RenderQueueCancelError::QueueJobNotFound { .. } => VerbError::Custom(value.to_string()),
        }
    }
}

/// The §0.8 verb for `render.queue.cancel`.
#[derive(Debug, Default)]
pub struct RenderQueueCancelVerb;

impl Verb for RenderQueueCancelVerb {
    fn verb(&self) -> &'static str {
        "render.queue.cancel"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: RenderQueueCancelArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("render.queue.cancel: args deserialize failed: {err}"),
            })?;

        // v1 floor: compute_patch always returns Err with
        // E_QUEUE_JOB_NOT_FOUND, so the `Ok` branch below is
        // structurally unreachable and only exists to keep the trait
        // shape consistent with other verbs.
        match compute_patch(prior, &typed) {
            Ok((patch_value, warnings, data)) => {
                let patch: json_patch::Patch =
                    serde_json::from_value(patch_value).map_err(|err| {
                        VerbError::Custom(format!(
                            "render.queue.cancel: patch construction failed: {err}"
                        ))
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
        let _typed: RenderQueueCancelArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "RenderQueueCancelArgs",
            })?;

        // v1 floor: no successful event is ever recorded for this verb,
        // so the reconstructed envelope is null. See module doc.
        Ok(Value::Null)
    }
}
