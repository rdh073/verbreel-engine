//! Tests for `track.add` (§4.1) — first kind-block insertion verb.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::{
    MutateOutcome, Project, RecordedEvent, Track, TrackAddArgs, TrackAddData, TrackAddError,
    TrackAddVerb, TrackKind, VerbRegistry, validate_reconstructors,
    verbs::track_add::{compute_patch, data_envelope_from_post_state},
};

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";

const TRACK_VIDEO_A: &str = "0190b8d3-15e3-7000-bd00-0000000aa101";
const TRACK_VIDEO_B: &str = "0190b8d3-15e3-7000-bd00-0000000aa102";
const TRACK_VIDEO_C: &str = "0190b8d3-15e3-7000-bd00-0000000aa103";
const TRACK_AUDIO_A: &str = "0190b8d3-15e3-7000-bd00-0000000bb101";
const TRACK_AUDIO_B: &str = "0190b8d3-15e3-7000-bd00-0000000bb102";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture → Project")
}

fn project_with_tracks(tracks: Vec<Track>) -> Project {
    let mut project = empty_project();
    project.tracks = tracks;
    project
}

fn track(id: &str, kind: TrackKind, name: &str) -> Track {
    serde_json::from_value(json!({
        "id": id,
        "kind": kind,
        "name": name,
        "clips": [],
    }))
    .expect("track fixture value should deserialize")
}

fn patch_track_value(patch: &Value) -> &Value {
    let arr = patch.as_array().expect("patch is an array");
    assert_eq!(arr.len(), 1, "track.add emits single-op add");
    let op = arr[0].as_object().expect("patch op is object");
    assert_eq!(op.get("op").and_then(Value::as_str), Some("add"));
    op.get("value").expect("add op carries value")
}

fn patch_track_id(patch: &Value) -> String {
    patch_track_value(patch)
        .get("id")
        .and_then(Value::as_str)
        .expect("track value has id")
        .to_string()
}

fn assert_track_name_and_path(patch: &Value, name: &str, path: &str) {
    assert_eq!(patch_track_value(patch)["name"], name);
    let path_actual = patch.as_array().expect("patch is array")[0]
        .get("path")
        .and_then(Value::as_str)
        .expect("patch path exists");
    assert_eq!(path_actual, path);
}

#[test]
fn compute_patch_first_video_track_auto_name_is_video_1() {
    let prior = project_with_tracks(vec![]);
    let (patch, _) = compute_patch(
        &prior,
        &TrackAddArgs {
            project_id: FIXTURE_PROJECT_ID
                .parse()
                .expect("fixture project id parses"),
            kind: TrackKind::Video,
            name: None,
            index: None,
        },
    )
    .expect("compute_patch should succeed");

    assert_track_name_and_path(&patch, "Video 1", "/tracks/0");
}

#[test]
fn compute_patch_second_video_track_auto_name_is_video_2() {
    let prior = project_with_tracks(vec![track(TRACK_VIDEO_A, TrackKind::Video, "Video 1")]);
    let (patch, _) = compute_patch(
        &prior,
        &TrackAddArgs {
            project_id: FIXTURE_PROJECT_ID
                .parse()
                .expect("fixture project id parses"),
            kind: TrackKind::Video,
            name: None,
            index: None,
        },
    )
    .expect("compute_patch should succeed");

    assert_track_name_and_path(&patch, "Video 2", "/tracks/1");
}

#[test]
fn compute_patch_auto_name_skips_leading_zero_digits() {
    let prior = project_with_tracks(vec![track(TRACK_VIDEO_A, TrackKind::Video, "Video 02")]);
    let (patch, _) = compute_patch(
        &prior,
        &TrackAddArgs {
            project_id: FIXTURE_PROJECT_ID
                .parse()
                .expect("fixture project id parses"),
            kind: TrackKind::Video,
            name: None,
            index: None,
        },
    )
    .expect("compute_patch should succeed");

    assert_track_name_and_path(&patch, "Video 1", "/tracks/1");
}

#[test]
fn compute_patch_auto_name_max_is_one_plus_max() {
    let prior = project_with_tracks(vec![
        track(TRACK_VIDEO_A, TrackKind::Video, "Video 5"),
        track(TRACK_VIDEO_B, TrackKind::Video, "Video 3"),
    ]);

    let (patch, _) = compute_patch(
        &prior,
        &TrackAddArgs {
            project_id: FIXTURE_PROJECT_ID
                .parse()
                .expect("fixture project id parses"),
            kind: TrackKind::Video,
            name: None,
            index: None,
        },
    )
    .expect("compute_patch should succeed");

    assert_track_name_and_path(&patch, "Video 6", "/tracks/2");
}

#[test]
fn compute_patch_auto_name_zero_is_valid_suffix() {
    let prior = project_with_tracks(vec![track(TRACK_VIDEO_A, TrackKind::Video, "Video 0")]);
    let (patch, _) = compute_patch(
        &prior,
        &TrackAddArgs {
            project_id: FIXTURE_PROJECT_ID
                .parse()
                .expect("fixture project id parses"),
            kind: TrackKind::Video,
            name: None,
            index: None,
        },
    )
    .expect("compute_patch should succeed");

    assert_track_name_and_path(&patch, "Video 1", "/tracks/1");
}

#[test]
fn compute_patch_auto_name_ignores_custom_names() {
    let prior = project_with_tracks(vec![track(TRACK_VIDEO_A, TrackKind::Video, "Master Bus")]);
    let (patch, _) = compute_patch(
        &prior,
        &TrackAddArgs {
            project_id: FIXTURE_PROJECT_ID
                .parse()
                .expect("fixture project id parses"),
            kind: TrackKind::Video,
            name: None,
            index: None,
        },
    )
    .expect("compute_patch should succeed");

    assert_track_name_and_path(&patch, "Video 1", "/tracks/1");
}

#[test]
fn compute_patch_auto_name_per_kind_independent() {
    let prior = project_with_tracks(vec![track(TRACK_AUDIO_A, TrackKind::Audio, "Audio 3")]);

    let (patch, _) = compute_patch(
        &prior,
        &TrackAddArgs {
            project_id: FIXTURE_PROJECT_ID
                .parse()
                .expect("fixture project id parses"),
            kind: TrackKind::Video,
            name: None,
            index: None,
        },
    )
    .expect("compute_patch should succeed");

    assert_track_name_and_path(&patch, "Video 1", "/tracks/1");
}

#[test]
fn compute_patch_explicit_name_used() {
    let prior = project_with_tracks(vec![]);
    let (patch, _) = compute_patch(
        &prior,
        &TrackAddArgs {
            project_id: FIXTURE_PROJECT_ID
                .parse()
                .expect("fixture project id parses"),
            kind: TrackKind::Video,
            name: Some("Custom".to_string()),
            index: None,
        },
    )
    .expect("compute_patch should succeed");

    assert_track_name_and_path(&patch, "Custom", "/tracks/0");
}

#[test]
fn compute_patch_empty_name_errors() {
    let prior = project_with_tracks(vec![]);
    let err = compute_patch(
        &prior,
        &TrackAddArgs {
            project_id: FIXTURE_PROJECT_ID
                .parse()
                .expect("fixture project id parses"),
            kind: TrackKind::Video,
            name: Some("".to_string()),
            index: None,
        },
    )
    .expect_err("empty names must reject");

    assert!(matches!(err, TrackAddError::NameEmpty));
}

#[test]
fn compute_patch_129_char_name_errors() {
    let prior = project_with_tracks(vec![]);
    let err = compute_patch(
        &prior,
        &TrackAddArgs {
            project_id: FIXTURE_PROJECT_ID
                .parse()
                .expect("fixture project id parses"),
            kind: TrackKind::Video,
            name: Some("a".repeat(129)),
            index: None,
        },
    )
    .expect_err("129-char names must reject");

    match err {
        TrackAddError::NameTooLong { actual, max } => {
            assert_eq!(actual, 129);
            assert_eq!(max, 128);
        }
        other => panic!("expected NameTooLong, got {other:?}"),
    }
}

#[test]
fn compute_patch_name_conflict_within_kind_errors() {
    let prior = project_with_tracks(vec![track(TRACK_VIDEO_A, TrackKind::Video, "Foo")]);

    let err = compute_patch(
        &prior,
        &TrackAddArgs {
            project_id: FIXTURE_PROJECT_ID
                .parse()
                .expect("fixture project id parses"),
            kind: TrackKind::Video,
            name: Some("Foo".to_string()),
            index: None,
        },
    )
    .expect_err("same-kind name conflict must reject");

    match err {
        TrackAddError::NameConflict { name, kind } => {
            assert_eq!(name, "Foo");
            assert_eq!(kind, TrackKind::Video);
        }
        other => panic!("expected NameConflict, got {other:?}"),
    }
}

#[test]
fn compute_patch_name_no_conflict_across_kinds() {
    let prior = project_with_tracks(vec![track(TRACK_VIDEO_A, TrackKind::Video, "Foo")]);
    let (patch, _) = compute_patch(
        &prior,
        &TrackAddArgs {
            project_id: FIXTURE_PROJECT_ID
                .parse()
                .expect("fixture project id parses"),
            kind: TrackKind::Audio,
            name: Some("Foo".to_string()),
            index: None,
        },
    )
    .expect("compute_patch should allow cross-kind same name");

    assert_track_name_and_path(&patch, "Foo", "/tracks/1");
}

#[test]
fn compute_patch_explicit_index_inserts_at_position() {
    let prior = project_with_tracks(vec![
        track(TRACK_AUDIO_A, TrackKind::Audio, "Audio 1"),
        track(TRACK_VIDEO_A, TrackKind::Video, "Video 1"),
        track(TRACK_VIDEO_B, TrackKind::Video, "Video 2"),
        track(TRACK_VIDEO_C, TrackKind::Video, "Video 3"),
        track(TRACK_AUDIO_B, TrackKind::Audio, "Audio 2"),
    ]);

    let (patch, _) = compute_patch(
        &prior,
        &TrackAddArgs {
            project_id: FIXTURE_PROJECT_ID
                .parse()
                .expect("fixture project id parses"),
            kind: TrackKind::Video,
            name: None,
            index: Some(1),
        },
    )
    .expect("compute_patch should succeed");

    assert_track_name_and_path(&patch, "Video 4", "/tracks/2");
}

#[test]
fn compute_patch_index_zero_inserts_at_head_of_kind_block() {
    let prior = project_with_tracks(vec![
        track(TRACK_AUDIO_A, TrackKind::Audio, "Audio 1"),
        track(TRACK_VIDEO_A, TrackKind::Video, "Video 1"),
        track(TRACK_VIDEO_B, TrackKind::Video, "Video 2"),
    ]);

    let (patch, _) = compute_patch(
        &prior,
        &TrackAddArgs {
            project_id: FIXTURE_PROJECT_ID
                .parse()
                .expect("fixture project id parses"),
            kind: TrackKind::Video,
            name: None,
            index: Some(0),
        },
    )
    .expect("compute_patch should succeed");

    assert_track_name_and_path(&patch, "Video 3", "/tracks/1");
}

#[test]
fn compute_patch_index_equal_count_appends() {
    let prior = project_with_tracks(vec![
        track(TRACK_VIDEO_A, TrackKind::Video, "Video 1"),
        track(TRACK_VIDEO_B, TrackKind::Video, "Video 2"),
        track(TRACK_VIDEO_C, TrackKind::Video, "Video 3"),
    ]);

    let (patch, _) = compute_patch(
        &prior,
        &TrackAddArgs {
            project_id: FIXTURE_PROJECT_ID
                .parse()
                .expect("fixture project id parses"),
            kind: TrackKind::Video,
            name: None,
            index: Some(3),
        },
    )
    .expect("compute_patch should succeed");

    assert_track_name_and_path(&patch, "Video 4", "/tracks/3");
}

#[test]
fn compute_patch_bad_index_errors() {
    let prior = project_with_tracks(vec![
        track(TRACK_VIDEO_A, TrackKind::Video, "Video 1"),
        track(TRACK_VIDEO_B, TrackKind::Video, "Video 2"),
        track(TRACK_VIDEO_C, TrackKind::Video, "Video 3"),
    ]);

    let err = compute_patch(
        &prior,
        &TrackAddArgs {
            project_id: FIXTURE_PROJECT_ID
                .parse()
                .expect("fixture project id parses"),
            kind: TrackKind::Video,
            name: None,
            index: Some(4),
        },
    )
    .expect_err("index > count must reject");

    match err {
        TrackAddError::BadIndex {
            requested,
            max_allowed,
            kind,
        } => {
            assert_eq!(requested, 4);
            assert_eq!(max_allowed, 3);
            assert_eq!(kind, TrackKind::Video);
        }
        other => panic!("expected BadIndex, got {other:?}"),
    }
}

#[test]
fn compute_patch_first_track_of_kind_inserts_at_end_of_tracks_vec() {
    let prior = project_with_tracks(vec![track(TRACK_AUDIO_A, TrackKind::Audio, "Audio 1")]);

    let (patch, _) = compute_patch(
        &prior,
        &TrackAddArgs {
            project_id: FIXTURE_PROJECT_ID
                .parse()
                .expect("fixture project id parses"),
            kind: TrackKind::Video,
            name: Some("Video 1".to_string()),
            index: None,
        },
    )
    .expect("compute_patch should append a first kind track at end");

    assert_track_name_and_path(&patch, "Video 1", "/tracks/1");
}

#[test]
fn compute_patch_minted_track_id_is_unique_per_call() {
    let prior = project_with_tracks(vec![]);
    let args = TrackAddArgs {
        project_id: FIXTURE_PROJECT_ID
            .parse()
            .expect("fixture project id parses"),
        kind: TrackKind::Video,
        name: None,
        index: None,
    };

    let (patch1, _) = compute_patch(&prior, &args).expect("first compute should succeed");
    let (patch2, _) = compute_patch(&prior, &args).expect("second compute should succeed");

    assert_ne!(patch_track_id(&patch1), patch_track_id(&patch2));
}

#[test]
fn compute_patch_new_track_has_default_fields() {
    let prior = project_with_tracks(vec![track(TRACK_AUDIO_A, TrackKind::Audio, "Audio 1")]);
    let (patch, _) = compute_patch(
        &prior,
        &TrackAddArgs {
            project_id: FIXTURE_PROJECT_ID
                .parse()
                .expect("fixture project id parses"),
            kind: TrackKind::Video,
            name: Some("Explicit".to_string()),
            index: None,
        },
    )
    .expect("compute_patch should succeed");

    let track = patch_track_value(&patch);
    assert_eq!(track["muted"], false);
    assert_eq!(track["solo"], false);
    assert_eq!(track["locked"], false);
    assert_eq!(track["hidden"], false);
    assert_eq!(track["volume"], 1.0);
    assert_eq!(track["pan"], 0.0);
    assert_eq!(track["clips"].as_array().expect("clips is array").len(), 0);
    assert_eq!(
        track["effects"].as_array().expect("effects is array").len(),
        0
    );
}

#[test]
fn data_envelope_returns_kind_relative_index() {
    let prior = project_with_tracks(vec![
        track(TRACK_AUDIO_A, TrackKind::Audio, "Audio 1"),
        track(TRACK_VIDEO_A, TrackKind::Video, "Video 1"),
        track(TRACK_VIDEO_B, TrackKind::Video, "Video 2"),
    ]);

    let (patch, _) = compute_patch(
        &prior,
        &TrackAddArgs {
            project_id: FIXTURE_PROJECT_ID
                .parse()
                .expect("fixture project id parses"),
            kind: TrackKind::Video,
            name: None,
            index: Some(1),
        },
    )
    .expect("compute_patch should succeed");

    let typed_patch: json_patch::Patch = serde_json::from_value(patch.clone())
        .expect("track.add compute returns valid RFC 6902 patch");
    let post_state = prior
        .apply(&typed_patch)
        .expect("track.add patch should apply cleanly");

    let data = data_envelope_from_post_state(&patch, &post_state)
        .expect("post-state envelope should be readable");
    assert_eq!(data.index, 1);
    assert_eq!(data.kind, TrackKind::Video);
}

#[test]
fn reconstructor_round_trip() {
    let prior = project_with_tracks(vec![
        track(TRACK_AUDIO_A, TrackKind::Audio, "Audio 1"),
        track(TRACK_VIDEO_A, TrackKind::Video, "Video 1"),
    ]);
    let args = TrackAddArgs {
        project_id: FIXTURE_PROJECT_ID
            .parse()
            .expect("fixture project id parses"),
        kind: TrackKind::Video,
        name: None,
        index: Some(1),
    };

    let (patch, _) = compute_patch(&prior, &args).expect("compute_patch ok");
    let typed_patch: json_patch::Patch =
        serde_json::from_value(patch.clone()).expect("patch parses to typed JSON patch");
    let post_state = prior
        .apply(&typed_patch)
        .expect("track.add patch should apply cleanly");

    let expected_data = serde_json::to_value(
        data_envelope_from_post_state(&patch, &post_state)
            .expect("envelope")
            .clone(),
    )
    .expect("envelope serializes");

    let recorded = RecordedEvent {
        verb: "track.add".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch,
        warnings: vec![],
        post_state,
        expected_data,
    };

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(TrackAddVerb))
        .expect("register track.add verb");

    let report = validate_reconstructors(&registry, std::slice::from_ref(&recorded))
        .expect("reconstructor validation should pass");
    assert_eq!(report.verbs_checked, vec!["track.add"]);
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
        &verbreel_state::default_registry(),
        &verbreel_state::default_fixtures(),
    )
    .expect("create_with_registry clears the gate and writes project.json");

    let args = json!({"project_id": FIXTURE_PROJECT_ID, "kind": "video"});
    let outcome = store
        .mutate_via_verb("track.add", args, None)
        .expect("mutate_via_verb happy path");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("happy path must return Applied, got {outcome:?}");
    };

    let data: TrackAddData = serde_json::from_value(data).expect("track.add data is TrackAddData");
    assert_eq!(data.kind, TrackKind::Video);
    assert_eq!(data.index, 1);
    assert_eq!(data.track_id, store.project().tracks[1].id);
    assert_eq!(store.project().tracks[1].name, "Video 2");
    assert_eq!(store.project().tracks.len(), 3);
    assert!(warnings.is_empty());
}

#[cfg(feature = "native")]
#[test]
fn replay_returns_same_track_id() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        empty_project(),
        &verbreel_state::default_registry(),
        &verbreel_state::default_fixtures(),
    )
    .expect("create_with_registry clears the gate and writes project.json");

    let args = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "kind": "video",
    });

    let first = store
        .mutate_via_verb("track.add", args.clone(), Some("idem-track-add".into()))
        .expect("first call should apply");
    let MutateOutcome::Applied {
        data: first_data, ..
    } = first
    else {
        panic!("first apply must be Applied, got {first:?}");
    };
    let first_data: TrackAddData =
        serde_json::from_value(first_data).expect("first call data is TrackAddData");

    let second = store
        .mutate_via_verb("track.add", args, Some("idem-track-add".into()))
        .expect("replay call should be Replayed");
    let MutateOutcome::Replayed {
        data: second_data,
        warnings,
        ..
    } = second
    else {
        panic!("second call must be Replayed, got {second:?}");
    };
    let second_data: TrackAddData =
        serde_json::from_value(second_data).expect("second call data is TrackAddData");

    assert_eq!(first_data.track_id, second_data.track_id);
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], "W_REPLAY");
}

#[test]
fn apply_chain_preserves_track_contiguity() {
    let prior = empty_project();
    let (patch, _) = compute_patch(
        &prior,
        &TrackAddArgs {
            project_id: FIXTURE_PROJECT_ID
                .parse()
                .expect("fixture project id parses"),
            kind: TrackKind::Video,
            name: None,
            index: Some(1),
        },
    )
    .expect("compute_patch should succeed");

    let typed_patch: json_patch::Patch =
        serde_json::from_value(patch).expect("track.add patch parses to RFC 6902");
    let post_state = prior
        .apply(&typed_patch)
        .expect("apply should preserve contiguity");

    assert_eq!(post_state.tracks[0].kind, TrackKind::Video);
    assert_eq!(post_state.tracks[1].kind, TrackKind::Video);
    assert_eq!(post_state.tracks[2].kind, TrackKind::Audio);
}
