//! `preview.session.create` (§15.1) — v1 session-manager floor.
//!
//! ## Spec quote (`spec/commands/preview-session.md` §15.1, abbreviated)
//!
//! > CLI: `verbreel preview session create [--project <id>] ...`
//! > MCP: `preview.session.create`
//! > Args: `project_id`, optional `playback_rate`, `width_px`,
//! > `audio_enabled`, `format`, `start_at_tk`.
//! > Returns (`data`): `{ session_id, playback_rate, width_px,
//! > height_px, audio_enabled, frame_count_estimate, channel_kind }`.
//! > Errors: `E_PREVIEW_SESSION_LIMIT`, `E_PROJECT_NOT_FOUND`,
//! > `E_BAD_RANGE`, `E_BAD_TIME`, `E_UNKNOWN_KIND`.
//!
//! ## v1 floor
//!
//! This pure verb slice validates only local argument shape/range
//! pinned by §15.1, then always returns `E_PREVIEW_SESSION_LIMIT`
//! because session-manager/runtime allocation is intentionally
//! unavailable at this layer.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use verbreel_types::ProjectId;

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

const PLAYBACK_RATE_ALLOWED: &str = "[0.1, 8.0]";
const FORMAT_NDJSON: &str = "ndjson";
const FORMAT_HINT: &str = "webrtc and mse channel kinds are reserved for v1.x; use ndjson at v1.1";
const DEFAULT_SESSION_CAP: u32 = 4;

/// Channel-kind selector for `preview.session.create`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PreviewSessionChannelKind {
    /// §0.9-style streaming notifications over NDJSON/progress/SSE.
    Ndjson,
    /// Reserved in v1.1 (rejected with `E_UNKNOWN_KIND`).
    Webrtc,
    /// Reserved in v1.1 (rejected with `E_UNKNOWN_KIND`).
    Mse,
}

fn default_audio_enabled() -> bool {
    true
}

fn default_channel_kind() -> PreviewSessionChannelKind {
    PreviewSessionChannelKind::Ndjson
}

/// Arguments for `preview.session.create`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewSessionCreateArgs {
    /// Target project id.
    pub project_id: ProjectId,
    /// Optional playback rate; omitted resolves to 1.0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playback_rate: Option<f64>,
    /// Optional width override; omitted defers to runtime/project canvas width.
    ///
    /// Signed by design so range failures map to `E_BAD_RANGE`
    /// (`VerbError::BadArgs`) instead of serde-level type errors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width_px: Option<i64>,
    /// Whether streaming includes audio chunks; omitted defaults to `true`.
    #[serde(default = "default_audio_enabled")]
    pub audio_enabled: bool,
    /// Streaming channel kind; omitted defaults to `ndjson`.
    #[serde(default = "default_channel_kind")]
    pub format: PreviewSessionChannelKind,
    /// Optional initial playhead tick; omitted resolves to `0`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_at_tk: Option<i64>,
}

/// Resolve `playback_rate` with the §15.1 default (`1.0`).
#[must_use]
pub fn resolved_playback_rate(args: &PreviewSessionCreateArgs) -> f64 {
    args.playback_rate.unwrap_or(1.0)
}

/// Resolve `audio_enabled` with the §15.1 default (`true`).
#[must_use]
pub fn resolved_audio_enabled(args: &PreviewSessionCreateArgs) -> bool {
    args.audio_enabled
}

/// Resolve `format` with the §15.1 default (`ndjson`).
#[must_use]
pub fn resolved_channel_kind(args: &PreviewSessionCreateArgs) -> PreviewSessionChannelKind {
    args.format
}

/// Resolve `start_at_tk` with the §15.1 default (`0`).
#[must_use]
pub fn resolved_start_at_tk(args: &PreviewSessionCreateArgs) -> i64 {
    args.start_at_tk.unwrap_or(0)
}

/// Envelope `data` for a future successful `preview.session.create`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PreviewSessionCreateData {
    /// Allocated preview-session id.
    pub session_id: String,
    /// Effective playback rate.
    pub playback_rate: f64,
    /// Resolved output width.
    pub width_px: u32,
    /// Resolved output height.
    pub height_px: u32,
    /// Whether audio is enabled on the streaming channel.
    pub audio_enabled: bool,
    /// Estimated frame count for full-playback progress.
    pub frame_count_estimate: u64,
    /// Effective channel kind.
    pub channel_kind: PreviewSessionChannelKind,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Error)]
/// Verb-level failures for `preview.session.create`.
pub enum PreviewSessionCreateError {
    /// `playback_rate` or `width_px` is outside allowed local bounds.
    #[error("preview.session.create: E_BAD_RANGE — {field} {value} out of range {allowed}")]
    BadRange {
        /// Field name.
        field: String,
        /// Offending numeric value as string.
        value: String,
        /// Allowed range string.
        allowed: String,
    },
    /// `start_at_tk` is negative.
    #[error("preview.session.create: E_BAD_TIME — start_at_tk {start_at_tk} must be >= 0")]
    BadTime {
        /// Offending `start_at_tk`.
        start_at_tk: i64,
    },
    /// Requested channel kind is not supported in v1.1.
    #[error(
        "preview.session.create: E_UNKNOWN_KIND — requested `{requested}` unsupported; allowed {allowed:?}; hint: {hint}"
    )]
    UnknownKind {
        /// Requested channel literal from args.
        requested: String,
        /// Supported channel kinds for this build.
        allowed: Vec<String>,
        /// Recovery hint.
        hint: String,
    },
    /// Runtime/session-manager capacity floor in v1.
    #[error(
        "preview.session.create: E_PREVIEW_SESSION_LIMIT — project_id `{project_id}` at cap {cap}; active_session_ids={active_session_ids:?}; {detail}"
    )]
    SessionLimit {
        /// Target project id.
        project_id: String,
        /// Active session ids counted toward cap.
        active_session_ids: Vec<String>,
        /// Capacity limit.
        cap: u32,
        /// Runtime detail for operators.
        detail: String,
    },
    /// Reserved for §0.12 project lookup failures in runtime slices.
    #[error("preview.session.create: E_PROJECT_NOT_FOUND — project_id `{project_id}` not found")]
    ProjectNotFound {
        /// Missing project id.
        project_id: String,
    },
}

fn channel_kind_literal(kind: PreviewSessionChannelKind) -> &'static str {
    match kind {
        PreviewSessionChannelKind::Ndjson => FORMAT_NDJSON,
        PreviewSessionChannelKind::Webrtc => "webrtc",
        PreviewSessionChannelKind::Mse => "mse",
    }
}

/// Build the RFC 6902 patch for `preview.session.create`.
///
/// v1 floor: validates local argument bounds and then always returns
/// [`PreviewSessionCreateError::SessionLimit`].
///
/// # Errors
///
/// Returns [`PreviewSessionCreateError::BadRange`] when
/// `playback_rate` is outside `[0.1, 8.0]` or `width_px <= 0`.
/// Returns [`PreviewSessionCreateError::BadTime`] when `start_at_tk < 0`.
/// Returns [`PreviewSessionCreateError::UnknownKind`] for
/// `format in {"webrtc", "mse"}`.
/// Returns [`PreviewSessionCreateError::SessionLimit`] for every
/// otherwise well-formed request.
pub fn compute_patch(
    _prior: &Project,
    args: &PreviewSessionCreateArgs,
) -> Result<(Value, Vec<Value>, Value), PreviewSessionCreateError> {
    let playback_rate = resolved_playback_rate(args);
    if !(0.1..=8.0).contains(&playback_rate) {
        return Err(PreviewSessionCreateError::BadRange {
            field: "playback_rate".to_string(),
            value: playback_rate.to_string(),
            allowed: PLAYBACK_RATE_ALLOWED.to_string(),
        });
    }

    if let Some(width_px) = args.width_px
        && width_px <= 0
    {
        return Err(PreviewSessionCreateError::BadRange {
            field: "width_px".to_string(),
            value: width_px.to_string(),
            allowed: ">= 1".to_string(),
        });
    }

    let start_at_tk = resolved_start_at_tk(args);
    if start_at_tk < 0 {
        return Err(PreviewSessionCreateError::BadTime { start_at_tk });
    }

    let format = resolved_channel_kind(args);
    if matches!(
        format,
        PreviewSessionChannelKind::Webrtc | PreviewSessionChannelKind::Mse
    ) {
        return Err(PreviewSessionCreateError::UnknownKind {
            requested: channel_kind_literal(format).to_string(),
            allowed: vec![FORMAT_NDJSON.to_string()],
            hint: FORMAT_HINT.to_string(),
        });
    }

    Err(PreviewSessionCreateError::SessionLimit {
        project_id: args.project_id.to_string(),
        active_session_ids: vec![],
        cap: DEFAULT_SESSION_CAP,
        detail: format!(
            "session manager unavailable in v1 floor (playback_rate {playback_rate}, width_px {:?}, audio_enabled {}, format {}, start_at_tk {start_at_tk})",
            args.width_px,
            resolved_audio_enabled(args),
            channel_kind_literal(format)
        ),
    })
}

impl From<PreviewSessionCreateError> for VerbError {
    fn from(value: PreviewSessionCreateError) -> Self {
        match value {
            PreviewSessionCreateError::BadRange { .. }
            | PreviewSessionCreateError::BadTime { .. }
            | PreviewSessionCreateError::UnknownKind { .. } => VerbError::BadArgs {
                detail: value.to_string(),
            },
            PreviewSessionCreateError::SessionLimit { .. }
            | PreviewSessionCreateError::ProjectNotFound { .. } => {
                VerbError::Custom(value.to_string())
            }
        }
    }
}

/// The §0.8 verb for `preview.session.create`.
#[derive(Debug, Default)]
pub struct PreviewSessionCreateVerb;

impl Verb for PreviewSessionCreateVerb {
    fn verb(&self) -> &'static str {
        "preview.session.create"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: PreviewSessionCreateArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("preview.session.create: args deserialize failed: {err}"),
            })?;

        match compute_patch(prior, &typed) {
            Ok((patch_value, warnings, data)) => {
                let patch: json_patch::Patch =
                    serde_json::from_value(patch_value).map_err(|err| {
                        VerbError::Custom(format!(
                            "preview.session.create: patch construction failed: {err}"
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
        let _typed: PreviewSessionCreateArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "PreviewSessionCreateArgs",
            })?;

        Ok(Value::Null)
    }
}
