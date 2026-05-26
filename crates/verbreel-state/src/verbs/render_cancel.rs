//! `render.cancel` (§11.3) — seventy-seventh production verb in the engine.
//!
//! ## Spec quote (`spec/commands/render.md` §11.3, abbreviated)
//!
//! > CLI: `verbreel render cancel [--project <id>] --job_id <id>`
//! > MCP: `render.cancel`
//! > Args: `project_id: string`, `job_id: string`.
//! > Returns (`data`): `{ job_id, state: "canceled", partial_path? }`.
//! > Errors: `E_JOB_NOT_FOUND` — `job_id` does not resolve to a render
//! >   job for this project. `details.job_id`.
//!
//! ## v1 floor — always errors with `E_JOB_NOT_FOUND`.
//!
//! No `render.start` verb exists yet, so no render job is ever in
//! flight. Every queried `job_id` truly does not resolve. The actual
//! render-worker cancel (signaling the in-progress encoder, renaming
//! its `.tmp` output to `.partial`, and surfacing that path through
//! `RenderCancelData.partial_path`) is deferred until the render
//! engine ships. The same `VerbContext` / storage facade plumbing that
//! `render.queue.cancel` (§21.4) defers is required here — when it
//! lands, several deferred render-arc features wire at once.
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

/// Arguments for `render.cancel`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderCancelArgs {
    /// Target project id.
    pub project_id: ProjectId,
    /// The render job to cancel. v1 floor: never resolves.
    pub job_id: String,
}

/// Response envelope for a successful `render.cancel`.
///
/// v1 floor never constructs this shape (every call errors), but the
/// type is defined here so downstream consumers (CLI, MCP) can pin
/// against the spec'd response shape. `state` is always `"canceled"`
/// on the success path; `partial_path` is `Some(...)` only when the
/// canceled job had persisted partial output left behind by the
/// encoder (render-worker integration is deferred).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderCancelData {
    /// Echo of the canceled job's id.
    pub job_id: String,
    /// Always `"canceled"` on a successful cancel.
    pub state: String,
    /// Path to any persisted partial output left behind by the
    /// canceled encoder. `None` when no partial output was written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partial_path: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Error)]
/// Verb-level error type for `render.cancel`.
pub enum RenderCancelError {
    /// `job_id` does not resolve to a render job for this project. Maps
    /// to `E_JOB_NOT_FOUND`. In v1 floor this is returned for every
    /// well-formed call regardless of the id supplied.
    #[error(
        "render.cancel: E_JOB_NOT_FOUND — job_id `{job_id}` does not resolve to a render job for \
         this project"
    )]
    JobNotFound {
        /// The id the caller supplied — surfaced as `details.job_id`.
        job_id: String,
    },
}

/// Build the RFC 6902 patch for `render.cancel`.
///
/// v1 floor: always returns [`RenderCancelError::JobNotFound`].
///
/// # Errors
///
/// Always errors with [`RenderCancelError::JobNotFound`] in v1 — no
/// render job exists so no id resolves.
pub fn compute_patch(
    _prior: &Project,
    args: &RenderCancelArgs,
) -> Result<(Value, Vec<Value>, Value), RenderCancelError> {
    Err(RenderCancelError::JobNotFound {
        job_id: args.job_id.clone(),
    })
}

impl From<RenderCancelError> for VerbError {
    fn from(value: RenderCancelError) -> Self {
        match value {
            // JobNotFound is a runtime-state error (no render job with that id),
            // not an arg-shape failure. Mapping to Custom keeps validate_command
            // (§1.4) honest: BadArgs there means "args malformed" and would
            // mis-report well-formed {project_id, job_id} as invalid.
            RenderCancelError::JobNotFound { .. } => VerbError::Custom(value.to_string()),
        }
    }
}

/// The §0.8 verb for `render.cancel`.
#[derive(Debug, Default)]
pub struct RenderCancelVerb;

impl Verb for RenderCancelVerb {
    fn verb(&self) -> &'static str {
        "render.cancel"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: RenderCancelArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("render.cancel: args deserialize failed: {err}"),
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
                            "render.cancel: patch construction failed: {err}"
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
        let _typed: RenderCancelArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "RenderCancelArgs",
            })?;

        // v1 floor: no successful event is ever recorded for this verb,
        // so the reconstructed envelope is null. See module doc.
        Ok(Value::Null)
    }
}
