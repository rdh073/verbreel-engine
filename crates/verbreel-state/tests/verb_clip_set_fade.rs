//! Tests for `clip.set_fade` (§5.12) — fortieth production verb.

use serde_json::{Value, json};
use std::sync::Arc;

use verbreel_state::verbs::clip_set_fade::{
    W_NOOP_CODE, W_TIME_SNAPPED_CODE, compute_patch, data_envelope_from_post_state,
};
use verbreel_state::{
    ClipSetFadeArgs, ClipSetFadeData, ClipSetFadeError, ClipSetFadeVerb, FadeCurve, MutateOutcome,
    Project, Track, TrackKind, VerbRegistry, default_fixtures, default_registry,
    validate_reconstructors,
};
use verbreel_types::{ProjectId, Tick};

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const TRACK_TEXT_A: &str = "0190b8d3-15e3-7000-bd00-0000000aa101";
const TRACK_VIDEO_A: &str = "0190b8d3-15e3-7000-bd00-0000000aa201";
const TRACK_AUDIO_A: &str = "0190b8d3-15e3-7000-bd00-0000000aa301";
const CLIP_TEXT_A: &str = "0190b8d3-15e3-7000-bd00-0000000bb101";
const CLIP_VIDEO_A: &str = "0190b8d3-15e3-7000-bd00-0000000bb201";
const CLIP_AUDIO_A: &str = "0190b8d3-15e3-7000-bd00-0000000bb301";
const MISSING_CLIP: &str = "0190b8d3-15e3-7000-bd00-0000000cc101";
const ASSET_VIDEO_ID: &str = "0190b8d3-15e3-7000-bd00-0000000dd101";
const ASSET_AUDIO_ID: &str = "0190b8d3-15e3-7000-bd00-0000000dd102";
const ASSET_IMAGE_ID: &str = "0190b8d3-15e3-7000-bd00-0000000dd103";

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
    fade_in_tk: i64,
    fade_out_tk: i64,
    fade_in_curve: FadeCurve,
    fade_out_curve: FadeCurve,
}

fn clip_track(fixture: ClipTrackFixture<'_>) -> Track {
    let (asset_id, text) = match fixture.kind {
        TrackKind::Text => (
            "00000000-0000-0000-0000-000000000000",
            Some(json!({
                "content": "Fade",
                "font_family": "Arial",
                "font_size_px": 24,
            })),
        ),
        TrackKind::Audio => (ASSET_AUDIO_ID, None),
        TrackKind::Video | TrackKind::Effect => (ASSET_VIDEO_ID, None),
    };

    let mut clip = json!({
        "id": fixture.clip_id,
        "name": "Fade Clip",
        "asset_id": asset_id,
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

fn text_track(
    track_locked: bool,
    clip_locked: bool,
    fade_in_tk: i64,
    fade_out_tk: i64,
    fade_in_curve: FadeCurve,
    fade_out_curve: FadeCurve,
) -> Track {
    clip_track(ClipTrackFixture {
        kind: TrackKind::Text,
        track_id: TRACK_TEXT_A,
        track_locked,
        clip_id: CLIP_TEXT_A,
        clip_locked,
        fade_in_tk,
        fade_out_tk,
        fade_in_curve,
        fade_out_curve,
    })
}

fn project_with_tracks(tracks: Vec<Track>) -> Project {
    let mut project = empty_project();
    project.tracks = tracks;
    project.duration_tk = Tick::new(240_000);
    project
}

fn project_with_audio_asset() -> Project {
    let mut project = project_with_tracks(vec![clip_track(ClipTrackFixture {
        kind: TrackKind::Audio,
        track_id: TRACK_AUDIO_A,
        track_locked: false,
        clip_id: CLIP_AUDIO_A,
        clip_locked: false,
        fade_in_tk: 0,
        fade_out_tk: 0,
        fade_in_curve: FadeCurve::Linear,
        fade_out_curve: FadeCurve::Linear,
    })]);
    project.assets.push(
        serde_json::from_value(json!({
            "id": ASSET_AUDIO_ID,
            "kind": "audio",
            "hash": "36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da",
            "path": "assets/36/36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da.m4a",
            "original_filename": "audio.m4a",
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
        }))
        .expect("audio asset parses"),
    );
    project
}

fn args_for(clip: &str) -> ClipSetFadeArgs {
    ClipSetFadeArgs {
        project_id: fixture_project_id(),
        clip: clip.to_string(),
        fade_in_tk: None,
        fade_out_tk: None,
        fade_in_curve: None,
        fade_out_curve: None,
    }
}

fn patch_values(patch: &Value) -> (i64, i64, String, String) {
    let arr = patch.as_array().expect("patch is array");
    assert_eq!(arr.len(), 4, "clip.set_fade emits four replace ops");
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
fn compute_patch_fade_in_only() {
    let prior = project_with_tracks(vec![text_track(
        false,
        false,
        0,
        0,
        FadeCurve::Linear,
        FadeCurve::Linear,
    )]);
    let mut args = args_for(CLIP_TEXT_A);
    args.fade_in_tk = Some(8_000);

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("fade in only");
    assert_eq!(
        patch_values(&patch),
        (8_000, 0, "linear".into(), "linear".into())
    );
    assert!(warnings.is_empty());
    assert_eq!(data.fade_in_tk, 8_000);
}

#[test]
fn compute_patch_fade_out_only() {
    let prior = project_with_tracks(vec![text_track(
        false,
        false,
        0,
        0,
        FadeCurve::Linear,
        FadeCurve::Linear,
    )]);
    let mut args = args_for(CLIP_TEXT_A);
    args.fade_out_tk = Some(16_000);

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("fade out only");
    assert_eq!(
        patch_values(&patch),
        (0, 16_000, "linear".into(), "linear".into())
    );
    assert!(warnings.is_empty());
    assert_eq!(data.fade_out_tk, 16_000);
}

#[test]
fn compute_patch_curves_only() {
    let prior = project_with_tracks(vec![text_track(
        false,
        false,
        8_000,
        16_000,
        FadeCurve::Linear,
        FadeCurve::Linear,
    )]);
    let mut args = args_for(CLIP_TEXT_A);
    args.fade_in_curve = Some(FadeCurve::Exp);
    args.fade_out_curve = Some(FadeCurve::Log);

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("curves only");
    assert_eq!(
        patch_values(&patch),
        (8_000, 16_000, "exp".into(), "log".into())
    );
    assert!(warnings.is_empty());
    assert_eq!(data.fade_in_curve, FadeCurve::Exp);
    assert_eq!(data.fade_out_curve, FadeCurve::Log);
}

#[test]
fn compute_patch_all_four_fields() {
    let prior = project_with_tracks(vec![text_track(
        false,
        false,
        0,
        0,
        FadeCurve::Linear,
        FadeCurve::Linear,
    )]);
    let mut args = args_for(CLIP_TEXT_A);
    args.fade_in_tk = Some(8_000);
    args.fade_out_tk = Some(16_000);
    args.fade_in_curve = Some(FadeCurve::Exp);
    args.fade_out_curve = Some(FadeCurve::Log);

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("all four fields");
    assert_eq!(
        patch_values(&patch),
        (8_000, 16_000, "exp".into(), "log".into())
    );
    assert!(warnings.is_empty());
    assert_eq!(data.clip_id.to_string(), CLIP_TEXT_A);
}

#[test]
fn compute_patch_missing_clip_errors() {
    let prior = project_with_tracks(vec![text_track(
        false,
        false,
        0,
        0,
        FadeCurve::Linear,
        FadeCurve::Linear,
    )]);
    let mut args = args_for(MISSING_CLIP);
    args.fade_in_tk = Some(8_000);

    let err = compute_patch(&prior, &args).expect_err("missing clip");
    assert!(matches!(err, ClipSetFadeError::ClipNotFound { .. }));
}

#[test]
fn compute_patch_bad_selector_errors() {
    let prior = project_with_tracks(vec![text_track(
        false,
        false,
        0,
        0,
        FadeCurve::Linear,
        FadeCurve::Linear,
    )]);
    let mut args = args_for("not-a-uuid");
    args.fade_in_tk = Some(8_000);

    let err = compute_patch(&prior, &args).expect_err("bad selector");
    assert!(matches!(err, ClipSetFadeError::BadSelector { .. }));
}

#[test]
fn compute_patch_locked_clip_errors() {
    let prior = project_with_tracks(vec![text_track(
        false,
        true,
        0,
        0,
        FadeCurve::Linear,
        FadeCurve::Linear,
    )]);
    let mut args = args_for(CLIP_TEXT_A);
    args.fade_in_tk = Some(8_000);

    let err = compute_patch(&prior, &args).expect_err("locked clip");
    assert!(matches!(err, ClipSetFadeError::Locked { kind: "clip", .. }));
}

#[test]
fn compute_patch_locked_track_errors() {
    let prior = project_with_tracks(vec![text_track(
        true,
        false,
        0,
        0,
        FadeCurve::Linear,
        FadeCurve::Linear,
    )]);
    let mut args = args_for(CLIP_TEXT_A);
    args.fade_in_tk = Some(8_000);

    let err = compute_patch(&prior, &args).expect_err("locked track");
    assert!(matches!(
        err,
        ClipSetFadeError::Locked { kind: "track", .. }
    ));
}

#[test]
fn compute_patch_negative_fade_in_errors() {
    let prior = project_with_tracks(vec![text_track(
        false,
        false,
        0,
        0,
        FadeCurve::Linear,
        FadeCurve::Linear,
    )]);
    let mut args = args_for(CLIP_TEXT_A);
    args.fade_in_tk = Some(-1);

    let err = compute_patch(&prior, &args).expect_err("negative fade in");
    assert!(matches!(
        err,
        ClipSetFadeError::BadTime {
            field: "fade_in_tk",
            ..
        }
    ));
}

#[test]
fn compute_patch_negative_fade_out_errors() {
    let prior = project_with_tracks(vec![text_track(
        false,
        false,
        0,
        0,
        FadeCurve::Linear,
        FadeCurve::Linear,
    )]);
    let mut args = args_for(CLIP_TEXT_A);
    args.fade_out_tk = Some(-1);

    let err = compute_patch(&prior, &args).expect_err("negative fade out");
    assert!(matches!(
        err,
        ClipSetFadeError::BadTime {
            field: "fade_out_tk",
            ..
        }
    ));
}

#[test]
fn compute_patch_bad_range_before_snap_errors() {
    let prior = project_with_tracks(vec![text_track(
        false,
        false,
        0,
        0,
        FadeCurve::Linear,
        FadeCurve::Linear,
    )]);
    let mut args = args_for(CLIP_TEXT_A);
    args.fade_in_tk = Some(200_000);
    args.fade_out_tk = Some(80_001);

    let err = compute_patch(&prior, &args).expect_err("pre-snap overflow");
    assert!(matches!(err, ClipSetFadeError::BadRange { .. }));
}

#[test]
fn compute_patch_bad_range_after_snap_errors() {
    let mut prior = project_with_tracks(vec![text_track(
        false,
        false,
        0,
        0,
        FadeCurve::Linear,
        FadeCurve::Linear,
    )]);
    prior.tracks[0].clips[0].source_out_tk = Tick::new(12_000);
    prior.duration_tk = Tick::new(12_000);
    let mut args = args_for(CLIP_TEXT_A);
    args.fade_in_tk = Some(4_001);
    args.fade_out_tk = Some(4_001);

    let err = compute_patch(&prior, &args).expect_err("post-snap overflow");
    assert!(matches!(err, ClipSetFadeError::BadRange { .. }));
}

#[test]
fn compute_patch_snaps_off_frame_video_fade() {
    let prior = project_with_tracks(vec![clip_track(ClipTrackFixture {
        kind: TrackKind::Video,
        track_id: TRACK_VIDEO_A,
        track_locked: false,
        clip_id: CLIP_VIDEO_A,
        clip_locked: false,
        fade_in_tk: 0,
        fade_out_tk: 0,
        fade_in_curve: FadeCurve::Linear,
        fade_out_curve: FadeCurve::Linear,
    })]);
    let mut args = args_for(CLIP_VIDEO_A);
    args.fade_in_tk = Some(8_001);

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("video snap");
    assert_eq!(patch_values(&patch).0, 8_000);
    assert_eq!(data.fade_in_tk, 8_000);
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_TIME_SNAPPED_CODE);
    assert_eq!(warnings[0]["details"]["field"], "fade_in_tk");
    assert_eq!(warnings[0]["details"]["from_tk"], 8_001);
    assert_eq!(warnings[0]["details"]["to_tk"], 8_000);
}

#[test]
fn compute_patch_snaps_off_frame_image_fade() {
    let mut track = clip_track(ClipTrackFixture {
        kind: TrackKind::Video,
        track_id: TRACK_VIDEO_A,
        track_locked: false,
        clip_id: CLIP_VIDEO_A,
        clip_locked: false,
        fade_in_tk: 0,
        fade_out_tk: 0,
        fade_in_curve: FadeCurve::Linear,
        fade_out_curve: FadeCurve::Linear,
    });
    track.clips[0].asset_id =
        verbreel_state::AssetRef::try_from(ASSET_IMAGE_ID.to_string()).expect("asset ref parses");
    let prior = project_with_tracks(vec![track]);
    let mut args = args_for(CLIP_VIDEO_A);
    args.fade_out_tk = Some(8_001);

    let (_patch, warnings, data) = compute_patch(&prior, &args).expect("image snap");
    assert_eq!(data.fade_out_tk, 8_000);
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_TIME_SNAPPED_CODE);
    assert_eq!(warnings[0]["details"]["field"], "fade_out_tk");
}

#[test]
fn compute_patch_snaps_off_frame_text_fades() {
    let prior = project_with_tracks(vec![text_track(
        false,
        false,
        0,
        0,
        FadeCurve::Linear,
        FadeCurve::Linear,
    )]);
    let mut args = args_for(CLIP_TEXT_A);
    args.fade_in_tk = Some(7_999);
    args.fade_out_tk = Some(8_001);

    let (_patch, warnings, data) = compute_patch(&prior, &args).expect("text snap");
    assert_eq!(data.fade_in_tk, 8_000);
    assert_eq!(data.fade_out_tk, 8_000);
    assert_eq!(warnings.len(), 2);
    assert_eq!(warnings[0]["details"]["field"], "fade_in_tk");
    assert_eq!(warnings[1]["details"]["field"], "fade_out_tk");
}

#[test]
fn compute_patch_audio_does_not_snap_off_frame_fades() {
    let prior = project_with_tracks(vec![clip_track(ClipTrackFixture {
        kind: TrackKind::Audio,
        track_id: TRACK_AUDIO_A,
        track_locked: false,
        clip_id: CLIP_AUDIO_A,
        clip_locked: false,
        fade_in_tk: 0,
        fade_out_tk: 0,
        fade_in_curve: FadeCurve::Linear,
        fade_out_curve: FadeCurve::Linear,
    })]);
    let mut args = args_for(CLIP_AUDIO_A);
    args.fade_in_tk = Some(8_001);

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("audio no snap");
    assert_eq!(patch_values(&patch).0, 8_001);
    assert_eq!(data.fade_in_tk, 8_001);
    assert!(warnings.is_empty());
}

#[test]
fn compute_patch_noop_warns() {
    let prior = project_with_tracks(vec![text_track(
        false,
        false,
        8_000,
        16_000,
        FadeCurve::Exp,
        FadeCurve::Log,
    )]);
    let mut args = args_for(CLIP_TEXT_A);
    args.fade_in_tk = Some(8_000);
    args.fade_out_tk = Some(16_000);
    args.fade_in_curve = Some(FadeCurve::Exp);
    args.fade_out_curve = Some(FadeCurve::Log);

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("noop");
    assert_eq!(patch, json!([]));
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_NOOP_CODE);
    assert_eq!(warnings[0]["message"], "clip.set_fade no-op");
    assert_eq!(warnings[0]["details"]["clip_id"], CLIP_TEXT_A);
    assert_eq!(data.fade_in_curve, FadeCurve::Exp);
}

#[test]
fn compute_patch_rejects_all_optionals_omitted() {
    let prior = project_with_tracks(vec![text_track(
        false,
        false,
        0,
        0,
        FadeCurve::Linear,
        FadeCurve::Linear,
    )]);
    let args = args_for(CLIP_TEXT_A);

    let err = compute_patch(&prior, &args).expect_err("empty partial update rejected");
    assert!(matches!(err, ClipSetFadeError::BadArgs));
}

#[test]
fn data_envelope_from_post_state_returns_post_fade_state() {
    let post_state = project_with_tracks(vec![text_track(
        false,
        false,
        8_000,
        16_000,
        FadeCurve::Exp,
        FadeCurve::Log,
    )]);
    let mut args = args_for(CLIP_TEXT_A);
    args.fade_in_tk = Some(24_000);

    let data = data_envelope_from_post_state(&args, &post_state)
        .expect("post-state envelope should be readable");
    assert_eq!(data.clip_id.to_string(), CLIP_TEXT_A);
    assert_eq!(data.fade_in_tk, 8_000);
    assert_eq!(data.fade_out_tk, 16_000);
    assert_eq!(data.fade_in_curve, FadeCurve::Exp);
    assert_eq!(data.fade_out_curve, FadeCurve::Log);
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "clip.set_fade")
        .expect("default_fixtures includes clip.set_fade");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(ClipSetFadeVerb))
        .expect("register clip.set_fade verb");

    let report = validate_reconstructors(&registry, &[fixture])
        .expect("clip.set_fade reconstruct from fixture");
    assert_eq!(report.verbs_checked, vec!["clip.set_fade"]);
    assert_eq!(report.fixtures_run, 1);
}

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb() {
    use tempfile::TempDir;

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
            "clip.set_fade",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "clip": CLIP_AUDIO_A,
                "fade_in_tk": 8_001,
                "fade_out_curve": "log",
            }),
            None,
        )
        .expect("mutate_via_verb happy path");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("happy path must return Applied, got {outcome:?}");
    };

    let data: ClipSetFadeData =
        serde_json::from_value(data).expect("clip.set_fade data is ClipSetFadeData");
    assert_eq!(data.clip_id.to_string(), CLIP_AUDIO_A);
    assert_eq!(data.fade_in_tk, 8_001);
    assert_eq!(data.fade_out_curve, FadeCurve::Log);
    assert_eq!(
        store.project().tracks[0].clips[0].fade_in_tk,
        Tick::new(8_001)
    );
    assert!(warnings.is_empty());
}
