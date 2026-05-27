//! `preview.session.close` (§15.5) — seventy-ninth production verb in
//! the engine. Opens the preview-session arc (0/6 → 1/6).
//!
//! ## Spec quote (`spec/commands/preview.md` §15.5, abbreviated)
//!
//! > CLI: `verbreel preview session close [--project <id>] --session_id <id>`
//! > MCP: `preview.session.close`
//! > Args: `project_id: string`, `session_id: string`.
//! > Returns (`data`): `{ closed, forced, final_at_tk, dropped_frames }`.
//! > Errors: `E_PREVIEW_SESSION_NOT_FOUND` — `session_id` does not
//! >   resolve to an active preview session for this project.
//! >   `details.session_id`.
//!
//! ## v1 floor — always errors with `E_PREVIEW_SESSION_NOT_FOUND`.
//!
//! No `preview.session.create` verb exists yet, so no preview session
//! is ever in flight. Every queried `session_id` truly does not
//! resolve. The actual session-worker termination (cooperative-cancel
//! handshake, channel teardown, final-notification emission on the
//! streaming channel, drop-frame accounting since the most recent
//! `preview.play`) is deferred until the preview engine ships. The
//! same `VerbContext` / storage facade plumbing that `render.cancel`
//! (§11.3) and `render.status` (§11.2) defer is required here — when
//! it lands, several deferred preview-arc features wire at once.
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

/// Arguments for `preview.session.close`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreviewSessionCloseArgs {
    /// Target project id.
    pub project_id: ProjectId,
    /// The preview session to close. v1 floor: never resolves.
    pub session_id: String,
}

/// Response envelope for a successful `preview.session.close`.
///
/// v1 floor never constructs this shape (every call errors), but the
/// type is defined here so downstream consumers (CLI, MCP) can pin
/// against the spec'd response shape. All four fields are populated
/// unconditionally on the success path:
/// - `closed` is always `true` on a successful close (the verb only
///   returns `Ok` if termination actually happened).
/// - `forced` is `true` iff the session worker did not respond to the
///   cooperative-cancel signal within the spec'd timeout and was torn
///   down forcibly.
/// - `final_at_tk` is the playhead position at the moment termination
///   completed.
/// - `dropped_frames` is the cumulative dropped-frame count since the
///   most recent `preview.play` call on this session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviewSessionCloseData {
    /// Always `true` on a successful close.
    pub closed: bool,
    /// `true` iff the worker did not respond within the timeout and
    /// the engine performed a forced tear-down.
    pub forced: bool,
    /// Playhead at the moment termination completed, in ticks.
    pub final_at_tk: i64,
    /// Cumulative dropped frames since the most recent `preview.play`.
    pub dropped_frames: u64,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Error)]
/// Verb-level error type for `preview.session.close`.
pub enum PreviewSessionCloseError {
    /// `session_id` does not resolve to an active preview session for
    /// this project. Maps to `E_PREVIEW_SESSION_NOT_FOUND`. In v1 floor
    /// this is returned for every well-formed call regardless of the id
    /// supplied.
    #[error(
        "preview.session.close: E_PREVIEW_SESSION_NOT_FOUND — session_id `{session_id}` does not \
         resolve to an active preview session for this project"
    )]
    SessionNotFound {
        /// The id the caller supplied — surfaced as `details.session_id`.
        session_id: String,
    },
}

/// Build the RFC 6902 patch for `preview.session.close`.
///
/// v1 floor: always returns [`PreviewSessionCloseError::SessionNotFound`].
///
/// # Errors
///
/// Always errors with [`PreviewSessionCloseError::SessionNotFound`] in
/// v1 — no preview session exists so no id resolves.
pub fn compute_patch(
    _prior: &Project,
    args: &PreviewSessionCloseArgs,
) -> Result<(Value, Vec<Value>, Value), PreviewSessionCloseError> {
    Err(PreviewSessionCloseError::SessionNotFound {
        session_id: args.session_id.clone(),
    })
}

impl From<PreviewSessionCloseError> for VerbError {
    fn from(value: PreviewSessionCloseError) -> Self {
        match value {
            // SessionNotFound is a runtime-state error (no preview session with
            // that id), not an arg-shape failure. Mapping to Custom keeps
            // validate_command (§1.4) honest: BadArgs there means "args
            // malformed" and would mis-report well-formed
            // {project_id, session_id} as invalid.
            PreviewSessionCloseError::SessionNotFound { .. } => {
                VerbError::Custom(value.to_string())
            }
        }
    }
}

/// The §0.8 verb for `preview.session.close`.
#[derive(Debug, Default)]
pub struct PreviewSessionCloseVerb;

impl Verb for PreviewSessionCloseVerb {
    fn verb(&self) -> &'static str {
        "preview.session.close"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: PreviewSessionCloseArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("preview.session.close: args deserialize failed: {err}"),
            })?;

        // v1 floor: compute_patch always returns Err with
        // E_PREVIEW_SESSION_NOT_FOUND, so the `Ok` branch below is
        // structurally unreachable and only exists to keep the trait
        // shape consistent with other verbs.
        match compute_patch(prior, &typed) {
            Ok((patch_value, warnings, data)) => {
                let patch: json_patch::Patch =
                    serde_json::from_value(patch_value).map_err(|err| {
                        VerbError::Custom(format!(
                            "preview.session.close: patch construction failed: {err}"
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
        let _typed: PreviewSessionCloseArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "PreviewSessionCloseArgs",
            })?;

        // v1 floor: no successful event is ever recorded for this verb,
        // so the reconstructed envelope is null. See module doc.
        Ok(Value::Null)
    }
}
