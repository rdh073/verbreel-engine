//! Tests for `text.animate` (§7.4) — thirty-sixth production verb.

use std::collections::HashSet;
use std::sync::Arc;

use serde_json::{Value, json};
use tempfile::TempDir;
use verbreel_state::ProjectStore;
use verbreel_state::verbs::text_animate::{
    TextAnimateArgs, TextAnimateData, TextAnimateError, TextAnimateVerb, W_KEYFRAMES_DEDUPED,
    W_PRESET_KEYFRAMES_CLAMPED, compute_patch, data_envelope_from_args_patch_warnings, presets,
};
use verbreel_state::{
    Easing, Keyframe, MutateOutcome, Project, TextElement, Track, TrackKind, VerbRegistry,
    default_fixtures, default_registry, validate_reconstructors,
};
use verbreel_types::{KeyframeId, ProjectId, Tick};

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const TRACK_TEXT_A: &str = "0190b8d3-15e3-7000-bd00-0000000aa101";
const TRACK_VIDEO_A: &str = "0190b8d3-15e3-7000-bd00-0000000aa201";
const CLIP_TEXT_A: &str = "0190b8d3-15e3-7000-bd00-0000000bb101";
const CLIP_VIDEO_A: &str = "0190b8d3-15e3-7000-bd00-0000000bb201";
const ASSET_VIDEO_ID: &str = "0190b8d3-15e3-7000-bd00-0000000dd001";
const EXISTING_KEYFRAME_ID: &str = "0190b8d3-15e3-7000-bd00-0000000ee001";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture -> Project")
}

fn fixture_project_id() -> ProjectId {
    FIXTURE_PROJECT_ID
        .parse()
        .expect("fixture project id parses")
}

fn base_text() -> TextElement {
    TextElement {
        content: "Hello".to_string(),
        font_family: "Arial".to_string(),
        font_size_px: 24.0,
        ..TextElement::default()
    }
}

fn text_track(duration_tk: i64, locked: bool, clip_locked: bool) -> Track {
    text_track_with_keyframes(duration_tk, locked, clip_locked, json!([]))
}

fn text_track_with_keyframes(
    duration_tk: i64,
    locked: bool,
    clip_locked: bool,
    keyframes: Value,
) -> Track {
    serde_json::from_value(json!({
        "id": TRACK_TEXT_A,
        "kind": TrackKind::Text,
        "name": "Text 1",
        "locked": locked,
        "clips": [{
            "id": CLIP_TEXT_A,
            "name": "Text Clip",
            "asset_id": "00000000-0000-0000-0000-000000000000",
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": duration_tk,
            "locked": clip_locked,
            "keyframes": keyframes,
            "text": base_text(),
        }],
    }))
    .expect("text track fixture parses")
}

fn video_track() -> Track {
    serde_json::from_value(json!({
        "id": TRACK_VIDEO_A,
        "kind": TrackKind::Video,
        "name": "Video 1",
        "locked": false,
        "clips": [{
            "id": CLIP_VIDEO_A,
            "name": "Video Clip",
            "asset_id": ASSET_VIDEO_ID,
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 100,
            "locked": false,
        }],
    }))
    .expect("video track fixture parses")
}

fn video_asset() -> Value {
    json!({
        "id": ASSET_VIDEO_ID,
        "kind": "video",
        "hash": "53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658",
        "path": "assets/53/53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.mp4",
        "original_filename": "text-animate-video.mp4",
        "imported_at": "2026-05-24T00:00:00Z",
        "metadata": {
            "duration_tk": 100,
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

fn project_with_tracks(tracks: Vec<Track>) -> Project {
    let mut project = empty_project();
    project.tracks = tracks;
    project.assets.clear();
    project.duration_tk = Tick::new(100);
    project
}

fn project_with_video_track() -> Project {
    let mut project = project_with_tracks(vec![video_track()]);
    project
        .assets
        .push(serde_json::from_value(video_asset()).expect("asset fixture parses"));
    project
}

fn project(duration_tk: i64) -> Project {
    let mut project = project_with_tracks(vec![text_track(duration_tk, false, false)]);
    project.duration_tk = Tick::new(duration_tk);
    project
}

fn args(preset: &str) -> TextAnimateArgs {
    TextAnimateArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_A.to_string(),
        preset: preset.to_string(),
        in_tk: Some(0),
        out_tk: Some(99),
    }
}

fn args_window(preset: &str, in_tk: Option<i64>, out_tk: Option<i64>) -> TextAnimateArgs {
    TextAnimateArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_A.to_string(),
        preset: preset.to_string(),
        in_tk,
        out_tk,
    }
}

fn apply_patch(prior: &Project, patch: Value) -> Project {
    let patch: json_patch::Patch = serde_json::from_value(patch).expect("patch parses");
    prior.apply(&patch).expect("patch applies")
}

fn patch_keyframes(patch: &Value) -> Vec<Keyframe> {
    patch
        .as_array()
        .expect("patch is array")
        .iter()
        .map(|op| {
            assert_eq!(op["op"], "add");
            assert_eq!(op["path"], "/tracks/0/clips/0/keyframes/-");
            serde_json::from_value(op["value"].clone()).expect("patch value is keyframe")
        })
        .collect()
}

fn warning<'a>(warnings: &'a [Value], code: &str) -> &'a Value {
    warnings
        .iter()
        .find(|warning| warning["code"] == code)
        .expect("warning exists")
}

fn assert_preset_smoke(preset: &str) {
    let prior = project(100);
    let template = presets::get(preset).expect("registered preset");
    let (patch, warnings, _data) = compute_patch(&prior, &args(preset)).expect("happy path");
    let keyframes = patch_keyframes(&patch);
    let expected_properties: Vec<&str> = template.iter().map(|entry| entry.property).collect();
    let actual_properties: Vec<&str> = keyframes
        .iter()
        .map(|keyframe| keyframe.property.as_str())
        .collect();

    assert!(warnings.is_empty());
    assert_eq!(keyframes.len(), template.len());
    assert_eq!(actual_properties, expected_properties);
}

#[test]
fn compute_patch_fade_in_emits_two_opacity_keyframes() {
    let prior = project(100);
    let (patch, warnings, _data) = compute_patch(&prior, &args("fade_in")).expect("happy path");
    let keyframes = patch_keyframes(&patch);

    assert!(warnings.is_empty());
    assert_eq!(keyframes.len(), presets::FADE_IN.len());
    assert!(
        keyframes
            .iter()
            .all(|keyframe| keyframe.property.as_str() == "opacity")
    );
    assert_eq!(keyframes[0].time_tk, Tick::new(0));
    assert_eq!(keyframes[1].time_tk, Tick::new(99));
    assert_eq!(keyframes[0].easing, Easing::EaseOut);
    assert_eq!(keyframes[1].easing, Easing::EaseOut);
}

#[test]
fn compute_patch_pop_emits_four_scale_keyframes() {
    let prior = project(100);
    let (patch, warnings, _data) = compute_patch(&prior, &args("pop")).expect("happy path");
    let keyframes = patch_keyframes(&patch);
    let properties: Vec<&str> = keyframes
        .iter()
        .map(|keyframe| keyframe.property.as_str())
        .collect();

    assert!(warnings.is_empty());
    assert_eq!(keyframes.len(), presets::POP.len());
    assert_eq!(
        properties,
        vec![
            "transform.scale_x",
            "transform.scale_x",
            "transform.scale_y",
            "transform.scale_y",
        ]
    );
}

#[test]
fn compute_patch_default_window_uses_full_clip_duration() {
    let prior = project(100);
    let mut text_args = args("fade_in");
    text_args.in_tk = None;
    text_args.out_tk = None;

    let (patch, warnings, _data) = compute_patch(&prior, &text_args).expect("happy path");
    let keyframes = patch_keyframes(&patch);

    assert_eq!(keyframes[0].time_tk, Tick::new(0));
    assert_eq!(keyframes[1].time_tk, Tick::new(99));
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_PRESET_KEYFRAMES_CLAMPED);
    assert_eq!(warnings[0]["details"]["clamped_count"], 1);
}

#[test]
fn compute_patch_explicit_window_scales_keyframe_times() {
    let prior = project(100);
    let text_args = args_window("fade_in", Some(10), Some(90));

    let (patch, warnings, _data) = compute_patch(&prior, &text_args).expect("happy path");
    let keyframes = patch_keyframes(&patch);

    assert!(warnings.is_empty());
    assert_eq!(keyframes[0].time_tk, Tick::new(10));
    assert_eq!(keyframes[1].time_tk, Tick::new(90));
}

#[test]
fn compute_patch_in_tk_only_is_schema_violation() {
    let prior = project(100);
    let err = compute_patch(&prior, &args_window("fade_in", Some(0), None))
        .expect_err("pair-required rejects");
    assert!(matches!(
        err,
        TextAnimateError::SchemaViolation {
            field: "out_tk",
            ..
        }
    ));
}

#[test]
fn compute_patch_out_tk_only_is_schema_violation() {
    let prior = project(100);
    let err = compute_patch(&prior, &args_window("fade_in", None, Some(99)))
        .expect_err("pair-required rejects");
    assert!(matches!(
        err,
        TextAnimateError::SchemaViolation { field: "in_tk", .. }
    ));
}

#[test]
fn compute_patch_in_tk_equal_out_tk_is_bad_time() {
    let prior = project(100);
    let err = compute_patch(&prior, &args_window("fade_in", Some(10), Some(10)))
        .expect_err("empty window rejects");
    assert!(matches!(err, TextAnimateError::BadTime { .. }));
}

#[test]
fn compute_patch_in_tk_negative_is_bad_time() {
    let prior = project(100);
    let err = compute_patch(&prior, &args_window("fade_in", Some(-1), Some(10)))
        .expect_err("negative start rejects");
    assert!(matches!(err, TextAnimateError::BadTime { .. }));
}

#[test]
fn compute_patch_unknown_preset_is_preset_unknown() {
    let prior = project(100);
    let err = compute_patch(&prior, &args("sparkle")).expect_err("unknown preset rejects");
    assert!(matches!(err, TextAnimateError::PresetUnknown { .. }));
}

#[test]
fn compute_patch_window_beyond_clip_duration_clamps_with_warning() {
    let prior = project(100);
    let text_args = args_window("fade_in", Some(0), Some(120));

    let (patch, warnings, data) = compute_patch(&prior, &text_args).expect("happy path");
    let keyframes = patch_keyframes(&patch);

    assert_eq!(keyframes[1].time_tk, Tick::new(99));
    assert_eq!(warnings.len(), 1);
    assert_eq!(
        warning(&warnings, W_PRESET_KEYFRAMES_CLAMPED)["details"]["clamped_count"],
        1
    );
    assert_eq!(data.clamped_keyframe_ids.len(), 1);
}

#[test]
fn compute_patch_clamp_creating_duplicate_ticks_dedupes_with_warning() {
    let prior = project(100);
    let text_args = args_window("typewriter", Some(98), Some(200));

    let (patch, warnings, _data) = compute_patch(&prior, &text_args).expect("happy path");
    let keyframes = patch_keyframes(&patch);

    assert_eq!(keyframes.len(), 2);
    assert_eq!(keyframes[0].time_tk, Tick::new(98));
    assert_eq!(keyframes[1].time_tk, Tick::new(99));
    assert_eq!(
        warning(&warnings, W_PRESET_KEYFRAMES_CLAMPED)["details"]["clamped_count"],
        4
    );
    assert_eq!(
        warning(&warnings, W_KEYFRAMES_DEDUPED)["details"]["deduped_count"],
        3
    );
}

#[test]
fn compute_patch_clamped_keyframe_ids_subset_of_added() {
    let prior = project(100);
    let text_args = args_window("fade_in", Some(0), Some(120));

    let (_patch, _warnings, data) = compute_patch(&prior, &text_args).expect("happy path");
    let added: HashSet<KeyframeId> = data.added_keyframe_ids.iter().copied().collect();

    assert!(
        data.clamped_keyframe_ids
            .iter()
            .all(|keyframe_id| added.contains(keyframe_id))
    );
}

#[test]
fn compute_patch_video_clip_is_not_text_clip() {
    let prior = project_with_video_track();
    let mut text_args = args("fade_in");
    text_args.clip = CLIP_VIDEO_A.to_string();

    let err = compute_patch(&prior, &text_args).expect_err("video clip rejects");
    assert!(matches!(err, TextAnimateError::ClipKindMismatch { .. }));
}

#[test]
fn compute_patch_locked_clip_rejects_with_e_locked() {
    let prior = project_with_tracks(vec![text_track(100, false, true)]);
    let err = compute_patch(&prior, &args("fade_in")).expect_err("locked clip rejects");
    assert!(matches!(err, TextAnimateError::Locked { kind: "clip", .. }));
}

#[test]
fn compute_patch_locked_precedes_bad_time() {
    let prior = project_with_tracks(vec![text_track(100, false, true)]);
    let text_args = args_window("fade_in", Some(10), Some(10));

    let err = compute_patch(&prior, &text_args).expect_err("locked clip rejects first");
    assert!(matches!(err, TextAnimateError::Locked { kind: "clip", .. }));
}

#[test]
fn compute_patch_existing_keyframe_at_same_property_time_is_duplicate() {
    let prior = project_with_tracks(vec![text_track_with_keyframes(
        100,
        false,
        false,
        json!([{
            "id": EXISTING_KEYFRAME_ID,
            "property": "opacity",
            "time_tk": 0,
            "value": 0.5,
        }]),
    )]);

    let err = compute_patch(&prior, &args("fade_in")).expect_err("duplicate rejects");
    assert!(matches!(
        err,
        TextAnimateError::Duplicate {
            existing_keyframe_id
        } if existing_keyframe_id.to_string() == EXISTING_KEYFRAME_ID
    ));
}

#[test]
fn verb_routes_through_mutate_via_verb() {
    let dir = TempDir::new().expect("tempdir");
    let prior = project(100);
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        prior,
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry succeeds");

    let outcome = store
        .mutate_via_verb(
            "text.animate",
            serde_json::to_value(args("fade_in")).expect("args serialize"),
            None,
        )
        .expect("text.animate should route");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("happy path must apply");
    };
    assert!(warnings.is_empty());
    let envelope: TextAnimateData = serde_json::from_value(data).expect("data parses");
    assert_eq!(envelope.clip_id.to_string(), CLIP_TEXT_A);
    assert_eq!(store.project().tracks[0].clips[0].keyframes.len(), 2);
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "text.animate")
        .expect("default_fixtures includes text.animate");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(TextAnimateVerb))
        .expect("register text.animate verb");

    let report = validate_reconstructors(&registry, &[fixture])
        .expect("text.animate reconstructor should pass");
    assert_eq!(report.verbs_checked, vec!["text.animate"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn round_trip_text_animate() {
    let prior = project(100);
    let text_args = args("fade_in");
    let (patch_value, warnings, data) = compute_patch(&prior, &text_args).expect("happy path");
    let post = apply_patch(&prior, patch_value.clone());
    let envelope = data_envelope_from_args_patch_warnings(&text_args, &patch_value, &warnings)
        .expect("envelope reconstructs");

    assert!(warnings.is_empty());
    assert_eq!(data, envelope);
    let post_ids: Vec<KeyframeId> = post.tracks[0].clips[0]
        .keyframes
        .iter()
        .map(|keyframe| keyframe.id)
        .collect();
    assert_eq!(post_ids, data.added_keyframe_ids);
}

#[test]
fn data_envelope_includes_clamped_keyframe_ids_when_clamping_fired() {
    let prior = project(100);
    let text_args = args_window("fade_in", Some(0), Some(120));

    let (patch, warnings, data) = compute_patch(&prior, &text_args).expect("happy path");
    let envelope = data_envelope_from_args_patch_warnings(&text_args, &patch, &warnings)
        .expect("envelope reconstructs");

    assert_eq!(data, envelope);
    assert_eq!(data.clamped_keyframe_ids.len(), 1);
    assert_eq!(
        warning(&warnings, W_PRESET_KEYFRAMES_CLAMPED)["details"]["clamped_keyframe_ids"]
            .as_array()
            .expect("ids array")
            .len(),
        1
    );
}

#[test]
fn compute_patch_fade_in_smoke() {
    assert_preset_smoke("fade_in");
}

#[test]
fn compute_patch_fade_out_smoke() {
    assert_preset_smoke("fade_out");
}

#[test]
fn compute_patch_pop_smoke() {
    assert_preset_smoke("pop");
}

#[test]
fn compute_patch_slide_left_smoke() {
    assert_preset_smoke("slide_left");
}

#[test]
fn compute_patch_slide_right_smoke() {
    assert_preset_smoke("slide_right");
}

#[test]
fn compute_patch_slide_up_smoke() {
    assert_preset_smoke("slide_up");
}

#[test]
fn compute_patch_slide_down_smoke() {
    assert_preset_smoke("slide_down");
}

#[test]
fn compute_patch_typewriter_smoke() {
    assert_preset_smoke("typewriter");
}

#[test]
fn compute_patch_bounce_smoke() {
    assert_preset_smoke("bounce");
}
