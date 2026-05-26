//! Tests for `clip.trim` (§5.2) — forty-seventh production verb.

use serde_json::{Value, json};
use std::sync::Arc;

use verbreel_state::verbs::clip_trim::{
    ARGS_INCOMPATIBLE_HINT, LINK_GROUP_SEMANTICS_MIX_HINT, W_FADE_CLAMPED_CODE,
    W_KEYFRAMES_REMOVED_CODE, W_NOOP_FLAG_CODE, W_TIME_SNAPPED_CODE, compute_patch,
    data_envelope_from_post_state,
};
use verbreel_state::{
    ClipTrimArgs, ClipTrimData, ClipTrimError, ClipTrimVerb, MutateOutcome, Project, RecordedEvent,
    TrackKind, Verb, VerbError, VerbRegistry, default_fixtures, default_registry,
    validate_reconstructors,
};
use verbreel_types::{ProjectId, Tick};

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const TRACK_VIDEO_A: &str = "01900000-0000-7000-8000-0000000aa701";
const TRACK_AUDIO_A: &str = "01900000-0000-7000-8000-0000000aa702";
const TRACK_TEXT_A: &str = "01900000-0000-7000-8000-0000000aa703";
const CLIP_VIDEO_A: &str = "01900000-0000-7000-8000-0000000bb701";
const CLIP_AUDIO_A: &str = "01900000-0000-7000-8000-0000000bb703";
const CLIP_AUDIO_B: &str = "01900000-0000-7000-8000-0000000bb704";
const CLIP_IMAGE_A: &str = "01900000-0000-7000-8000-0000000bb705";
const CLIP_TEXT_A: &str = "01900000-0000-7000-8000-0000000bb706";
const MISSING_CLIP: &str = "01900000-0000-7000-8000-0000000cc701";
const LINK_GROUP_ID: &str = "01900000-0000-7000-8000-0000000dd701";
const ASSET_VIDEO_ID: &str = "01900000-0000-7000-8000-0000000ee701";
const ASSET_AUDIO_ID: &str = "01900000-0000-7000-8000-0000000ee702";
const ASSET_IMAGE_ID: &str = "01900000-0000-7000-8000-0000000ee703";
const KEYFRAME_A: &str = "01900000-0000-7000-8000-0000000ff701";
const KEYFRAME_B: &str = "01900000-0000-7000-8000-0000000ff702";

#[derive(Debug, Clone)]
struct ClipFixture {
    id: &'static str,
    position_tk: i64,
    source_in_tk: i64,
    source_out_tk: i64,
    locked: bool,
    link_group: Option<&'static str>,
    asset_id: &'static str,
    fade_in_tk: i64,
    fade_out_tk: i64,
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

fn trim_args(
    clip: &str,
    source_in_tk: Option<i64>,
    source_out_tk: Option<i64>,
    keep_end: Option<bool>,
) -> ClipTrimArgs {
    ClipTrimArgs {
        project_id: fixture_project_id(),
        clip: clip.to_string(),
        source_in_tk,
        source_out_tk,
        keep_end,
    }
}

fn clip(id: &'static str, asset_id: &'static str, position_tk: i64) -> ClipFixture {
    ClipFixture {
        id,
        position_tk,
        source_in_tk: 0,
        source_out_tk: 240_000,
        locked: false,
        link_group: None,
        asset_id,
        fade_in_tk: 0,
        fade_out_tk: 0,
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

fn with_fades(mut fixture: ClipFixture, fade_in_tk: i64, fade_out_tk: i64) -> ClipFixture {
    fixture.fade_in_tk = fade_in_tk;
    fixture.fade_out_tk = fade_out_tk;
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

fn audio_track(clips: Vec<ClipFixture>) -> TrackFixture {
    TrackFixture {
        id: TRACK_AUDIO_A,
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
        "locked": fixture.locked,
        "fade_in_tk": fixture.fade_in_tk,
        "fade_out_tk": fixture.fade_out_tk,
        "keyframes": fixture.keyframes,
    });
    if kind == TrackKind::Audio {
        value["volume"] = json!(1.0);
    }
    if kind == TrackKind::Text {
        value["text"] = json!({
            "content": "Trim",
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
    let duration_tk = tracks
        .iter()
        .flat_map(|track| track.clips.iter())
        .map(|clip| clip.position_tk + clip.source_out_tk - clip.source_in_tk)
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
        "original_filename": "clip-trim.mp4",
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
        "original_filename": "clip-trim.m4a",
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
        .expect("clip.trim patch applies cleanly")
}

fn keyframe(id: &str, time_tk: i64) -> Value {
    json!({
        "id": id,
        "property": "opacity",
        "time_tk": time_tk,
        "value": 0.5,
    })
}

fn warning_codes(warnings: &[Value]) -> Vec<String> {
    warnings
        .iter()
        .map(|warning| warning["code"].as_str().expect("warning code").to_string())
        .collect()
}

#[test]
fn errors_when_no_source_field_supplied() {
    let prior = project_with_tracks(vec![video_track(vec![video_clip(CLIP_VIDEO_A, 0)])]);
    let err = compute_patch(&prior, &trim_args(CLIP_VIDEO_A, None, None, None))
        .expect_err("missing fields");
    assert!(matches!(
        err,
        ClipTrimError::ArgsIncompatible {
            hint: ARGS_INCOMPATIBLE_HINT
        }
    ));
}

#[test]
fn happy_path_source_out_shrink_singleton() {
    let prior = project_with_tracks(vec![video_track(vec![video_clip(CLIP_VIDEO_A, 0)])]);
    let (patch, warnings, data) =
        compute_patch(&prior, &trim_args(CLIP_VIDEO_A, None, Some(120_000), None))
            .expect("trim source_out");
    let post = apply_patch(&prior, &patch);

    assert!(warnings.is_empty());
    assert_eq!(post.tracks[0].clips[0].source_out_tk.get(), 120_000);
    assert_eq!(post.duration_tk.get(), 120_000);
    assert_eq!(data.duration_tk, 120_000);
}

#[test]
fn happy_path_source_in_grow_singleton() {
    let prior = project_with_tracks(vec![video_track(vec![video_clip(CLIP_VIDEO_A, 0)])]);
    let (patch, warnings, data) =
        compute_patch(&prior, &trim_args(CLIP_VIDEO_A, Some(8_000), None, None))
            .expect("trim source_in");
    let post = apply_patch(&prior, &patch);

    assert!(warnings.is_empty());
    assert_eq!(post.tracks[0].clips[0].source_in_tk.get(), 8_000);
    assert_eq!(post.tracks[0].clips[0].track_position_tk.get(), 0);
    assert_eq!(data.duration_tk, 232_000);
}

#[test]
fn happy_path_both_source_fields_supplied() {
    let prior = project_with_tracks(vec![video_track(vec![video_clip(CLIP_VIDEO_A, 0)])]);
    let (patch, warnings, data) = compute_patch(
        &prior,
        &trim_args(CLIP_VIDEO_A, Some(8_000), Some(120_000), None),
    )
    .expect("trim both");
    let post = apply_patch(&prior, &patch);

    assert!(warnings.is_empty());
    assert_eq!(post.tracks[0].clips[0].source_in_tk.get(), 8_000);
    assert_eq!(post.tracks[0].clips[0].source_out_tk.get(), 120_000);
    assert_eq!(data.duration_tk, 112_000);
}

#[test]
fn keep_end_true_with_source_in_change_shifts_track_position() {
    let prior = project_with_tracks(vec![video_track(vec![video_clip(CLIP_VIDEO_A, 0)])]);
    let (patch, warnings, data) = compute_patch(
        &prior,
        &trim_args(CLIP_VIDEO_A, Some(8_000), None, Some(true)),
    )
    .expect("keep end");
    let post = apply_patch(&prior, &patch);

    assert!(warnings.is_empty());
    assert_eq!(post.tracks[0].clips[0].track_position_tk.get(), 8_000);
    assert_eq!(data.track_position_tk, 8_000);
}

#[test]
fn keep_end_true_with_source_out_supplied_warns_and_does_not_shift() {
    let prior = project_with_tracks(vec![video_track(vec![video_clip(CLIP_VIDEO_A, 0)])]);
    let (patch, warnings, _data) = compute_patch(
        &prior,
        &trim_args(CLIP_VIDEO_A, Some(8_000), Some(120_000), Some(true)),
    )
    .expect("keep end ignored");
    let post = apply_patch(&prior, &patch);

    assert_eq!(warning_codes(&warnings), vec![W_NOOP_FLAG_CODE]);
    assert_eq!(warnings[0]["details"]["flag"], "keep_end");
    assert_eq!(post.tracks[0].clips[0].track_position_tk.get(), 0);
}

#[test]
fn linked_video_audio_group_shifts_both_source_windows() {
    let prior = project_with_tracks(vec![
        video_track(vec![linked(video_clip(CLIP_VIDEO_A, 0))]),
        audio_track(vec![linked(audio_clip(CLIP_AUDIO_A, 0))]),
    ]);
    let (patch, warnings, data) = compute_patch(
        &prior,
        &trim_args(CLIP_VIDEO_A, Some(8_000), Some(120_000), None),
    )
    .expect("linked trim");
    let post = apply_patch(&prior, &patch);

    assert!(warnings.is_empty());
    assert_eq!(post.tracks[0].clips[0].source_in_tk.get(), 8_000);
    assert_eq!(post.tracks[1].clips[0].source_in_tk.get(), 8_000);
    assert_eq!(post.tracks[0].clips[0].source_out_tk.get(), 120_000);
    assert_eq!(post.tracks[1].clips[0].source_out_tk.get(), 120_000);
    assert_eq!(data.linked_clip_ids, vec![CLIP_AUDIO_A.parse().unwrap()]);
}

#[test]
fn linked_group_video_member_out_of_bounds_aborts_atomically() {
    let prior = project_with_tracks(vec![
        video_track(vec![linked(video_clip(CLIP_VIDEO_A, 0))]),
        audio_track(vec![linked(audio_clip(CLIP_AUDIO_A, 0))]),
    ]);
    let err = compute_patch(&prior, &trim_args(CLIP_AUDIO_A, None, Some(500_000), None))
        .expect_err("video sibling exceeds asset bound");

    assert!(matches!(
        err,
        ClipTrimError::ClipOutOfBounds {
            failed_clip,
            bound_min: 0,
            bound_max: 480_000,
            proposed_in: 0,
            proposed_out: 500_000,
        } if failed_clip == CLIP_VIDEO_A
    ));
}

#[test]
fn linked_group_overlap_aborts_atomically() {
    let prior = project_with_tracks(vec![
        video_track(vec![linked(video_clip(CLIP_VIDEO_A, 0))]),
        audio_track(vec![
            linked(audio_clip(CLIP_AUDIO_A, 0)),
            audio_clip(CLIP_AUDIO_B, 250_000),
        ]),
    ]);
    let err = compute_patch(&prior, &trim_args(CLIP_VIDEO_A, None, Some(300_000), None))
        .expect_err("audio sibling would overlap");

    assert!(matches!(
        err,
        ClipTrimError::ClipOverlap { failed_clip } if failed_clip == CLIP_AUDIO_A
    ));
}

#[test]
fn missing_clip_returns_not_found() {
    let prior = project_with_tracks(vec![video_track(vec![video_clip(CLIP_VIDEO_A, 0)])]);
    let err = compute_patch(&prior, &trim_args(MISSING_CLIP, None, Some(120_000), None))
        .expect_err("missing clip");
    assert!(matches!(err, ClipTrimError::ClipNotFound { .. }));
}

#[test]
fn malformed_uuid_returns_bad_selector() {
    let prior = project_with_tracks(vec![video_track(vec![video_clip(CLIP_VIDEO_A, 0)])]);
    let err = compute_patch(&prior, &trim_args("not-a-uuid", None, Some(120_000), None))
        .expect_err("bad selector");
    assert!(matches!(
        err,
        ClipTrimError::BadSelector { field: "clip", .. }
    ));
}

#[test]
fn negative_source_in_returns_bad_time() {
    let prior = project_with_tracks(vec![video_track(vec![video_clip(CLIP_VIDEO_A, 0)])]);
    let err = compute_patch(&prior, &trim_args(CLIP_VIDEO_A, Some(-1), None, None))
        .expect_err("negative source_in");
    assert!(matches!(
        err,
        ClipTrimError::BadTime {
            field: "source_in_tk",
            value: -1
        }
    ));
}

#[test]
fn source_in_greater_or_equal_source_out_returns_bad_time() {
    let prior = project_with_tracks(vec![video_track(vec![video_clip(CLIP_VIDEO_A, 0)])]);
    let err = compute_patch(&prior, &trim_args(CLIP_VIDEO_A, Some(240_000), None, None))
        .expect_err("degenerate source range");
    assert!(matches!(err, ClipTrimError::BadTime { .. }));
}

#[test]
fn locked_target_clip_returns_locked() {
    let prior = project_with_tracks(vec![video_track(vec![locked(video_clip(CLIP_VIDEO_A, 0))])]);
    let err = compute_patch(&prior, &trim_args(CLIP_VIDEO_A, None, Some(120_000), None))
        .expect_err("locked target");
    assert!(matches!(
        err,
        ClipTrimError::Locked { failed_clip } if failed_clip == CLIP_VIDEO_A
    ));
}

#[test]
fn locked_parent_track_returns_locked() {
    let prior = project_with_tracks(vec![track_locked(video_track(vec![video_clip(
        CLIP_VIDEO_A,
        0,
    )]))]);
    let err = compute_patch(&prior, &trim_args(CLIP_VIDEO_A, None, Some(120_000), None))
        .expect_err("locked track");
    assert!(matches!(
        err,
        ClipTrimError::Locked { failed_clip } if failed_clip == CLIP_VIDEO_A
    ));
}

#[test]
fn locked_linked_sibling_returns_locked() {
    let prior = project_with_tracks(vec![
        video_track(vec![linked(video_clip(CLIP_VIDEO_A, 0))]),
        audio_track(vec![linked(locked(audio_clip(CLIP_AUDIO_A, 0)))]),
    ]);
    let err = compute_patch(&prior, &trim_args(CLIP_VIDEO_A, None, Some(120_000), None))
        .expect_err("locked sibling");
    assert!(matches!(
        err,
        ClipTrimError::Locked { failed_clip } if failed_clip == CLIP_AUDIO_A
    ));
}

#[test]
fn video_clip_out_of_bounds_on_asset_duration() {
    let prior = project_with_tracks(vec![video_track(vec![video_clip(CLIP_VIDEO_A, 0)])]);
    let err = compute_patch(&prior, &trim_args(CLIP_VIDEO_A, None, Some(500_000), None))
        .expect_err("out of asset duration");
    assert!(matches!(
        err,
        ClipTrimError::ClipOutOfBounds {
            failed_clip,
            bound_max: 480_000,
            proposed_out: 500_000,
            ..
        } if failed_clip == CLIP_VIDEO_A
    ));
}

#[test]
fn linked_group_semantics_mix_errors() {
    let prior = project_with_tracks(vec![video_track(vec![
        linked(video_clip(CLIP_VIDEO_A, 0)),
        linked(image_clip(CLIP_IMAGE_A, 240_000)),
    ])]);
    let err = compute_patch(&prior, &trim_args(CLIP_VIDEO_A, None, Some(120_000), None))
        .expect_err("mixed semantics");
    assert!(matches!(
        err,
        ClipTrimError::LinkGroupSemanticsMix {
            hint: LINK_GROUP_SEMANTICS_MIX_HINT,
            ..
        }
    ));
}

#[test]
fn fade_clamps_when_new_duration_is_shorter_than_fade_sum() {
    let prior = project_with_tracks(vec![video_track(vec![with_fades(
        video_clip(CLIP_VIDEO_A, 0),
        5_000,
        5_000,
    )])]);
    let (patch, warnings, _data) =
        compute_patch(&prior, &trim_args(CLIP_VIDEO_A, None, Some(8_000), None))
            .expect("fade clamp");
    let post = apply_patch(&prior, &patch);

    assert_eq!(warning_codes(&warnings), vec![W_FADE_CLAMPED_CODE]);
    assert_eq!(post.tracks[0].clips[0].fade_in_tk.get(), 4_000);
    assert_eq!(post.tracks[0].clips[0].fade_out_tk.get(), 4_000);
}

#[test]
fn keyframes_beyond_new_duration_are_removed() {
    let prior = project_with_tracks(vec![video_track(vec![with_keyframes(
        video_clip(CLIP_VIDEO_A, 0),
        vec![keyframe(KEYFRAME_A, 1_000), keyframe(KEYFRAME_B, 2_000)],
    )])]);
    let (patch, warnings, _data) =
        compute_patch(&prior, &trim_args(CLIP_VIDEO_A, None, Some(1_000), None))
            .expect("keyframe cascade");
    let post = apply_patch(&prior, &patch);

    assert_eq!(warning_codes(&warnings), vec![W_KEYFRAMES_REMOVED_CODE]);
    assert_eq!(
        warnings[0]["details"]["removed_keyframe_ids"],
        json!([KEYFRAME_B])
    );
    assert_eq!(post.tracks[0].clips[0].keyframes.len(), 1);
}

#[test]
fn time_snapped_on_keep_end_source_in_change() {
    let prior = project_with_tracks(vec![video_track(vec![video_clip(CLIP_VIDEO_A, 0)])]);
    let (patch, warnings, _data) = compute_patch(
        &prior,
        &trim_args(CLIP_VIDEO_A, Some(1_000), None, Some(true)),
    )
    .expect("snap keep_end position");
    let post = apply_patch(&prior, &patch);

    assert_eq!(warning_codes(&warnings), vec![W_TIME_SNAPPED_CODE]);
    assert_eq!(warnings[0]["details"]["from_tk"], 1_000);
    assert_eq!(warnings[0]["details"]["to_tk"], 0);
    assert_eq!(post.tracks[0].clips[0].track_position_tk.get(), 0);
}

#[test]
fn image_and_text_source_in_remains_zero_when_trimming_duration() {
    let prior = project_with_tracks(vec![
        video_track(vec![image_clip(CLIP_IMAGE_A, 0)]),
        text_track(vec![text_clip(CLIP_TEXT_A, 0)]),
    ]);
    let (image_patch, _warnings, _data) =
        compute_patch(&prior, &trim_args(CLIP_IMAGE_A, None, Some(120_000), None))
            .expect("image trim");
    let image_post = apply_patch(&prior, &image_patch);
    let (text_patch, _warnings, _data) = compute_patch(
        &image_post,
        &trim_args(CLIP_TEXT_A, None, Some(120_000), None),
    )
    .expect("text trim");
    let text_post = apply_patch(&image_post, &text_patch);

    assert_eq!(text_post.tracks[0].clips[0].source_in_tk.get(), 0);
    assert_eq!(text_post.tracks[1].clips[0].source_in_tk.get(), 0);
    assert_eq!(text_post.tracks[0].clips[0].source_out_tk.get(), 120_000);
    assert_eq!(text_post.tracks[1].clips[0].source_out_tk.get(), 120_000);
}

#[test]
fn reconstructor_round_trip() {
    let prior = project_with_tracks(vec![video_track(vec![video_clip(CLIP_VIDEO_A, 0)])]);
    let args = trim_args(CLIP_VIDEO_A, Some(8_000), Some(120_000), None);
    let (patch, warnings, _data) = compute_patch(&prior, &args).expect("trim");
    let post = apply_patch(&prior, &patch);
    let expected_data = serde_json::to_value(
        data_envelope_from_post_state(&args, &post).expect("post-state envelope"),
    )
    .expect("data serializes");
    let recorded = RecordedEvent {
        verb: "clip.trim".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch,
        warnings,
        post_state: post,
        expected_data,
    };

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(ClipTrimVerb))
        .expect("register clip.trim");
    let report = validate_reconstructors(&registry, &[recorded]).expect("round trip");
    assert_eq!(report.verbs_checked, vec!["clip.trim"]);
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "clip.trim")
        .expect("default_fixtures includes clip.trim");
    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(ClipTrimVerb))
        .expect("register clip.trim");
    let report = validate_reconstructors(&registry, &[fixture]).expect("default fixture");
    assert_eq!(report.verbs_checked, vec!["clip.trim"]);
}

#[test]
fn verb_trait_returns_data_and_warnings() {
    let prior = project_with_tracks(vec![video_track(vec![video_clip(CLIP_VIDEO_A, 0)])]);
    let verb = ClipTrimVerb;
    let (patch, data, warnings) = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "clip": CLIP_VIDEO_A,
                "source_out_tk": 120_000,
            }),
        )
        .expect("verb route");

    let data: ClipTrimData = serde_json::from_value(data).expect("data envelope");
    assert_eq!(patch.0.len(), 2);
    assert_eq!(data.duration_tk, 120_000);
    assert!(warnings.is_empty());
}

#[test]
fn verb_trait_maps_errors_to_bad_args() {
    let prior = project_with_tracks(vec![video_track(vec![video_clip(CLIP_VIDEO_A, 0)])]);
    let verb = ClipTrimVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "clip": CLIP_VIDEO_A,
            }),
        )
        .expect_err("bad args");

    assert!(matches!(
        err,
        VerbError::BadArgs { detail } if detail.contains("E_ARGS_INCOMPATIBLE")
    ));
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
            "clip.trim",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "clip": CLIP_VIDEO_A,
                "source_out_tk": 120_000,
            }),
            None,
        )
        .expect("mutate_via_verb happy path");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("happy path must return Applied, got {outcome:?}");
    };

    let data: ClipTrimData = serde_json::from_value(data).expect("clip.trim data");
    assert_eq!(
        store.project().tracks[0].clips[0].source_out_tk.get(),
        120_000
    );
    assert_eq!(data.duration_tk, 120_000);
    assert_eq!(warnings, Vec::<Value>::new());
}
