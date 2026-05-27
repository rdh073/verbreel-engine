//! `timeline.diff` (§12.2) — event-log range verb, v1 floor.
//!
//! ## Spec quote (`spec/commands/timeline.md` §12.2, condensed)
//!
//! > CLI: `verbreel timeline diff [--project <id>] --since <event_id> [--until <event_id>]`
//! > MCP: `timeline.diff`
//! > Args: `project_id: string`, `since: string`, `until?: string`
//! > Returns (`data`): `{ patches: JsonPatch[]; events: { id: string; verb: string; ts: string }[] }`
//! > Errors: `E_EVENT_NOT_FOUND`
//!
//! ## v1 floor — always errors with `E_EVENT_NOT_FOUND`.
//!
//! Real `timeline.diff` requires reading `events.jsonl`, resolving
//! `(since, until)` against event-log order, and returning the recorded
//! patch/event rows. The pure `Verb::compute_patch` surface receives
//! only `&Project`, so it cannot reconstruct event-log ranges in this
//! slice. Well-formed args therefore map to one runtime-state error:
//! `E_EVENT_NOT_FOUND`.
//!
//! ## Reconstructor framing for an always-errors verb.
//!
//! Because `compute_patch` always returns `Err`, no successful
//! `timeline.diff` event is appended in v1. The reconstructor still has
//! to satisfy the §0.8 startup gate against `default_fixtures()`, so it
//! deserializes args and returns `Value::Null`.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use verbreel_types::ProjectId;

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// Arguments for `timeline.diff`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimelineDiffArgs {
    /// Target project id.
    pub project_id: ProjectId,
    /// Lower bound event token (opaque at args layer in this slice).
    pub since: String,
    /// Optional upper bound event token (opaque at args layer in this
    /// slice).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
}

/// Event metadata row returned by successful `timeline.diff`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimelineDiffEvent {
    /// Event id from `events.jsonl`.
    pub id: String,
    /// Verb id recorded on the event line.
    pub verb: String,
    /// RFC 3339 event timestamp.
    pub ts: String,
}

/// Data envelope for successful `timeline.diff`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimelineDiffData {
    /// Recorded RFC 6902 patches for the selected range.
    pub patches: Vec<Value>,
    /// Event metadata rows corresponding to `patches`.
    pub events: Vec<TimelineDiffEvent>,
}

/// Verb-level errors for `timeline.diff`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TimelineDiffError {
    /// Event range cannot be resolved for this request.
    #[error("timeline.diff: E_EVENT_NOT_FOUND — {detail}")]
    EventNotFound {
        /// Human-readable runtime detail.
        detail: String,
    },
}

/// Build the RFC 6902 patch for `timeline.diff`.
///
/// v1 floor: always returns [`TimelineDiffError::EventNotFound`].
///
/// # Errors
///
/// Always errors in v1 because event-log range resolution is deferred.
pub fn compute_patch(
    _prior: &Project,
    args: &TimelineDiffArgs,
) -> Result<(Value, Vec<Value>, TimelineDiffData), TimelineDiffError> {
    let detail = match &args.until {
        Some(until) => format!(
            "v1 floor: event-log range unavailable for since=`{}` until=`{until}`",
            args.since
        ),
        None => format!(
            "v1 floor: event-log range unavailable for since=`{}` until=<head>",
            args.since
        ),
    };

    Err(TimelineDiffError::EventNotFound { detail })
}

impl From<TimelineDiffError> for VerbError {
    fn from(value: TimelineDiffError) -> Self {
        match value {
            // Runtime-state miss (`since`/`until` resolution against event log),
            // not an arg-shape failure.
            TimelineDiffError::EventNotFound { .. } => VerbError::Custom(value.to_string()),
        }
    }
}

/// The §0.8 verb for `timeline.diff`.
#[derive(Debug, Default)]
pub struct TimelineDiffVerb;

impl Verb for TimelineDiffVerb {
    fn verb(&self) -> &'static str {
        "timeline.diff"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: TimelineDiffArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("timeline.diff: args deserialize failed: {err}"),
            })?;

        match compute_patch(prior, &typed) {
            Ok((patch_value, warnings, data)) => {
                let patch: json_patch::Patch =
                    serde_json::from_value(patch_value).map_err(|err| {
                        VerbError::Custom(format!(
                            "timeline.diff: patch construction failed: {err}"
                        ))
                    })?;
                let data = serde_json::to_value(&data).map_err(|err| {
                    VerbError::Custom(format!("timeline.diff: data envelope failed: {err}"))
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
        let _typed: TimelineDiffArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "TimelineDiffArgs",
            })?;

        Ok(Value::Null)
    }
}
