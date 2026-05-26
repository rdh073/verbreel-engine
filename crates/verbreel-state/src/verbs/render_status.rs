//! `render.status` (§11.2) — seventy-eighth production verb in the engine.
//!
//! ## Spec quote (`spec/commands/render.md` §11.2, abbreviated)
//!
//! > CLI: `verbreel render status [--project <id>] --job_id <id>`
//! > MCP: `render.status`
//! > Args: `project_id: string`, `job_id: string`.
//! > Returns (`data`): `{ job_id, state: "queued"|"running"|"completed"|
//! >   "failed"|"canceled", progress, started_at?, finished_at?,
//! >   error?: { code, message }, output_path? }`.
//! > Errors: `E_JOB_NOT_FOUND` — `job_id` does not resolve to a render
//! >   job for this project. `details.job_id`.
//!
//! ## v1 floor — always errors with `E_JOB_NOT_FOUND`.
//!
//! No `render.start` verb exists yet, so no render job is ever in
//! flight. Every queried `job_id` truly does not resolve. The actual
//! status read (polling the render-worker thread for `state`,
//! `progress`, `started_at`, `finished_at`, `error`, and `output_path`)
//! is deferred until the render engine ships. The same `VerbContext`
//! / storage facade plumbing that `render.queue.status` (§21.3) and
//! `render.cancel` (§11.3) defer is required here — when it lands,
//! several deferred render-arc features wire at once.
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

/// Arguments for `render.status`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderStatusArgs {
    /// Target project id.
    pub project_id: ProjectId,
    /// The render job to query. v1 floor: never resolves.
    pub job_id: String,
}

/// Lifecycle state of a render job — the closed set from §11.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RenderJobState {
    /// Sub-second window between `render.start` returning a `job_id`
    /// and the worker acquiring it (§11.2). Not a wait-in-line state.
    Queued,
    /// Worker is encoding.
    Running,
    /// Render finished and the output container was atomically renamed
    /// into place.
    Completed,
    /// Render finished with an error; `error` is populated.
    Failed,
    /// Job was canceled before completion; partial output (if any) was
    /// renamed to `<out_path>.partial`.
    Canceled,
}

/// Failure detail returned in the `error` field of [`RenderStatusData`]
/// when [`RenderJobState::Failed`] is reported.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderJobError {
    /// Spec error code (e.g. `E_RENDER_FAIL`).
    pub code: String,
    /// Human-readable message.
    pub message: String,
}

/// Response envelope for a successful `render.status`.
///
/// v1 floor never constructs this shape (every call errors), but the
/// type is defined here so downstream consumers (CLI, MCP) can pin
/// against the spec'd response shape. The four `Option` fields are
/// populated only when their corresponding lifecycle moment has been
/// reached:
/// - `started_at` is `Some` once `state ∈ {running, completed, failed,
///   canceled}` and the worker thread acquired the job.
/// - `finished_at` is `Some` only for terminal states (`completed`,
///   `failed`, `canceled`).
/// - `error` is `Some` only for `state == failed`.
/// - `output_path` is `Some` only for `state == completed` (and points
///   at the canonical `<out_path>`, never a `.tmp` / `.partial`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderStatusData {
    /// Echo of the queried job's id.
    pub job_id: String,
    /// Current lifecycle state.
    pub state: RenderJobState,
    /// Encode progress in `0.0..=1.0`. `0.0` until the worker reports
    /// frames; `1.0` for terminal `completed` state.
    pub progress: f64,
    /// RFC 3339 timestamp the worker started the job.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    /// RFC 3339 timestamp the job reached a terminal state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    /// Failure detail; populated iff `state == failed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RenderJobError>,
    /// Canonical output path; populated iff `state == completed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_path: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Error)]
/// Verb-level error type for `render.status`.
pub enum RenderStatusError {
    /// `job_id` does not resolve to a render job for this project. Maps
    /// to `E_JOB_NOT_FOUND`. In v1 floor this is returned for every
    /// well-formed call regardless of the id supplied.
    #[error(
        "render.status: E_JOB_NOT_FOUND — job_id `{job_id}` does not resolve to a render job for \
         this project"
    )]
    JobNotFound {
        /// The id the caller supplied — surfaced as `details.job_id`.
        job_id: String,
    },
}

/// Build the RFC 6902 patch for `render.status`.
///
/// v1 floor: always returns [`RenderStatusError::JobNotFound`].
///
/// # Errors
///
/// Always errors with [`RenderStatusError::JobNotFound`] in v1 — no
/// render job exists so no id resolves.
pub fn compute_patch(
    _prior: &Project,
    args: &RenderStatusArgs,
) -> Result<(Value, Vec<Value>, Value), RenderStatusError> {
    Err(RenderStatusError::JobNotFound {
        job_id: args.job_id.clone(),
    })
}

impl From<RenderStatusError> for VerbError {
    fn from(value: RenderStatusError) -> Self {
        match value {
            // JobNotFound is a runtime-state error (no render job with that id),
            // not an arg-shape failure. Mapping to Custom keeps validate_command
            // (§1.4) honest: BadArgs there means "args malformed" and would
            // mis-report well-formed {project_id, job_id} as invalid.
            RenderStatusError::JobNotFound { .. } => VerbError::Custom(value.to_string()),
        }
    }
}

/// The §0.8 verb for `render.status`.
#[derive(Debug, Default)]
pub struct RenderStatusVerb;

impl Verb for RenderStatusVerb {
    fn verb(&self) -> &'static str {
        "render.status"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: RenderStatusArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("render.status: args deserialize failed: {err}"),
            })?;

        // v1 floor: compute_patch always returns Err with
        // E_JOB_NOT_FOUND, so the `Ok` branch below is structurally
        // unreachable and only exists to keep the trait shape
        // consistent with other verbs.
        match compute_patch(prior, &typed) {
            Ok((patch_value, warnings, data)) => {
                let patch: json_patch::Patch =
                    serde_json::from_value(patch_value).map_err(|err| {
                        VerbError::Custom(format!(
                            "render.status: patch construction failed: {err}"
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
        let _typed: RenderStatusArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "RenderStatusArgs",
            })?;

        // v1 floor: no successful event is ever recorded for this verb,
        // so the reconstructed envelope is null. See module doc.
        Ok(Value::Null)
    }
}
