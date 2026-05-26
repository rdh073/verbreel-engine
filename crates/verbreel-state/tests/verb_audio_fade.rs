//! Tests for `audio.fade` (§9.3) — fifty-ninth production verb.

use serde_json::{Value, json};
use std::sync::Arc;

use verbreel_state::verbs::audio_fade::{
    CURVE_COMBO_HINT, compute_patch, data_envelope_from_post_state,
};
use verbreel_state::{
    AudioFadeArgs, AudioFadeData, AudioFadeError, AudioFadeVerb, FadeCurve, MutateOutcome, Project,
    Track, TrackKind, VerbRegistry, default_fixtures, default_registry, validate_reconstructors,
};
use verbreel_types::{ProjectId, Tick};

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const TRACK_AUDIO_A: &str = "0190b8d3-15e3-7000-bd00-0000000aa901";
const TRACK_VIDEO_A: &str = "0190b8d3-15e3-7000-bd00-0000000aa902";
const TRACK_TEXT_A: &str = "0190b8d3-15e3-7000-bd00-0000000aa903";
const CLIP_AUDIO_A: &str = "0190b8d3-15e3-7000-bd00-0000000bb901";
const CLIP_VIDEO_A: &str = "0190b8d3-15e3-7000-bd00-0000000bb902";
const CLIP_TEXT_A: &str = "0190b8d3-15e3-7000-bd00-0000000bb903";
const CLIP_IMAGE_A: &str = "0190b8d3-15e3-7000-bd00-0000000bb904";
const MISSING_CLIP: &str = "0190b8d3-15e3-7000-bd00-0000000cc999";
const ASSET_AUDIO_ID: &str = "0190b8d3-15e3-7000-bd00-0000000dd901";
const ASSET_VIDEO_ID: &str = "0190b8d3-15e3-7000-bd00-0000000dd902";
const ASSET_IMAGE_ID: &str = "0190b8d3-15e3-7000-bd00-0000000dd904";

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
    asset_id: &'a str,
    fade_in_tk: i64,
    fade_out_tk: i64,
    fade_in_curve: FadeCurve,
    fade_out_curve: FadeCurve,
}

fn clip_track(fixture: ClipTrackFixture<'_>) -> Track {
    let text = matches!(fixture.kind, TrackKind::Text).then(|| {
        json!({
            "content": "Fade",
            "font_family": "Arial",
            "font_size_px": 24,
        })
    });

    let mut clip = json!({
        "id": fixture.clip_id,
        "name": "Fade Clip",
        "asset_id": fixture.asset_id,
        "track_position_tk": 0,
        "source_in_tk": 0,
        "source_out_tk": 240_000,
        "fade_in_tk": fixture.fade_in_tk,
        "fade_out_tk": fixture.fade_out_tk,
        "fade_in_curve": fixture.fade_in_curve,
        "fade_out_curve": fixture.fade_out_curve,
        "locked": fixture.clip_locked,
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

fn project_with_tracks(tracks: Vec<Track>) -> Project {
    let mut project = empty_project();
    project.tracks = tracks;
    project.duration_tk = Tick::new(240_000);
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

fn audio_track(
    track_locked: bool,
    clip_locked: bool,
    fade_in_tk: i64,
    fade_out_tk: i64,
    fade_in_curve: FadeCurve,
    fade_out_curve: FadeCurve,
) -> Track {
    clip_track(ClipTrackFixture {
        kind: TrackKind::Audio,
        track_id: TRACK_AUDIO_A,
        track_locked,
        clip_id: CLIP_AUDIO_A,
        clip_locked,
        asset_id: ASSET_AUDIO_ID,
        fade_in_tk,
        fade_out_tk,
        fade_in_curve,
        fade_out_curve,
    })
}

fn project_with_audio_asset() -> Project {
    let mut project = project_with_tracks(vec![audio_track(
        false,
        false,
        0,
        0,
        FadeCurve::Linear,
        FadeCurve::Linear,
    )]);
    project.assets.push(
        serde_json::from_value(audio_asset_json(ASSET_AUDIO_ID, "audio-fade.m4a"))
            .expect("audio asset parses"),
    );
    project
}

fn args_for(clip: &str) -> AudioFadeArgs {
    AudioFadeArgs {
        project_id: fixture_project_id(),
        clip: clip.to_string(),
        fade_in_tk: None,
        fade_out_tk: None,
        curve: None,
        curve_in: None,
        curve_out: None,
    }
}

fn patch_values(patch: &Value) -> (i64, i64, String, String) {
    let arr = patch.as_array().expect("patch is array");
    assert_eq!(arr.len(), 4, "audio.fade emits four replace ops");
    (
        arr[0]["value"].as_i64().expect("fade_in_tk is int"),
        arr[1]["value"].as_i64().expect("fade_out_tk is int"),
        arr[2]["value"]
            .as_str()
            .expect("fade_in_curve is str")
            .to_string(),
        arr[3]["value"]
            .as_str()
            .expect("fade_out_curve is str")
            .to_string(),
    )
}

#[test]
fn args_deserialize_round_trip() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "clip": CLIP_AUDIO_A,
        "fade_in_tk": 1_000,
        "fade_out_tk": 2_000,
        "curve_in": "exp",
        "curve_out": "log",
    });
    let args: AudioFadeArgs = serde_json::from_value(raw).expect("args deserialize");
    assert_eq!(args.clip, CLIP_AUDIO_A);
    assert_eq!(args.fade_in_tk, Some(1_000));
    assert_eq!(args.fade_out_tk, Some(2_000));
    assert_eq!(args.curve, None);
    assert_eq!(args.curve_in, Some(FadeCurve::Exp));
    assert_eq!(args.curve_out, Some(FadeCurve::Log));
}

#[test]
fn args_missing_clip_field_errors() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "fade_in_tk": 1_000,
    });
    let err = serde_json::from_value::<AudioFadeArgs>(raw).expect_err("missing `clip` must reject");
    let msg = err.to_string();
    assert!(msg.contains("clip"), "error mentions missing field: {msg}");
}

#[test]
fn args_wrong_type_errors() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "clip": 42,
    });
    let err =
        serde_json::from_value::<AudioFadeArgs>(raw).expect_err("non-string `clip` must reject");
    let msg = err.to_string();
    assert!(
        msg.contains("string") || msg.contains("clip"),
        "error mentions string-shape rejection: {msg}"
    );
}

#[test]
fn compute_patch_bad_selector_errors() {
    let prior = project_with_audio_asset();
    let mut args = args_for("not-a-uuid");
    args.fade_in_tk = Some(8_000);

    let err = compute_patch(&prior, &args).expect_err("bad selector");
    assert!(matches!(err, AudioFadeError::BadSelector { .. }));
}

#[test]
fn compute_patch_missing_clip_errors_not_found() {
    let prior = project_with_audio_asset();
    let mut args = args_for(MISSING_CLIP);
    args.fade_in_tk = Some(8_000);

    let err = compute_patch(&prior, &args).expect_err("missing clip");
    assert!(matches!(err, AudioFadeError::NotFound { .. }));
}

#[test]
fn compute_patch_qualified_selector_no_match_errors_no_match() {
    let prior = project_with_audio_asset();
    let mut args = args_for(&format!("clip:{MISSING_CLIP}"));
    args.fade_in_tk = Some(8_000);

    let err = compute_patch(&prior, &args).expect_err("qualified selector no match");
    assert!(matches!(err, AudioFadeError::NoMatch { .. }));
}

#[test]
fn compute_patch_qualified_track_prefix_errors_selector_kind_mismatch() {
    let prior = project_with_audio_asset();
    let mut args = args_for(&format!("track:{TRACK_AUDIO_A}"));
    args.fade_in_tk = Some(8_000);

    let err = compute_patch(&prior, &args).expect_err("track selector rejected");
    assert!(matches!(
        err,
        AudioFadeError::SelectorKindMismatch { ref actual_prefix } if actual_prefix == "track"
    ));
}

#[test]
fn compute_patch_video_clip_errors_clip_kind_mismatch() {
    let mut prior = project_with_tracks(vec![clip_track(ClipTrackFixture {
        kind: TrackKind::Video,
        track_id: TRACK_VIDEO_A,
        track_locked: false,
        clip_id: CLIP_VIDEO_A,
        clip_locked: false,
        asset_id: ASSET_VIDEO_ID,
        fade_in_tk: 0,
        fade_out_tk: 0,
        fade_in_curve: FadeCurve::Linear,
        fade_out_curve: FadeCurve::Linear,
    })]);
    prior.assets.push(
        serde_json::from_value(video_asset_json(ASSET_VIDEO_ID, "video.mp4"))
            .expect("video asset parses"),
    );
    let mut args = args_for(CLIP_VIDEO_A);
    args.fade_in_tk = Some(8_000);

    let err = compute_patch(&prior, &args).expect_err("video clip rejected");
    assert!(matches!(
        err,
        AudioFadeError::ClipKindMismatch {
            actual_kind: "video",
            ..
        }
    ));
}

#[test]
fn compute_patch_text_clip_errors_clip_kind_mismatch() {
    let prior = project_with_tracks(vec![clip_track(ClipTrackFixture {
        kind: TrackKind::Text,
        track_id: TRACK_TEXT_A,
        track_locked: false,
        clip_id: CLIP_TEXT_A,
        clip_locked: false,
        asset_id: "00000000-0000-0000-0000-000000000000",
        fade_in_tk: 0,
        fade_out_tk: 0,
        fade_in_curve: FadeCurve::Linear,
        fade_out_curve: FadeCurve::Linear,
    })]);
    let mut args = args_for(CLIP_TEXT_A);
    args.fade_in_tk = Some(8_000);

    let err = compute_patch(&prior, &args).expect_err("text clip rejected");
    assert!(matches!(
        err,
        AudioFadeError::ClipKindMismatch {
            actual_kind: "text",
            ..
        }
    ));
}

#[test]
fn compute_patch_image_clip_errors_clip_kind_mismatch() {
    let mut prior = project_with_tracks(vec![clip_track(ClipTrackFixture {
        kind: TrackKind::Video,
        track_id: TRACK_VIDEO_A,
        track_locked: false,
        clip_id: CLIP_IMAGE_A,
        clip_locked: false,
        asset_id: ASSET_IMAGE_ID,
        fade_in_tk: 0,
        fade_out_tk: 0,
        fade_in_curve: FadeCurve::Linear,
        fade_out_curve: FadeCurve::Linear,
    })]);
    prior.assets.push(
        serde_json::from_value(image_asset_json(ASSET_IMAGE_ID, "image.png"))
            .expect("image asset parses"),
    );
    let mut args = args_for(CLIP_IMAGE_A);
    args.fade_in_tk = Some(8_000);

    // Image clips live on `kind: "video"` tracks but use image assets;
    // audio.fade still rejects them since the track kind is non-audio.
    let err = compute_patch(&prior, &args).expect_err("image clip rejected");
    assert!(matches!(
        err,
        AudioFadeError::ClipKindMismatch {
            actual_kind: "video",
            ..
        }
    ));
}

#[test]
fn compute_patch_negative_fade_in_errors() {
    let prior = project_with_audio_asset();
    let mut args = args_for(CLIP_AUDIO_A);
    args.fade_in_tk = Some(-1);

    let err = compute_patch(&prior, &args).expect_err("negative fade in");
    assert!(matches!(
        err,
        AudioFadeError::BadTime {
            field: "fade_in_tk",
            ..
        }
    ));
}

#[test]
fn compute_patch_negative_fade_out_errors() {
    let prior = project_with_audio_asset();
    let mut args = args_for(CLIP_AUDIO_A);
    args.fade_out_tk = Some(-1);

    let err = compute_patch(&prior, &args).expect_err("negative fade out");
    assert!(matches!(
        err,
        AudioFadeError::BadTime {
            field: "fade_out_tk",
            ..
        }
    ));
}

#[test]
fn compute_patch_fade_in_exceeds_duration_errors() {
    let prior = project_with_audio_asset();
    let mut args = args_for(CLIP_AUDIO_A);
    args.fade_in_tk = Some(240_001);

    let err = compute_patch(&prior, &args).expect_err("fade_in over duration");
    assert!(matches!(err, AudioFadeError::BadRange { .. }));
}

#[test]
fn compute_patch_fade_out_exceeds_duration_errors() {
    let prior = project_with_audio_asset();
    let mut args = args_for(CLIP_AUDIO_A);
    args.fade_out_tk = Some(240_001);

    let err = compute_patch(&prior, &args).expect_err("fade_out over duration");
    assert!(matches!(err, AudioFadeError::BadRange { .. }));
}

#[test]
fn compute_patch_curve_with_curve_in_errors_args_incompatible() {
    let prior = project_with_audio_asset();
    let mut args = args_for(CLIP_AUDIO_A);
    args.curve = Some(FadeCurve::Exp);
    args.curve_in = Some(FadeCurve::Log);

    let err = compute_patch(&prior, &args).expect_err("curve + curve_in");
    let AudioFadeError::ArgsIncompatible { hint, .. } = err else {
        panic!("expected ArgsIncompatible, got {err:?}");
    };
    assert_eq!(hint, CURVE_COMBO_HINT);
}

#[test]
fn compute_patch_curve_with_curve_out_errors_args_incompatible() {
    let prior = project_with_audio_asset();
    let mut args = args_for(CLIP_AUDIO_A);
    args.curve = Some(FadeCurve::Exp);
    args.curve_out = Some(FadeCurve::Log);

    let err = compute_patch(&prior, &args).expect_err("curve + curve_out");
    let AudioFadeError::ArgsIncompatible { hint, .. } = err else {
        panic!("expected ArgsIncompatible, got {err:?}");
    };
    assert_eq!(hint, CURVE_COMBO_HINT);
}

#[test]
fn compute_patch_zero_fade_fields_errors_args_incompatible() {
    let prior = project_with_audio_asset();
    let args = args_for(CLIP_AUDIO_A);

    let err = compute_patch(&prior, &args).expect_err("empty fade-update rejected");
    assert!(matches!(err, AudioFadeError::ArgsIncompatible { .. }));
}

#[test]
fn compute_patch_locked_clip_errors() {
    let prior = project_with_tracks(vec![audio_track(
        false,
        true,
        0,
        0,
        FadeCurve::Linear,
        FadeCurve::Linear,
    )]);
    let mut args = args_for(CLIP_AUDIO_A);
    args.fade_in_tk = Some(8_000);

    let err = compute_patch(&prior, &args).expect_err("locked clip");
    assert!(matches!(err, AudioFadeError::Locked { kind: "clip", .. }));
}

#[test]
fn compute_patch_locked_track_errors() {
    let prior = project_with_tracks(vec![audio_track(
        true,
        false,
        0,
        0,
        FadeCurve::Linear,
        FadeCurve::Linear,
    )]);
    let mut args = args_for(CLIP_AUDIO_A);
    args.fade_in_tk = Some(8_000);

    let err = compute_patch(&prior, &args).expect_err("locked track");
    assert!(matches!(err, AudioFadeError::Locked { kind: "track", .. }));
}

#[test]
fn compute_patch_fade_in_only_writes_field() {
    let prior = project_with_audio_asset();
    let mut args = args_for(CLIP_AUDIO_A);
    args.fade_in_tk = Some(8_001);

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("fade_in only");
    assert_eq!(
        patch_values(&patch),
        (8_001, 0, "linear".into(), "linear".into())
    );
    assert!(warnings.is_empty());
    assert_eq!(data.fade_in_tk, 8_001);
    assert_eq!(data.fade_out_tk, 0);
    assert_eq!(data.fade_in_curve, FadeCurve::Linear);
}

#[test]
fn compute_patch_fade_out_only_writes_field() {
    let prior = project_with_audio_asset();
    let mut args = args_for(CLIP_AUDIO_A);
    args.fade_out_tk = Some(16_001);

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("fade_out only");
    assert_eq!(
        patch_values(&patch),
        (0, 16_001, "linear".into(), "linear".into())
    );
    assert!(warnings.is_empty());
    assert_eq!(data.fade_out_tk, 16_001);
}

#[test]
fn compute_patch_both_tk_writes_both_fields() {
    let prior = project_with_audio_asset();
    let mut args = args_for(CLIP_AUDIO_A);
    args.fade_in_tk = Some(8_001);
    args.fade_out_tk = Some(16_001);

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("both tk");
    assert_eq!(
        patch_values(&patch),
        (8_001, 16_001, "linear".into(), "linear".into())
    );
    assert!(warnings.is_empty());
    assert_eq!(data.fade_in_tk, 8_001);
    assert_eq!(data.fade_out_tk, 16_001);
}

#[test]
fn compute_patch_curve_convenience_writes_both_curves() {
    let prior = project_with_audio_asset();
    let mut args = args_for(CLIP_AUDIO_A);
    args.curve = Some(FadeCurve::Exp);

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("curve convenience");
    assert_eq!(patch_values(&patch).2, "exp");
    assert_eq!(patch_values(&patch).3, "exp");
    assert!(warnings.is_empty());
    assert_eq!(data.fade_in_curve, FadeCurve::Exp);
    assert_eq!(data.fade_out_curve, FadeCurve::Exp);
}

#[test]
fn compute_patch_curve_in_only_leaves_curve_out_unchanged() {
    let prior = project_with_tracks(vec![audio_track(
        false,
        false,
        0,
        0,
        FadeCurve::Linear,
        FadeCurve::Log,
    )]);
    let mut args = args_for(CLIP_AUDIO_A);
    args.curve_in = Some(FadeCurve::Exp);

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("curve_in only");
    assert_eq!(patch_values(&patch).2, "exp");
    assert_eq!(patch_values(&patch).3, "log");
    assert!(warnings.is_empty());
    assert_eq!(data.fade_in_curve, FadeCurve::Exp);
    assert_eq!(data.fade_out_curve, FadeCurve::Log);
}

#[test]
fn compute_patch_curve_out_only_leaves_curve_in_unchanged() {
    let prior = project_with_tracks(vec![audio_track(
        false,
        false,
        0,
        0,
        FadeCurve::Exp,
        FadeCurve::Linear,
    )]);
    let mut args = args_for(CLIP_AUDIO_A);
    args.curve_out = Some(FadeCurve::Log);

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("curve_out only");
    assert_eq!(patch_values(&patch).2, "exp");
    assert_eq!(patch_values(&patch).3, "log");
    assert!(warnings.is_empty());
    assert_eq!(data.fade_in_curve, FadeCurve::Exp);
    assert_eq!(data.fade_out_curve, FadeCurve::Log);
}

#[test]
fn compute_patch_curve_in_and_curve_out_combo_writes_both() {
    let prior = project_with_audio_asset();
    let mut args = args_for(CLIP_AUDIO_A);
    args.curve_in = Some(FadeCurve::Exp);
    args.curve_out = Some(FadeCurve::Log);

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("curve_in + curve_out");
    assert_eq!(patch_values(&patch).2, "exp");
    assert_eq!(patch_values(&patch).3, "log");
    assert!(warnings.is_empty());
    assert_eq!(data.fade_in_curve, FadeCurve::Exp);
    assert_eq!(data.fade_out_curve, FadeCurve::Log);
}

#[test]
fn compute_patch_qualified_selector_resolves_audio_clip() {
    let prior = project_with_audio_asset();
    let mut args = args_for(&format!("clip:{CLIP_AUDIO_A}"));
    args.fade_in_tk = Some(8_001);

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("qualified selector");
    assert_eq!(patch_values(&patch).0, 8_001);
    assert!(warnings.is_empty());
    assert_eq!(data.clip_id.to_string(), CLIP_AUDIO_A);
}

#[test]
fn compute_patch_audio_does_not_snap_off_frame_fades() {
    let prior = project_with_audio_asset();
    let mut args = args_for(CLIP_AUDIO_A);
    args.fade_in_tk = Some(8_001);
    args.fade_out_tk = Some(8_001);

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("audio off-frame ok");
    assert_eq!(patch_values(&patch).0, 8_001);
    assert_eq!(patch_values(&patch).1, 8_001);
    assert!(warnings.is_empty(), "audio path does not snap");
    assert_eq!(data.fade_in_tk, 8_001);
    assert_eq!(data.fade_out_tk, 8_001);
}

#[test]
fn data_envelope_from_post_state_returns_post_fade_state() {
    let post_state = project_with_tracks(vec![audio_track(
        false,
        false,
        8_001,
        16_001,
        FadeCurve::Exp,
        FadeCurve::Log,
    )]);
    let mut args = args_for(CLIP_AUDIO_A);
    args.fade_in_tk = Some(24_000);

    let data = data_envelope_from_post_state(&args, &post_state)
        .expect("post-state envelope should be readable");
    assert_eq!(data.clip_id.to_string(), CLIP_AUDIO_A);
    assert_eq!(data.fade_in_tk, 8_001);
    assert_eq!(data.fade_out_tk, 16_001);
    assert_eq!(data.fade_in_curve, FadeCurve::Exp);
    assert_eq!(data.fade_out_curve, FadeCurve::Log);
}

#[test]
fn reconstruct_round_trip_via_compute_patch_then_apply() {
    let prior = project_with_audio_asset();
    let mut args = args_for(CLIP_AUDIO_A);
    args.fade_in_tk = Some(8_001);
    args.fade_out_tk = Some(16_001);
    args.curve_in = Some(FadeCurve::Exp);
    args.curve_out = Some(FadeCurve::Log);

    let (patch_value, _warnings, data) =
        compute_patch(&prior, &args).expect("compute_patch happy path");
    let patch: json_patch::Patch =
        serde_json::from_value(patch_value).expect("patch is valid RFC 6902");
    let post_state = prior.apply(&patch).expect("apply audio.fade patch");

    let reconstructed: AudioFadeData =
        data_envelope_from_post_state(&args, &post_state).expect("reconstruct from post-state");
    assert_eq!(reconstructed, data);
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "audio.fade")
        .expect("default_fixtures includes audio.fade");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(AudioFadeVerb))
        .expect("register audio.fade verb");

    let report = validate_reconstructors(&registry, &[fixture])
        .expect("audio.fade reconstruct from fixture");
    assert_eq!(report.verbs_checked, vec!["audio.fade"]);
    assert_eq!(report.fixtures_run, 1);
}

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb() {
    use tempfile::TempDir;

    let mut prior = project_with_audio_asset();
    prior.assets.push(
        serde_json::from_value(audio_asset_json(ASSET_AUDIO_ID, "audio-fade.m4a"))
            .expect("audio asset parses"),
    );
    // dedup the duplicate — project_with_audio_asset already pushed once.
    prior.assets.dedup_by(|a, b| a.id() == b.id());

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        project_with_audio_asset(),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry clears the gate and writes project.json");

    let outcome = store
        .mutate_via_verb(
            "audio.fade",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "clip": CLIP_AUDIO_A,
                "fade_in_tk": 8_001,
                "curve_out": "log",
            }),
            None,
        )
        .expect("mutate_via_verb happy path");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("happy path must return Applied, got {outcome:?}");
    };

    let data: AudioFadeData =
        serde_json::from_value(data).expect("audio.fade data is AudioFadeData");
    assert_eq!(data.clip_id.to_string(), CLIP_AUDIO_A);
    assert_eq!(data.fade_in_tk, 8_001);
    assert_eq!(data.fade_out_curve, FadeCurve::Log);
    assert_eq!(
        store.project().tracks[0].clips[0].fade_in_tk,
        Tick::new(8_001)
    );
    assert!(warnings.is_empty());
}
