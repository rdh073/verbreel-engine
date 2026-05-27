//! `timeline.undo` (§12.3) — v1 undo-stack floor.
//!
//! Real undo execution needs lifecycle undo-stack state and event-log
//! append context that are not available to pure `Verb::compute_patch`.
//! This slice therefore mirrors the `timeline.diff` / `render.status`
//! always-error pattern:
//! - argument-schema failure for `steps < 1` => `E_SCHEMA_VIOLATION`
//! - otherwise runtime-state floor => `E_NOTHING_TO_UNDO`
//! - `reconstruct()` returns `Value::Null`

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use verbreel_types::ProjectId;

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// Arguments for `timeline.undo`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimelineUndoArgs {
    /// Target project id.
    pub project_id: ProjectId,
    /// Requested undo steps. Defaults to `1` when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps: Option<i64>,
}

/// Data envelope for successful `timeline.undo`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimelineUndoData {
    /// Event ids that were undone.
    pub undone_event_ids: Vec<String>,
    /// Event id at the new history head after undo.
    pub current_event_id: String,
    /// Caller-requested undo step count.
    pub requested_steps: u32,
    /// Actual undo steps applied.
    pub actual_steps: u32,
}

/// Verb-level errors for `timeline.undo`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TimelineUndoError {
    /// `steps` violates the local schema minimum (`>= 1`).
    #[error("timeline.undo: E_SCHEMA_VIOLATION — {detail}")]
    SchemaViolation {
        /// Human-readable detail for schema failure.
        detail: String,
    },
    /// Runtime history state has nothing undoable.
    #[error("timeline.undo: E_NOTHING_TO_UNDO — {detail}")]
    NothingToUndo {
        /// Human-readable detail for runtime failure.
        detail: String,
    },
}

/// Resolve the effective undo step count, defaulting to `1`.
#[must_use]
pub fn resolved_steps(args: &TimelineUndoArgs) -> i64 {
    args.steps.unwrap_or(1)
}

/// Build the RFC 6902 patch for `timeline.undo`.
///
/// # Errors
///
/// Returns [`TimelineUndoError::SchemaViolation`] when `steps < 1`.
/// Returns [`TimelineUndoError::NothingToUndo`] for every well-formed
/// request in this v1 floor.
pub fn compute_patch(
    _prior: &Project,
    args: &TimelineUndoArgs,
) -> Result<(Value, Vec<Value>, TimelineUndoData), TimelineUndoError> {
    let requested_steps = resolved_steps(args);
    if requested_steps < 1 {
        return Err(TimelineUndoError::SchemaViolation {
            detail: format!("`steps` must be >= 1, got {requested_steps}"),
        });
    }

    Err(TimelineUndoError::NothingToUndo {
        detail: format!(
            "v1 floor: runtime undo stack unavailable; zero events undone for requested_steps={requested_steps}"
        ),
    })
}

impl From<TimelineUndoError> for VerbError {
    fn from(value: TimelineUndoError) -> Self {
        match value {
            // Arg-schema failure, not runtime state.
            TimelineUndoError::SchemaViolation { .. } => VerbError::BadArgs {
                detail: value.to_string(),
            },
            // Runtime-state error for well-formed args.
            TimelineUndoError::NothingToUndo { .. } => VerbError::Custom(value.to_string()),
        }
    }
}

/// The §0.8 verb for `timeline.undo`.
#[derive(Debug, Default)]
pub struct TimelineUndoVerb;

impl Verb for TimelineUndoVerb {
    fn verb(&self) -> &'static str {
        "timeline.undo"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: TimelineUndoArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("timeline.undo: args deserialize failed: {err}"),
            })?;

        match compute_patch(prior, &typed) {
            Ok((patch_value, warnings, data)) => {
                let patch: json_patch::Patch =
                    serde_json::from_value(patch_value).map_err(|err| {
                        VerbError::Custom(format!(
                            "timeline.undo: patch construction failed: {err}"
                        ))
                    })?;
                let data = serde_json::to_value(&data).map_err(|err| {
                    VerbError::Custom(format!("timeline.undo: data envelope failed: {err}"))
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
        let _typed: TimelineUndoArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "TimelineUndoArgs",
            })?;

        Ok(Value::Null)
    }
}
