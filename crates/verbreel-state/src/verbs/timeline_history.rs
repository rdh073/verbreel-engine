//! `timeline.history` (§12.6) — v1 event-ring floor.
//!
//! Real `timeline.history` requires lifecycle-owned in-memory event-ring
//! context that is not available to the pure `Verb::compute_patch`
//! contract. This slice therefore ships the published read-only floor:
//! well-formed args always succeed with `patch: []`, no warnings, and
//! `data: { events: [] }`.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::ProjectId;

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// Arguments for `timeline.history`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimelineHistoryArgs {
    /// Target project id.
    pub project_id: ProjectId,
    /// Optional maximum number of rows requested by the caller.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<i64>,
    /// Optional lower-bound event token (`"empty"` is accepted).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    /// Optional flag to include effectively-undone rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_undone: Option<bool>,
}

/// Timeline history event kind wire literals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TimelineHistoryEventKind {
    /// `apply` event row.
    Apply,
    /// `undo` event row.
    Undo,
    /// `redo` event row.
    Redo,
}

/// Single event row returned by `timeline.history`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimelineHistoryEvent {
    /// Event id from history storage.
    pub id: String,
    /// Recorded verb id.
    pub verb: String,
    /// Recorded verb args payload.
    pub args: Value,
    /// RFC 3339 event timestamp.
    pub ts: String,
    /// Event kind.
    pub kind: TimelineHistoryEventKind,
    /// Parent event id for undo/redo rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_event_id: Option<String>,
    /// Whether the target apply event is currently effectively undone.
    pub effectively_undone: bool,
}

/// Data envelope for successful `timeline.history`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimelineHistoryData {
    /// Event rows in bounded history order.
    pub events: Vec<TimelineHistoryEvent>,
}

/// Verb-level errors for `timeline.history`.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TimelineHistoryError {
    /// `timeline.history` has no runtime error variants in v1.
    #[error("timeline.history: unreachable (no error variants)")]
    Unreachable,
}

fn build_data() -> TimelineHistoryData {
    TimelineHistoryData { events: Vec::new() }
}

/// Build the RFC 6902 patch for `timeline.history`.
///
/// # Errors
///
/// No runtime errors are produced by this v1 read-only floor.
pub fn compute_patch(
    _prior: &Project,
    _args: &TimelineHistoryArgs,
) -> Result<(Value, Vec<Value>, TimelineHistoryData), TimelineHistoryError> {
    Ok((json!([]), Vec::new(), build_data()))
}

/// Build the data envelope from `(args, post_state)`.
///
/// # Errors
///
/// Reuses [`compute_patch`] and only returns reconstruction errors
/// introduced while rebuilding the deterministic envelope.
pub fn data_envelope_from_args(
    args: &TimelineHistoryArgs,
    post_state: &Project,
) -> Result<TimelineHistoryData, ReconstructError> {
    let (_, _, data) =
        compute_patch(post_state, args).map_err(|e| ReconstructError::Custom(e.to_string()))?;
    Ok(data)
}

impl From<TimelineHistoryError> for VerbError {
    fn from(value: TimelineHistoryError) -> Self {
        match value {
            TimelineHistoryError::Unreachable => VerbError::BadArgs {
                detail: value.to_string(),
            },
        }
    }
}

/// The §0.8 verb for `timeline.history`.
#[derive(Debug, Default)]
pub struct TimelineHistoryVerb;

impl Verb for TimelineHistoryVerb {
    fn verb(&self) -> &'static str {
        "timeline.history"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: TimelineHistoryArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("timeline.history: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch = serde_json::from_value(patch_value).map_err(|err| {
            VerbError::Custom(format!(
                "timeline.history: patch construction failed: {err}"
            ))
        })?;
        let data = serde_json::to_value(&data).map_err(|err| {
            VerbError::Custom(format!("timeline.history: data envelope failed: {err}"))
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
        let typed: TimelineHistoryArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "TimelineHistoryArgs",
            })?;
        let envelope = data_envelope_from_args(&typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
