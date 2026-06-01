#[cfg(not(feature = "native-render"))]
fn main() -> anyhow::Result<()> {
    if std::env::var_os("VERBREEL_EXAMPLES_NATIVE_REEXEC").is_some() {
        anyhow::bail!(
            "examples was re-entered without the `native-render` feature; run `cargo run -p examples --features native-render`"
        );
    }

    let mut cmd = std::process::Command::new("cargo");
    cmd.args(["run", "-p", "examples", "--features", "native-render", "--"])
        .args(std::env::args_os().skip(1))
        .env("VERBREEL_EXAMPLES_NATIVE_REEXEC", "1");
    let status = cmd.status()?;
    std::process::exit(status.code().unwrap_or(1));
}

#[cfg(feature = "native-render")]
fn main() -> anyhow::Result<()> {
    use std::fs;
    use std::time::Instant;

    use serde_json::json;
    use verbreel_state::{
        AssetImportData, ProjectCreateArgs, ProjectStore, RenderStartArgs, default_fixtures,
        default_registry, project_create,
    };

    const PROJECT_NAME: &str = "native-render-spine-project";
    const WIDTH: u32 = 1920;
    const HEIGHT: u32 = 1080;
    const FPS_NUM: u32 = 30;
    const FPS_DEN: u32 = 1;
    const FRAME_COUNT: usize = 30;
    const TICKS_PER_FRAME: i64 = 8_000;
    const DURATION_TK: i64 = FRAME_COUNT as i64 * TICKS_PER_FRAME;

    let out_dir = output_dir();
    fs::create_dir_all(&out_dir)?;
    let project_root = out_dir.join(PROJECT_NAME);
    if project_root.exists() {
        fs::remove_dir_all(&project_root)?;
    }

    let source_path = out_dir.join("native-render-source.ppm");
    write_ppm_fixture(&source_path, WIDTH, HEIGHT)?;

    let create_data = project_create(&ProjectCreateArgs {
        name: PROJECT_NAME.to_string(),
        canvas: format!("{WIDTH}x{HEIGHT}"),
        fps_num: Some(FPS_NUM),
        fps_den: Some(FPS_DEN),
        at: Some(out_dir.clone()),
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
    let imported_asset = import_data
        .assets
        .first()
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| anyhow::anyhow!("asset.import returned no asset"))?;
    let asset_id = required_str(imported_asset, "id")?.to_string();
    let asset_hash = required_str(imported_asset, "hash")?.to_string();

    let video_track_id = create_data.seeded_track_ids.video.to_string();
    apply_verb(
        &mut store,
        "track.rename",
        json!({
            "project_id": create_data.project_id,
            "track": video_track_id,
            "name": "Main video",
        }),
    )?;

    let clip_data = apply_verb(
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
    let clip_id = clip_data
        .get("clip_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("clip.add returned no clip_id"))?
        .to_string();

    apply_verb(
        &mut store,
        "clip.set_opacity",
        json!({
            "project_id": create_data.project_id,
            "clip": clip_id,
            "opacity": 0.92,
        }),
    )?;
    store.save()?;

    let output_path = out_dir.join("native-render-spine-s1-1080p.mp4");
    let render_args = RenderStartArgs {
        project_id: create_data.project_id,
        preset: "deterministic".to_string(),
        out_path: output_path.display().to_string(),
        from_tk: Some(0),
        to_tk: Some(DURATION_TK),
        video_codec: None,
        audio_codec: None,
        bitrate_bps: None,
        crf: None,
        deterministic: true,
        keep_temp: false,
        overwrite: true,
    };

    let started = Instant::now();
    let render_data = run_render_start(store.project(), &render_args)?;
    let elapsed = started.elapsed();
    let output = fs::read(&output_path)?;
    anyhow::ensure!(output.len() > 64, "encoded MP4 is suspiciously small");

    let render_fps = FRAME_COUNT as f64 / elapsed.as_secs_f64();
    let min_fps = min_render_fps();
    anyhow::ensure!(
        render_fps >= min_fps,
        "render perf smoke regression: {render_fps:.2} fps < {min_fps:.2} fps"
    );

    let manifest_path = out_dir.join("native-render-spine-s1-1080p.json");
    let manifest = json!({
        "example": "native-render-spine",
        "project_path": create_data.path.display().to_string(),
        "source_path": source_path.display().to_string(),
        "commands": [
            {
                "verb": "project.create",
                "project_id": create_data.project_id,
                "seeded_track_ids": create_data.seeded_track_ids,
            },
            {
                "verb": "asset.import",
                "asset_id": asset_id,
                "asset_hash": asset_hash,
            },
            {
                "verb": "track.rename",
                "track_id": video_track_id,
                "name": "Main video",
            },
            {
                "verb": "clip.add",
                "clip_id": clip_id,
            },
            {
                "verb": "clip.set_opacity",
                "clip_id": clip_id,
                "opacity": 0.92,
            },
            {
                "verb": "render.start",
                "job_id": render_data.job_id,
                "output_path": render_data.output_path,
                "duration_tk": render_data.duration_tk,
            }
        ],
        "perf_smoke": {
            "fixture": "S1-1080p",
            "width": WIDTH,
            "height": HEIGHT,
            "fps_num": FPS_NUM,
            "fps_den": FPS_DEN,
            "frames": FRAME_COUNT,
            "elapsed_ms": elapsed.as_millis(),
            "render_fps": render_fps,
            "min_render_fps": min_fps,
        },
        "output": {
            "path": output_path.display().to_string(),
            "bytes": output.len(),
            "sha256": sha256_hex(&output),
        }
    });
    fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest)?)?;

    println!("{}", serde_json::to_string_pretty(&manifest)?);
    eprintln!(
        "wrote {} and {}",
        output_path.display(),
        manifest_path.display()
    );
    Ok(())
}

#[cfg(feature = "native-render")]
fn apply_verb(
    store: &mut verbreel_state::ProjectStore,
    verb: &str,
    args: serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    let outcome = store.mutate_via_verb(verb, args, None)?;
    match outcome {
        verbreel_state::MutateOutcome::Applied { data, .. }
        | verbreel_state::MutateOutcome::Replayed { data, .. }
        | verbreel_state::MutateOutcome::NoOp { data, .. } => Ok(data),
    }
}

#[cfg(feature = "native-render")]
fn run_render_start(
    project: &verbreel_state::Project,
    args: &verbreel_state::RenderStartArgs,
) -> anyhow::Result<verbreel_state::RenderStartData> {
    anyhow::ensure!(
        args.preset == "deterministic",
        "example expects deterministic preset"
    );
    let from_tk = args.from_tk.unwrap_or(0);
    let to_tk = args.to_tk.unwrap_or_else(|| project.duration_tk.get());
    anyhow::ensure!(from_tk >= 0, "render.start from_tk must be >= 0");
    anyhow::ensure!(to_tk > from_tk, "render.start range must be non-empty");

    let output_path = std::path::PathBuf::from(&args.out_path);
    if output_path.exists() && !args.overwrite {
        anyhow::bail!("render.start out_path exists and overwrite=false");
    }

    let frame_count = frames_for_range(from_tk, to_tk, project.fps_num, project.fps_den)?;
    let frames = render_plans(project, from_tk, frame_count)?;
    let decoded = decoded_sources(project, frame_count)?;
    let spec = verbreel_render::RenderJobSpec {
        preset: verbreel_render::RenderPreset::Deterministic,
        width: project.canvas.width,
        height: project.canvas.height,
        fps_num: project.fps_num,
        fps_den: project.fps_den,
        frames,
        decoded,
    };

    let registry = verbreel_render::JobRegistry::new();
    let job_id = registry.start_render(&spec)?;
    let verbreel_render::RenderStatus::Done {
        frame_count: rendered_frames,
        output,
    } = registry.status(job_id)?
    else {
        anyhow::bail!("native render example did not reach Done status");
    };
    anyhow::ensure!(
        rendered_frames == frame_count,
        "rendered {rendered_frames} frames, expected {frame_count}"
    );
    std::fs::write(&output_path, output)?;

    Ok(verbreel_state::RenderStartData {
        job_id: job_id.to_string(),
        output_path: output_path.display().to_string(),
        duration_tk: to_tk - from_tk,
        // Example drives JobRegistry directly, recording no render.start event,
        // so there is no event id to correlate — empty per §0.1 (no-event path).
        event_id: String::new(),
    })
}

#[cfg(feature = "native-render")]
fn frames_for_range(from_tk: i64, to_tk: i64, fps_num: u32, fps_den: u32) -> anyhow::Result<usize> {
    anyhow::ensure!(fps_num > 0 && fps_den > 0, "frame rate must be non-zero");
    let duration = u128::try_from(to_tk - from_tk)?;
    let numerator = duration * u128::from(fps_num);
    let denominator = u128::from(verbreel_ir::TICK_RATE_HZ) * u128::from(fps_den);
    usize::try_from(numerator.div_ceil(denominator)).map_err(Into::into)
}

#[cfg(feature = "native-render")]
fn render_plans(
    project: &verbreel_state::Project,
    from_tk: i64,
    frame_count: usize,
) -> anyhow::Result<Vec<verbreel_render::RenderPlan>> {
    let mut plans = Vec::with_capacity(frame_count);
    for frame in 0..frame_count {
        let tick = from_tk + i64::try_from(frame)? * ticks_per_frame(project)?;
        let graph = verbreel_ir::build(&composition_at(project, tick)?);
        let evaluator = verbreel_ir::Evaluator::new(&graph);
        plans.push(verbreel_render::RenderPlan::from_evaluator(
            &graph, &evaluator,
        ));
    }
    Ok(plans)
}

#[cfg(feature = "native-render")]
fn composition_at(
    project: &verbreel_state::Project,
    tick: i64,
) -> anyhow::Result<verbreel_ir::CompositionInput> {
    let mut tracks = Vec::new();
    for track in project.tracks.iter().filter(|track| {
        track.kind == verbreel_state::TrackKind::Video && !track.hidden && !track.muted
    }) {
        let mut clips = Vec::new();
        for clip in track.clips.iter().filter(|clip| clip_active_at(clip, tick)) {
            let clip_asset_id = clip.asset_id.id();
            let asset_hash = project
                .assets
                .iter()
                .find(|asset| Some(asset.id()) == clip_asset_id)
                .map(|asset| verbreel_ir::AssetHash::new(asset.hash().as_str()))
                .transpose()?;
            clips.push(verbreel_ir::ClipInput {
                source_node_id: ir_node_id(&clip.id.to_string())?,
                args_hash: canonical_args_hash(&serde_json::to_value(clip)?)?,
                asset: asset_hash,
                effects: Vec::new(),
            });
        }
        tracks.push(verbreel_ir::TrackInput {
            composite_node_id: ir_node_id(&track.id.to_string())?,
            composite_args_hash: canonical_args_hash(&serde_json::json!({
                "id": track.id,
                "name": &track.name,
                "volume": track.volume,
                "pan": track.pan,
            }))?,
            clips,
        });
    }

    Ok(verbreel_ir::CompositionInput {
        tracks,
        output: verbreel_ir::OutputInput {
            node_id: ir_node_id(&project.id.to_string())?,
            args_hash: canonical_args_hash(&serde_json::json!({
                "width": project.canvas.width,
                "height": project.canvas.height,
                "fps_num": project.fps_num,
                "fps_den": project.fps_den,
            }))?,
        },
        tick: u64::try_from(tick)?,
    })
}

#[cfg(feature = "native-render")]
fn clip_active_at(clip: &verbreel_state::Clip, tick: i64) -> bool {
    let start = clip.track_position_tk.get();
    let end = start + clip.source_out_tk.get() - clip.source_in_tk.get();
    start <= tick && tick < end
}

#[cfg(feature = "native-render")]
fn decoded_sources(
    project: &verbreel_state::Project,
    frame_count: usize,
) -> anyhow::Result<std::collections::HashMap<verbreel_ir::AssetHash, verbreel_render::DecodedSource>>
{
    let mut decoded = std::collections::HashMap::new();
    for asset in &project.assets {
        let key = verbreel_ir::AssetHash::new(asset.hash().as_str())?;
        let frames = (0..frame_count)
            .map(|i| {
                let y = 48 + u8::try_from((i * 5) % 96).expect("modulo keeps value in u8 range");
                verbreel_codec_native::Frame::new(
                    project.canvas.width,
                    project.canvas.height,
                    solid_yuv(project.canvas.width, project.canvas.height, y, 112, 168),
                )
            })
            .collect();
        decoded.insert(key, verbreel_render::DecodedSource { frames });
    }
    Ok(decoded)
}

#[cfg(feature = "native-render")]
fn ticks_per_frame(project: &verbreel_state::Project) -> anyhow::Result<i64> {
    anyhow::ensure!(
        project.fps_num > 0 && project.fps_den > 0,
        "project frame rate must be non-zero"
    );
    let numerator = i64::from(verbreel_ir::TICK_RATE_HZ) * i64::from(project.fps_den);
    Ok(numerator / i64::from(project.fps_num))
}

#[cfg(feature = "native-render")]
fn solid_yuv(width: u32, height: u32, y: u8, u: u8, v: u8) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    let cw = w / 2;
    let ch = h / 2;
    let mut buf = vec![y; w * h];
    buf.extend(std::iter::repeat_n(u, cw * ch));
    buf.extend(std::iter::repeat_n(v, cw * ch));
    buf
}

#[cfg(feature = "native-render")]
fn write_ppm_fixture(path: &std::path::Path, width: u32, height: u32) -> anyhow::Result<()> {
    let mut bytes =
        format!("P6\n# verbreel native render example\n{width} {height}\n255\n").into_bytes();
    bytes.reserve((width as usize) * (height as usize) * 3);
    for y in 0..height {
        for x in 0..width {
            bytes.push(u8::try_from(x * 255 / width).unwrap_or(u8::MAX));
            bytes.push(u8::try_from(y * 255 / height).unwrap_or(u8::MAX));
            bytes.push(160);
        }
    }
    std::fs::write(path, bytes)?;
    Ok(())
}

#[cfg(feature = "native-render")]
fn canonical_args_hash(value: &serde_json::Value) -> anyhow::Result<[u8; 32]> {
    let hex = verbreel_canon::sha256_hex(value)?;
    hex_to_32(&hex)
}

#[cfg(feature = "native-render")]
fn hex_to_32(hex: &str) -> anyhow::Result<[u8; 32]> {
    anyhow::ensure!(hex.len() == 64, "expected 64 hex chars");
    let mut out = [0u8; 32];
    for (idx, slot) in out.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&hex[idx * 2..idx * 2 + 2], 16)?;
    }
    Ok(out)
}

#[cfg(feature = "native-render")]
fn ir_node_id(raw: &str) -> anyhow::Result<verbreel_ir::IrNodeId> {
    Ok(raw.parse()?)
}

#[cfg(feature = "native-render")]
fn required_str<'a>(
    map: &'a serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> anyhow::Result<&'a str> {
    map.get(key)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("missing string field `{key}`"))
}

#[cfg(feature = "native-render")]
fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;

    use sha2::Digest;

    let digest = sha2::Sha256::digest(bytes);
    digest.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

#[cfg(feature = "native-render")]
fn min_render_fps() -> f64 {
    std::env::var("VERBREEL_EXAMPLES_MIN_RENDER_FPS")
        .ok()
        .and_then(|raw| raw.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(0.1)
}

#[cfg(feature = "native-render")]
fn output_dir() -> std::path::PathBuf {
    if let Some(target_dir) = std::env::var_os("CARGO_TARGET_DIR") {
        return std::path::PathBuf::from(target_dir).join("verbreel-examples");
    }
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .expect("examples crate lives under the workspace root")
        .join("target")
        .join("verbreel-examples")
}
