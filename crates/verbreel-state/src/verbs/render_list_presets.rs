//! `render.list_presets` (§11.4) — seventy-sixth production verb in the engine.
//!
//! ## Spec quote (`spec/commands/render.md` §11.4, verbatim)
//!
//! > CLI: `verbreel render list_presets`
//! > MCP: `render.list_presets`
//! > Args: none
//! > Returns (`data`): `{ presets: { name: string; canvas: string;
//! >   fps_num: integer; fps_den: integer; video_codec: string;
//! >   audio_codec: string; bitrate_bps?: integer; crf?: integer }[] }`.
//! > Bundled v1 presets: `youtube-1080p`, `youtube-shorts-1080x1920`,
//! > `tiktok-1080p`, `instagram-reel-1080x1920`, `square-1080p`,
//! > `prores-master`, `web-h264-720p`, `web-vp9-1080p`.
//!
//! ## Bundle metadata, not project state.
//!
//! `render.list_presets` is read-only and does not read or mutate
//! project state; it only exposes curated preset metadata baked into
//! this engine build.
//!
//! ## `project_id` accommodation (spec says `Args: none`).
//!
//! The spec quote above declares `Args: none`. Per the existing
//! convention for read-only metadata verbs whose returned shape is
//! project-agnostic (`effect.list_available` §6.5, `font.list` §7.5,
//! `list_capabilities` §1.5, `stock.list_providers` §17.1), the args
//! struct carries an **optional** `project_id` field. The compute impl
//! ignores it; it exists so the kernel's `mutate_via_verb` path can
//! resolve a `prior` project to satisfy the `Verb` trait shape, and
//! so CLI / MCP transports that thread `--project` through generic
//! plumbing keep working. Spec-compliant `{}` args succeed and return
//! the same data.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::ProjectId;

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// Arguments for `render.list_presets`.
///
/// Per the spec quote above, `Args: none`. `project_id` is accepted as
/// an **optional** accommodation field so kernel dispatch
/// (`mutate_via_verb`, which resolves the `prior` project from the
/// args envelope) and CLI / MCP transports that pass `--project` keep
/// working. Spec-compliant callers can pass `{}` (or no body at all)
/// and the verb still succeeds — the returned data is project-agnostic
/// either way.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RenderListPresetsArgs {
    /// Optional target project id. Ignored at compute time; only used by
    /// kernel dispatch to resolve the `prior` project for the Verb
    /// trait's `compute_patch` shape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<ProjectId>,
}

/// Single render preset entry in the returned list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preset {
    /// Preset identifier (e.g. `"youtube-1080p"`).
    pub name: String,
    /// Output canvas as `"<width>x<height>"` (e.g. `"1920x1080"`).
    pub canvas: String,
    /// Frame-rate numerator.
    pub fps_num: u32,
    /// Frame-rate denominator.
    pub fps_den: u32,
    /// Video codec identifier (e.g. `"h264"`, `"prores"`, `"vp9"`).
    pub video_codec: String,
    /// Audio codec identifier (e.g. `"aac"`, `"opus"`, `"pcm_s16le"`).
    pub audio_codec: String,
    /// Target video bitrate in bits-per-second, if pinned by the preset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bitrate_bps: Option<u64>,
    /// Constant Rate Factor, if pinned by the preset. `None` for
    /// lossless presets like `prores-master`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crf: Option<u32>,
}

/// Envelope returned by `render.list_presets`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderListPresetsData {
    /// Bundled v1 presets in spec order.
    pub presets: Vec<Preset>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Error)]
/// Verb-level error type for `render.list_presets`.
pub enum RenderListPresetsError {
    /// No verb-level runtime errors.
    #[error("render.list_presets: unreachable (no error variants)")]
    Unreachable,
}

/// Return the v1.0 bundled render presets in spec order.
///
/// Spec order is `spec/commands/render.md` §11.4's bundled list:
/// youtube-1080p, youtube-shorts-1080x1920, tiktok-1080p,
/// instagram-reel-1080x1920, square-1080p, prores-master,
/// web-h264-720p, web-vp9-1080p.
///
/// # Errors
///
/// This helper currently cannot fail.
#[must_use]
pub fn bundled_presets() -> Vec<Preset> {
    vec![
        Preset {
            name: "youtube-1080p".into(),
            canvas: "1920x1080".into(),
            fps_num: 30,
            fps_den: 1,
            video_codec: "h264".into(),
            audio_codec: "aac".into(),
            bitrate_bps: None,
            crf: Some(23),
        },
        Preset {
            name: "youtube-shorts-1080x1920".into(),
            canvas: "1080x1920".into(),
            fps_num: 30,
            fps_den: 1,
            video_codec: "h264".into(),
            audio_codec: "aac".into(),
            bitrate_bps: None,
            crf: Some(23),
        },
        Preset {
            name: "tiktok-1080p".into(),
            canvas: "1080x1920".into(),
            fps_num: 30,
            fps_den: 1,
            video_codec: "h264".into(),
            audio_codec: "aac".into(),
            bitrate_bps: None,
            crf: Some(23),
        },
        Preset {
            name: "instagram-reel-1080x1920".into(),
            canvas: "1080x1920".into(),
            fps_num: 30,
            fps_den: 1,
            video_codec: "h264".into(),
            audio_codec: "aac".into(),
            bitrate_bps: None,
            crf: Some(23),
        },
        Preset {
            name: "square-1080p".into(),
            canvas: "1080x1080".into(),
            fps_num: 30,
            fps_den: 1,
            video_codec: "h264".into(),
            audio_codec: "aac".into(),
            bitrate_bps: None,
            crf: Some(23),
        },
        Preset {
            name: "prores-master".into(),
            canvas: "1920x1080".into(),
            fps_num: 30_000,
            fps_den: 1_001,
            video_codec: "prores".into(),
            audio_codec: "pcm_s16le".into(),
            bitrate_bps: None,
            crf: None,
        },
        Preset {
            name: "web-h264-720p".into(),
            canvas: "1280x720".into(),
            fps_num: 30,
            fps_den: 1,
            video_codec: "h264".into(),
            audio_codec: "aac".into(),
            bitrate_bps: None,
            crf: Some(26),
        },
        Preset {
            name: "web-vp9-1080p".into(),
            canvas: "1920x1080".into(),
            fps_num: 30,
            fps_den: 1,
            video_codec: "vp9".into(),
            audio_codec: "opus".into(),
            bitrate_bps: None,
            crf: Some(32),
        },
    ]
}

/// Build the RFC 6902 patch for `render.list_presets`.
///
/// # Errors
///
/// No runtime errors are produced by this verb; the returned `Result` exists
/// for parity with the broader compute-patch API.
pub fn compute_patch(
    _prior: &Project,
    _args: &RenderListPresetsArgs,
) -> Result<(Value, Vec<Value>, RenderListPresetsData), RenderListPresetsError> {
    Ok((
        json!([]),
        Vec::new(),
        RenderListPresetsData {
            presets: bundled_presets(),
        },
    ))
}

/// Build the data envelope from `(args, post_state)`.
///
/// # Errors
///
/// Reuses [`compute_patch`], so this can only return reconstruction errors
/// introduced while rebuilding the deterministic envelope.
pub fn data_envelope_from_args(
    args: &RenderListPresetsArgs,
    post_state: &Project,
) -> Result<RenderListPresetsData, ReconstructError> {
    let (_, _, data) =
        compute_patch(post_state, args).map_err(|e| ReconstructError::Custom(e.to_string()))?;
    Ok(data)
}

impl From<RenderListPresetsError> for VerbError {
    fn from(value: RenderListPresetsError) -> Self {
        match value {
            RenderListPresetsError::Unreachable => VerbError::BadArgs {
                detail: value.to_string(),
            },
        }
    }
}

/// The §0.8 verb for `render.list_presets`.
#[derive(Debug, Default)]
pub struct RenderListPresetsVerb;

impl Verb for RenderListPresetsVerb {
    fn verb(&self) -> &'static str {
        "render.list_presets"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: RenderListPresetsArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("render.list_presets: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch = serde_json::from_value(patch_value).map_err(|err| {
            VerbError::Custom(format!(
                "render.list_presets: patch construction failed: {err}"
            ))
        })?;

        let data = serde_json::to_value(&data).map_err(|err| {
            VerbError::Custom(format!("render.list_presets: data envelope failed: {err}"))
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
        let typed: RenderListPresetsArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "RenderListPresetsArgs",
            })?;

        let envelope = data_envelope_from_args(&typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
