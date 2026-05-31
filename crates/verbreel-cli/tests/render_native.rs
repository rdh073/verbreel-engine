#![cfg(feature = "native-render")]

use clap::Parser;
use tempfile::TempDir;
use verbreel_cli::{Cli, run_with_runtime};
use verbreel_runtime::RenderRuntimeConfig;
use verbreel_state::{ProjectCreateArgs, project_create};

#[test]
fn render_start_delegates_to_runtime() -> anyhow::Result<()> {
    let fixture = ProjectFixture::new()?;
    let cli = Cli::try_parse_from([
        "verbreel",
        "render",
        "start",
        "--project",
        &fixture.project_id.to_string(),
        "--preset",
        "definitely-not-a-preset",
        "--out_path",
        "exports/out.mp4",
        "--from_tk",
        "0",
        "--to_tk",
        "8000",
        "--overwrite",
    ])?;
    let runtime = RenderRuntimeConfig::new().with_project_root(&fixture.root);
    let mut out = Vec::new();
    let err = run_with_runtime(cli, &mut out, &runtime).expect_err("runtime should reject preset");
    assert!(
        err.to_string().contains("E_RENDER_PRESET_UNKNOWN"),
        "CLI must surface the runtime error code, got: {err}"
    );
    assert!(out.is_empty(), "failed render must not write stdout");
    Ok(())
}

#[test]
fn render_start_parses_codec_flags() {
    let pid = verbreel_state::ProjectId::now().to_string();
    let cli = Cli::try_parse_from([
        "verbreel",
        "render",
        "start",
        "--project",
        &pid,
        "--preset",
        "deterministic",
        "--out_path",
        "exports/out.mp4",
        "--video_codec",
        "h264",
        "--audio_codec",
        "aac",
        "--deterministic",
    ])
    .expect("codec flags should parse");

    let verbreel_cli::Command::Render(cmd) = cli.command else {
        panic!("render command should parse as Command::Render");
    };
    let verbreel_cli::RenderAction::Start(start) = cmd.action;
    assert_eq!(start.preset, "deterministic");
}

struct ProjectFixture {
    _tmp: TempDir,
    root: std::path::PathBuf,
    project_id: verbreel_state::ProjectId,
}

impl ProjectFixture {
    fn new() -> anyhow::Result<Self> {
        let tmp = TempDir::new()?;
        let create_data = project_create(&ProjectCreateArgs {
            name: "cli-render-project".to_string(),
            canvas: "64x64".to_string(),
            fps_num: Some(30),
            fps_den: Some(1),
            at: Some(tmp.path().to_path_buf()),
            activate: false,
            metadata: serde_json::Map::new(),
        })?;
        Ok(Self {
            _tmp: tmp,
            root: create_data.path,
            project_id: create_data.project_id,
        })
    }
}
