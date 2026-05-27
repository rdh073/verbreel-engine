//! `tracker.run` (§18.2) — ninety-fourth production verb in the engine.
//!
//! ## v1 floor — always errors for existing trackers.
//!
//! Real `tracker.run` needs runtime/codec/cache/streaming context that
//! is unavailable inside pure [`Verb::compute_patch`]. This slice keeps
//! the published §18.2 wire surface and validates cheap pure-time
//! constraints (`from_tk`, `to_tk`, `sample_every_ticks`) plus tracker
//! existence in `Project.trackers[]`. For every existing tracker, the
//! verb returns `E_TRACKER_RUN_FAILED` with stage `algorithm_step`.
//!
//! ## Reconstructor framing for an always-errors verb.
//!
//! `compute_patch` always returns `Err`, so no successful event is
//! recorded in v1. `reconstruct()` still deserializes args for the
//! startup gate fixture and returns `Value::Null`.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;
use thiserror::Error;
use verbreel_types::ProjectId;

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// Arguments for `tracker.run`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrackerRunArgs {
    /// Target project id.
    pub project_id: ProjectId,
    /// Existing tracker id from `tracker.create`.
    pub tracker_id: String,
    /// Optional project-time lower bound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_tk: Option<i64>,
    /// Optional project-time upper bound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_tk: Option<i64>,
    /// Optional cadence in ticks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample_every_ticks: Option<i64>,
}

/// `BBox` trace extent summary for successful `tracker.run`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TrackerBBoxTraceSummary {
    /// Minimum x across all samples.
    pub min_x: f64,
    /// Maximum x across all samples.
    pub max_x: f64,
    /// Minimum y across all samples.
    pub min_y: f64,
    /// Maximum y across all samples.
    pub max_y: f64,
}

/// Envelope returned by a future successful `tracker.run`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackerRunData {
    /// Tracker id that was run.
    pub tracker_id: String,
    /// Number of samples produced.
    pub sample_count: i64,
    /// Absolute cache file path.
    pub cache_path: String,
    /// `true` when result was served from cache.
    pub cache_hit: bool,
    /// Arithmetic mean confidence across samples.
    pub mean_confidence: f64,
    /// Bounding extent of the trace.
    pub bbox_trace_summary: TrackerBBoxTraceSummary,
}

/// Runtime stage for `E_TRACKER_RUN_FAILED`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackerRunStage {
    /// Decoder/bootstrap stage.
    DecoderInit,
    /// Frame decode stage.
    FrameDecode,
    /// Per-frame tracker step stage.
    AlgorithmStep,
    /// Cache JSON write stage.
    CacheWrite,
}

impl fmt::Display for TrackerRunStage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let literal = match self {
            Self::DecoderInit => "decoder_init",
            Self::FrameDecode => "frame_decode",
            Self::AlgorithmStep => "algorithm_step",
            Self::CacheWrite => "cache_write",
        };
        f.write_str(literal)
    }
}

/// Verb-level errors for `tracker.run`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TrackerRunError {
    /// `tracker_id` does not resolve on `Project.trackers[]`.
    #[error("tracker.run: E_TRACKER_NOT_FOUND — tracker_id `{tracker_id}` not found")]
    TrackerNotFound {
        /// Missing tracker id.
        tracker_id: String,
    },

    /// Analysis pass failed at runtime.
    #[error(
        "tracker.run: E_TRACKER_RUN_FAILED — tracker `{tracker_id}` algorithm `{algorithm}` \
         failed at stage `{stage}`: {error}"
    )]
    RunFailed {
        /// Tracker id being executed.
        tracker_id: String,
        /// Tracker algorithm name from `Project.trackers[]`.
        algorithm: String,
        /// Runtime stage that failed.
        stage: TrackerRunStage,
        /// Human-readable failure detail.
        error: String,
    },

    /// Writer-class contention with another active long-op.
    #[error(
        "tracker.run: E_BUSY — active_verb `{active_verb}` job_id `{job_id}` running_since \
         `{running_since}`"
    )]
    Busy {
        /// Active writer-class verb.
        active_verb: String,
        /// Active job id.
        job_id: String,
        /// RFC 3339 start timestamp.
        running_since: String,
    },

    /// Requested analysis window fully outside bounds.
    #[error("tracker.run: E_BAD_RANGE — {bound}")]
    BadRange {
        /// Human-readable bound detail.
        bound: String,
    },

    /// Negative/zero time-like input where forbidden by §18.2.
    #[error("tracker.run: E_BAD_TIME — field `{field}` has invalid value `{value}`")]
    BadTime {
        /// Failing field name (`from_tk`, `to_tk`, or `sample_every_ticks`).
        field: String,
        /// Offending numeric value.
        value: i64,
    },
}

impl From<TrackerRunError> for VerbError {
    fn from(value: TrackerRunError) -> Self {
        VerbError::Custom(value.to_string())
    }
}

/// Build the RFC 6902 patch for `tracker.run`.
///
/// v1 floor: always returns an error. For existing trackers the error is
/// `E_TRACKER_RUN_FAILED`.
///
/// # Errors
///
/// Returns [`TrackerRunError`] for bad time values, missing tracker, or
/// the v1 floor runtime/context unavailability.
pub fn compute_patch(
    prior: &Project,
    args: &TrackerRunArgs,
) -> Result<(Value, Vec<Value>, Value), TrackerRunError> {
    if let Some(from_tk) = args.from_tk
        && from_tk < 0
    {
        return Err(TrackerRunError::BadTime {
            field: "from_tk".to_string(),
            value: from_tk,
        });
    }

    if let Some(to_tk) = args.to_tk
        && to_tk < 0
    {
        return Err(TrackerRunError::BadTime {
            field: "to_tk".to_string(),
            value: to_tk,
        });
    }

    if let Some(sample_every_ticks) = args.sample_every_ticks
        && sample_every_ticks <= 0
    {
        return Err(TrackerRunError::BadTime {
            field: "sample_every_ticks".to_string(),
            value: sample_every_ticks,
        });
    }

    let tracker = prior
        .trackers
        .iter()
        .find(|tracker| {
            tracker.0.get("tracker_id").and_then(Value::as_str) == Some(args.tracker_id.as_str())
        })
        .ok_or_else(|| TrackerRunError::TrackerNotFound {
            tracker_id: args.tracker_id.clone(),
        })?;

    let algorithm = tracker
        .0
        .get("algorithm")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    Err(TrackerRunError::RunFailed {
        tracker_id: args.tracker_id.clone(),
        algorithm,
        stage: TrackerRunStage::AlgorithmStep,
        error: "tracker runtime/cache context unavailable in the v1 floor".to_string(),
    })
}

/// The §0.8 verb for `tracker.run`.
#[derive(Debug, Default)]
pub struct TrackerRunVerb;

impl Verb for TrackerRunVerb {
    fn verb(&self) -> &'static str {
        "tracker.run"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: TrackerRunArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("tracker.run: args deserialize failed: {err}"),
            })?;

        match compute_patch(prior, &typed) {
            Ok((patch_value, warnings, data)) => {
                let patch: json_patch::Patch =
                    serde_json::from_value(patch_value).map_err(|err| {
                        VerbError::Custom(format!("tracker.run: patch construction failed: {err}"))
                    })?;
                Ok((patch, data, warnings))
            }
            Err(err) => Err(err.into()),
        }
    }

    fn reconstruct(
        &self,
        args: &Value,
        _patch: &Value,
        _warnings: &[Value],
        _post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let _typed: TrackerRunArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "TrackerRunArgs",
            })?;

        Ok(Value::Null)
    }
}
