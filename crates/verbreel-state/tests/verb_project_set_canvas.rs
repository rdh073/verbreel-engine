//! Tests for `project.set_canvas` (§2.10) — the second production verb.
//!
//! Covers `compute_patch` happy paths (minimal, all-optionals,
//! partial-update), the malformed-arg matrix (canvas string,
//! width/height range, background hex, pixel-aspect floor), the
//! `data_envelope` helper, the reconstructor round-trip via
//! [`validate_reconstructors`], and one end-to-end exercise through
//! [`ProjectStore::mutate_via_verb`] proving the verb is wired into
//! the kernel's default registry + fixtures.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::{
    Asset, CANVAS_MAX_DIM, CANVAS_MIN_DIM, Canvas, Clip, MutateOutcome, PIXEL_ASPECT_MIN, Project,
    ProjectSetCanvasArgs, ProjectSetCanvasError, ProjectSetCanvasVerb, ProjectStore, RecordedEvent,
    Track, TrackKind, Transform, VerbRegistry, default_fixtures, default_registry,
    validate_reconstructors,
    verbs::project_set_canvas::{W_CANVAS_CLIPS_OUT_OF_FRAME, compute_patch, data_envelope},
};
use verbreel_types::{ProjectId, Tick};

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const SAMPLE_VIDEO_ASSET_ID: &str = "0190b8d3-15e3-7000-bd00-00000000a101";
const SAMPLE_IMAGE_ASSET_ID: &str = "0190b8d3-15e3-7000-bd00-00000000a102";
const SAMPLE_MISSING_ASSET_ID: &str = "0190b8d3-15e3-7000-bd00-00000000a199";
const NIL_ASSET_ID: &str = "00000000-0000-0000-0000-000000000000";
const SAMPLE_VIDEO_TRACK_ID: &str = "0190b8d3-15e3-7000-bd00-00000000b101";
const SAMPLE_TEXT_TRACK_ID: &str = "0190b8d3-15e3-7000-bd00-00000000b102";
const SAMPLE_AUDIO_TRACK_ID: &str = "0190b8d3-15e3-7000-bd00-00000000b103";
const SAMPLE_VIDEO_TRACK_ID_2: &str = "0190b8d3-15e3-7000-bd00-00000000b104";
const CLIP_ID_A: &str = "0190b8d3-15e3-7000-bd00-00000000c201";
const CLIP_ID_B: &str = "0190b8d3-15e3-7000-bd00-00000000c202";
const CLIP_ID_C: &str = "0190b8d3-15e3-7000-bd00-00000000c203";
const CLIP_ID_AUDIO: &str = "0190b8d3-15e3-7000-bd00-00000000c204";
const CLIP_ID_TEXT: &str = "0190b8d3-15e3-7000-bd00-00000000c205";
const CLIP_ID_MISSING: &str = "0190b8d3-15e3-7000-bd00-00000000c206";
const CLIP_ID_NIL: &str = "0190b8d3-15e3-7000-bd00-00000000c207";
const CLIP_ID_ROTATE: &str = "0190b8d3-15e3-7000-bd00-00000000c208";
const DUMMY_HASH: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const DUMMY_VIDEO_PATH: &str =
    "assets/11/1111111111111111111111111111111111111111111111111111111111111111.mp4";
const DUMMY_IMAGE_PATH: &str =
    "assets/11/1111111111111111111111111111111111111111111111111111111111111111.png";

/// Load the canonical empty-project fixture as the prior state.
fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture → Project")
}

/// The fixture project's own id — every test reuses it as
/// `args.project_id` so the round-trip envelope's `project_id` field
/// matches by construction.
fn fixture_project_id() -> ProjectId {
    empty_project().id
}

/// Build a [`Project`] whose `canvas` field is set to `c`. Everything
/// else mirrors `empty_project_create.json`.
fn project_with_canvas(c: Canvas) -> Project {
    let mut p = empty_project();
    p.canvas = c;
    p
}

/// Convenience: build `ProjectSetCanvasArgs` with the fixture project
/// id and the supplied `canvas`. All optional fields default to `None`
/// (partial-update: leave the prior canvas's background/pixel-aspect
/// unchanged).
fn minimal_args(canvas: &str) -> ProjectSetCanvasArgs {
    ProjectSetCanvasArgs {
        project_id: fixture_project_id(),
        canvas: canvas.to_string(),
        background: None,
        pixel_aspect_num: None,
        pixel_aspect_den: None,
    }
}

fn make_video_asset(id: &str, width: u32, height: u32) -> Asset {
    serde_json::from_value(json!({
        "kind": "video",
        "id": id,
        "hash": DUMMY_HASH,
        "path": DUMMY_VIDEO_PATH,
        "original_filename": "sample.mp4",
        "imported_at": "2024-01-01T00:00:00Z",
        "metadata": {
            "duration_tk": 240000,
            "width": width,
            "height": height,
            "fps_num": 30,
            "fps_den": 1,
            "video_codec": "h264",
            "container": "mp4",
            "fingerprint": {
                "mtime_ms": 0,
                "size_bytes": 1234
            }
        }
    }))
    .expect("video asset JSON → Asset")
}

fn make_image_asset(id: &str, width: u32, height: u32) -> Asset {
    serde_json::from_value(json!({
        "kind": "image",
        "id": id,
        "hash": DUMMY_HASH,
        "path": DUMMY_IMAGE_PATH,
        "original_filename": "sample.png",
        "imported_at": "2024-01-01T00:00:00Z",
        "metadata": {
            "width": width,
            "height": height,
            "container": "png",
            "fingerprint": {
                "mtime_ms": 0,
                "size_bytes": 1234
            }
        }
    }))
    .expect("image asset JSON → Asset")
}

fn make_clip(id: &str, asset_id: &str, transform: Transform) -> Clip {
    serde_json::from_value(json!({
        "id": id,
        "name": "warn-clip",
        "asset_id": asset_id,
        "track_position_tk": 0,
        "source_in_tk": 0,
        "source_out_tk": 240000,
        "transform": transform,
    }))
    .expect("clip JSON → Clip")
}

fn make_track(kind: TrackKind, id: &str, clips: Vec<Clip>) -> Track {
    let kind = match kind {
        TrackKind::Video => "video",
        TrackKind::Audio => "audio",
        TrackKind::Text => "text",
        TrackKind::Effect => "effect",
    };
    let clip_values = serde_json::to_value(clips).expect("track clips → JSON");
    serde_json::from_value(json!({
        "id": id,
        "kind": kind,
        "name": "warn-track",
        "clips": clip_values,
    }))
    .expect("track JSON → Track")
}

fn make_project_with_assets_and_tracks(assets: Vec<Asset>, tracks: Vec<Track>) -> Project {
    let mut project = empty_project();
    project.assets = assets;
    project.tracks = tracks;
    project
}

/// Convenience: extract the `replace`-op value off the patch. Panics
/// (test-only) if the patch shape isn't the wholesale-replace form.
fn patch_canvas_value(patch: &Value) -> Value {
    let arr = patch.as_array().expect("patch is an array");
    assert_eq!(arr.len(), 1, "wholesale-replace patch has exactly one op");
    let op = arr[0].as_object().expect("op is an object");
    assert_eq!(op.get("op").and_then(Value::as_str), Some("replace"));
    assert_eq!(op.get("path").and_then(Value::as_str), Some("/canvas"));
    op.get("value")
        .cloned()
        .expect("replace op carries a value")
}

// ---------------------------------------------------------------------
// Happy paths
// ---------------------------------------------------------------------

#[test]
fn compute_patch_minimal_canvas_succeeds() {
    // No optionals supplied — width/height update, background and
    // pixel-aspect stay at the prior values (default `#000000ff` / 1/1).
    let prior = empty_project();
    let args = minimal_args("1920x1080");
    let (patch, new_canvas, _warnings) =
        compute_patch(&prior, &args).expect("happy-path minimal compute_patch");

    assert_eq!(new_canvas.width, 1920);
    assert_eq!(new_canvas.height, 1080);
    assert_eq!(new_canvas.background, prior.canvas.background);
    assert_eq!(new_canvas.pixel_aspect_num, prior.canvas.pixel_aspect_num);
    assert_eq!(new_canvas.pixel_aspect_den, prior.canvas.pixel_aspect_den);

    // Patch's /canvas value carries the new dims.
    let patch_canvas: Canvas =
        serde_json::from_value(patch_canvas_value(&patch)).expect("patch /canvas → Canvas");
    assert_eq!(patch_canvas, new_canvas);
}

#[test]
fn compute_patch_with_all_optionals_succeeds() {
    let prior = empty_project();
    let args = ProjectSetCanvasArgs {
        project_id: fixture_project_id(),
        canvas: "1280x720".to_string(),
        background: Some("#abcdef12".to_string()),
        pixel_aspect_num: Some(4),
        pixel_aspect_den: Some(3),
    };
    let (_, new_canvas, _warnings) =
        compute_patch(&prior, &args).expect("happy-path all-optionals compute_patch");

    assert_eq!(new_canvas.width, 1280);
    assert_eq!(new_canvas.height, 720);
    assert_eq!(new_canvas.background.as_str(), "#abcdef12");
    assert_eq!(new_canvas.pixel_aspect_num, 4);
    assert_eq!(new_canvas.pixel_aspect_den, 3);
}

#[test]
fn compute_patch_partial_update_preserves_existing() {
    // Prior carries a non-default background; args supplies only the
    // canvas string; new canvas must keep the prior background.
    let prior = project_with_canvas(Canvas {
        width: 1080,
        height: 1920,
        background: verbreel_state::Color::new("#ff0000ff".to_string())
            .expect("valid color literal"),
        pixel_aspect_num: 2,
        pixel_aspect_den: 1,
    });
    let args = minimal_args("640x480");
    let (_, new_canvas, _warnings) =
        compute_patch(&prior, &args).expect("partial-update compute_patch ok");

    assert_eq!(new_canvas.width, 640);
    assert_eq!(new_canvas.height, 480);
    // Untouched fields preserved.
    assert_eq!(new_canvas.background.as_str(), "#ff0000ff");
    assert_eq!(new_canvas.pixel_aspect_num, 2);
    assert_eq!(new_canvas.pixel_aspect_den, 1);
}

#[test]
fn compute_patch_background_uppercase_normalized_to_lowercase() {
    // The Color newtype's regex `^#[0-9a-fA-F]{8}$` accepts mixed case
    // and lowercases on construction (§0.5.2 canonical form). The verb
    // routes through Color::try_from so an uppercase background string
    // is accepted but stored as lowercase. This deviates from the
    // task-prompt's "uppercase rejected" expectation — see the
    // module-level rustdoc and the task response's Deviations section
    // for the rationale (Color/spec both allow mixed case).
    let prior = empty_project();
    let args = ProjectSetCanvasArgs {
        project_id: fixture_project_id(),
        canvas: "320x240".to_string(),
        background: Some("#FFFFFFFF".to_string()),
        pixel_aspect_num: None,
        pixel_aspect_den: None,
    };
    let (_, new_canvas, _warnings) =
        compute_patch(&prior, &args).expect("uppercase background normalizes, not rejects");
    assert_eq!(new_canvas.background.as_str(), "#ffffffff");
}

// ---------------------------------------------------------------------
// Canvas-string error paths
// ---------------------------------------------------------------------

#[test]
fn compute_patch_canvas_malformed_errors() {
    let prior = empty_project();
    let args = minimal_args("1920");
    let err = compute_patch(&prior, &args).expect_err("missing-x must reject");
    assert_eq!(
        err,
        ProjectSetCanvasError::CanvasMalformed {
            value: "1920".to_string()
        }
    );
}

#[test]
fn compute_patch_canvas_lowercase_x_only() {
    // Uppercase `X` is NOT in the regex `^[0-9]+x[0-9]+$` — must
    // reject (matches §2.10's literal pattern).
    let prior = empty_project();
    let args = minimal_args("1920X1080");
    let err = compute_patch(&prior, &args).expect_err("uppercase X must reject");
    assert_eq!(
        err,
        ProjectSetCanvasError::CanvasMalformed {
            value: "1920X1080".to_string()
        }
    );
}

// ---------------------------------------------------------------------
// Width / height range error paths
// ---------------------------------------------------------------------

#[test]
fn compute_patch_width_below_min() {
    let prior = empty_project();
    let args = minimal_args("8x100");
    match compute_patch(&prior, &args).expect_err("width below min must reject") {
        ProjectSetCanvasError::WidthOutOfRange { value, min, max } => {
            assert_eq!(value, 8);
            assert_eq!(min, CANVAS_MIN_DIM);
            assert_eq!(max, CANVAS_MAX_DIM);
        }
        other => panic!("expected WidthOutOfRange, got {other:?}"),
    }
}

#[test]
fn compute_patch_width_above_max() {
    let prior = empty_project();
    let args = minimal_args("9000x100");
    match compute_patch(&prior, &args).expect_err("width above max must reject") {
        ProjectSetCanvasError::WidthOutOfRange { value, min, max } => {
            assert_eq!(value, 9000);
            assert_eq!(min, CANVAS_MIN_DIM);
            assert_eq!(max, CANVAS_MAX_DIM);
        }
        other => panic!("expected WidthOutOfRange, got {other:?}"),
    }
}

#[test]
fn compute_patch_height_below_min() {
    let prior = empty_project();
    let args = minimal_args("100x8");
    match compute_patch(&prior, &args).expect_err("height below min must reject") {
        ProjectSetCanvasError::HeightOutOfRange { value, min, max } => {
            assert_eq!(value, 8);
            assert_eq!(min, CANVAS_MIN_DIM);
            assert_eq!(max, CANVAS_MAX_DIM);
        }
        other => panic!("expected HeightOutOfRange, got {other:?}"),
    }
}

#[test]
fn compute_patch_height_above_max() {
    let prior = empty_project();
    let args = minimal_args("100x9000");
    match compute_patch(&prior, &args).expect_err("height above max must reject") {
        ProjectSetCanvasError::HeightOutOfRange { value, min, max } => {
            assert_eq!(value, 9000);
            assert_eq!(min, CANVAS_MIN_DIM);
            assert_eq!(max, CANVAS_MAX_DIM);
        }
        other => panic!("expected HeightOutOfRange, got {other:?}"),
    }
}

#[test]
fn compute_patch_width_overflow_routes_to_out_of_range() {
    // A capture group above u32::MAX surfaces as WidthOutOfRange with
    // value: u32::MAX (it's, by definition, above the 8192 cap).
    // Documented design choice — see the module rustdoc on
    // parse_canvas_dims.
    let prior = empty_project();
    let args = minimal_args("99999999999x100");
    match compute_patch(&prior, &args).expect_err("width overflow must reject") {
        ProjectSetCanvasError::WidthOutOfRange { value, max, .. } => {
            assert_eq!(value, u32::MAX);
            assert_eq!(max, CANVAS_MAX_DIM);
        }
        other => panic!("expected WidthOutOfRange on overflow, got {other:?}"),
    }
}

// ---------------------------------------------------------------------
// Background error paths
// ---------------------------------------------------------------------

#[test]
fn compute_patch_background_invalid_hex() {
    let prior = empty_project();
    let args = ProjectSetCanvasArgs {
        project_id: fixture_project_id(),
        canvas: "640x480".to_string(),
        background: Some("rgb(0,0,0)".to_string()),
        pixel_aspect_num: None,
        pixel_aspect_den: None,
    };
    match compute_patch(&prior, &args).expect_err("non-hex background must reject") {
        ProjectSetCanvasError::BackgroundInvalid { detail } => {
            assert!(detail.contains("Color"), "detail mentions Color: {detail}");
        }
        other => panic!("expected BackgroundInvalid, got {other:?}"),
    }
}

#[test]
fn compute_patch_background_missing_alpha() {
    // 6-digit hex doesn't match the schema's 8-digit Color pattern.
    let prior = empty_project();
    let args = ProjectSetCanvasArgs {
        project_id: fixture_project_id(),
        canvas: "640x480".to_string(),
        background: Some("#ffffff".to_string()),
        pixel_aspect_num: None,
        pixel_aspect_den: None,
    };
    let err = compute_patch(&prior, &args).expect_err("missing-alpha background must reject");
    assert!(
        matches!(err, ProjectSetCanvasError::BackgroundInvalid { .. }),
        "expected BackgroundInvalid, got {err:?}"
    );
}

// ---------------------------------------------------------------------
// Pixel-aspect error paths
// ---------------------------------------------------------------------

#[test]
fn compute_patch_pixel_aspect_num_zero() {
    let prior = empty_project();
    let args = ProjectSetCanvasArgs {
        project_id: fixture_project_id(),
        canvas: "640x480".to_string(),
        background: None,
        pixel_aspect_num: Some(0),
        pixel_aspect_den: None,
    };
    match compute_patch(&prior, &args).expect_err("pixel_aspect_num=0 must reject") {
        ProjectSetCanvasError::PixelAspectNumOutOfRange { value, min } => {
            assert_eq!(value, 0);
            assert_eq!(min, PIXEL_ASPECT_MIN);
        }
        other => panic!("expected PixelAspectNumOutOfRange, got {other:?}"),
    }
}

#[test]
fn compute_patch_pixel_aspect_den_zero() {
    let prior = empty_project();
    let args = ProjectSetCanvasArgs {
        project_id: fixture_project_id(),
        canvas: "640x480".to_string(),
        background: None,
        pixel_aspect_num: None,
        pixel_aspect_den: Some(0),
    };
    match compute_patch(&prior, &args).expect_err("pixel_aspect_den=0 must reject") {
        ProjectSetCanvasError::PixelAspectDenOutOfRange { value, min } => {
            assert_eq!(value, 0);
            assert_eq!(min, PIXEL_ASPECT_MIN);
        }
        other => panic!("expected PixelAspectDenOutOfRange, got {other:?}"),
    }
}

// ---------------------------------------------------------------------
// Boundary dimensions
// ---------------------------------------------------------------------

#[test]
fn boundary_dimensions_16x16_accepted() {
    let prior = empty_project();
    let args = minimal_args("16x16");
    let (_, new_canvas, _warnings) = compute_patch(&prior, &args).expect("16x16 must be accepted");
    assert_eq!(new_canvas.width, CANVAS_MIN_DIM);
    assert_eq!(new_canvas.height, CANVAS_MIN_DIM);
}

#[test]
fn boundary_dimensions_8192x8192_accepted() {
    let prior = empty_project();
    let args = minimal_args("8192x8192");
    let (_, new_canvas, _warnings) =
        compute_patch(&prior, &args).expect("8192x8192 must be accepted");
    assert_eq!(new_canvas.width, CANVAS_MAX_DIM);
    assert_eq!(new_canvas.height, CANVAS_MAX_DIM);
}

// ---------------------------------------------------------------------
// Envelope helper
// ---------------------------------------------------------------------

#[test]
fn data_envelope_returns_post_state_canvas() {
    let post_state = project_with_canvas(Canvas {
        width: 1920,
        height: 1080,
        background: verbreel_state::Color::new("#deadbeef".to_string())
            .expect("valid color literal"),
        pixel_aspect_num: 1,
        pixel_aspect_den: 1,
    });
    let args = minimal_args("1920x1080");
    let env = data_envelope(&args, &post_state);
    assert_eq!(env.project_id, args.project_id);
    assert_eq!(env.canvas, post_state.canvas);
}

#[test]
fn compute_patch_no_clips_emits_no_warnings() {
    let prior = empty_project();
    let args = minimal_args("1920x1080");
    let (_, _, warnings) = compute_patch(&prior, &args).expect("no clips => no warnings");

    assert!(warnings.is_empty(), "empty project has no clips to test");
}

#[test]
fn compute_patch_clip_fully_inside_emits_no_warnings() {
    let prior = {
        let assets = vec![make_video_asset(SAMPLE_VIDEO_ASSET_ID, 1920, 1080)];
        let clips = vec![make_clip(
            CLIP_ID_AUDIO,
            SAMPLE_VIDEO_ASSET_ID,
            Transform::default(),
        )];
        let tracks = vec![make_track(TrackKind::Video, SAMPLE_VIDEO_TRACK_ID, clips)];
        make_project_with_assets_and_tracks(assets, tracks)
    };
    let args = minimal_args("1280x720");
    let (_, _, warnings) = compute_patch(&prior, &args).expect("clip still partially overlaps");

    assert!(
        warnings.is_empty(),
        "clip is partly in-frame on resized canvas => no warning"
    );
}

#[test]
fn compute_patch_clip_translated_off_canvas_emits_warning() {
    let prior = {
        let assets = vec![make_video_asset(SAMPLE_VIDEO_ASSET_ID, 1920, 1080)];
        let transform = Transform {
            x: 10000.0,
            ..Transform::default()
        };
        let clips = vec![make_clip(CLIP_ID_A, SAMPLE_VIDEO_ASSET_ID, transform)];
        let tracks = vec![make_track(TrackKind::Video, SAMPLE_VIDEO_TRACK_ID, clips)];
        make_project_with_assets_and_tracks(assets, tracks)
    };
    let args = minimal_args("1920x1080");
    let (_, _, warnings) = compute_patch(&prior, &args).expect("off-canvas clip emits warning");

    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_CANVAS_CLIPS_OUT_OF_FRAME);
    assert_eq!(
        warnings[0]["details"]["affected_clip_ids"]
            .as_array()
            .expect("id list exists"),
        &vec![CLIP_ID_A.to_string()]
    );
}

#[test]
fn compute_patch_clip_negative_x_off_canvas_emits_warning() {
    let prior = {
        let assets = vec![make_video_asset(SAMPLE_VIDEO_ASSET_ID, 1920, 1080)];
        let transform = Transform {
            x: -5000.0,
            ..Transform::default()
        };
        let clips = vec![make_clip(CLIP_ID_B, SAMPLE_VIDEO_ASSET_ID, transform)];
        let tracks = vec![make_track(TrackKind::Video, SAMPLE_VIDEO_TRACK_ID, clips)];
        make_project_with_assets_and_tracks(assets, tracks)
    };
    let args = minimal_args("1920x1080");
    let (_, _, warnings) = compute_patch(&prior, &args).expect("off-canvas clip emits warning");

    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_CANVAS_CLIPS_OUT_OF_FRAME);
}

#[test]
fn compute_patch_clip_off_canvas_below_emits_warning() {
    let prior = {
        let assets = vec![make_video_asset(SAMPLE_VIDEO_ASSET_ID, 1920, 1080)];
        let transform = Transform {
            y: 5000.0,
            ..Transform::default()
        };
        let clips = vec![make_clip(CLIP_ID_C, SAMPLE_VIDEO_ASSET_ID, transform)];
        let tracks = vec![make_track(TrackKind::Video, SAMPLE_VIDEO_TRACK_ID, clips)];
        make_project_with_assets_and_tracks(assets, tracks)
    };
    let args = minimal_args("1920x1080");
    let (_, _, warnings) = compute_patch(&prior, &args).expect("off-canvas clip emits warning");

    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_CANVAS_CLIPS_OUT_OF_FRAME);
}

#[test]
fn compute_patch_clip_partial_overlap_no_warning() {
    let prior = {
        let assets = vec![make_video_asset(SAMPLE_VIDEO_ASSET_ID, 1920, 1080)];
        let transform = Transform {
            x: 640.0,
            ..Transform::default()
        };
        let clips = vec![make_clip(CLIP_ID_B, SAMPLE_VIDEO_ASSET_ID, transform)];
        let tracks = vec![make_track(TrackKind::Video, SAMPLE_VIDEO_TRACK_ID, clips)];
        make_project_with_assets_and_tracks(assets, tracks)
    };
    let args = minimal_args("1280x720");
    let (_, _, warnings) = compute_patch(&prior, &args).expect("partial overlap => no warning");

    assert!(
        warnings.is_empty(),
        "half in / half out is not fully outside"
    );
}

#[test]
fn compute_patch_multiple_off_canvas_clips_all_listed() {
    let prior = {
        let assets = vec![make_video_asset(SAMPLE_VIDEO_ASSET_ID, 1920, 1080)];
        let right = Transform {
            x: 10000.0,
            ..Transform::default()
        };
        let left = Transform {
            x: -5000.0,
            ..Transform::default()
        };
        let below = Transform {
            y: 5000.0,
            ..Transform::default()
        };
        let clips = vec![
            make_clip(CLIP_ID_B, SAMPLE_VIDEO_ASSET_ID, right),
            make_clip(CLIP_ID_A, SAMPLE_VIDEO_ASSET_ID, left),
            make_clip(CLIP_ID_C, SAMPLE_VIDEO_ASSET_ID, below),
        ];
        let tracks = vec![make_track(TrackKind::Video, SAMPLE_VIDEO_TRACK_ID, clips)];
        make_project_with_assets_and_tracks(assets, tracks)
    };
    let args = minimal_args("1920x1080");
    let (_, _, warnings) = compute_patch(&prior, &args).expect("three off-canvas clips");

    let ids = warnings[0]["details"]["affected_clip_ids"]
        .as_array()
        .expect("id list exists")
        .iter()
        .map(|v| v.as_str().expect("id string"))
        .collect::<Vec<_>>();
    assert_eq!(ids, vec![CLIP_ID_A, CLIP_ID_B, CLIP_ID_C]);
}

#[test]
fn compute_patch_warning_skips_audio_track() {
    let prior = {
        let assets = vec![make_video_asset(SAMPLE_VIDEO_ASSET_ID, 1920, 1080)];
        let transform = Transform {
            x: 10000.0,
            ..Transform::default()
        };
        let clips = vec![make_clip(CLIP_ID_AUDIO, SAMPLE_VIDEO_ASSET_ID, transform)];
        let tracks = vec![make_track(TrackKind::Audio, SAMPLE_AUDIO_TRACK_ID, clips)];
        make_project_with_assets_and_tracks(assets, tracks)
    };
    let args = minimal_args("1920x1080");
    let (_, _, warnings) = compute_patch(&prior, &args).expect("audio track is ignored");

    assert!(warnings.is_empty());
}

#[test]
fn compute_patch_warning_skips_text_track() {
    let prior = {
        let assets = vec![make_image_asset(SAMPLE_IMAGE_ASSET_ID, 1920, 1080)];
        let transform = Transform {
            x: 10000.0,
            ..Transform::default()
        };
        let clips = vec![make_clip(CLIP_ID_TEXT, SAMPLE_IMAGE_ASSET_ID, transform)];
        let tracks = vec![make_track(TrackKind::Text, SAMPLE_TEXT_TRACK_ID, clips)];
        make_project_with_assets_and_tracks(assets, tracks)
    };
    let args = minimal_args("1920x1080");
    let (_, _, warnings) = compute_patch(&prior, &args).expect("text track is ignored");

    assert!(warnings.is_empty());
}

#[test]
fn compute_patch_warning_skips_unresolved_asset_id() {
    let prior = {
        let assets = vec![make_video_asset(SAMPLE_VIDEO_ASSET_ID, 1920, 1080)];
        let nil_transform = Transform::default();
        let clips = vec![
            make_clip(CLIP_ID_NIL, NIL_ASSET_ID, nil_transform),
            make_clip(
                CLIP_ID_MISSING,
                SAMPLE_MISSING_ASSET_ID,
                Transform::default(),
            ),
        ];
        let tracks = vec![make_track(TrackKind::Video, SAMPLE_VIDEO_TRACK_ID, clips)];
        make_project_with_assets_and_tracks(assets, tracks)
    };
    let args = minimal_args("1920x1080");
    let (_, _, warnings) = compute_patch(&prior, &args).expect("unresolved asset ids are skipped");

    assert!(warnings.is_empty());
}

#[test]
fn compute_patch_warning_message_format() {
    let prior = {
        let assets = vec![make_video_asset(SAMPLE_VIDEO_ASSET_ID, 1920, 1080)];
        let transform = Transform {
            x: 10000.0,
            ..Transform::default()
        };
        let clips = vec![make_clip(CLIP_ID_A, SAMPLE_VIDEO_ASSET_ID, transform)];
        let tracks = vec![make_track(TrackKind::Video, SAMPLE_VIDEO_TRACK_ID, clips)];
        make_project_with_assets_and_tracks(assets, tracks)
    };
    let args = minimal_args("1920x1080");
    let (_, _, warnings) = compute_patch(&prior, &args).expect("warning includes message + ids");

    let warning = warnings
        .into_iter()
        .next()
        .expect("one warning must be emitted");
    assert_eq!(warning["code"], W_CANVAS_CLIPS_OUT_OF_FRAME);
    let msg = warning["message"].as_str().expect("message exists");
    assert!(!msg.is_empty(), "message is non-empty");

    let ids = warning["details"]["affected_clip_ids"]
        .as_array()
        .expect("affected_clip_ids exists");
    assert!(!ids.is_empty(), "affected_clip_ids is non-empty");
    for id in ids {
        assert!(id.as_str().is_some(), "ids are strings");
    }
}

#[cfg(feature = "native")]
#[test]
fn compute_patch_warning_persists_through_mutate_via_verb() {
    use tempfile::TempDir;

    let project = {
        let assets = vec![make_video_asset(SAMPLE_VIDEO_ASSET_ID, 1920, 1080)];
        let transform = Transform {
            x: 10000.0,
            ..Transform::default()
        };
        let clips = vec![make_clip(CLIP_ID_A, SAMPLE_VIDEO_ASSET_ID, transform)];
        let tracks = vec![make_track(TrackKind::Video, SAMPLE_VIDEO_TRACK_ID, clips)];
        let mut project = make_project_with_assets_and_tracks(assets, tracks);
        project.duration_tk = Tick::new(240000);
        project
    };

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        project,
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry clears the gate and writes project.json");

    let args = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "canvas": "1920x1080",
    });

    let outcome = store
        .mutate_via_verb("project.set_canvas", args, None)
        .expect("mutate_via_verb happy path");

    let MutateOutcome::Applied { warnings, .. } = outcome else {
        panic!("expected Applied, got {outcome:?}");
    };

    assert_eq!(warnings.len(), 1, "warning is persisted by verb output");
    assert_eq!(warnings[0]["code"], W_CANVAS_CLIPS_OUT_OF_FRAME);
}

#[cfg(feature = "native")]
#[test]
fn compute_patch_warning_persists_on_replay() {
    use tempfile::TempDir;

    let project = {
        let assets = vec![make_video_asset(SAMPLE_VIDEO_ASSET_ID, 1920, 1080)];
        let transform = Transform {
            x: 10000.0,
            ..Transform::default()
        };
        let clips = vec![make_clip(CLIP_ID_A, SAMPLE_VIDEO_ASSET_ID, transform)];
        let tracks = vec![make_track(TrackKind::Video, SAMPLE_VIDEO_TRACK_ID, clips)];
        let mut project = make_project_with_assets_and_tracks(assets, tracks);
        project.duration_tk = Tick::new(240000);
        project
    };

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        project,
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry clears the gate and writes project.json");

    let args = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "canvas": "1920x1080",
    });

    let first = store
        .mutate_via_verb("project.set_canvas", args.clone(), Some("k1".into()))
        .expect("first keyed mutate");
    let first_warnings = if let MutateOutcome::Applied { warnings, .. } = first {
        warnings
    } else {
        panic!("expected Applied, got {first:?}");
    };

    assert_eq!(first_warnings.len(), 1, "first call emits one warning");
    assert_eq!(first_warnings[0]["code"], W_CANVAS_CLIPS_OUT_OF_FRAME);

    let replay = store
        .mutate_via_verb("project.set_canvas", args, Some("k1".into()))
        .expect("replay keyed mutate");
    let replay_warnings = if let MutateOutcome::Replayed { warnings, .. } = replay {
        warnings
    } else {
        panic!("expected Replayed, got {replay:?}");
    };

    assert_eq!(replay_warnings.len(), 2, "replay appends W_REPLAY");
    assert_eq!(replay_warnings[0]["code"], W_CANVAS_CLIPS_OUT_OF_FRAME);
    assert_eq!(replay_warnings[1]["code"], "W_REPLAY");
}

#[test]
fn compute_patch_rotation_keeps_partial_overlap() {
    let prior = {
        let assets = vec![make_video_asset(SAMPLE_VIDEO_ASSET_ID, 100, 100)];
        let transform = Transform {
            rotation_deg: 45.0,
            x: 250.0,
            y: 250.0,
            ..Transform::default()
        };
        let clips = vec![make_clip(CLIP_ID_ROTATE, SAMPLE_VIDEO_ASSET_ID, transform)];
        let tracks = vec![make_track(TrackKind::Video, SAMPLE_VIDEO_TRACK_ID_2, clips)];
        make_project_with_assets_and_tracks(assets, tracks)
    };
    let args = minimal_args("300x300");
    let (_, _, warnings) = compute_patch(&prior, &args).expect("45° overlap remains partial");

    assert!(
        warnings.is_empty(),
        "45° clip with one corner inside is not fully outside"
    );
}

// ---------------------------------------------------------------------
// Reconstructor round-trip — the §0.8 startup-gate exercise
// ---------------------------------------------------------------------

#[test]
fn reconstructor_round_trip() {
    let prior = empty_project();
    let args = ProjectSetCanvasArgs {
        project_id: fixture_project_id(),
        canvas: "1920x1080".to_string(),
        background: Some("#11223344".to_string()),
        pixel_aspect_num: Some(2),
        pixel_aspect_den: Some(1),
    };

    let (patch, new_canvas, _warnings) = compute_patch(&prior, &args).expect("compute_patch ok");

    let mut post_state = prior.clone();
    post_state.canvas = new_canvas;

    let expected_envelope = data_envelope(&args, &post_state);
    let expected_data = serde_json::to_value(&expected_envelope).expect("envelope → Value");

    let recorded = RecordedEvent {
        verb: "project.set_canvas".to_owned(),
        args: serde_json::to_value(&args).expect("args → Value"),
        patch,
        warnings: vec![],
        post_state,
        expected_data,
    };

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(ProjectSetCanvasVerb))
        .expect("register ok");

    let report = validate_reconstructors(&registry, &[recorded])
        .expect("reconstructor round-trip must pass");
    assert_eq!(report.verbs_checked, vec!["project.set_canvas"]);
    assert_eq!(report.fixtures_run, 1);
}

// ---------------------------------------------------------------------
// End-to-end through ProjectStore::mutate_via_verb — native only
// ---------------------------------------------------------------------

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        empty_project(),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry clears the gate and writes project.json");

    let args = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "canvas": "1920x1080",
        "background": "#abcdef12",
        "pixel_aspect_num": 4,
        "pixel_aspect_den": 3,
    });

    let outcome = store
        .mutate_via_verb("project.set_canvas", args, None)
        .expect("mutate_via_verb happy path");

    let MutateOutcome::Applied {
        event_id,
        data,
        warnings: _,
        ..
    } = outcome
    else {
        panic!("happy path must return Applied, got {outcome:?}");
    };

    assert_eq!(
        store.last_applied_event_id(),
        Some(event_id),
        "store tracks the just-applied event"
    );

    // In-memory project reflects the new canvas.
    assert_eq!(store.project().canvas.width, 1920);
    assert_eq!(store.project().canvas.height, 1080);
    assert_eq!(store.project().canvas.background.as_str(), "#abcdef12");
    assert_eq!(store.project().canvas.pixel_aspect_num, 4);
    assert_eq!(store.project().canvas.pixel_aspect_den, 3);

    // Data envelope shape: `{ project_id, canvas: {...} }`.
    let expected = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "canvas": {
            "width": 1920,
            "height": 1080,
            "background": "#abcdef12",
            "pixel_aspect_num": 4,
            "pixel_aspect_den": 3,
        },
    });
    assert_eq!(
        data, expected,
        "data envelope is the verb's typed `{{ project_id, canvas }}` shape"
    );
}
