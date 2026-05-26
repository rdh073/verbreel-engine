//! Tests for `clip.split` (§5.3) — forty-eighth production verb.

use serde_json::{Value, json};
use std::sync::Arc;

use verbreel_state::verbs::clip_split::{
    W_CLIP_SPLIT_ENVELOPE_CODE, W_FADE_CLAMPED_CODE, W_KEYFRAMES_REMOVED_CODE, compute_patch,
    data_envelope_from_args_warnings,
};
use verbreel_state::{
    ClipSplitArgs, ClipSplitData, ClipSplitError, ClipSplitVerb, MutateOutcome, Project,
    RecordedEvent, TrackKind, Verb, VerbRegistry, default_fixtures, default_registry,
    validate_reconstructors,
};
use verbreel_types::{ProjectId, Tick};

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const TRACK_VIDEO_A: &str = "01900000-0000-7000-8000-0000000aa801";
const TRACK_AUDIO_A: &str = "01900000-0000-7000-8000-0000000aa802";
const TRACK_VIDEO_B: &str = "01900000-0000-7000-8000-0000000aa803";
const CLIP_VIDEO_A: &str = "01900000-0000-7000-8000-0000000bb801";
const CLIP_AUDIO_A: &str = "01900000-0000-7000-8000-0000000bb803";
const CLIP_IMAGE_A: &str = "01900000-0000-7000-8000-0000000bb804";
const MISSING_CLIP: &str = "01900000-0000-7000-8000-0000000cc801";
const LINK_GROUP_ID: &str = "01900000-0000-7000-8000-0000000dd801";
const ASSET_VIDEO_ID: &str = "01900000-0000-7000-8000-0000000ee801";
const ASSET_AUDIO_ID: &str = "01900000-0000-7000-8000-0000000ee802";
const ASSET_IMAGE_ID: &str = "01900000-0000-7000-8000-0000000ee803";
const KEYFRAME_A: &str = "01900000-0000-7000-8000-0000000ff801";
const KEYFRAME_B: &str = "01900000-0000-7000-8000-0000000ff802";
const EFFECT_A: &str = "01900000-0000-7000-8000-0000000ef801";

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
    fade_in_curve: &'static str,
    fade_out_curve: &'static str,
    keyframes: Vec<Value>,
    effects: Vec<Value>,
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

fn split_args(clip: &str, at_tk: i64) -> ClipSplitArgs {
    ClipSplitArgs {
        project_id: fixture_project_id(),
        clip: clip.to_string(),
        at_tk,
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
        fade_in_curve: "linear",
        fade_out_curve: "linear",
        keyframes: Vec::new(),
        effects: Vec::new(),
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

fn linked(mut fixture: ClipFixture) -> ClipFixture {
    fixture.link_group = Some(LINK_GROUP_ID);
    fixture
}

fn locked(mut fixture: ClipFixture) -> ClipFixture {
    fixture.locked = true;
    fixture
}

fn with_source_out(mut fixture: ClipFixture, source_out_tk: i64) -> ClipFixture {
    fixture.source_out_tk = source_out_tk;
    fixture
}

fn with_speed(mut fixture: ClipFixture, speed: f64) -> ClipFixture {
    fixture.speed = speed;
    fixture
}

fn with_fades(mut fixture: ClipFixture, fade_in_tk: i64, fade_out_tk: i64) -> ClipFixture {
    fixture.fade_in_tk = fade_in_tk;
    fixture.fade_out_tk = fade_out_tk;
    fixture.fade_in_curve = "exp";
    fixture.fade_out_curve = "log";
    fixture
}

fn with_keyframes(mut fixture: ClipFixture, keyframes: Vec<Value>) -> ClipFixture {
    fixture.keyframes = keyframes;
    fixture
}

fn with_effect(mut fixture: ClipFixture) -> ClipFixture {
    fixture.effects = vec![json!({
        "id": EFFECT_A,
        "kind": "blur",
        "enabled": true,
        "params": {
            "radius_px": 4
        }
    })];
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

fn second_video_track(clips: Vec<ClipFixture>) -> TrackFixture {
    TrackFixture {
        id: TRACK_VIDEO_B,
        kind: TrackKind::Video,
        locked: false,
        clips,
    }
}

fn audio_track(clips: Vec<ClipFixture>) -> TrackFixture {
    TrackFixture {
        id: TRACK_AUDIO_A,
        kind: TrackKind::Audio,
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
        "fade_in_curve": fixture.fade_in_curve,
        "fade_out_curve": fixture.fade_out_curve,
        "keyframes": fixture.keyframes,
        "effects": fixture.effects,
    });
    if kind == TrackKind::Audio {
        value["volume"] = json!(1.0);
    }
    if let Some(link_group) = fixture.link_group {
        value["link_group"] = json!(link_group);
    }
    value
}

fn project_with_tracks(tracks: Vec<TrackFixture>) -> Project {
    let mut project = empty_project();
    let duration_tk = tracks
        .iter()
        .flat_map(|track| track.clips.iter())
        .map(|clip| {
            clip.position_tk
                + (((clip.source_out_tk - clip.source_in_tk) as f64) / clip.speed) as i64
        })
        .max()
        .unwrap_or(0);
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
    project.duration_tk = Tick::new(duration_tk);
    project
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
        "original_filename": "clip-split.mp4",
        "imported_at": "2026-05-24T00:00:00Z",
        "metadata": {
            "duration_tk": 480_000,
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
        "original_filename": "clip-split.m4a",
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
        .expect("clip.split patch applies cleanly")
}

fn keyframe(id: &str, time_tk: i64) -> Value {
    json!({
        "id": id,
        "property": "opacity",
        "time_tk": time_tk,
        "value": 0.5,
    })
}

fn visible_warning_codes(warnings: &[Value]) -> Vec<String> {
    warnings
        .iter()
        .filter_map(|warning| {
            let code = warning["code"].as_str().expect("warning code");
            (code != W_CLIP_SPLIT_ENVELOPE_CODE).then(|| code.to_string())
        })
        .collect()
}

#[test]
fn happy_path_split_singleton_midpoint() {
    let prior = project_with_tracks(vec![video_track(vec![video_clip(CLIP_VIDEO_A, 0)])]);
    let (patch, warnings, data) =
        compute_patch(&prior, &split_args(CLIP_VIDEO_A, 120_000)).expect("split midpoint");
    let post = apply_patch(&prior, &patch);

    assert_eq!(visible_warning_codes(&warnings), Vec::<String>::new());
    assert_eq!(post.tracks[0].clips.len(), 2);
    assert_eq!(post.tracks[0].clips[0].id, CLIP_VIDEO_A.parse().unwrap());
    assert_ne!(post.tracks[0].clips[1].id, CLIP_VIDEO_A.parse().unwrap());
    assert_eq!(post.tracks[0].clips[0].source_out_tk.get(), 120_000);
    assert_eq!(post.tracks[0].clips[1].source_in_tk.get(), 120_000);
    assert_eq!(post.tracks[0].clips[1].track_position_tk.get(), 120_000);
    assert_eq!(post.duration_tk.get(), 240_000);
    assert_eq!(data.left_clip_id, CLIP_VIDEO_A.parse().unwrap());
    assert_eq!(data.right_clip_id, post.tracks[0].clips[1].id);
    assert!(data.sibling_splits.is_empty());
}

#[test]
fn boundary_at_start_returns_bad_time() {
    let prior = project_with_tracks(vec![video_track(vec![video_clip(CLIP_VIDEO_A, 10_000)])]);
    let err =
        compute_patch(&prior, &split_args(CLIP_VIDEO_A, 10_000)).expect_err("degenerate left");
    assert!(matches!(
        err,
        ClipSplitError::BadTime {
            field: "at_tk",
            value: 10_000
        }
    ));
}

#[test]
fn boundary_at_end_returns_bad_time() {
    let prior = project_with_tracks(vec![video_track(vec![video_clip(CLIP_VIDEO_A, 10_000)])]);
    let err =
        compute_patch(&prior, &split_args(CLIP_VIDEO_A, 250_000)).expect_err("degenerate right");
    assert!(matches!(
        err,
        ClipSplitError::BadTime { field: "at_tk", .. }
    ));
}

#[test]
fn boundary_before_start_returns_out_of_bounds() {
    let prior = project_with_tracks(vec![video_track(vec![video_clip(CLIP_VIDEO_A, 10_000)])]);
    let err = compute_patch(&prior, &split_args(CLIP_VIDEO_A, 9_999)).expect_err("before start");
    assert!(matches!(
        err,
        ClipSplitError::ClipOutOfBounds {
            field: "at_tk",
            failed_clip,
            range_start_tk: 10_000,
            range_end_tk: 250_000,
            ..
        } if failed_clip == CLIP_VIDEO_A
    ));
}

#[test]
fn boundary_after_end_returns_out_of_bounds() {
    let prior = project_with_tracks(vec![video_track(vec![video_clip(CLIP_VIDEO_A, 10_000)])]);
    let err = compute_patch(&prior, &split_args(CLIP_VIDEO_A, 250_001)).expect_err("after end");
    assert!(
        matches!(err, ClipSplitError::ClipOutOfBounds { failed_clip, .. } if failed_clip == CLIP_VIDEO_A)
    );
}

#[test]
fn boundary_negative_returns_bad_time() {
    let prior = project_with_tracks(vec![video_track(vec![video_clip(CLIP_VIDEO_A, 0)])]);
    let err = compute_patch(&prior, &split_args(CLIP_VIDEO_A, -1)).expect_err("negative");
    assert!(matches!(
        err,
        ClipSplitError::BadTime {
            field: "at_tk",
            value: -1
        }
    ));
}

#[test]
fn missing_and_malformed_clip_selectors_error() {
    let prior = project_with_tracks(vec![video_track(vec![video_clip(CLIP_VIDEO_A, 0)])]);
    assert!(matches!(
        compute_patch(&prior, &split_args(MISSING_CLIP, 120_000)).expect_err("missing"),
        ClipSplitError::ClipNotFound { .. }
    ));
    assert!(matches!(
        compute_patch(&prior, &split_args("not-a-uuid", 120_000)).expect_err("bad selector"),
        ClipSplitError::BadSelector { .. }
    ));
}

#[test]
fn linked_video_audio_group_splits_all_members() {
    let prior = project_with_tracks(vec![
        video_track(vec![linked(video_clip(CLIP_VIDEO_A, 0))]),
        audio_track(vec![linked(audio_clip(CLIP_AUDIO_A, 0))]),
    ]);
    let (patch, warnings, data) =
        compute_patch(&prior, &split_args(CLIP_VIDEO_A, 120_000)).expect("linked split");
    let post = apply_patch(&prior, &patch);

    assert_eq!(visible_warning_codes(&warnings), Vec::<String>::new());
    assert_eq!(post.tracks[0].clips.len(), 2);
    assert_eq!(post.tracks[1].clips.len(), 2);
    assert_eq!(
        post.tracks[0].clips[0].link_group,
        Some(LINK_GROUP_ID.parse().unwrap())
    );
    assert_eq!(
        post.tracks[1].clips[0].link_group,
        Some(LINK_GROUP_ID.parse().unwrap())
    );
    assert_eq!(post.tracks[0].clips[1].link_group, data.right_link_group);
    assert_eq!(post.tracks[1].clips[1].link_group, data.right_link_group);
    assert_eq!(data.sibling_splits.len(), 1);
    assert_eq!(
        data.sibling_splits[0].source_clip_id,
        CLIP_AUDIO_A.parse().unwrap()
    );
}

#[test]
fn linked_group_semantics_mix_is_exempt() {
    let prior = project_with_tracks(vec![
        video_track(vec![linked(video_clip(CLIP_VIDEO_A, 0))]),
        second_video_track(vec![linked(image_clip(CLIP_IMAGE_A, 0))]),
    ]);
    let (patch, _warnings, _data) =
        compute_patch(&prior, &split_args(CLIP_VIDEO_A, 120_000)).expect("mixed group split");
    let post = apply_patch(&prior, &patch);

    assert_eq!(post.tracks[0].clips.len(), 2);
    assert_eq!(post.tracks[1].clips.len(), 2);
}

#[test]
fn fade_partition_assigns_curves_to_halves() {
    let prior = project_with_tracks(vec![video_track(vec![with_fades(
        video_clip(CLIP_VIDEO_A, 0),
        10_000,
        20_000,
    )])]);
    let (patch, _warnings, _data) =
        compute_patch(&prior, &split_args(CLIP_VIDEO_A, 120_000)).expect("split fades");
    let post = apply_patch(&prior, &patch);

    assert_eq!(post.tracks[0].clips[0].fade_in_tk.get(), 10_000);
    assert_eq!(post.tracks[0].clips[0].fade_out_tk.get(), 0);
    assert_eq!(
        serde_json::to_value(post.tracks[0].clips[0].fade_in_curve).unwrap(),
        json!("exp")
    );
    assert_eq!(
        serde_json::to_value(post.tracks[0].clips[0].fade_out_curve).unwrap(),
        json!("linear")
    );
    assert_eq!(post.tracks[0].clips[1].fade_in_tk.get(), 0);
    assert_eq!(post.tracks[0].clips[1].fade_out_tk.get(), 20_000);
    assert_eq!(
        serde_json::to_value(post.tracks[0].clips[1].fade_in_curve).unwrap(),
        json!("linear")
    );
    assert_eq!(
        serde_json::to_value(post.tracks[0].clips[1].fade_out_curve).unwrap(),
        json!("log")
    );
}

#[test]
fn keyframes_partition_and_right_side_rebases_with_new_ids() {
    let prior = project_with_tracks(vec![video_track(vec![with_keyframes(
        video_clip(CLIP_VIDEO_A, 0),
        vec![keyframe(KEYFRAME_A, 60_000), keyframe(KEYFRAME_B, 120_000)],
    )])]);
    let (patch, _warnings, _data) =
        compute_patch(&prior, &split_args(CLIP_VIDEO_A, 120_000)).expect("split keyframes");
    let post = apply_patch(&prior, &patch);

    assert_eq!(post.tracks[0].clips[0].keyframes.len(), 1);
    assert_eq!(
        post.tracks[0].clips[0].keyframes[0].id,
        KEYFRAME_A.parse().unwrap()
    );
    assert_eq!(post.tracks[0].clips[0].keyframes[0].time_tk.get(), 60_000);
    assert_eq!(post.tracks[0].clips[1].keyframes.len(), 1);
    assert_ne!(
        post.tracks[0].clips[1].keyframes[0].id,
        KEYFRAME_B.parse().unwrap()
    );
    assert_eq!(post.tracks[0].clips[1].keyframes[0].time_tk.get(), 0);
}

#[test]
fn fade_clamps_when_inherited_fade_exceeds_half_duration() {
    let prior = project_with_tracks(vec![video_track(vec![with_fades(
        video_clip(CLIP_VIDEO_A, 0),
        180_000,
        180_000,
    )])]);
    let (patch, warnings, _data) =
        compute_patch(&prior, &split_args(CLIP_VIDEO_A, 120_000)).expect("fade clamp");
    let post = apply_patch(&prior, &patch);

    assert_eq!(
        visible_warning_codes(&warnings),
        vec![W_FADE_CLAMPED_CODE, W_FADE_CLAMPED_CODE]
    );
    assert_eq!(post.tracks[0].clips[0].fade_in_tk.get(), 120_000);
    assert_eq!(post.tracks[0].clips[1].fade_out_tk.get(), 120_000);
}

#[test]
fn keyframe_overflow_removes_rebased_right_keyframe_defensively() {
    let prior = project_with_tracks(vec![video_track(vec![with_keyframes(
        video_clip(CLIP_VIDEO_A, 0),
        vec![keyframe(KEYFRAME_A, 250_000)],
    )])]);
    let (patch, warnings, _data) =
        compute_patch(&prior, &split_args(CLIP_VIDEO_A, 120_000)).expect("keyframe overflow");
    let post = apply_patch(&prior, &patch);

    assert_eq!(
        visible_warning_codes(&warnings),
        vec![W_KEYFRAMES_REMOVED_CODE]
    );
    assert_eq!(post.tracks[0].clips[1].keyframes.len(), 0);
}

#[test]
fn locked_target_parent_track_and_linked_sibling_error() {
    let locked_target =
        project_with_tracks(vec![video_track(vec![locked(video_clip(CLIP_VIDEO_A, 0))])]);
    assert!(matches!(
        compute_patch(&locked_target, &split_args(CLIP_VIDEO_A, 120_000)).expect_err("target"),
        ClipSplitError::Locked { failed_clip } if failed_clip == CLIP_VIDEO_A
    ));

    let locked_track = project_with_tracks(vec![track_locked(video_track(vec![video_clip(
        CLIP_VIDEO_A,
        0,
    )]))]);
    assert!(matches!(
        compute_patch(&locked_track, &split_args(CLIP_VIDEO_A, 120_000)).expect_err("track"),
        ClipSplitError::Locked { failed_clip } if failed_clip == CLIP_VIDEO_A
    ));

    let locked_sibling = project_with_tracks(vec![
        video_track(vec![linked(video_clip(CLIP_VIDEO_A, 0))]),
        audio_track(vec![linked(locked(audio_clip(CLIP_AUDIO_A, 0)))]),
    ]);
    assert!(matches!(
        compute_patch(&locked_sibling, &split_args(CLIP_VIDEO_A, 120_000)).expect_err("sibling"),
        ClipSplitError::Locked { failed_clip } if failed_clip == CLIP_AUDIO_A
    ));
}

#[test]
fn linked_sibling_bounds_mismatch_aborts_atomically() {
    let prior = project_with_tracks(vec![
        video_track(vec![linked(video_clip(CLIP_VIDEO_A, 0))]),
        audio_track(vec![linked(with_source_out(
            audio_clip(CLIP_AUDIO_A, 0),
            100_000,
        ))]),
    ]);
    let err = compute_patch(&prior, &split_args(CLIP_VIDEO_A, 120_000))
        .expect_err("sibling out of bounds");

    assert!(matches!(
        err,
        ClipSplitError::ClipOutOfBounds { failed_clip, .. } if failed_clip == CLIP_AUDIO_A
    ));
}

#[test]
fn data_envelope_has_sibling_splits_and_unlinked_group_nulls() {
    let linked_prior = project_with_tracks(vec![
        video_track(vec![linked(video_clip(CLIP_VIDEO_A, 0))]),
        audio_track(vec![linked(audio_clip(CLIP_AUDIO_A, 0))]),
    ]);
    let (_patch, _warnings, linked_data) =
        compute_patch(&linked_prior, &split_args(CLIP_VIDEO_A, 120_000)).expect("linked split");
    assert_eq!(linked_data.sibling_splits.len(), 1);
    assert_eq!(
        linked_data.left_link_group,
        Some(LINK_GROUP_ID.parse().unwrap())
    );
    assert!(linked_data.right_link_group.is_some());

    let unlinked_prior = project_with_tracks(vec![video_track(vec![video_clip(CLIP_VIDEO_A, 0)])]);
    let (_patch, _warnings, unlinked_data) =
        compute_patch(&unlinked_prior, &split_args(CLIP_VIDEO_A, 120_000)).expect("unlinked");
    assert!(unlinked_data.left_link_group.is_none());
    assert!(unlinked_data.right_link_group.is_none());
    assert!(unlinked_data.sibling_splits.is_empty());
}

#[test]
fn reconstructor_round_trip_singleton() {
    let prior = project_with_tracks(vec![video_track(vec![video_clip(CLIP_VIDEO_A, 0)])]);
    let args = split_args(CLIP_VIDEO_A, 120_000);
    let (patch, warnings, _data) = compute_patch(&prior, &args).expect("split");
    let post = apply_patch(&prior, &patch);
    let expected_data = serde_json::to_value(
        data_envelope_from_args_warnings(&args, &warnings).expect("warning envelope"),
    )
    .expect("data serializes");
    let recorded = RecordedEvent {
        verb: "clip.split".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch,
        warnings,
        post_state: post,
        expected_data,
    };

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(ClipSplitVerb))
        .expect("register clip.split");
    let report = validate_reconstructors(&registry, &[recorded]).expect("round trip");
    assert_eq!(report.verbs_checked, vec!["clip.split"]);
}

#[test]
fn reconstructor_round_trip_linked_group() {
    let prior = project_with_tracks(vec![
        video_track(vec![linked(video_clip(CLIP_VIDEO_A, 0))]),
        audio_track(vec![linked(audio_clip(CLIP_AUDIO_A, 0))]),
    ]);
    let args = split_args(CLIP_VIDEO_A, 120_000);
    let (patch, warnings, _data) = compute_patch(&prior, &args).expect("split linked");
    let post = apply_patch(&prior, &patch);
    let expected_data = serde_json::to_value(
        data_envelope_from_args_warnings(&args, &warnings).expect("warning envelope"),
    )
    .expect("data serializes");
    let recorded = RecordedEvent {
        verb: "clip.split".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch,
        warnings,
        post_state: post,
        expected_data,
    };

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(ClipSplitVerb))
        .expect("register clip.split");
    let report = validate_reconstructors(&registry, &[recorded]).expect("round trip");
    assert_eq!(report.verbs_checked, vec!["clip.split"]);
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "clip.split")
        .expect("default_fixtures includes clip.split");
    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(ClipSplitVerb))
        .expect("register clip.split");
    let report = validate_reconstructors(&registry, &[fixture]).expect("default fixture");
    assert_eq!(report.verbs_checked, vec!["clip.split"]);
}

#[test]
fn effects_are_deep_copied_to_right_half_with_same_effect_id() {
    let prior = project_with_tracks(vec![video_track(vec![with_effect(video_clip(
        CLIP_VIDEO_A,
        0,
    ))])]);
    let (patch, _warnings, _data) =
        compute_patch(&prior, &split_args(CLIP_VIDEO_A, 120_000)).expect("split effect clip");
    let post = apply_patch(&prior, &patch);

    assert_eq!(post.tracks[0].clips[0].effects.len(), 1);
    assert_eq!(post.tracks[0].clips[1].effects.len(), 1);
    assert_eq!(
        post.tracks[0].clips[0].effects[0].id,
        EFFECT_A.parse().unwrap()
    );
    assert_eq!(
        post.tracks[0].clips[1].effects[0].id,
        EFFECT_A.parse().unwrap()
    );
    assert_eq!(
        post.tracks[0].clips[0].effects,
        post.tracks[0].clips[1].effects
    );
}

#[test]
fn verb_trait_returns_data_and_internal_envelope_warning() {
    let prior = project_with_tracks(vec![video_track(vec![video_clip(CLIP_VIDEO_A, 0)])]);
    let verb = ClipSplitVerb;
    let (_patch, data, warnings) = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "clip": CLIP_VIDEO_A,
                "at_tk": 120_000,
            }),
        )
        .expect("verb route");

    let data: ClipSplitData = serde_json::from_value(data).expect("data envelope");
    assert_eq!(data.left_clip_id, CLIP_VIDEO_A.parse().unwrap());
    assert_eq!(
        warnings.last().and_then(|warning| warning["code"].as_str()),
        Some(W_CLIP_SPLIT_ENVELOPE_CODE)
    );
}

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        project_with_tracks(vec![video_track(vec![video_clip(CLIP_VIDEO_A, 0)])]),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry clears the gate");

    let outcome = store
        .mutate_via_verb(
            "clip.split",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "clip": CLIP_VIDEO_A,
                "at_tk": 120_000,
            }),
            None,
        )
        .expect("mutate_via_verb happy path");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("happy path must return Applied, got {outcome:?}");
    };

    let data: ClipSplitData = serde_json::from_value(data).expect("clip.split data");
    assert_eq!(store.project().tracks[0].clips.len(), 2);
    assert_eq!(store.project().tracks[0].clips[1].id, data.right_clip_id);
    assert_eq!(visible_warning_codes(&warnings), Vec::<String>::new());
}

#[test]
fn speed_split_uses_straightforward_source_window_math() {
    let prior = project_with_tracks(vec![video_track(vec![with_speed(
        video_clip(CLIP_VIDEO_A, 0),
        2.0,
    )])]);
    let (patch, _warnings, _data) =
        compute_patch(&prior, &split_args(CLIP_VIDEO_A, 60_000)).expect("speed split");
    let post = apply_patch(&prior, &patch);

    assert_eq!(post.tracks[0].clips[0].source_out_tk.get(), 120_000);
    assert_eq!(post.tracks[0].clips[1].source_in_tk.get(), 120_000);
}
