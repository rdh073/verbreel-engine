use std::fs;

use serde_json::json;
use tempfile::TempDir;
use verbreel_runtime::RenderRuntimeConfig;
use verbreel_state::{
    AssetImportData, ProjectCreateArgs, ProjectStore, RenderStartArgs, default_fixtures,
    default_registry, project_create,
};

const WIDTH: u32 = 64;
const HEIGHT: u32 = 64;
const FPS_NUM: u32 = 30;
const FPS_DEN: u32 = 1;
const FRAME_COUNT: i64 = 2;
const TICKS_PER_FRAME: i64 = 8_000;
const DURATION_TK: i64 = FRAME_COUNT * TICKS_PER_FRAME;

#[test]
fn render_start_rejects_unregistered_project() {
    let args = render_args(verbreel_state::ProjectId::now(), "out.mp4");
    let err = RenderRuntimeConfig::new()
        .render_start(&args)
        .expect_err("project id is not registered");
    assert!(
        err.to_string().contains("E_PROJECT_NOT_FOUND"),
        "error must carry published code, got: {err}"
    );
}

#[test]
fn render_start_rejects_path_escape() -> anyhow::Result<()> {
    let fixture = ProjectFixture::new()?;
    let args = render_args(fixture.project_id, "../escape.mp4");
    let err = RenderRuntimeConfig::new()
        .with_project_root(&fixture.root)
        .render_start(&args)
        .expect_err("path escape must be rejected");
    assert!(
        err.to_string().contains("E_PATH_ESCAPE"),
        "error must carry published code, got: {err}"
    );
    Ok(())
}

#[test]
fn render_start_rejects_unknown_preset() -> anyhow::Result<()> {
    let fixture = ProjectFixture::new()?;
    let mut args = render_args(fixture.project_id, "exports/out.mp4");
    args.preset = "definitely-not-a-preset".to_string();
    let err = RenderRuntimeConfig::new()
        .with_project_root(&fixture.root)
        .render_start(&args)
        .expect_err("unknown preset must be rejected");
    assert!(
        err.to_string().contains("E_RENDER_PRESET_UNKNOWN"),
        "error must carry published code, got: {err}"
    );
    Ok(())
}

#[test]
fn render_start_rejects_incompatible_rate_controls() -> anyhow::Result<()> {
    let fixture = ProjectFixture::new()?;
    let mut args = render_args(fixture.project_id, "exports/out.mp4");
    args.crf = Some(23);
    args.bitrate_bps = Some(4_000_000);
    let err = RenderRuntimeConfig::new()
        .with_project_root(&fixture.root)
        .render_start(&args)
        .expect_err("mutually exclusive rate controls must be rejected");
    assert!(
        err.to_string().contains("E_ARGS_INCOMPATIBLE"),
        "error must carry published code, got: {err}"
    );
    Ok(())
}

#[test]
fn render_start_rejects_oversized_bitrate() -> anyhow::Result<()> {
    let fixture = ProjectFixture::new()?;
    let mut args = render_args(fixture.project_id, "exports/out.mp4");
    args.bitrate_bps = Some(1_000_000_001);
    let err = RenderRuntimeConfig::new()
        .with_project_root(&fixture.root)
        .render_start(&args)
        .expect_err("oversized bitrate must be rejected");
    assert!(
        err.to_string().contains("E_BAD_RANGE"),
        "error must carry published code, got: {err}"
    );
    Ok(())
}

#[cfg(feature = "native-render")]
#[test]
fn render_start_writes_mp4_from_project_asset() -> anyhow::Result<()> {
    let fixture = ProjectFixture::new()?;
    let out_path = fixture.root.join("exports").join("runtime-out.mp4");
    let args = render_args(fixture.project_id, &out_path.display().to_string());

    let data = RenderRuntimeConfig::new()
        .with_project_root(&fixture.root)
        .render_start(&args)?;

    let bytes = fs::read(&out_path)?;
    assert!(bytes.len() > 64, "encoded output is suspiciously small");
    assert_eq!(data.output_path, out_path.display().to_string());
    assert_eq!(data.duration_tk, DURATION_TK);
    assert!(!data.job_id.is_empty());
    let event = assert_render_event_recorded(&fixture.root, &data)?;
    // §0.1: a verb that recorded an event must surface its truthful id, not
    // the read-only/dry-run `""` placeholder. §0.3: it is a strict UUIDv7 —
    // canonical 36-char form whose version nibble (index 14) is `7`. And it
    // equals the recorded events.jsonl line's `id`.
    assert!(
        !data.event_id.is_empty(),
        "render.start recorded an event but returned event_id=\"\""
    );
    assert_eq!(data.event_id.len(), 36, "event_id must be a canonical UUID");
    assert_eq!(
        data.event_id.as_bytes()[14],
        b'7',
        "event_id must be a UUIDv7 (§0.3), got {}",
        data.event_id
    );
    assert_eq!(
        event["id"], data.event_id,
        "returned event_id must match the recorded events.jsonl line id",
    );
    Ok(())
}

#[cfg(feature = "native-render")]
#[test]
fn render_start_clamps_to_tk_past_project_duration() -> anyhow::Result<()> {
    let fixture = ProjectFixture::new()?;
    let out_path = fixture.root.join("exports").join("clamped-out.mp4");
    let mut args = render_args(fixture.project_id, &out_path.display().to_string());
    args.to_tk = Some(DURATION_TK + TICKS_PER_FRAME);

    let data = RenderRuntimeConfig::new()
        .with_project_root(&fixture.root)
        .render_start(&args)?;

    let bytes = fs::read(&out_path)?;
    assert!(bytes.len() > 64, "encoded output is suspiciously small");
    assert_eq!(data.duration_tk, DURATION_TK);
    let event = assert_render_event_recorded(&fixture.root, &data)?;
    assert!(
        event["warnings"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|warning| warning["code"] == "W_RANGE_CLAMPED"),
        "clamped render must record W_RANGE_CLAMPED, got: {event}"
    );
    Ok(())
}

struct ProjectFixture {
    _tmp: TempDir,
    root: std::path::PathBuf,
    project_id: verbreel_state::ProjectId,
}

impl ProjectFixture {
    fn new() -> anyhow::Result<Self> {
        let tmp = TempDir::new()?;
        let source_path = tmp.path().join("source.ppm");
        write_ppm_fixture(&source_path, WIDTH, HEIGHT)?;
        let create_data = project_create(&ProjectCreateArgs {
            name: "runtime-render-project".to_string(),
            canvas: format!("{WIDTH}x{HEIGHT}"),
            fps_num: Some(FPS_NUM),
            fps_den: Some(FPS_DEN),
            at: Some(tmp.path().to_path_buf()),
            activate: false,
            metadata: serde_json::Map::new(),
        })?;

        let registry = default_registry();
        let fixtures = default_fixtures();
        let mut store = ProjectStore::open_with_registry(&create_data.path, &registry, &fixtures)?;
        let import_data: AssetImportData = serde_json::from_value(apply_verb(
            &mut store,
            "asset.import",
            json!({
                "project_id": create_data.project_id,
                "paths": [source_path.display().to_string()],
            }),
        )?)?;
        let asset_id = import_data
            .assets
            .first()
            .and_then(|asset| asset.get("id"))
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("asset.import returned no asset id"))?;
        let video_track_id = create_data.seeded_track_ids.video.to_string();
        apply_verb(
            &mut store,
            "clip.add",
            json!({
                "project_id": create_data.project_id,
                "asset_id": asset_id,
                "track": video_track_id,
                "track_position_tk": 0,
                "source_in_tk": 0,
                "source_out_tk": DURATION_TK,
                "name": "Generated PPM card",
            }),
        )?;
        store.save()?;

        Ok(Self {
            _tmp: tmp,
            root: create_data.path,
            project_id: create_data.project_id,
        })
    }
}

fn apply_verb(
    store: &mut ProjectStore,
    verb: &str,
    args: serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    let outcome = store.mutate_via_verb(verb, args, None)?;
    match outcome {
        // `NoOp` is the empty-patch read/no-op outcome added alongside
        // the §0.1 patch surfacing; it carries the same `data` envelope.
        verbreel_state::MutateOutcome::Applied { data, .. }
        | verbreel_state::MutateOutcome::Replayed { data, .. }
        | verbreel_state::MutateOutcome::NoOp { data, .. } => Ok(data),
    }
}

fn render_args(project_id: verbreel_state::ProjectId, out_path: &str) -> RenderStartArgs {
    RenderStartArgs {
        project_id,
        preset: "deterministic".to_string(),
        out_path: out_path.to_string(),
        from_tk: Some(0),
        to_tk: Some(DURATION_TK),
        video_codec: None,
        audio_codec: None,
        bitrate_bps: None,
        crf: None,
        deterministic: true,
        keep_temp: false,
        overwrite: true,
    }
}

fn write_ppm_fixture(path: &std::path::Path, width: u32, height: u32) -> anyhow::Result<()> {
    let mut bytes = format!("P6\n{width} {height}\n255\n").into_bytes();
    bytes.reserve((width as usize) * (height as usize) * 3);
    for y in 0..height {
        for x in 0..width {
            bytes.push(u8::try_from(x * 255 / width).unwrap_or(u8::MAX));
            bytes.push(u8::try_from(y * 255 / height).unwrap_or(u8::MAX));
            bytes.push(160);
        }
    }
    fs::write(path, bytes)?;
    Ok(())
}

#[cfg(feature = "native-render")]
fn assert_render_event_recorded(
    root: &std::path::Path,
    data: &verbreel_state::RenderStartData,
) -> anyhow::Result<serde_json::Value> {
    let events = fs::read_to_string(root.join(".verbreel").join("events.jsonl"))?;
    let event = events
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(serde_json::from_str::<serde_json::Value>)
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .rev()
        .find(|event| event["verb"] == "render.start")
        .ok_or_else(|| anyhow::anyhow!("render.start event was not recorded"))?;
    let recorded = event["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|warning| warning["code"] == verbreel_state::RENDER_START_RESULT_WARNING)
        .ok_or_else(|| anyhow::anyhow!("render.start result warning missing: {event}"))?;
    assert_eq!(recorded["details"]["job_id"], data.job_id);
    assert_eq!(recorded["details"]["output_path"], data.output_path);
    assert_eq!(recorded["details"]["duration_tk"], data.duration_tk);
    Ok(event)
}
