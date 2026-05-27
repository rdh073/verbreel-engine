//! `tracker.apply` (§18.3) — v1 floor.
//!
//! Real `tracker.apply` needs cached-trace I/O plus keyframe mutation
//! context that is intentionally outside pure [`Verb::compute_patch`].
//! This slice preserves the published wire surface and validates only
//! cheap pure-time constraints. For every existing tracker, the verb
//! returns `E_TRACKER_NOT_RUN`.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use verbreel_types::ProjectId;

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// Default properties used by `tracker.apply` when `properties` is omitted.
pub const DEFAULT_TRACKER_APPLY_PROPERTIES: [&str; 2] = ["transform.x", "transform.y"];

const SCALE_TRACKER_APPLY_PROPERTIES: [&str; 4] = [
    "transform.x",
    "transform.y",
    "transform.scale_x",
    "transform.scale_y",
];

const TRACKER_NOT_RUN_HINT: &str = "call tracker.run before tracker.apply";
const TRACKER_CACHE_MISSING_HINT: &str =
    "cache file missing; tracker.run will auto-rebuild per §18.2 dangling-cache recovery";

const fn default_offset_x() -> f64 {
    0.0
}

const fn default_offset_y() -> f64 {
    0.0
}

const fn default_offset_scale() -> f64 {
    1.0
}

/// Optional constant offset applied to every trace sample.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrackerApplyOffset {
    /// Constant X offset in canvas units.
    #[serde(default = "default_offset_x")]
    pub x: f64,
    /// Constant Y offset in canvas units.
    #[serde(default = "default_offset_y")]
    pub y: f64,
    /// Constant multiplicative scale offset.
    #[serde(default = "default_offset_scale")]
    pub scale: f64,
}

/// Arguments for `tracker.apply`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrackerApplyArgs {
    /// Target project id.
    pub project_id: ProjectId,
    /// Existing tracker id from `tracker.create`.
    pub tracker_id: String,
    /// Target clip selector (bare UUID or `clip:<UUID>`).
    pub to_clip: String,
    /// Requested transform properties.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub properties: Option<Vec<String>>,
    /// Constant trace offset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<TrackerApplyOffset>,
    /// Optional decimation cadence in ticks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decimate_to_every: Option<i64>,
}

/// Envelope returned by a future successful `tracker.apply`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackerApplyData {
    /// Resolved target clip id.
    pub to_clip_id: String,
    /// Keyframe ids added by this application.
    pub added_keyframe_ids: Vec<String>,
    /// Prior tracker-owned keyframe ids removed by re-apply.
    pub removed_keyframe_ids: Vec<String>,
    /// Properties that received keyframes.
    pub properties: Vec<String>,
    /// Number of applied trace samples.
    pub applied_sample_count: i64,
}

/// Verb-level errors for `tracker.apply`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TrackerApplyError {
    /// `tracker_id` does not resolve in `Project.trackers[]`.
    #[error("tracker.apply: E_TRACKER_NOT_FOUND — tracker_id `{tracker_id}` not found")]
    TrackerNotFound {
        /// Missing tracker id.
        tracker_id: String,
    },

    /// Tracker exists but has no usable trace cache in v1 floor.
    #[error(
        "tracker.apply: E_TRACKER_NOT_RUN — tracker `{tracker_id}` cache_missing={cache_missing} \
         expected_cache_path={expected_cache_path:?} hint=`{hint}`"
    )]
    TrackerNotRun {
        /// Existing tracker id.
        tracker_id: String,
        /// `false` for never-run, `true` for dangling-cache shape.
        cache_missing: bool,
        /// Missing cache path for dangling-cache shape.
        expected_cache_path: Option<String>,
        /// Operator hint.
        hint: String,
    },

    /// Pure-args failure mapped to `E_TRACKER_BAD_PARAMS`.
    #[error("tracker.apply: E_TRACKER_BAD_PARAMS — field `{field}`: {hint}")]
    BadParams {
        /// Failing field.
        field: String,
        /// Guidance text.
        hint: String,
    },

    /// Selector parse failure for `to_clip`.
    #[error("tracker.apply: E_BAD_SELECTOR — {detail}")]
    BadSelector {
        /// Parse detail.
        detail: String,
    },

    /// Selector noun prefix was not `clip`.
    #[error("tracker.apply: E_SELECTOR_KIND_MISMATCH — actual_kind `{actual_kind}`")]
    SelectorKindMismatch {
        /// Offending noun prefix.
        actual_kind: String,
    },

    /// Reserved v1-floor variant per §18.3.
    #[error("tracker.apply: E_NOT_FOUND — clip_id `{clip_id}` not found")]
    NotFound {
        /// Missing clip id.
        clip_id: String,
    },

    /// Reserved v1-floor variant per §18.3.
    #[error("tracker.apply: E_NO_MATCH — selector `{selector}` matched no clip")]
    NoMatch {
        /// Selector that matched nothing.
        selector: String,
    },

    /// Reserved v1-floor variant per §18.3.
    #[error("tracker.apply: E_LOCKED — clip `{clip_id}` is locked")]
    Locked {
        /// Locked clip id.
        clip_id: String,
    },

    /// Reserved v1-floor variant per §18.3.
    #[error("tracker.apply: E_IO — cache_path `{cache_path}` errno `{errno}`")]
    Io {
        /// Cache path involved in I/O failure.
        cache_path: String,
        /// Platform errno or code.
        errno: String,
    },
}

impl From<TrackerApplyError> for VerbError {
    fn from(value: TrackerApplyError) -> Self {
        VerbError::Custom(value.to_string())
    }
}

fn resolve_properties(raw: Option<&Vec<String>>) -> Result<Vec<String>, TrackerApplyError> {
    match raw {
        None => Ok(DEFAULT_TRACKER_APPLY_PROPERTIES
            .iter()
            .map(|value| (*value).to_string())
            .collect()),
        Some(properties)
            if properties
                == &DEFAULT_TRACKER_APPLY_PROPERTIES
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                || properties
                    == &SCALE_TRACKER_APPLY_PROPERTIES
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>() =>
        {
            Ok(properties.clone())
        }
        Some(_) => Err(TrackerApplyError::BadParams {
            field: "properties".to_string(),
            hint: "expected one of the two accepted §18.3 property lists".to_string(),
        }),
    }
}

fn resolve_to_clip_id(raw: &str) -> Result<String, TrackerApplyError> {
    if let Some((prefix, body)) = raw.split_once(':') {
        match prefix {
            "clip" => {
                if body.parse::<uuid::Uuid>().is_err() {
                    return Err(TrackerApplyError::BadSelector {
                        detail: format!("`{body}` is not a valid UUID"),
                    });
                }
                Ok(body.to_string())
            }
            other => Err(TrackerApplyError::SelectorKindMismatch {
                actual_kind: other.to_string(),
            }),
        }
    } else {
        if raw.parse::<uuid::Uuid>().is_err() {
            return Err(TrackerApplyError::BadSelector {
                detail: format!("`{raw}` is not a valid UUID"),
            });
        }
        Ok(raw.to_string())
    }
}

fn has_scale_properties(properties: &[String]) -> bool {
    properties
        .iter()
        .any(|value| value == "transform.scale_x" || value == "transform.scale_y")
}

fn expected_cache_path(cache_hash: &str, cache_path: Option<&str>) -> Option<String> {
    if let Some(path) = cache_path.filter(|path| !path.is_empty()) {
        return Some(path.to_string());
    }

    let prefix: String = cache_hash.chars().take(16).collect();
    if prefix.is_empty() {
        return None;
    }
    Some(format!("cache/trackers/{prefix}.json"))
}

/// Build the RFC 6902 patch for `tracker.apply`.
///
/// v1 floor: always returns an error.
///
/// # Errors
///
/// Returns [`TrackerApplyError`] for malformed cheap params, selector
/// parse failures, missing tracker, or `E_TRACKER_NOT_RUN`.
pub fn compute_patch(
    prior: &Project,
    args: &TrackerApplyArgs,
) -> Result<(Value, Vec<Value>, Value), TrackerApplyError> {
    let properties = resolve_properties(args.properties.as_ref())?;

    if let Some(decimate_to_every) = args.decimate_to_every
        && decimate_to_every <= 0
    {
        return Err(TrackerApplyError::BadParams {
            field: "decimate_to_every".to_string(),
            hint: "must be a positive integer in ticks".to_string(),
        });
    }

    let _to_clip_id = resolve_to_clip_id(&args.to_clip)?;

    let tracker = prior
        .trackers
        .iter()
        .find(|tracker| {
            tracker.0.get("tracker_id").and_then(Value::as_str) == Some(args.tracker_id.as_str())
        })
        .ok_or_else(|| TrackerApplyError::TrackerNotFound {
            tracker_id: args.tracker_id.clone(),
        })?;

    let algorithm = tracker
        .0
        .get("algorithm")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    if algorithm == "optical_flow" && has_scale_properties(&properties) {
        return Err(TrackerApplyError::BadParams {
            field: "properties".to_string(),
            hint: "optical_flow tracks points; use ['transform.x','transform.y'] only".to_string(),
        });
    }

    let cache_hash = tracker
        .0
        .get("cache_hash")
        .and_then(Value::as_str)
        .unwrap_or("");

    if cache_hash.is_empty() {
        return Err(TrackerApplyError::TrackerNotRun {
            tracker_id: args.tracker_id.clone(),
            cache_missing: false,
            expected_cache_path: None,
            hint: TRACKER_NOT_RUN_HINT.to_string(),
        });
    }

    let cache_path = tracker.0.get("cache_path").and_then(Value::as_str);
    Err(TrackerApplyError::TrackerNotRun {
        tracker_id: args.tracker_id.clone(),
        cache_missing: true,
        expected_cache_path: expected_cache_path(cache_hash, cache_path),
        hint: TRACKER_CACHE_MISSING_HINT.to_string(),
    })
}

/// The §0.8 verb for `tracker.apply`.
#[derive(Debug, Default)]
pub struct TrackerApplyVerb;

impl Verb for TrackerApplyVerb {
    fn verb(&self) -> &'static str {
        "tracker.apply"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: TrackerApplyArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("tracker.apply: args deserialize failed: {err}"),
            })?;

        match compute_patch(prior, &typed) {
            Ok((patch_value, warnings, data)) => {
                let patch: json_patch::Patch =
                    serde_json::from_value(patch_value).map_err(|err| {
                        VerbError::Custom(format!(
                            "tracker.apply: patch construction failed: {err}"
                        ))
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
        let _typed: TrackerApplyArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "TrackerApplyArgs",
            })?;

        Ok(Value::Null)
    }
}
