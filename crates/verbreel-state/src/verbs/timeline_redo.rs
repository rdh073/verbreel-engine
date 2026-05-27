//! `timeline.redo` (§12.4) — v1 redo-stack floor.
//!
//! Real redo execution needs lifecycle redo-stack state and event-log
//! append context that are not available to pure `Verb::compute_patch`.
//! This slice mirrors the `timeline.diff` / `render.status`
//! always-error pattern:
//! - argument-schema failure for `steps < 1` => `E_SCHEMA_VIOLATION`
//! - otherwise runtime-state floor => `E_NOTHING_TO_REDO`
//! - `reconstruct()` returns `Value::Null`

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use verbreel_types::ProjectId;

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// Arguments for `timeline.redo`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimelineRedoArgs {
    /// Target project id.
    pub project_id: ProjectId,
    /// Requested redo steps. Defaults to `1` when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps: Option<i64>,
}

/// Data envelope for successful `timeline.redo`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimelineRedoData {
    /// Event ids that were redone.
    pub redone_event_ids: Vec<String>,
    /// Event id at the new history head after redo.
    pub current_event_id: String,
    /// Caller-requested redo step count.
    pub requested_steps: u32,
    /// Actual redo steps applied.
    pub actual_steps: u32,
}

/// Verb-level errors for `timeline.redo`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TimelineRedoError {
    /// `steps` violates the local schema minimum (`>= 1`).
    #[error("timeline.redo: E_SCHEMA_VIOLATION — {detail}")]
    SchemaViolation {
        /// Human-readable detail for schema failure.
        detail: String,
    },
    /// Runtime history state has nothing redoable.
    #[error("timeline.redo: E_NOTHING_TO_REDO — {detail}")]
    NothingToRedo {
        /// Human-readable detail for runtime failure.
        detail: String,
    },
}

/// Resolve the effective redo step count, defaulting to `1`.
#[must_use]
pub fn resolved_steps(args: &TimelineRedoArgs) -> i64 {
    args.steps.unwrap_or(1)
}

/// Build the RFC 6902 patch for `timeline.redo`.
///
/// # Errors
///
/// Returns [`TimelineRedoError::SchemaViolation`] when `steps < 1`.
/// Returns [`TimelineRedoError::NothingToRedo`] for every well-formed
/// request in this v1 floor.
pub fn compute_patch(
    _prior: &Project,
    args: &TimelineRedoArgs,
) -> Result<(Value, Vec<Value>, TimelineRedoData), TimelineRedoError> {
    let requested_steps = resolved_steps(args);
    if requested_steps < 1 {
        return Err(TimelineRedoError::SchemaViolation {
            detail: format!("`steps` must be >= 1, got {requested_steps}"),
        });
    }

    Err(TimelineRedoError::NothingToRedo {
        detail: format!(
            "v1 floor: runtime redo stack unavailable; zero events redone for requested_steps={requested_steps}"
        ),
    })
}

impl From<TimelineRedoError> for VerbError {
    fn from(value: TimelineRedoError) -> Self {
        match value {
            // Arg-schema failure, not runtime state.
            TimelineRedoError::SchemaViolation { .. } => VerbError::BadArgs {
                detail: value.to_string(),
            },
            // Runtime-state error for well-formed args.
            TimelineRedoError::NothingToRedo { .. } => VerbError::Custom(value.to_string()),
        }
    }
}

/// The §0.8 verb for `timeline.redo`.
#[derive(Debug, Default)]
pub struct TimelineRedoVerb;

impl Verb for TimelineRedoVerb {
    fn verb(&self) -> &'static str {
        "timeline.redo"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: TimelineRedoArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("timeline.redo: args deserialize failed: {err}"),
            })?;

        match compute_patch(prior, &typed) {
            Ok((patch_value, warnings, data)) => {
                let patch: json_patch::Patch =
                    serde_json::from_value(patch_value).map_err(|err| {
                        VerbError::Custom(format!(
                            "timeline.redo: patch construction failed: {err}"
                        ))
                    })?;
                let data = serde_json::to_value(&data).map_err(|err| {
                    VerbError::Custom(format!("timeline.redo: data envelope failed: {err}"))
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
        let _typed: TimelineRedoArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "TimelineRedoArgs",
            })?;

        Ok(Value::Null)
    }
}
