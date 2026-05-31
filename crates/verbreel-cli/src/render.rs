//! CLI wrapper for `render.start`.
//!
//! This module is compiled only with the `native-render` feature. It owns the
//! clap-to-DTO translation and delegates all side effects to
//! `verbreel-runtime`.

use std::io::Write;

use clap::Args;
use verbreel_state::{ProjectId, RenderAudioCodec, RenderStartArgs, RenderVideoCodec};

/// Arguments for `verbreel render start`.
#[derive(Debug, Args)]
pub struct RenderStartCmd {
    /// Target project id.
    #[arg(long = "project", value_name = "ID")]
    pub project_id: ProjectId,
    /// Render preset name.
    #[arg(long)]
    pub preset: String,
    /// Output path, resolved relative to the project root.
    #[arg(long = "out_path")]
    pub out_path: String,
    /// Optional start tick.
    #[arg(long = "from_tk")]
    pub from_tk: Option<i64>,
    /// Optional end tick.
    #[arg(long = "to_tk")]
    pub to_tk: Option<i64>,
    /// Optional video codec override.
    #[arg(long = "video_codec", value_parser = parse_video_codec)]
    pub video_codec: Option<RenderVideoCodec>,
    /// Optional audio codec override.
    #[arg(long = "audio_codec", value_parser = parse_audio_codec)]
    pub audio_codec: Option<RenderAudioCodec>,
    /// Optional target bitrate in bits/sec.
    #[arg(long = "bitrate_bps")]
    pub bitrate_bps: Option<i64>,
    /// Optional codec CRF value.
    #[arg(long)]
    pub crf: Option<i64>,
    /// Use deterministic render settings.
    #[arg(long)]
    pub deterministic: bool,
    /// Keep temporary artifacts when supported by the runtime.
    #[arg(long = "keep_temp")]
    pub keep_temp: bool,
    /// Replace an existing output path.
    #[arg(long)]
    pub overwrite: bool,
}

/// Run `render.start` and emit the published data envelope as JSON.
///
/// # Errors
///
/// Returns an error when the runtime rejects the args or the JSON envelope
/// cannot be written to stdout.
pub fn start(
    cmd: &RenderStartCmd,
    out: &mut dyn Write,
    runtime: &verbreel_runtime::RenderRuntimeConfig,
) -> anyhow::Result<i32> {
    let data = runtime.render_start(&cmd.to_args())?;
    serde_json::to_writer_pretty(&mut *out, &data)?;
    writeln!(out)?;
    Ok(0)
}

impl RenderStartCmd {
    fn to_args(&self) -> RenderStartArgs {
        RenderStartArgs {
            project_id: self.project_id,
            preset: self.preset.clone(),
            out_path: self.out_path.clone(),
            from_tk: self.from_tk,
            to_tk: self.to_tk,
            video_codec: self.video_codec,
            audio_codec: self.audio_codec,
            bitrate_bps: self.bitrate_bps,
            crf: self.crf,
            deterministic: self.deterministic,
            keep_temp: self.keep_temp,
            overwrite: self.overwrite,
        }
    }
}

fn parse_video_codec(raw: &str) -> Result<RenderVideoCodec, String> {
    serde_json::from_value(serde_json::Value::String(raw.to_string()))
        .map_err(|err| format!("invalid video_codec `{raw}`: {err}"))
}

fn parse_audio_codec(raw: &str) -> Result<RenderAudioCodec, String> {
    serde_json::from_value(serde_json::Value::String(raw.to_string()))
        .map_err(|err| format!("invalid audio_codec `{raw}`: {err}"))
}
