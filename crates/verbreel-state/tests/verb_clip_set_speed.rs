//! Tests for `clip.set_speed` (§5.7) — fifty-third production verb.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::clip_set_speed::{
    W_EFFECT_WINDOW_CLAMPED_CODE, W_FADE_CLAMPED_CODE, W_KEYFRAMES_REMOVED_CODE,
    W_SPEED_EXTREME_CODE, compute_patch, data_envelope_from_post_state,
};
use verbreel_state::{
    ClipSetSpeedArgs, ClipSetSpeedData, ClipSetSpeedError, ClipSetSpeedVerb, MutateOutcome,
    Project, TrackKind, VerbRegistry, default_fixtures, default_registry, timeline_duration_tk,
    validate_reconstructors,
};
use verbreel_types::{ProjectId, Tick};

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const TRACK_VIDEO_A: &str = "01900000-0000-7000-8000-0000000aa801";
const TRACK_AUDIO_A: &str = "01900000-0000-7000-8000-0000000aa802";
const TRACK_AUDIO_B: &str = "01900000-0000-7000-8000-0000000aa803";
const TRACK_TEXT_A: &str = "01900000-0000-7000-8000-0000000aa804";
const CLIP_VIDEO_A: &str = "01900000-0000-7000-8000-0000000bb801";
const CLIP_VIDEO_B: &str = "01900000-0000-7000-8000-0000000bb802";
const CLIP_AUDIO_A: &str = "01900000-0000-7000-8000-0000000bb803";
const CLIP_AUDIO_B: &str = "01900000-0000-7000-8000-0000000bb800";
const CLIP_IMAGE_A: &str = "01900000-0000-7000-8000-0000000bb805";
const CLIP_TEXT_A: &str = "01900000-0000-7000-8000-0000000bb806";
const MISSING_CLIP: &str = "01900000-0000-7000-8000-0000000bb899";
const LINK_GROUP_ID: &str = "01900000-0000-7000-8000-0000000dd801";
const ASSET_VIDEO_ID: &str = "01900000-0000-7000-8000-0000000ee801";
const ASSET_AUDIO_ID: &str = "01900000-0000-7000-8000-0000000ee802";
const ASSET_IMAGE_ID: &str = "01900000-0000-7000-8000-0000000ee803";
const KEYFRAME_A: &str = "01900000-0000-7000-8000-0000000ff801";
const KEYFRAME_B: &str = "01900000-0000-7000-8000-0000000ff802";
const EFFECT_A: &str = "01900000-0000-7000-8000-0000000aa901";
const EFFECT_B: &str = "01900000-0000-7000-8000-0000000aa902";

#[derive(Debug, Clone)]
struct ClipFixture {
    id: &'static str,
    position_tk: i64,
    source_in_tk: i64,
    source_out_tk: i64,
    speed: f64,
    locked: bool,
    link_group: Option<&'static str>,
    asset_id: &'static str,
    fade_in_tk: i64,
    fade_out_tk: i64,
    effects: Vec<Value>,
    keyframes: Vec<Value>,
}

#[derive(Debug, Clone)]
struct TrackFixture {
    id: &'static str,
    kind: TrackKind,
    locked: bool,
    clips: Vec<ClipFixture>,
}

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture -> Project")
}

fn fixture_project_id() -> ProjectId {
    FIXTURE_PROJECT_ID
        .parse()
        .expect("fixture project id parses")
}

fn args(clip: &str, factor: f64) -> ClipSetSpeedArgs {
    ClipSetSpeedArgs {
        project_id: fixture_project_id(),
        clip: clip.to_string(),
        factor,
    }
}

fn clip(id: &'static str, asset_id: &'static str, position_tk: i64) -> ClipFixture {
    ClipFixture {
        id,
        position_tk,
        source_in_tk: 0,
        source_out_tk: 240_000,
        speed: 1.0,
        locked: false,
        link_group: None,
        asset_id,
        fade_in_tk: 0,
        fade_out_tk: 0,
        effects: Vec::new(),
        keyframes: Vec::new(),
    }
}

fn video_clip(id: &'static str, position_tk: i64) -> ClipFixture {
    clip(id, ASSET_VIDEO_ID, position_tk)
}

fn audio_clip(id: &'static str, position_tk: i64) -> ClipFixture {
    clip(id, ASSET_AUDIO_ID, position_tk)
}

fn image_clip(id: &'static str, position_tk: i64) -> ClipFixture {
    clip(id, ASSET_IMAGE_ID, position_tk)
}

fn text_clip(id: &'static str, position_tk: i64) -> ClipFixture {
    clip(id, "00000000-0000-0000-0000-000000000000", position_tk)
}

fn linked(mut fixture: ClipFixture) -> ClipFixture {
    fixture.link_group = Some(LINK_GROUP_ID);
    fixture
}

fn locked(mut fixture: ClipFixture) -> ClipFixture {
    fixture.locked = true;
    fixture
}

fn with_speed(mut fixture: ClipFixture, speed: f64) -> ClipFixture {
    fixture.speed = speed;
    fixture
}

fn with_source_out(mut fixture: ClipFixture, source_out_tk: i64) -> ClipFixture {
    fixture.source_out_tk = source_out_tk;
    fixture
}

fn with_fades(mut fixture: ClipFixture, fade_in_tk: i64, fade_out_tk: i64) -> ClipFixture {
    fixture.fade_in_tk = fade_in_tk;
    fixture.fade_out_tk = fade_out_tk;
    fixture
}

fn with_effects(mut fixture: ClipFixture, effects: Vec<Value>) -> ClipFixture {
    fixture.effects = effects;
    fixture
}

fn with_keyframes(mut fixture: ClipFixture, keyframes: Vec<Value>) -> ClipFixture {
    fixture.keyframes = keyframes;
    fixture
}

fn video_track(clips: Vec<ClipFixture>) -> TrackFixture {
    TrackFixture {
        id: TRACK_VIDEO_A,
        kind: TrackKind::Video,
        locked: false,
        clips,
    }
}

fn audio_track(id: &'static str, clips: Vec<ClipFixture>) -> TrackFixture {
    TrackFixture {
        id,
        kind: TrackKind::Audio,
        locked: false,
        clips,
    }
}

fn text_track(clips: Vec<ClipFixture>) -> TrackFixture {
    TrackFixture {
        id: TRACK_TEXT_A,
        kind: TrackKind::Text,
        locked: false,
        clips,
    }
}

fn track_locked(mut track: TrackFixture) -> TrackFixture {
    track.locked = true;
    track
}

fn clip_value(kind: TrackKind, fixture: &ClipFixture) -> Value {
    let mut value = json!({
        "id": fixture.id,
        "name": "Clip",
        "asset_id": fixture.asset_id,
        "track_position_tk": fixture.position_tk,
        "source_in_tk": fixture.source_in_tk,
        "source_out_tk": fixture.source_out_tk,
        "speed": fixture.speed,
        "locked": fixture.locked,
        "fade_in_tk": fixture.fade_in_tk,
        "fade_out_tk": fixture.fade_out_tk,
        "effects": fixture.effects,
        "keyframes": fixture.keyframes,
    });
    if kind == TrackKind::Audio {
        value["volume"] = json!(1.0);
    }
    if kind == TrackKind::Text {
        value["text"] = json!({
            "content": "Speed",
            "font_family": "Arial",
            "font_size_px": 24,
        });
    }
    if let Some(link_group) = fixture.link_group {
        value["link_group"] = json!(link_group);
    }
    value
}

fn project_with_tracks(tracks: Vec<TrackFixture>) -> Project {
    let mut project = empty_project();
    project.tracks = tracks
        .into_iter()
        .map(|track| {
            let clips = track
                .clips
                .iter()
                .map(|clip| clip_value(track.kind, clip))
                .collect::<Vec<_>>();
            serde_json::from_value(json!({
                "id": track.id,
                "kind": track.kind,
                "name": "Track",
                "locked": track.locked,
                "clips": clips,
            }))
            .expect("track fixture parses")
        })
        .collect();
    project.assets.clear();
    add_assets(&mut project);
    project.duration_tk = Tick::new(computed_project_duration_tk(&project));
    project
}

fn computed_project_duration_tk(project: &Project) -> i64 {
    project
        .tracks
        .iter()
        .flat_map(|track| &track.clips)
        .map(|clip| {
            clip.track_position_tk.get()
                + timeline_duration_tk(clip.source_in_tk, clip.source_out_tk, clip.speed).get()
        })
        .max()
        .unwrap_or(0)
}

fn add_assets(project: &mut Project) {
    project
        .assets
        .push(serde_json::from_value(video_asset()).expect("video asset parses"));
    project
        .assets
        .push(serde_json::from_value(audio_asset()).expect("audio asset parses"));
    project
        .assets
        .push(serde_json::from_value(image_asset()).expect("image asset parses"));
}

fn video_asset() -> Value {
    json!({
        "id": ASSET_VIDEO_ID,
        "kind": "video",
        "hash": "36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da",
        "path": "assets/36/36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da.mp4",
        "original_filename": "clip-set-speed.mp4",
        "imported_at": "2026-05-24T00:00:00Z",
        "metadata": {
            "duration_tk": 9_007_199_254_740_991_i64,
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

fn audio_asset() -> Value {
    json!({
        "id": ASSET_AUDIO_ID,
        "kind": "audio",
        "hash": "53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658",
        "path": "assets/53/53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.m4a",
        "original_filename": "clip-set-speed.m4a",
        "imported_at": "2026-05-24T00:00:00Z",
        "metadata": {
            "duration_tk": 480_000,
            "audio_codec": "aac",
            "audio_channels": 2,
            "audio_sample_rate_hz": 48000,
            "container": "m4a",
            "fingerprint": {
                "mtime_ms": 1_700_000_000_000_i64,
                "size_bytes": 1024,
            }
        }
    })
}

fn image_asset() -> Value {
    json!({
        "id": ASSET_IMAGE_ID,
        "kind": "image",
        "hash": "d78685cbed99000e92b1e62dae6cc40404f68c3f5069135de242ad7201a3d552",
        "path": "assets/d7/d78685cbed99000e92b1e62dae6cc40404f68c3f5069135de242ad7201a3d552.png",
        "original_filename": "still.png",
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

fn apply_patch(prior: &Project, patch: &Value) -> Project {
    let patch: json_patch::Patch =
        serde_json::from_value(patch.clone()).expect("patch parses to typed JSON patch");
    prior
        .apply(&patch)
        .expect("clip.set_speed patch applies cleanly")
}

fn keyframe(id: &str, property: &str, time_tk: i64) -> Value {
    json!({
        "id": id,
        "property": property,
        "time_tk": time_tk,
        "value": 0.5,
    })
}

fn effect(id: &str, kind: &str, in_tk: Option<i64>, out_tk: Option<i64>) -> Value {
    let mut value = json!({
        "id": id,
        "kind": kind,
        "enabled": true,
        "params": {
            "radius_px": 8,
            "factor": 1.0,
        },
    });
    if let (Some(in_tk), Some(out_tk)) = (in_tk, out_tk) {
        value["in_tk"] = json!(in_tk);
        value["out_tk"] = json!(out_tk);
    }
    value
}

fn warning_codes(warnings: &[Value]) -> Vec<String> {
    warnings
        .iter()
        .map(|warning| warning["code"].as_str().expect("warning code").to_string())
        .collect()
}

fn singleton_video_project() -> Project {
    project_with_tracks(vec![video_track(vec![video_clip(CLIP_VIDEO_A, 0)])])
}

#[test]
fn happy_path_scalar_speed_one_to_two_halves_duration() {
    let prior = singleton_video_project();
    let (patch, warnings, data) =
        compute_patch(&prior, &args(CLIP_VIDEO_A, 2.0)).expect("set speed");
    let post = apply_patch(&prior, &patch);

    assert!(warnings.is_empty());
    assert_eq!(post.tracks[0].clips[0].speed, 2.0);
    assert_eq!(post.duration_tk.get(), 120_000);
    assert_eq!(data.duration_tk, 120_000);
    assert_eq!(data.speed, 2.0);
}

#[test]
fn happy_path_speed_two_to_half_doubles_duration() {
    let prior = project_with_tracks(vec![video_track(vec![with_speed(
        video_clip(CLIP_VIDEO_A, 0),
        2.0,
    )])]);
    let (patch, warnings, data) =
        compute_patch(&prior, &args(CLIP_VIDEO_A, 0.5)).expect("set speed");
    let post = apply_patch(&prior, &patch);

    assert!(warnings.is_empty());
    assert_eq!(post.tracks[0].clips[0].speed, 0.5);
    assert_eq!(post.duration_tk.get(), 480_000);
    assert_eq!(data.duration_tk, 480_000);
}

#[test]
fn linked_video_audio_group_syncs_speed_to_every_member() {
    let prior = project_with_tracks(vec![
        video_track(vec![linked(video_clip(CLIP_VIDEO_A, 0))]),
        audio_track(TRACK_AUDIO_A, vec![linked(audio_clip(CLIP_AUDIO_A, 0))]),
    ]);
    let (patch, warnings, data) =
        compute_patch(&prior, &args(CLIP_VIDEO_A, 2.0)).expect("linked speed");
    let post = apply_patch(&prior, &patch);

    assert!(warnings.is_empty());
    assert_eq!(post.tracks[0].clips[0].speed, 2.0);
    assert_eq!(post.tracks[1].clips[0].speed, 2.0);
    assert_eq!(data.linked_clip_ids, vec![CLIP_AUDIO_A.parse().unwrap()]);
}

#[test]
fn factor_zero_returns_schema_violation() {
    let prior = singleton_video_project();
    let err = compute_patch(&prior, &args(CLIP_VIDEO_A, 0.0)).expect_err("invalid factor");
    assert!(matches!(
        err,
        ClipSetSpeedError::SchemaViolation {
            field: "factor",
            allowed: "(0, 100]",
            value: 0.0,
        }
    ));
}

#[test]
fn factor_above_hundred_returns_schema_violation() {
    let prior = singleton_video_project();
    let err = compute_patch(&prior, &args(CLIP_VIDEO_A, 100.1)).expect_err("invalid factor");
    assert!(matches!(
        err,
        ClipSetSpeedError::SchemaViolation {
            field: "factor",
            allowed: "(0, 100]",
            value,
        } if (value - 100.1).abs() < f64::EPSILON
    ));
}

#[test]
fn text_clip_returns_kind_mismatch() {
    let prior = project_with_tracks(vec![text_track(vec![text_clip(CLIP_TEXT_A, 0)])]);
    let err = compute_patch(&prior, &args(CLIP_TEXT_A, 2.0)).expect_err("text mismatch");
    assert!(matches!(
        err,
        ClipSetSpeedError::ClipKindMismatch {
            actual_kind: "text",
            ..
        }
    ));
}

#[test]
fn image_clip_returns_kind_mismatch() {
    let prior = project_with_tracks(vec![video_track(vec![image_clip(CLIP_IMAGE_A, 0)])]);
    let err = compute_patch(&prior, &args(CLIP_IMAGE_A, 2.0)).expect_err("image mismatch");
    assert!(matches!(
        err,
        ClipSetSpeedError::ClipKindMismatch {
            actual_kind: "image",
            ..
        }
    ));
}

#[test]
fn missing_clip_returns_not_found() {
    let prior = singleton_video_project();
    let err = compute_patch(&prior, &args(MISSING_CLIP, 2.0)).expect_err("missing clip");
    assert!(matches!(err, ClipSetSpeedError::ClipNotFound { .. }));
}

#[test]
fn malformed_uuid_returns_bad_selector() {
    let prior = singleton_video_project();
    let err = compute_patch(&prior, &args("not-a-uuid", 2.0)).expect_err("bad selector");
    assert!(matches!(err, ClipSetSpeedError::BadSelector { .. }));
}

#[test]
fn locked_target_clip_returns_locked() {
    let prior = project_with_tracks(vec![video_track(vec![locked(video_clip(CLIP_VIDEO_A, 0))])]);
    let err = compute_patch(&prior, &args(CLIP_VIDEO_A, 2.0)).expect_err("locked");
    assert!(
        matches!(err, ClipSetSpeedError::Locked { failed_clip } if failed_clip == CLIP_VIDEO_A)
    );
}

#[test]
fn locked_parent_track_returns_locked() {
    let prior = project_with_tracks(vec![track_locked(video_track(vec![video_clip(
        CLIP_VIDEO_A,
        0,
    )]))]);
    let err = compute_patch(&prior, &args(CLIP_VIDEO_A, 2.0)).expect_err("locked");
    assert!(
        matches!(err, ClipSetSpeedError::Locked { failed_clip } if failed_clip == CLIP_VIDEO_A)
    );
}

#[test]
fn locked_linked_sibling_returns_locked() {
    let prior = project_with_tracks(vec![
        video_track(vec![linked(video_clip(CLIP_VIDEO_A, 0))]),
        audio_track(
            TRACK_AUDIO_A,
            vec![linked(locked(audio_clip(CLIP_AUDIO_A, 0)))],
        ),
    ]);
    let err = compute_patch(&prior, &args(CLIP_VIDEO_A, 2.0)).expect_err("locked sibling");
    assert!(
        matches!(err, ClipSetSpeedError::Locked { failed_clip } if failed_clip == CLIP_AUDIO_A)
    );
}

#[test]
fn overlap_after_slowdown_aborts_atomically() {
    let prior = project_with_tracks(vec![video_track(vec![
        video_clip(CLIP_VIDEO_A, 0),
        video_clip(CLIP_VIDEO_B, 250_000),
    ])]);
    let err = compute_patch(&prior, &args(CLIP_VIDEO_A, 0.5)).expect_err("overlap");
    assert!(
        matches!(err, ClipSetSpeedError::ClipOverlap { failed_clip } if failed_clip == CLIP_VIDEO_A)
    );
}

#[test]
fn linked_group_mixing_source_and_display_semantics_is_rejected() {
    let prior = project_with_tracks(vec![
        video_track(vec![linked(video_clip(CLIP_VIDEO_A, 0))]),
        text_track(vec![linked(text_clip(CLIP_TEXT_A, 0))]),
    ]);
    let err = compute_patch(&prior, &args(CLIP_VIDEO_A, 2.0)).expect_err("semantics mix");
    assert!(matches!(
        err,
        ClipSetSpeedError::LinkGroupSemanticsMix {
            hint: "call clip.unlink first, then mutate each clip independently",
            ..
        }
    ));
}

#[test]
fn fade_sum_over_new_duration_clamps_and_warns() {
    let prior = project_with_tracks(vec![video_track(vec![with_fades(
        video_clip(CLIP_VIDEO_A, 0),
        80_000,
        80_000,
    )])]);
    let (patch, warnings, _data) =
        compute_patch(&prior, &args(CLIP_VIDEO_A, 2.0)).expect("set speed");
    let post = apply_patch(&prior, &patch);

    assert_eq!(warning_codes(&warnings), vec![W_FADE_CLAMPED_CODE]);
    assert_eq!(post.tracks[0].clips[0].fade_in_tk.get(), 60_000);
    assert_eq!(post.tracks[0].clips[0].fade_out_tk.get(), 60_000);
}

#[test]
fn keyframes_after_new_duration_are_removed_and_warn() {
    let prior = project_with_tracks(vec![video_track(vec![with_keyframes(
        video_clip(CLIP_VIDEO_A, 0),
        vec![
            keyframe(KEYFRAME_A, "opacity", 60_000),
            keyframe(KEYFRAME_B, "opacity", 180_000),
        ],
    )])]);
    let (patch, warnings, _data) =
        compute_patch(&prior, &args(CLIP_VIDEO_A, 2.0)).expect("set speed");
    let post = apply_patch(&prior, &patch);

    assert_eq!(warning_codes(&warnings), vec![W_KEYFRAMES_REMOVED_CODE]);
    assert_eq!(post.tracks[0].clips[0].keyframes.len(), 1);
    assert_eq!(
        warnings[0]["details"]["removed_keyframe_ids"],
        json!([KEYFRAME_B])
    );
}

#[test]
fn effect_window_out_after_new_duration_is_clamped() {
    let prior = project_with_tracks(vec![video_track(vec![with_effects(
        video_clip(CLIP_VIDEO_A, 0),
        vec![effect(EFFECT_A, "blur", Some(60_000), Some(180_000))],
    )])]);
    let (patch, warnings, _data) =
        compute_patch(&prior, &args(CLIP_VIDEO_A, 2.0)).expect("set speed");
    let post = apply_patch(&prior, &patch);

    assert_eq!(warning_codes(&warnings), vec![W_EFFECT_WINDOW_CLAMPED_CODE]);
    assert_eq!(
        post.tracks[0].clips[0].effects[0]
            .window
            .unwrap()
            .out_tk
            .get(),
        120_000
    );
    assert_eq!(warnings[0]["details"]["effect_id"], EFFECT_A);
}

#[test]
fn effect_window_start_after_new_duration_removes_effect_and_cascades_keyframes() {
    let property = format!("effects[{EFFECT_A}].params.radius_px");
    let prior = project_with_tracks(vec![video_track(vec![with_keyframes(
        with_effects(
            video_clip(CLIP_VIDEO_A, 0),
            vec![effect(EFFECT_A, "blur", Some(130_000), Some(200_000))],
        ),
        vec![keyframe(KEYFRAME_A, &property, 100_000)],
    )])]);
    let (patch, warnings, _data) =
        compute_patch(&prior, &args(CLIP_VIDEO_A, 2.0)).expect("set speed");
    let post = apply_patch(&prior, &patch);

    assert_eq!(warning_codes(&warnings), vec![W_KEYFRAMES_REMOVED_CODE]);
    assert!(post.tracks[0].clips[0].effects.is_empty());
    assert!(post.tracks[0].clips[0].keyframes.is_empty());
}

#[test]
fn speed_extreme_warns_when_time_stretch_effect_exists() {
    let prior = project_with_tracks(vec![video_track(vec![with_effects(
        with_source_out(video_clip(CLIP_VIDEO_A, 0), 4_800_000),
        vec![effect(EFFECT_A, "time_stretch", None, None)],
    )])]);
    let (_patch, warnings, _data) =
        compute_patch(&prior, &args(CLIP_VIDEO_A, 20.0)).expect("set speed");

    assert_eq!(warning_codes(&warnings), vec![W_SPEED_EXTREME_CODE]);
    assert_eq!(warnings[0]["details"]["factor"], 20.0);
}

#[test]
fn speed_extreme_does_not_warn_without_time_stretch_effect() {
    let prior = project_with_tracks(vec![video_track(vec![with_source_out(
        video_clip(CLIP_VIDEO_A, 0),
        4_800_000,
    )])]);
    let (_patch, warnings, _data) =
        compute_patch(&prior, &args(CLIP_VIDEO_A, 20.0)).expect("set speed");

    assert!(warnings.is_empty());
}

#[test]
fn data_envelope_time_stretch_fields_are_always_null_and_empty() {
    let prior = singleton_video_project();
    let (_patch, _warnings, data) =
        compute_patch(&prior, &args(CLIP_VIDEO_A, 2.0)).expect("set speed");
    let value = serde_json::to_value(data).expect("data serializes");

    assert_eq!(value["time_stretch_effect_id"], Value::Null);
    assert_eq!(value["removed_time_stretch_effect_id"], Value::Null);
    assert_eq!(value["linked_time_stretch_effect_ids"], json!([]));
    assert_eq!(value["removed_linked_time_stretch_effect_ids"], json!([]));
}

#[test]
fn linked_clip_ids_exclude_target_and_are_sorted() {
    let prior = project_with_tracks(vec![
        video_track(vec![linked(video_clip(CLIP_VIDEO_A, 0))]),
        audio_track(TRACK_AUDIO_A, vec![linked(audio_clip(CLIP_AUDIO_A, 0))]),
        audio_track(TRACK_AUDIO_B, vec![linked(audio_clip(CLIP_AUDIO_B, 0))]),
    ]);
    let (_patch, _warnings, data) =
        compute_patch(&prior, &args(CLIP_VIDEO_A, 2.0)).expect("set speed");

    assert_eq!(
        data.linked_clip_ids,
        vec![CLIP_AUDIO_B.parse().unwrap(), CLIP_AUDIO_A.parse().unwrap()]
    );
}

#[test]
fn reconstructor_round_trips_singleton() {
    let prior = singleton_video_project();
    let (patch, warnings, data) =
        compute_patch(&prior, &args(CLIP_VIDEO_A, 2.0)).expect("set speed");
    let post = apply_patch(&prior, &patch);

    let reconstructed =
        data_envelope_from_post_state(&args(CLIP_VIDEO_A, 2.0), &post).expect("reconstructs");

    assert!(warnings.is_empty());
    assert_eq!(reconstructed, data);
}

#[test]
fn reconstructor_round_trips_linked_group() {
    let prior = project_with_tracks(vec![
        video_track(vec![linked(video_clip(CLIP_VIDEO_A, 0))]),
        audio_track(TRACK_AUDIO_A, vec![linked(audio_clip(CLIP_AUDIO_A, 0))]),
    ]);
    let (patch, warnings, data) =
        compute_patch(&prior, &args(CLIP_VIDEO_A, 2.0)).expect("linked speed");
    let post = apply_patch(&prior, &patch);

    let reconstructed =
        data_envelope_from_post_state(&args(CLIP_VIDEO_A, 2.0), &post).expect("reconstructs");

    assert!(warnings.is_empty());
    assert_eq!(reconstructed, data);
}

#[test]
fn reconstructor_round_trips_with_cascade_warnings() {
    let property = format!("effects[{EFFECT_A}].params.radius_px");
    let prior = project_with_tracks(vec![video_track(vec![with_keyframes(
        with_effects(
            video_clip(CLIP_VIDEO_A, 0),
            vec![
                effect(EFFECT_A, "blur", Some(130_000), Some(200_000)),
                effect(EFFECT_B, "glow", Some(60_000), Some(180_000)),
            ],
        ),
        vec![keyframe(KEYFRAME_A, &property, 100_000)],
    )])]);
    let (patch, warnings, data) =
        compute_patch(&prior, &args(CLIP_VIDEO_A, 2.0)).expect("set speed");
    let post = apply_patch(&prior, &patch);

    let reconstructed =
        data_envelope_from_post_state(&args(CLIP_VIDEO_A, 2.0), &post).expect("reconstructs");

    assert_eq!(
        warning_codes(&warnings),
        vec![W_EFFECT_WINDOW_CLAMPED_CODE, W_KEYFRAMES_REMOVED_CODE]
    );
    assert_eq!(reconstructed, data);
}

#[test]
fn project_duration_updates_when_max_extent_changes() {
    let prior = singleton_video_project();
    assert_eq!(prior.duration_tk.get(), 240_000);

    let (patch, _warnings, _data) =
        compute_patch(&prior, &args(CLIP_VIDEO_A, 0.5)).expect("set speed");
    let post = apply_patch(&prior, &patch);

    assert_eq!(post.duration_tk.get(), 480_000);
}

#[test]
fn default_fixture_reconstructs() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "clip.set_speed")
        .expect("default_fixtures includes clip.set_speed");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(ClipSetSpeedVerb))
        .expect("register clip.set_speed verb");

    let report =
        validate_reconstructors(&registry, &[fixture]).expect("clip.set_speed default fixture");
    assert_eq!(report.verbs_checked, vec!["clip.set_speed"]);
    assert_eq!(report.fixtures_run, 1);
}

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        singleton_video_project(),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry clears the gate and writes project.json");

    let outcome = store
        .mutate_via_verb(
            "clip.set_speed",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "clip": CLIP_VIDEO_A,
                "factor": 2.0,
            }),
            None,
        )
        .expect("mutate_via_verb happy path");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("happy path must return Applied, got {outcome:?}");
    };

    let data: ClipSetSpeedData = serde_json::from_value(data).expect("clip.set_speed data parses");
    assert_eq!(data.clip_id.to_string(), CLIP_VIDEO_A);
    assert_eq!(data.speed, 2.0);
    assert_eq!(store.project().tracks[0].clips[0].speed, 2.0);
    assert!(warnings.is_empty());
}
