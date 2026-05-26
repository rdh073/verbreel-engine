//! Tests for `audio.volume` (§9.2) — sixtieth production verb.

use serde_json::{Value, json};
use std::sync::Arc;

use verbreel_state::verbs::audio_volume::{
    GAIN_DB_HINT, SELECTOR_HINT, compute_patch, data_envelope_from_post_state,
};
use verbreel_state::{
    AudioVolumeArgs, AudioVolumeData, AudioVolumeError, AudioVolumeTargetKind, AudioVolumeVerb,
    MutateOutcome, Project, Track, TrackKind, VerbRegistry, default_fixtures, default_registry,
    validate_reconstructors,
};
use verbreel_types::{ProjectId, Tick};

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const TRACK_AUDIO_A: &str = "0190b8d3-15e3-7000-bd00-0000000aa902";
const TRACK_VIDEO_A: &str = "0190b8d3-15e3-7000-bd00-0000000aa9a2";
const TRACK_TEXT_A: &str = "0190b8d3-15e3-7000-bd00-0000000aa9a3";
const TRACK_EFFECT_A: &str = "0190b8d3-15e3-7000-bd00-0000000aa9a4";
const CLIP_AUDIO_A: &str = "0190b8d3-15e3-7000-bd00-0000000bb902";
const CLIP_VIDEO_A: &str = "0190b8d3-15e3-7000-bd00-0000000bb9a2";
const CLIP_TEXT_A: &str = "0190b8d3-15e3-7000-bd00-0000000bb9a3";
const CLIP_IMAGE_A: &str = "0190b8d3-15e3-7000-bd00-0000000bb9a4";
const MISSING_CLIP: &str = "0190b8d3-15e3-7000-bd00-0000000cc9aa";
const MISSING_TRACK: &str = "0190b8d3-15e3-7000-bd00-0000000cc9ab";
const ASSET_AUDIO_ID: &str = "0190b8d3-15e3-7000-bd00-0000000dd902";
const ASSET_VIDEO_ID: &str = "0190b8d3-15e3-7000-bd00-0000000dd9a2";
const ASSET_IMAGE_ID: &str = "0190b8d3-15e3-7000-bd00-0000000dd9a4";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture -> Project")
}

fn fixture_project_id() -> ProjectId {
    empty_project().id
}

struct ClipTrackFixture<'a> {
    kind: TrackKind,
    track_id: &'a str,
    track_locked: bool,
    clip_id: &'a str,
    clip_locked: bool,
    clip_volume: f64,
    asset_id: &'a str,
}

fn clip_track(fixture: ClipTrackFixture<'_>) -> Track {
    let text = matches!(fixture.kind, TrackKind::Text).then(|| {
        json!({
            "content": "Vol",
            "font_family": "Arial",
            "font_size_px": 24,
        })
    });

    let mut clip = json!({
        "id": fixture.clip_id,
        "name": "Vol Clip",
        "asset_id": fixture.asset_id,
        "track_position_tk": 0,
        "source_in_tk": 0,
        "source_out_tk": 240_000,
        "locked": fixture.clip_locked,
        "volume": fixture.clip_volume,
    });
    if let Some(text) = text {
        clip.as_object_mut()
            .expect("clip is object")
            .insert("text".to_string(), text);
    }

    serde_json::from_value(json!({
        "id": fixture.track_id,
        "kind": fixture.kind,
        "name": "Track",
        "locked": fixture.track_locked,
        "clips": [clip],
    }))
    .expect("track fixture parses")
}

fn empty_track(kind: TrackKind, track_id: &str, track_locked: bool, volume: f64) -> Track {
    serde_json::from_value(json!({
        "id": track_id,
        "kind": kind,
        "name": "Track",
        "locked": track_locked,
        "volume": volume,
        "clips": [],
    }))
    .expect("track parses")
}

fn project_with_tracks(tracks: Vec<Track>) -> Project {
    let mut project = empty_project();
    let any_clip = tracks.iter().any(|track| !track.clips.is_empty());
    project.tracks = tracks;
    // Empty-track projects must keep `duration_tk == 0`; the project
    // duration invariant recomputes from clip positions on `apply`.
    project.duration_tk = if any_clip {
        Tick::new(240_000)
    } else {
        Tick::new(0)
    };
    project
}

fn audio_asset_json(id: &str, filename: &str) -> Value {
    json!({
        "id": id,
        "kind": "audio",
        "hash": "53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658",
        "path": "assets/53/53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.m4a",
        "original_filename": filename,
        "imported_at": "2026-05-24T00:00:00Z",
        "metadata": {
            "duration_tk": 240_000,
            "audio_codec": "aac",
            "audio_channels": 2,
            "audio_sample_rate_hz": 48_000,
            "container": "m4a",
            "fingerprint": {
                "mtime_ms": 1_700_000_000_000_i64,
                "size_bytes": 512,
            }
        }
    })
}

fn video_asset_json(id: &str, filename: &str) -> Value {
    json!({
        "id": id,
        "kind": "video",
        "hash": "36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da",
        "path": "assets/36/36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da.mp4",
        "original_filename": filename,
        "imported_at": "2026-05-24T00:00:00Z",
        "metadata": {
            "duration_tk": 240_000,
            "width": 1920,
            "height": 1080,
            "fps_num": 30,
            "fps_den": 1,
            "video_codec": "h264",
            "container": "mp4",
            "fingerprint": {
                "mtime_ms": 1_700_000_000_000_i64,
                "size_bytes": 1024,
            }
        }
    })
}

fn image_asset_json(id: &str, filename: &str) -> Value {
    json!({
        "id": id,
        "kind": "image",
        "hash": "46edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da",
        "path": "assets/46/46edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da.png",
        "original_filename": filename,
        "imported_at": "2026-05-24T00:00:00Z",
        "metadata": {
            "width": 1920,
            "height": 1080,
            "container": "png",
            "fingerprint": {
                "mtime_ms": 1_700_000_000_000_i64,
                "size_bytes": 1024,
            }
        }
    })
}

fn audio_clip_track(track_locked: bool, clip_locked: bool, clip_volume: f64) -> Track {
    clip_track(ClipTrackFixture {
        kind: TrackKind::Audio,
        track_id: TRACK_AUDIO_A,
        track_locked,
        clip_id: CLIP_AUDIO_A,
        clip_locked,
        clip_volume,
        asset_id: ASSET_AUDIO_ID,
    })
}

fn project_with_audio_clip() -> Project {
    let mut project = project_with_tracks(vec![audio_clip_track(false, false, 1.0)]);
    project.assets.push(
        serde_json::from_value(audio_asset_json(ASSET_AUDIO_ID, "audio-volume.m4a"))
            .expect("audio asset parses"),
    );
    project
}

fn args_clip(target: &str) -> AudioVolumeArgs {
    AudioVolumeArgs {
        project_id: fixture_project_id(),
        target: target.to_string(),
        gain: None,
        db: None,
    }
}

fn first_op_value(patch: &Value) -> f64 {
    let arr = patch.as_array().expect("patch is array");
    assert_eq!(arr.len(), 1, "audio.volume emits a single op");
    arr[0]["value"].as_f64().expect("value is f64")
}

fn first_op_path(patch: &Value) -> String {
    let arr = patch.as_array().expect("patch is array");
    arr[0]["path"].as_str().expect("path is str").to_string()
}

#[test]
fn args_deserialize_round_trip() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "target": format!("clip:{CLIP_AUDIO_A}"),
        "gain": 0.5,
    });
    let args: AudioVolumeArgs = serde_json::from_value(raw).expect("args deserialize");
    assert_eq!(args.target, format!("clip:{CLIP_AUDIO_A}"));
    assert_eq!(args.gain, Some(0.5));
    assert_eq!(args.db, None);
}

#[test]
fn args_missing_target_errors() {
    let raw = json!({ "project_id": FIXTURE_PROJECT_ID, "gain": 1.0 });
    let err =
        serde_json::from_value::<AudioVolumeArgs>(raw).expect_err("missing `target` must reject");
    assert!(err.to_string().contains("target"));
}

#[test]
fn args_wrong_target_type_errors() {
    let raw = json!({ "project_id": FIXTURE_PROJECT_ID, "target": 42, "gain": 1.0 });
    let err = serde_json::from_value::<AudioVolumeArgs>(raw)
        .expect_err("non-string `target` must reject");
    let msg = err.to_string();
    assert!(
        msg.contains("string") || msg.contains("target"),
        "error mentions string-shape rejection: {msg}"
    );
}

#[test]
fn both_gain_and_db_errors_args_incompatible() {
    let prior = project_with_audio_clip();
    let mut args = args_clip(&format!("clip:{CLIP_AUDIO_A}"));
    args.gain = Some(1.0);
    args.db = Some(0.0);

    let err = compute_patch(&prior, &args).expect_err("both gain and db");
    let AudioVolumeError::ArgsIncompatible { hint, .. } = err else {
        panic!("expected ArgsIncompatible, got {err:?}");
    };
    assert_eq!(hint, GAIN_DB_HINT);
}

#[test]
fn neither_gain_nor_db_errors_args_incompatible() {
    let prior = project_with_audio_clip();
    let args = args_clip(&format!("clip:{CLIP_AUDIO_A}"));

    let err = compute_patch(&prior, &args).expect_err("neither gain nor db");
    let AudioVolumeError::ArgsIncompatible { hint, .. } = err else {
        panic!("expected ArgsIncompatible, got {err:?}");
    };
    assert_eq!(hint, GAIN_DB_HINT);
}

#[test]
fn gain_below_zero_errors_bad_range() {
    let prior = project_with_audio_clip();
    let mut args = args_clip(&format!("clip:{CLIP_AUDIO_A}"));
    args.gain = Some(-0.1);

    let err = compute_patch(&prior, &args).expect_err("gain < 0");
    assert!(matches!(
        err,
        AudioVolumeError::BadRange {
            field: "gain",
            min: 0.0,
            max: 4.0,
            ..
        }
    ));
}

#[test]
fn gain_above_four_errors_bad_range() {
    let prior = project_with_audio_clip();
    let mut args = args_clip(&format!("clip:{CLIP_AUDIO_A}"));
    args.gain = Some(4.1);

    let err = compute_patch(&prior, &args).expect_err("gain > 4");
    assert!(matches!(
        err,
        AudioVolumeError::BadRange { field: "gain", .. }
    ));
}

#[test]
fn db_below_minus_sixty_errors_bad_range() {
    let prior = project_with_audio_clip();
    let mut args = args_clip(&format!("clip:{CLIP_AUDIO_A}"));
    args.db = Some(-60.1);

    let err = compute_patch(&prior, &args).expect_err("db < -60");
    assert!(matches!(
        err,
        AudioVolumeError::BadRange {
            field: "db",
            min,
            max,
            ..
        } if (min - -60.0).abs() < 1e-9 && (max - 12.0).abs() < 1e-9
    ));
}

#[test]
fn db_above_plus_twelve_errors_bad_range() {
    let prior = project_with_audio_clip();
    let mut args = args_clip(&format!("clip:{CLIP_AUDIO_A}"));
    args.db = Some(12.1);

    let err = compute_patch(&prior, &args).expect_err("db > +12");
    assert!(matches!(
        err,
        AudioVolumeError::BadRange { field: "db", .. }
    ));
}

#[test]
fn bare_body_selector_errors_bad_selector() {
    let prior = project_with_audio_clip();
    let mut args = args_clip(CLIP_AUDIO_A);
    args.gain = Some(0.5);

    let err = compute_patch(&prior, &args).expect_err("bare body rejected");
    let AudioVolumeError::BadSelector { hint, .. } = err else {
        panic!("expected BadSelector, got {err:?}");
    };
    assert_eq!(hint, SELECTOR_HINT);
}

#[test]
fn empty_selector_errors_bad_selector() {
    let prior = project_with_audio_clip();
    let mut args = args_clip("");
    args.gain = Some(0.5);

    let err = compute_patch(&prior, &args).expect_err("empty target");
    assert!(matches!(err, AudioVolumeError::BadSelector { .. }));
}

#[test]
fn unknown_prefix_errors_bad_selector() {
    let prior = project_with_audio_clip();
    let mut args = args_clip(&format!("effect:{CLIP_AUDIO_A}"));
    args.gain = Some(0.5);

    let err = compute_patch(&prior, &args).expect_err("unknown prefix");
    let AudioVolumeError::BadSelector { detail, .. } = err else {
        panic!("expected BadSelector, got {err:?}");
    };
    assert!(
        detail.contains("effect"),
        "detail mentions prefix: {detail}"
    );
}

#[test]
fn malformed_clip_body_errors_bad_selector() {
    let prior = project_with_audio_clip();
    let mut args = args_clip("clip:not-a-uuid");
    args.gain = Some(0.5);

    let err = compute_patch(&prior, &args).expect_err("malformed clip body");
    assert!(matches!(err, AudioVolumeError::BadSelector { .. }));
}

#[test]
fn malformed_track_body_errors_bad_selector() {
    let prior = project_with_audio_clip();
    let mut args = args_clip("track:not-a-uuid");
    args.gain = Some(0.5);

    let err = compute_patch(&prior, &args).expect_err("malformed track body");
    assert!(matches!(err, AudioVolumeError::BadSelector { .. }));
}

#[test]
fn missing_clip_errors_not_found() {
    let prior = project_with_audio_clip();
    let mut args = args_clip(&format!("clip:{MISSING_CLIP}"));
    args.gain = Some(0.5);

    let err = compute_patch(&prior, &args).expect_err("clip not found");
    assert!(matches!(
        err,
        AudioVolumeError::NotFound {
            target_kind: "clip",
            ..
        }
    ));
}

#[test]
fn missing_track_errors_not_found() {
    let prior = project_with_audio_clip();
    let mut args = args_clip(&format!("track:{MISSING_TRACK}"));
    args.gain = Some(0.5);

    let err = compute_patch(&prior, &args).expect_err("track not found");
    assert!(matches!(
        err,
        AudioVolumeError::NotFound {
            target_kind: "track",
            ..
        }
    ));
}

#[test]
fn video_clip_errors_clip_kind_mismatch() {
    let mut prior = project_with_tracks(vec![clip_track(ClipTrackFixture {
        kind: TrackKind::Video,
        track_id: TRACK_VIDEO_A,
        track_locked: false,
        clip_id: CLIP_VIDEO_A,
        clip_locked: false,
        clip_volume: 1.0,
        asset_id: ASSET_VIDEO_ID,
    })]);
    prior.assets.push(
        serde_json::from_value(video_asset_json(ASSET_VIDEO_ID, "video.mp4"))
            .expect("video asset parses"),
    );
    let mut args = args_clip(&format!("clip:{CLIP_VIDEO_A}"));
    args.gain = Some(0.5);

    let err = compute_patch(&prior, &args).expect_err("video clip rejected");
    assert!(matches!(
        err,
        AudioVolumeError::ClipKindMismatch {
            actual_kind: "video",
            ..
        }
    ));
}

#[test]
fn text_clip_errors_clip_kind_mismatch() {
    let prior = project_with_tracks(vec![clip_track(ClipTrackFixture {
        kind: TrackKind::Text,
        track_id: TRACK_TEXT_A,
        track_locked: false,
        clip_id: CLIP_TEXT_A,
        clip_locked: false,
        clip_volume: 1.0,
        asset_id: "00000000-0000-0000-0000-000000000000",
    })]);
    let mut args = args_clip(&format!("clip:{CLIP_TEXT_A}"));
    args.gain = Some(0.5);

    let err = compute_patch(&prior, &args).expect_err("text clip rejected");
    assert!(matches!(
        err,
        AudioVolumeError::ClipKindMismatch {
            actual_kind: "text",
            ..
        }
    ));
}

#[test]
fn image_clip_errors_clip_kind_mismatch() {
    let mut prior = project_with_tracks(vec![clip_track(ClipTrackFixture {
        kind: TrackKind::Video,
        track_id: TRACK_VIDEO_A,
        track_locked: false,
        clip_id: CLIP_IMAGE_A,
        clip_locked: false,
        clip_volume: 1.0,
        asset_id: ASSET_IMAGE_ID,
    })]);
    prior.assets.push(
        serde_json::from_value(image_asset_json(ASSET_IMAGE_ID, "image.png"))
            .expect("image asset parses"),
    );
    let mut args = args_clip(&format!("clip:{CLIP_IMAGE_A}"));
    args.gain = Some(0.5);

    let err = compute_patch(&prior, &args).expect_err("image clip rejected");
    // Image clips live on `kind: "video"` tracks; the audio-only guard
    // surfaces the parent track kind, matching audio_fade behavior.
    assert!(matches!(
        err,
        AudioVolumeError::ClipKindMismatch {
            actual_kind: "video",
            ..
        }
    ));
}

#[test]
fn video_track_errors_track_kind_mismatch() {
    let prior = project_with_tracks(vec![empty_track(
        TrackKind::Video,
        TRACK_VIDEO_A,
        false,
        1.0,
    )]);
    let mut args = args_clip(&format!("track:{TRACK_VIDEO_A}"));
    args.gain = Some(0.5);

    let err = compute_patch(&prior, &args).expect_err("video track rejected");
    assert!(matches!(
        err,
        AudioVolumeError::TrackKindMismatch {
            actual_kind: "video",
            ..
        }
    ));
}

#[test]
fn text_track_errors_track_kind_mismatch() {
    let prior = project_with_tracks(vec![empty_track(TrackKind::Text, TRACK_TEXT_A, false, 1.0)]);
    let mut args = args_clip(&format!("track:{TRACK_TEXT_A}"));
    args.gain = Some(0.5);

    let err = compute_patch(&prior, &args).expect_err("text track rejected");
    assert!(matches!(
        err,
        AudioVolumeError::TrackKindMismatch {
            actual_kind: "text",
            ..
        }
    ));
}

#[test]
fn effect_track_errors_track_kind_mismatch() {
    let prior = project_with_tracks(vec![empty_track(
        TrackKind::Effect,
        TRACK_EFFECT_A,
        false,
        1.0,
    )]);
    let mut args = args_clip(&format!("track:{TRACK_EFFECT_A}"));
    args.gain = Some(0.5);

    let err = compute_patch(&prior, &args).expect_err("effect track rejected");
    assert!(matches!(
        err,
        AudioVolumeError::TrackKindMismatch {
            actual_kind: "effect",
            ..
        }
    ));
}

#[test]
fn locked_clip_errors() {
    let prior = project_with_tracks(vec![audio_clip_track(false, true, 1.0)]);
    let mut args = args_clip(&format!("clip:{CLIP_AUDIO_A}"));
    args.gain = Some(0.5);

    let err = compute_patch(&prior, &args).expect_err("locked clip");
    assert!(matches!(err, AudioVolumeError::Locked { kind: "clip", .. }));
}

#[test]
fn locked_parent_track_errors_for_clip() {
    let prior = project_with_tracks(vec![audio_clip_track(true, false, 1.0)]);
    let mut args = args_clip(&format!("clip:{CLIP_AUDIO_A}"));
    args.gain = Some(0.5);

    let err = compute_patch(&prior, &args).expect_err("locked parent track");
    assert!(matches!(
        err,
        AudioVolumeError::Locked { kind: "track", .. }
    ));
}

#[test]
fn locked_target_track_errors() {
    let prior = project_with_tracks(vec![empty_track(
        TrackKind::Audio,
        TRACK_AUDIO_A,
        true,
        1.0,
    )]);
    let mut args = args_clip(&format!("track:{TRACK_AUDIO_A}"));
    args.gain = Some(0.5);

    let err = compute_patch(&prior, &args).expect_err("locked target track");
    assert!(matches!(
        err,
        AudioVolumeError::Locked { kind: "track", .. }
    ));
}

#[test]
fn happy_path_clip_with_gain() {
    let prior = project_with_audio_clip();
    let mut args = args_clip(&format!("clip:{CLIP_AUDIO_A}"));
    args.gain = Some(0.5);

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("clip + gain happy path");
    assert!((first_op_value(&patch) - 0.5).abs() < 1e-12);
    assert!(first_op_path(&patch).ends_with("/volume"));
    assert!(warnings.is_empty());
    assert_eq!(data.target_kind, AudioVolumeTargetKind::Clip);
    assert_eq!(data.target_id, CLIP_AUDIO_A);
    assert!((data.volume - 0.5).abs() < 1e-12);
}

#[test]
fn happy_path_clip_with_db() {
    let prior = project_with_audio_clip();
    let mut args = args_clip(&format!("clip:{CLIP_AUDIO_A}"));
    args.db = Some(-6.0);

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("clip + db happy path");
    let expected_linear = 10.0_f64.powf(-6.0 / 20.0);
    assert!((first_op_value(&patch) - expected_linear).abs() < 1e-12);
    assert!(warnings.is_empty());
    assert_eq!(data.target_kind, AudioVolumeTargetKind::Clip);
    assert!((data.volume - expected_linear).abs() < 1e-12);
}

#[test]
fn happy_path_track_with_gain() {
    let prior = project_with_tracks(vec![empty_track(
        TrackKind::Audio,
        TRACK_AUDIO_A,
        false,
        1.0,
    )]);
    let mut args = args_clip(&format!("track:{TRACK_AUDIO_A}"));
    args.gain = Some(2.0);

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("track + gain happy path");
    assert!((first_op_value(&patch) - 2.0).abs() < 1e-12);
    // Track patch path is `/tracks/<idx>/volume` (no `/clips/...`).
    assert!(first_op_path(&patch).starts_with("/tracks/"));
    assert!(!first_op_path(&patch).contains("/clips/"));
    assert!(warnings.is_empty());
    assert_eq!(data.target_kind, AudioVolumeTargetKind::Track);
    assert_eq!(data.target_id, TRACK_AUDIO_A);
    assert!((data.volume - 2.0).abs() < 1e-12);
}

#[test]
fn happy_path_track_with_db() {
    let prior = project_with_tracks(vec![empty_track(
        TrackKind::Audio,
        TRACK_AUDIO_A,
        false,
        1.0,
    )]);
    let mut args = args_clip(&format!("track:{TRACK_AUDIO_A}"));
    args.db = Some(6.0);

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("track + db happy path");
    let expected_linear = 10.0_f64.powf(6.0 / 20.0);
    assert!((first_op_value(&patch) - expected_linear).abs() < 1e-12);
    assert!(warnings.is_empty());
    assert_eq!(data.target_kind, AudioVolumeTargetKind::Track);
    assert!((data.volume - expected_linear).abs() < 1e-12);
}

#[test]
fn db_zero_resolves_to_unity_gain() {
    let prior = project_with_audio_clip();
    let mut args = args_clip(&format!("clip:{CLIP_AUDIO_A}"));
    args.db = Some(0.0);

    let (_, _, data) = compute_patch(&prior, &args).expect("db=0 ok");
    assert!((data.volume - 1.0).abs() < 1e-12, "10^(0/20) = 1.0");
}

#[test]
fn db_plus_six_resolves_near_1_995() {
    let prior = project_with_audio_clip();
    let mut args = args_clip(&format!("clip:{CLIP_AUDIO_A}"));
    args.db = Some(6.0);

    let (_, _, data) = compute_patch(&prior, &args).expect("db=+6 ok");
    let expected = 10.0_f64.powf(6.0 / 20.0);
    assert!((data.volume - expected).abs() < 1e-12);
    assert!((data.volume - 1.9952623149688795).abs() < 1e-9);
}

#[test]
fn db_minus_six_resolves_near_0_5012() {
    let prior = project_with_audio_clip();
    let mut args = args_clip(&format!("clip:{CLIP_AUDIO_A}"));
    args.db = Some(-6.0);

    let (_, _, data) = compute_patch(&prior, &args).expect("db=-6 ok");
    let expected = 10.0_f64.powf(-6.0 / 20.0);
    assert!((data.volume - expected).abs() < 1e-12);
    assert!((data.volume - 0.5011872336272722).abs() < 1e-9);
}

#[test]
fn db_minus_sixty_resolves_to_one_thousandth() {
    let prior = project_with_audio_clip();
    let mut args = args_clip(&format!("clip:{CLIP_AUDIO_A}"));
    args.db = Some(-60.0);

    let (_, _, data) = compute_patch(&prior, &args).expect("db=-60 ok");
    let expected = 10.0_f64.powf(-60.0 / 20.0);
    assert!((data.volume - expected).abs() < 1e-12);
    assert!((data.volume - 0.001).abs() < 1e-9);
}

#[test]
fn db_plus_twelve_resolves_near_3_981() {
    let prior = project_with_audio_clip();
    let mut args = args_clip(&format!("clip:{CLIP_AUDIO_A}"));
    args.db = Some(12.0);

    let (_, _, data) = compute_patch(&prior, &args).expect("db=+12 ok");
    let expected = 10.0_f64.powf(12.0 / 20.0);
    assert!((data.volume - expected).abs() < 1e-12);
    assert!((data.volume - 3.9810717055349722).abs() < 1e-9);
}

#[test]
fn gain_boundary_zero_accepted() {
    let prior = project_with_audio_clip();
    let mut args = args_clip(&format!("clip:{CLIP_AUDIO_A}"));
    args.gain = Some(0.0);

    let (_, _, data) = compute_patch(&prior, &args).expect("gain=0 accepted");
    assert!(data.volume.abs() < 1e-12);
}

#[test]
fn gain_boundary_four_accepted() {
    let prior = project_with_audio_clip();
    let mut args = args_clip(&format!("clip:{CLIP_AUDIO_A}"));
    args.gain = Some(4.0);

    let (_, _, data) = compute_patch(&prior, &args).expect("gain=4 accepted");
    assert!((data.volume - 4.0).abs() < 1e-12);
}

#[test]
fn data_envelope_round_trip_clip() {
    let prior = project_with_audio_clip();
    let mut args = args_clip(&format!("clip:{CLIP_AUDIO_A}"));
    args.gain = Some(0.25);

    let (patch_value, _warnings, data) =
        compute_patch(&prior, &args).expect("compute_patch happy path");
    let patch: json_patch::Patch =
        serde_json::from_value(patch_value).expect("patch is valid RFC 6902");
    let post_state = prior.apply(&patch).expect("apply audio.volume patch");

    let reconstructed: AudioVolumeData =
        data_envelope_from_post_state(&args, &post_state).expect("reconstruct from post-state");
    assert_eq!(reconstructed, data);
}

#[test]
fn data_envelope_round_trip_track() {
    let prior = project_with_tracks(vec![empty_track(
        TrackKind::Audio,
        TRACK_AUDIO_A,
        false,
        1.0,
    )]);
    let mut args = args_clip(&format!("track:{TRACK_AUDIO_A}"));
    args.db = Some(-3.0);

    let (patch_value, _warnings, data) =
        compute_patch(&prior, &args).expect("compute_patch happy path");
    let patch: json_patch::Patch =
        serde_json::from_value(patch_value).expect("patch is valid RFC 6902");
    let post_state = prior.apply(&patch).expect("apply audio.volume patch");

    let reconstructed: AudioVolumeData =
        data_envelope_from_post_state(&args, &post_state).expect("reconstruct from post-state");
    assert_eq!(reconstructed, data);
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "audio.volume")
        .expect("default_fixtures includes audio.volume");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(AudioVolumeVerb))
        .expect("register audio.volume verb");

    let report = validate_reconstructors(&registry, &[fixture])
        .expect("audio.volume reconstruct from fixture");
    assert_eq!(report.verbs_checked, vec!["audio.volume"]);
    assert_eq!(report.fixtures_run, 1);
}

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        project_with_audio_clip(),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry clears the gate and writes project.json");

    let outcome = store
        .mutate_via_verb(
            "audio.volume",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "target": format!("clip:{CLIP_AUDIO_A}"),
                "db": -6.0,
            }),
            None,
        )
        .expect("mutate_via_verb happy path");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("happy path must return Applied, got {outcome:?}");
    };

    let data: AudioVolumeData =
        serde_json::from_value(data).expect("audio.volume data is AudioVolumeData");
    assert_eq!(data.target_kind, AudioVolumeTargetKind::Clip);
    assert_eq!(data.target_id, CLIP_AUDIO_A);
    let expected = 10.0_f64.powf(-6.0 / 20.0);
    assert!((data.volume - expected).abs() < 1e-12);
    assert!(
        (store.project().tracks[0].clips[0].volume - expected).abs() < 1e-12,
        "post-state clip volume matches resolved linear gain"
    );
    assert!(warnings.is_empty());
}
