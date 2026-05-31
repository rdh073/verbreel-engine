//! `render.start` (§11.1) — v1 streaming renderer floor.
//!
//! ## Spec quote (`spec/commands/render.md` §11.1, abbreviated)
//!
//! > CLI: `verbreel render start [--project <id>] --preset <name> --out_path <path> ...`
//! > MCP: `render.start`
//! > Args: `project_id`, `preset`, `out_path`, optional range/codec/rate/flags.
//! > Returns (`data`): `{ job_id: string; output_path: string; duration_tk: integer }`.
//! > Errors include `E_RENDER_FAIL`.
//!
//! ## v1 floor — always errors with `E_RENDER_FAIL`.
//!
//! Actual rendering is a long-running runtime concern (worker lifecycle,
//! progress streaming, codec probing, overwrite/path checks, and atomic
//! file writes). Those are intentionally out of scope for pure
//! `Verb::compute_patch`. In this slice every well-formed call returns
//! `E_RENDER_FAIL` as a runtime-state `Custom` error.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::ProjectId;

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// Event-warning code carrying runtime result data for replay.
///
/// The pure `Verb::compute_patch` path cannot mint or observe the runtime
/// render job id. Native composition roots record successful render events via
/// `ProjectStore::mutate` with an empty state patch, then stash the missing
/// result envelope here so the reconstructor remains pure.
pub const RENDER_START_RESULT_WARNING: &str = "W_RENDER_RESULT_RECORDED";

/// Video codec override accepted by `render.start`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RenderVideoCodec {
    /// H.264 / AVC.
    #[serde(rename = "h264")]
    H264,
    /// H.265 / HEVC.
    #[serde(rename = "h265")]
    H265,
    /// Apple `ProRes`.
    #[serde(rename = "prores")]
    Prores,
    /// Google VP9.
    #[serde(rename = "vp9")]
    Vp9,
    /// AOM AV1.
    #[serde(rename = "av1")]
    Av1,
}

/// Audio codec override accepted by `render.start`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RenderAudioCodec {
    /// AAC.
    #[serde(rename = "aac")]
    Aac,
    /// Opus.
    #[serde(rename = "opus")]
    Opus,
    /// PCM signed 16-bit little-endian.
    #[serde(rename = "pcm_s16le")]
    PcmS16Le,
}

/// Arguments for `render.start`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderStartArgs {
    /// Target project id.
    pub project_id: ProjectId,
    /// Render preset name.
    pub preset: String,
    /// Output container destination path.
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
    ///
    /// Signed by design in v1 floor so future range mapping can report
    /// `E_BAD_RANGE` instead of serde-level `BadArgs`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bitrate_bps: Option<i64>,
    /// Optional codec CRF value.
    ///
    /// Signed by design in v1 floor so future range mapping can report
    /// `E_BAD_RANGE` instead of serde-level `BadArgs`.
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
}

/// Envelope `data` for a future successful `render.start`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderStartData {
    /// Runtime-assigned render job id.
    pub job_id: String,
    /// Resolved final output path.
    pub output_path: String,
    /// Render range duration in ticks.
    pub duration_tk: i64,
    /// §0.1 envelope event id of the `render.start` event this call recorded.
    ///
    /// Filled by the native runtime on the returned copy *after* the event is
    /// written — never inside the recorded event payload, which would be a
    /// self-reference (the event referencing its own not-yet-minted id).
    #[serde(default)]
    pub event_id: String,
}

/// Build the event warning used by native runtime surfaces to record result data.
#[must_use]
pub fn render_start_result_warning(data: &RenderStartData) -> Value {
    json!({
        "code": RENDER_START_RESULT_WARNING,
        "message": "render.start result recorded for deterministic event replay",
        "details": data,
    })
}

/// Verb-level failures for `render.start`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RenderStartError {
    /// Runtime renderer floor is unavailable in pure v1 path.
    #[error("render.start: E_RENDER_FAIL — {detail}")]
    RenderFail {
        /// Human-readable runtime detail.
        detail: String,
    },
}

/// Build the RFC 6902 patch for `render.start`.
///
/// v1 floor: always returns [`RenderStartError::RenderFail`].
///
/// # Errors
///
/// Always errors with [`RenderStartError::RenderFail`] in v1.
pub fn compute_patch(
    _prior: &Project,
    args: &RenderStartArgs,
) -> Result<(Value, Vec<Value>, Value), RenderStartError> {
    Err(RenderStartError::RenderFail {
        detail: format!(
            "streaming renderer runtime unavailable in v1 floor (preset `{}`, out_path `{}`)",
            args.preset, args.out_path
        ),
    })
}

impl From<RenderStartError> for VerbError {
    fn from(value: RenderStartError) -> Self {
        match value {
            RenderStartError::RenderFail { .. } => VerbError::Custom(value.to_string()),
        }
    }
}

/// The §0.8 verb for `render.start`.
#[derive(Debug, Default)]
pub struct RenderStartVerb;

impl Verb for RenderStartVerb {
    fn verb(&self) -> &'static str {
        "render.start"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: RenderStartArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("render.start: args deserialize failed: {err}"),
            })?;

        match compute_patch(prior, &typed) {
            Ok((patch_value, warnings, data)) => {
                let patch: json_patch::Patch =
                    serde_json::from_value(patch_value).map_err(|err| {
                        VerbError::Custom(format!("render.start: patch construction failed: {err}"))
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
        warnings: &[Value],
        _post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let _typed: RenderStartArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "RenderStartArgs",
            })?;

        if let Some(data) = recorded_result_data(warnings)? {
            return serde_json::to_value(data).map_err(|err| {
                ReconstructError::Custom(format!("render.start data envelope failed: {err}"))
            });
        }

        Ok(Value::Null)
    }
}

fn recorded_result_data(warnings: &[Value]) -> Result<Option<RenderStartData>, ReconstructError> {
    let Some(details) = warnings
        .iter()
        .find(|warning| {
            warning.get("code").and_then(Value::as_str) == Some(RENDER_START_RESULT_WARNING)
        })
        .and_then(|warning| warning.get("details"))
    else {
        return Ok(None);
    };

    serde_json::from_value(details.clone())
        .map(Some)
        .map_err(|err| {
            ReconstructError::Custom(format!("render.start recorded data invalid: {err}"))
        })
}
