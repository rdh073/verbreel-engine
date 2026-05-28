//! `render.queue.add` (§21.1) — v1 queue-enqueue floor.
//!
//! Real `render.queue.add` needs queue persistence, runtime worker
//! scheduling, and wait-mode transport context outside pure
//! [`Verb::compute_patch`]. This v1 floor validates static arg shape
//! and cheap ranges, then returns a spec-coded queue-cap floor error.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use verbreel_types::ProjectId;

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use crate::verbs::render_list_presets;
use crate::verbs::render_queue_list::QueueJobState;
use crate::verbs::render_start::{RenderAudioCodec, RenderVideoCodec};

const BITRATE_MIN: i64 = 1;
const BITRATE_MAX: i64 = 1_000_000_000;
const H26X_CRF_MIN: i64 = 0;
const H26X_CRF_MAX: i64 = 51;
const VPX_AV1_CRF_MIN: i64 = 0;
const VPX_AV1_CRF_MAX: i64 = 63;
const V1_QUEUE_CAP: u32 = 0;
const V1_QUEUE_LENGTH: u32 = 0;
const CRF_BITRATE_HINT: &str = "spec forbids using crf and bitrate_bps together";
const PRORES_CRF_HINT: &str = "prores does not accept crf; remove crf or choose another codec";

/// Arguments for `render.queue.add`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(clippy::struct_excessive_bools)]
pub struct RenderQueueAddArgs {
    /// Target project id.
    pub project_id: ProjectId,
    /// Render preset name.
    pub preset: String,
    /// Output destination path.
    pub out_path: String,
    /// Optional start tick.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_tk: Option<i64>,
    /// Optional end tick.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_tk: Option<i64>,
    /// Optional video codec override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_codec: Option<RenderVideoCodec>,
    /// Optional audio codec override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_codec: Option<RenderAudioCodec>,
    /// Optional target bitrate in bits/sec.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bitrate_bps: Option<i64>,
    /// Optional codec CRF value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crf: Option<i64>,
    /// Determinism mode toggle (defaults false when omitted).
    #[serde(default)]
    pub deterministic: bool,
    /// Keep temp artifacts toggle (defaults false when omitted).
    #[serde(default)]
    pub keep_temp: bool,
    /// Overwrite destination toggle (defaults false when omitted).
    #[serde(default)]
    pub overwrite: bool,
    /// Queue priority (higher runs first).
    #[serde(default)]
    pub priority: i32,
    /// Wait for terminal state toggle (defaults false when omitted).
    #[serde(default)]
    pub wait: bool,
}

/// Terminal wait-mode error payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderQueueAddJobError {
    /// Terminal error code.
    pub code: String,
    /// Human-readable error message.
    pub message: String,
    /// Optional machine-readable details.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

/// Future success envelope for `render.queue.add`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderQueueAddData {
    /// Enqueued queue job id.
    pub queue_job_id: String,
    /// Project id associated with the queue entry.
    pub project_id: String,
    /// Queue position (`0` next to run; `-1` already running).
    pub position_in_queue: i64,
    /// Queue job state.
    pub state: QueueJobState,
    /// RFC3339 enqueue timestamp.
    pub added_at: String,
    /// RFC3339 start timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    /// RFC3339 finish timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    /// Final output path for completed jobs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_path: Option<String>,
    /// Partial output path for canceled jobs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partial_path: Option<String>,
    /// Terminal failure payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RenderQueueAddJobError>,
}

/// Verb-level failures for `render.queue.add`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RenderQueueAddError {
    /// Preset name does not exist in bundled presets.
    #[error("render.queue.add: E_RENDER_PRESET_UNKNOWN — preset `{preset}`")]
    RenderPresetUnknown {
        /// Unknown preset name.
        preset: String,
    },

    /// Numeric field is outside an accepted static range.
    #[error("render.queue.add: E_BAD_RANGE — {field}={value} outside {allowed}")]
    BadRange {
        /// Field name.
        field: String,
        /// Offending value.
        value: String,
        /// Accepted range description.
        allowed: String,
    },

    /// Start tick is negative.
    #[error("render.queue.add: E_BAD_TIME — from_tk {from_tk} must be >= 0")]
    BadTime {
        /// Offending `from_tk`.
        from_tk: i64,
    },

    /// Explicit range is empty or inverted.
    #[error("render.queue.add: E_RENDER_EMPTY_RANGE — from_tk={from_tk} to_tk={to_tk}")]
    RenderEmptyRange {
        /// Explicit start tick.
        from_tk: i64,
        /// Explicit end tick.
        to_tk: i64,
    },

    /// Incompatible arg combination.
    #[error("render.queue.add: E_ARGS_INCOMPATIBLE — {detail}; hint: {hint}")]
    ArgsIncompatible {
        /// Human-readable incompatibility detail.
        detail: String,
        /// Recovery hint.
        hint: String,
    },

    /// Reserved for path-root escape validation.
    #[error("render.queue.add: E_PATH_ESCAPE — path `{path}`")]
    PathEscape {
        /// Offending path.
        path: String,
    },

    /// Queue capacity exceeded.
    #[error(
        "render.queue.add: E_QUEUE_FULL — project_id `{project_id}` cap={cap} current_length={current_length}"
    )]
    QueueFull {
        /// Target project id.
        project_id: String,
        /// Effective cap.
        cap: u32,
        /// Current queue length.
        current_length: u32,
    },

    /// Reserved for transient enqueue busy state.
    #[error("render.queue.add: E_BUSY — {detail}")]
    Busy {
        /// Busy-state detail.
        detail: String,
    },
}

fn resolve_video_codec(args: &RenderQueueAddArgs) -> Result<RenderVideoCodec, RenderQueueAddError> {
    if let Some(codec) = args.video_codec {
        return Ok(codec);
    }

    let Some(preset) = render_list_presets::bundled_presets()
        .into_iter()
        .find(|preset| preset.name == args.preset)
    else {
        return Err(RenderQueueAddError::RenderPresetUnknown {
            preset: args.preset.clone(),
        });
    };

    match preset.video_codec.as_str() {
        "h264" => Ok(RenderVideoCodec::H264),
        "h265" => Ok(RenderVideoCodec::H265),
        "prores" => Ok(RenderVideoCodec::Prores),
        "vp9" => Ok(RenderVideoCodec::Vp9),
        "av1" => Ok(RenderVideoCodec::Av1),
        other => Err(RenderQueueAddError::ArgsIncompatible {
            detail: format!(
                "preset `{}` has unsupported video_codec `{other}` in bundled metadata",
                preset.name
            ),
            hint: "choose a different preset or pass video_codec explicitly".to_string(),
        }),
    }
}

fn validate_static(args: &RenderQueueAddArgs) -> Result<(), RenderQueueAddError> {
    if !render_list_presets::bundled_presets()
        .iter()
        .any(|preset| preset.name == args.preset)
    {
        return Err(RenderQueueAddError::RenderPresetUnknown {
            preset: args.preset.clone(),
        });
    }

    if let Some(from_tk) = args.from_tk
        && from_tk < 0
    {
        return Err(RenderQueueAddError::BadTime { from_tk });
    }

    if let (Some(from_tk), Some(to_tk)) = (args.from_tk, args.to_tk)
        && to_tk <= from_tk
    {
        return Err(RenderQueueAddError::RenderEmptyRange { from_tk, to_tk });
    }

    if let Some(bitrate_bps) = args.bitrate_bps
        && !(BITRATE_MIN..=BITRATE_MAX).contains(&bitrate_bps)
    {
        return Err(RenderQueueAddError::BadRange {
            field: "bitrate_bps".to_string(),
            value: bitrate_bps.to_string(),
            allowed: format!("[{BITRATE_MIN}, {BITRATE_MAX}]"),
        });
    }

    let resolved_video_codec = resolve_video_codec(args)?;

    if args.crf.is_some() && args.bitrate_bps.is_some() {
        return Err(RenderQueueAddError::ArgsIncompatible {
            detail: "crf cannot be combined with bitrate_bps".to_string(),
            hint: CRF_BITRATE_HINT.to_string(),
        });
    }

    if let Some(crf) = args.crf {
        match resolved_video_codec {
            RenderVideoCodec::Prores => {
                return Err(RenderQueueAddError::ArgsIncompatible {
                    detail: "crf is not allowed when video_codec resolves to prores".to_string(),
                    hint: PRORES_CRF_HINT.to_string(),
                });
            }
            RenderVideoCodec::H264 | RenderVideoCodec::H265 => {
                if !(H26X_CRF_MIN..=H26X_CRF_MAX).contains(&crf) {
                    return Err(RenderQueueAddError::BadRange {
                        field: "crf".to_string(),
                        value: crf.to_string(),
                        allowed: format!("[{H26X_CRF_MIN}, {H26X_CRF_MAX}]"),
                    });
                }
            }
            RenderVideoCodec::Vp9 | RenderVideoCodec::Av1 => {
                if !(VPX_AV1_CRF_MIN..=VPX_AV1_CRF_MAX).contains(&crf) {
                    return Err(RenderQueueAddError::BadRange {
                        field: "crf".to_string(),
                        value: crf.to_string(),
                        allowed: format!("[{VPX_AV1_CRF_MIN}, {VPX_AV1_CRF_MAX}]"),
                    });
                }
            }
        }
    }

    Ok(())
}

/// Build the RFC 6902 patch for `render.queue.add`.
///
/// v1 floor: static validation runs, then every otherwise-valid
/// request returns `E_QUEUE_FULL`.
///
/// # Errors
///
/// Returns static arg validation errors for caller-fixable issues.
/// Returns [`RenderQueueAddError::QueueFull`] for every otherwise-valid
/// enqueue request in this v1 floor.
pub fn compute_patch(
    _prior: &Project,
    args: &RenderQueueAddArgs,
) -> Result<(Value, Vec<Value>, Value), RenderQueueAddError> {
    validate_static(args)?;

    Err(RenderQueueAddError::QueueFull {
        project_id: args.project_id.to_string(),
        cap: V1_QUEUE_CAP,
        current_length: V1_QUEUE_LENGTH,
    })
}

impl From<RenderQueueAddError> for VerbError {
    fn from(value: RenderQueueAddError) -> Self {
        match value {
            RenderQueueAddError::RenderPresetUnknown { .. }
            | RenderQueueAddError::BadRange { .. }
            | RenderQueueAddError::BadTime { .. }
            | RenderQueueAddError::RenderEmptyRange { .. }
            | RenderQueueAddError::ArgsIncompatible { .. } => VerbError::BadArgs {
                detail: value.to_string(),
            },
            RenderQueueAddError::PathEscape { .. }
            | RenderQueueAddError::QueueFull { .. }
            | RenderQueueAddError::Busy { .. } => VerbError::Custom(value.to_string()),
        }
    }
}

/// The §0.8 verb entry for `render.queue.add`.
#[derive(Debug, Default)]
pub struct RenderQueueAddVerb;

impl Verb for RenderQueueAddVerb {
    fn verb(&self) -> &'static str {
        "render.queue.add"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: RenderQueueAddArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("render.queue.add: args deserialize failed: {err}"),
            })?;

        match compute_patch(prior, &typed) {
            Ok((patch_value, warnings, data)) => {
                let patch: json_patch::Patch =
                    serde_json::from_value(patch_value).map_err(|err| {
                        VerbError::Custom(format!(
                            "render.queue.add: patch construction failed: {err}"
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
        let _typed: RenderQueueAddArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "RenderQueueAddArgs",
            })?;
        Ok(Value::Null)
    }
}
