//! `render.queue.status` (§21.3) — seventy-fourth production verb in the engine.
//!
//! ## Spec quote (`spec/commands/render-queue.md` §21.3, abbreviated)
//!
//! > CLI: `verbreel render queue status [--project <id>] --queue_job_id <id>`
//! > MCP: `render.queue.status`
//! > Args: `project_id: string`, `queue_job_id: string`.
//! > Returns (`data`): same shape as one entry in
//! >   `render.queue.list.data.items[]` (a single `QueueEntry`).
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
//! read needs a `VerbContext` / storage facade threaded through
//! `ProjectStore::mutate_via_verb` — same architectural gap that
//! `render.queue.list` (§21.2) and `render.queue.clear` (§21.5) defer
//! queue persistence for. A future slice introduces `VerbContext` and
//! wires several deferred features at once.
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

/// Arguments for `render.queue.status`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderQueueStatusArgs {
    /// Target project id.
    pub project_id: ProjectId,
    /// The queue job to look up. v1 floor: never resolves.
    pub queue_job_id: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Error)]
/// Verb-level error type for `render.queue.status`.
pub enum RenderQueueStatusError {
    /// `queue_job_id` does not resolve in the project's queue. Maps to
    /// `E_QUEUE_JOB_NOT_FOUND`. In v1 floor this is returned for every
    /// well-formed call regardless of the id supplied.
    #[error(
        "render.queue.status: E_QUEUE_JOB_NOT_FOUND — queue_job_id `{queue_job_id}` does not \
         resolve in the project's queue"
    )]
    QueueJobNotFound {
        /// The id the caller supplied — surfaced as `details.queue_job_id`.
        queue_job_id: String,
    },
}

/// Build the RFC 6902 patch for `render.queue.status`.
///
/// v1 floor: always returns
/// [`RenderQueueStatusError::QueueJobNotFound`].
///
/// # Errors
///
/// Always errors with [`RenderQueueStatusError::QueueJobNotFound`] in
/// v1 — the queue is empty so no id resolves.
pub fn compute_patch(
    _prior: &Project,
    args: &RenderQueueStatusArgs,
) -> Result<(Value, Vec<Value>, Value), RenderQueueStatusError> {
    Err(RenderQueueStatusError::QueueJobNotFound {
        queue_job_id: args.queue_job_id.clone(),
    })
}

impl From<RenderQueueStatusError> for VerbError {
    fn from(value: RenderQueueStatusError) -> Self {
        match value {
            // QueueJobNotFound is a runtime-state error (queue miss), not an
            // arg-shape failure. Mapping to Custom keeps validate_command (§1.4)
            // honest: BadArgs there means "args malformed" and would mis-report
            // well-formed {project_id, queue_job_id} as invalid.
            RenderQueueStatusError::QueueJobNotFound { .. } => VerbError::Custom(value.to_string()),
        }
    }
}

/// The §0.8 verb for `render.queue.status`.
#[derive(Debug, Default)]
pub struct RenderQueueStatusVerb;

impl Verb for RenderQueueStatusVerb {
    fn verb(&self) -> &'static str {
        "render.queue.status"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: RenderQueueStatusArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("render.queue.status: args deserialize failed: {err}"),
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
                            "render.queue.status: patch construction failed: {err}"
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
        let _typed: RenderQueueStatusArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "RenderQueueStatusArgs",
            })?;

        // v1 floor: no successful event is ever recorded for this verb,
        // so the reconstructed envelope is null. See module doc.
        Ok(Value::Null)
    }
}
