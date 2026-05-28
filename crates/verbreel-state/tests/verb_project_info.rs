//! Tests for `project.info` (§2.4) — fifty-seventh production verb.

use std::sync::Arc;

use serde_json::json;
use verbreel_state::verbs::project_info::{compute_patch, data_envelope_from_post_state};
use verbreel_state::{
    Asset, MutateOutcome, Project, ProjectInfoArgs, ProjectInfoData, ProjectInfoVerb, Track, Verb,
    VerbError, VerbRegistry, default_fixtures, default_registry, validate_reconstructors,
};
use verbreel_types::Tick;

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const VIDEO_HASH: &str = "53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658";
const AUDIO_HASH: &str = "36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da";
const IMAGE_HASH: &str = "aa761291ff9d068556f2d1d6f63c53a4d22e44d65f882c1c252a04372123add3";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture → Project")
}

fn fixture_project_id() -> verbreel_types::ProjectId {
    FIXTURE_PROJECT_ID
        .parse()
        .expect("fixture project id parses")
}

fn make_args() -> ProjectInfoArgs {
    ProjectInfoArgs {
        project_id: fixture_project_id(),
    }
}

/// Build a project with zero tracks (the EMPTY_FIXTURE comes with 1
/// video + 1 audio — we override `tracks` to `[]` for the count-zero
/// case).
fn empty_project_no_tracks() -> Project {
    let mut p = empty_project();
    p.tracks.clear();
    p
}

fn make_track(kind: &str, id: &str, name: &str) -> Track {
    serde_json::from_value(json!({
        "id": id,
        "kind": kind,
        "name": name,
        "clips": [],
        "muted": false,
        "solo": false,
        "locked": false,
        "hidden": false,
        "volume": 1.0,
        "pan": 0.0,
        "effects": []
    }))
    .expect("track fixture parses")
}

fn make_video_asset(id: &str, imported_at: &str) -> Asset {
    serde_json::from_value(json!({
        "kind": "video",
        "id": id,
        "hash": VIDEO_HASH,
        "path": format!("assets/{}/{}.mp4", &VIDEO_HASH[0..2], VIDEO_HASH),
        "original_filename": "video.mp4",
        "imported_at": imported_at,
        "metadata": {
            "duration_tk": 480000,
            "width": 1920,
            "height": 1080,
            "fps_num": 30,
            "fps_den": 1,
            "video_codec": "h264",
            "container": "mp4",
            "fingerprint": {
                "mtime_ms": 1_700_000_000_000_i64,
                "size_bytes": 1024
            }
        }
    }))
    .expect("video asset fixture parses")
}

fn make_audio_asset(id: &str, imported_at: &str) -> Asset {
    serde_json::from_value(json!({
        "kind": "audio",
        "id": id,
        "hash": AUDIO_HASH,
        "path": format!("assets/{}/{}.mp3", &AUDIO_HASH[0..2], AUDIO_HASH),
        "original_filename": "audio.mp3",
        "imported_at": imported_at,
        "metadata": {
            "duration_tk": 240000,
            "audio_codec": "aac",
            "audio_channels": 2,
            "audio_sample_rate_hz": 48000,
            "container": "mp3",
            "fingerprint": {
                "mtime_ms": 1_700_000_000_001_i64,
                "size_bytes": 512
            }
        }
    }))
    .expect("audio asset fixture parses")
}

fn make_image_asset(id: &str, imported_at: &str) -> Asset {
    serde_json::from_value(json!({
        "kind": "image",
        "id": id,
        "hash": IMAGE_HASH,
        "path": format!("assets/{}/{}.png", &IMAGE_HASH[0..2], IMAGE_HASH),
        "original_filename": "image.png",
        "imported_at": imported_at,
        "metadata": {
            "width": 1920,
            "height": 1080,
            "container": "png",
            "fingerprint": {
                "mtime_ms": 1_700_000_000_002_i64,
                "size_bytes": 256
            }
        }
    }))
    .expect("image asset fixture parses")
}

#[test]
fn args_deserialize_ok() {
    let args: ProjectInfoArgs = serde_json::from_value(json!({
        "project_id": FIXTURE_PROJECT_ID,
    }))
    .expect("valid args deserialize");
    assert_eq!(args.project_id, fixture_project_id());
}

#[test]
fn args_missing_project_id_field_is_bad_args() {
    let prior = empty_project();
    let verb = ProjectInfoVerb;
    let err = verb
        .compute_patch(&prior, &json!({}))
        .expect_err("missing project_id should fail");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_wrong_type_is_bad_args() {
    let prior = empty_project();
    let verb = ProjectInfoVerb;
    let err = verb
        .compute_patch(&prior, &json!({ "project_id": 42 }))
        .expect_err("integer project_id should fail");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn basic_shape_from_empty_fixture() {
    let prior = empty_project();
    let args = make_args();
    let (patch, warnings, data) =
        compute_patch(&prior, &args).expect("empty fixture should compute successfully");

    assert_eq!(patch, json!([]));
    assert!(warnings.is_empty());
    assert_eq!(data.id, FIXTURE_PROJECT_ID);
    assert_eq!(data.name, "test");
    assert_eq!(data.fps_num, 30);
    assert_eq!(data.fps_den, 1);
}

#[test]
fn track_counts_zero_tracks() {
    let prior = empty_project_no_tracks();
    let (_, _, data) = compute_patch(&prior, &make_args()).expect("compute");
    assert_eq!(data.track_counts.video, 0);
    assert_eq!(data.track_counts.audio, 0);
    assert_eq!(data.track_counts.text, 0);
    assert_eq!(data.track_counts.effect, 0);
}

#[test]
fn track_counts_only_video() {
    let mut prior = empty_project_no_tracks();
    prior.tracks.push(make_track(
        "video",
        "01900000-0000-7000-8000-00000000a001",
        "Video 1",
    ));
    prior.tracks.push(make_track(
        "video",
        "01900000-0000-7000-8000-00000000a002",
        "Video 2",
    ));
    let (_, _, data) = compute_patch(&prior, &make_args()).expect("compute");
    assert_eq!(data.track_counts.video, 2);
    assert_eq!(data.track_counts.audio, 0);
    assert_eq!(data.track_counts.text, 0);
    assert_eq!(data.track_counts.effect, 0);
}

#[test]
fn track_counts_only_audio() {
    let mut prior = empty_project_no_tracks();
    prior.tracks.push(make_track(
        "audio",
        "01900000-0000-7000-8000-00000000b001",
        "Audio 1",
    ));
    let (_, _, data) = compute_patch(&prior, &make_args()).expect("compute");
    assert_eq!(data.track_counts.video, 0);
    assert_eq!(data.track_counts.audio, 1);
    assert_eq!(data.track_counts.text, 0);
    assert_eq!(data.track_counts.effect, 0);
}

#[test]
fn track_counts_only_text() {
    let mut prior = empty_project_no_tracks();
    prior.tracks.push(make_track(
        "text",
        "01900000-0000-7000-8000-00000000c001",
        "Text 1",
    ));
    prior.tracks.push(make_track(
        "text",
        "01900000-0000-7000-8000-00000000c002",
        "Text 2",
    ));
    prior.tracks.push(make_track(
        "text",
        "01900000-0000-7000-8000-00000000c003",
        "Text 3",
    ));
    let (_, _, data) = compute_patch(&prior, &make_args()).expect("compute");
    assert_eq!(data.track_counts.video, 0);
    assert_eq!(data.track_counts.audio, 0);
    assert_eq!(data.track_counts.text, 3);
    assert_eq!(data.track_counts.effect, 0);
}

#[test]
fn track_counts_only_effect() {
    let mut prior = empty_project_no_tracks();
    prior.tracks.push(make_track(
        "effect",
        "01900000-0000-7000-8000-00000000d001",
        "Effect 1",
    ));
    let (_, _, data) = compute_patch(&prior, &make_args()).expect("compute");
    assert_eq!(data.track_counts.video, 0);
    assert_eq!(data.track_counts.audio, 0);
    assert_eq!(data.track_counts.text, 0);
    assert_eq!(data.track_counts.effect, 1);
}

#[test]
fn track_counts_mixed_all_four_kinds() {
    let mut prior = empty_project_no_tracks();
    prior.tracks.push(make_track(
        "video",
        "01900000-0000-7000-8000-00000000a101",
        "V1",
    ));
    prior.tracks.push(make_track(
        "audio",
        "01900000-0000-7000-8000-00000000a102",
        "A1",
    ));
    prior.tracks.push(make_track(
        "audio",
        "01900000-0000-7000-8000-00000000a103",
        "A2",
    ));
    prior.tracks.push(make_track(
        "text",
        "01900000-0000-7000-8000-00000000a104",
        "T1",
    ));
    prior.tracks.push(make_track(
        "effect",
        "01900000-0000-7000-8000-00000000a105",
        "E1",
    ));
    let (_, _, data) = compute_patch(&prior, &make_args()).expect("compute");
    assert_eq!(data.track_counts.video, 1);
    assert_eq!(data.track_counts.audio, 2);
    assert_eq!(data.track_counts.text, 1);
    assert_eq!(data.track_counts.effect, 1);
}

#[test]
fn asset_count_zero() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &make_args()).expect("compute");
    assert_eq!(data.asset_count, 0);
}

#[test]
fn asset_count_one_of_each_kind() {
    let mut prior = empty_project();
    prior.assets.push(make_video_asset(
        "01900000-0000-7000-8000-00000000aaa1",
        "2026-05-01T00:00:00Z",
    ));
    prior.assets.push(make_audio_asset(
        "01900000-0000-7000-8000-00000000aaa2",
        "2026-05-02T00:00:00Z",
    ));
    prior.assets.push(make_image_asset(
        "01900000-0000-7000-8000-00000000aaa3",
        "2026-05-03T00:00:00Z",
    ));
    let (_, _, data) = compute_patch(&prior, &make_args()).expect("compute");
    assert_eq!(data.asset_count, 3);
}

#[test]
fn asset_count_multiple_of_same_kind() {
    let mut prior = empty_project();
    for i in 0..5 {
        prior.assets.push(make_video_asset(
            &format!("01900000-0000-7000-8000-00000000bb{i:02}"),
            "2026-05-01T00:00:00Z",
        ));
    }
    let (_, _, data) = compute_patch(&prior, &make_args()).expect("compute");
    assert_eq!(data.asset_count, 5);
}

#[test]
fn duration_tk_echoes_project_value() {
    let mut prior = empty_project();
    prior.duration_tk = Tick::new(1_234_567);
    let (_, _, data) = compute_patch(&prior, &make_args()).expect("compute");
    assert_eq!(data.duration_tk, 1_234_567);
}

#[test]
fn canvas_carries_only_width_and_height() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &make_args()).expect("compute");
    assert_eq!(data.canvas.width, 1080);
    assert_eq!(data.canvas.height, 1920);

    // Verify the serialized JSON contains ONLY width + height — no
    // background, no pixel_aspect_num, no pixel_aspect_den.
    let v = serde_json::to_value(&data).expect("serialize");
    let canvas = v
        .get("canvas")
        .expect("canvas field present")
        .as_object()
        .expect("canvas is object");
    assert_eq!(canvas.len(), 2, "canvas must carry exactly 2 fields");
    assert!(canvas.contains_key("width"));
    assert!(canvas.contains_key("height"));
    assert!(
        !canvas.contains_key("background"),
        "background must be trimmed per §2.4"
    );
    assert!(
        !canvas.contains_key("pixel_aspect_num"),
        "pixel_aspect_num must be trimmed per §2.4"
    );
    assert!(
        !canvas.contains_key("pixel_aspect_den"),
        "pixel_aspect_den must be trimmed per §2.4"
    );
}

#[test]
fn fps_num_den_echo_from_project() {
    let mut prior = empty_project();
    prior.fps_num = 24000;
    prior.fps_den = 1001;
    let (_, _, data) = compute_patch(&prior, &make_args()).expect("compute");
    assert_eq!(data.fps_num, 24000);
    assert_eq!(data.fps_den, 1001);
}

#[test]
fn event_count_is_zero_deferred() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &make_args()).expect("compute");
    assert_eq!(
        data.event_count, 0,
        "event_count is deferred to a follow-up slice — emits 0"
    );
}

#[test]
fn path_is_empty_string_deferred() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &make_args()).expect("compute");
    assert_eq!(
        data.path, "",
        "path is deferred to a follow-up slice — emits empty string"
    );
}

#[test]
fn updated_at_echoes_project_value() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &make_args()).expect("compute");
    assert_eq!(data.updated_at.as_str(), "2026-05-24T00:00:00Z");
}

#[test]
fn reconstructor_round_trip_matches_compute_patch() {
    let mut prior = empty_project();
    prior.assets.push(make_video_asset(
        "01900000-0000-7000-8000-00000000cc01",
        "2026-05-01T00:00:00Z",
    ));
    let args = make_args();
    let (patch, _, expected) = compute_patch(&prior, &args).expect("compute");
    let patch: json_patch::Patch = serde_json::from_value(patch).expect("valid patch");
    let post_state = prior
        .apply(&patch)
        .expect("applying empty patch should succeed");

    let data = data_envelope_from_post_state(&args, &post_state)
        .expect("data envelope from post state should rebuild");
    assert_eq!(data, expected);
}

#[test]
fn verb_trait_surface_via_serde_json_value() {
    let prior = empty_project();
    let verb = ProjectInfoVerb;
    let (patch, data, warnings) = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
            }),
        )
        .expect("verb should route ok");
    assert_eq!(patch.0.len(), 0, "patch must be empty");
    assert!(warnings.is_empty());
    let typed: ProjectInfoData = serde_json::from_value(data).expect("data parses");
    assert_eq!(typed.id, FIXTURE_PROJECT_ID);
    assert_eq!(typed.name, "test");
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "project.info")
        .expect("default_fixtures includes project.info");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(ProjectInfoVerb))
        .expect("register project.info");

    let report = validate_reconstructors(&registry, &[fixture])
        .expect("project.info reconstruct from fixture should pass");
    assert_eq!(report.verbs_checked, vec!["project.info"]);
    assert_eq!(report.fixtures_run, 1);
}

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
    .expect("create_with_registry succeeds");

    let outcome = store
        .mutate_via_verb(
            "project.info",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
            }),
            None,
        )
        .expect("project.info should route");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("project.info expected Applied outcome");
    };

    assert!(warnings.is_empty());
    let data: ProjectInfoData =
        serde_json::from_value(data).expect("project.info data deserializes");
    assert_eq!(data.id, FIXTURE_PROJECT_ID);
    assert_eq!(data.path, "");
    assert_eq!(data.event_count, 0);
}
