//! Tests for `track.rename` (§4.7) — tenth verb in track noun arc.

use std::sync::Arc;

use serde_json::{json, Value};
use verbreel_state::{
    MutateOutcome, Project, RecordedEvent, Track, TrackKind, TrackRenameArgs, TrackRenameData,
    TrackRenameError, TrackRenameVerb, VerbRegistry, default_fixtures, default_registry,
    validate_reconstructors,
    verbs::track_rename::{
        compute_patch, data_envelope_from_post_state, TRACK_NAME_MAX, W_NOOP_CODE,
    },
};
use verbreel_types::ProjectId;

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";

const TRACK_VIDEO_A: &str = "0190b8d3-15e3-7000-bd00-0000000aa101";
const TRACK_VIDEO_B: &str = "0190b8d3-15e3-7000-bd00-0000000aa102";
const TRACK_AUDIO_A: &str = "0190b8d3-15e3-7000-bd00-0000000bb101";
const MISSING_TRACK: &str = "0190b8d3-15e3-7000-bd00-0000000cc101";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture → Project")
}

fn fixture_project_id() -> ProjectId {
    empty_project().id
}

fn track(id: &str, kind: TrackKind, name: &str, locked: bool) -> Track {
    let mut track: Track =
        serde_json::from_value(json!({
            "id": id,
            "kind": kind,
            "name": name,
            "clips": [],
        }))
        .expect("track fixture value should deserialize");
    track.locked = locked;
    track
}

fn project_with_tracks(tracks: Vec<Track>) -> Project {
    let mut project = empty_project();
    project.tracks = tracks;
    project
}

fn patch_rename_name(patch: &Value) -> &Value {
    let arr = patch.as_array().expect("patch is an array");
    assert_eq!(arr.len(), 1, "rename emits single replace op");
    let op = arr[0].as_object().expect("patch op is object");
    assert_eq!(op.get("op").and_then(Value::as_str), Some("replace"));
    op.get("value").expect("replace op carries a value")
}

#[test]
fn compute_patch_simple_rename_succeeds() {
    let prior = project_with_tracks(vec![track(TRACK_VIDEO_A, TrackKind::Video, "Original", false)]);

    let args = TrackRenameArgs {
        project_id: fixture_project_id(),
        track: TRACK_VIDEO_A.to_string(),
        name: "Main Camera".to_string(),
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("happy-path rename");

    assert_eq!(patch_rename_name(&patch), "Main Camera");
    assert!(warnings.is_empty());
    assert_eq!(data.track_id.to_string(), TRACK_VIDEO_A);
    assert_eq!(data.name, "Main Camera");
    assert_eq!(
        patch
            .as_array()
            .expect("patch is array")[0]
            .get("path")
            .and_then(Value::as_str),
        Some("/tracks/0/name")
    );
}

#[test]
fn compute_patch_rename_to_same_name_emits_w_noop() {
    let prior = project_with_tracks(vec![track(TRACK_VIDEO_A, TrackKind::Video, "Main Camera", false)]);

    let args = TrackRenameArgs {
        project_id: fixture_project_id(),
        track: TRACK_VIDEO_A.to_string(),
        name: "Main Camera".to_string(),
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("same-name rename should succeed");
    let patch_arr = patch.as_array().expect("patch is array");
    assert!(patch_arr.is_empty(), "same-name rename is a no-op");
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_NOOP_CODE);
    assert_eq!(warnings[0]["message"], "track name unchanged");
    assert_eq!(warnings[0]["details"]["track_id"], TRACK_VIDEO_A);

    assert_eq!(data.track_id.to_string(), TRACK_VIDEO_A);
    assert_eq!(data.name, "Main Camera");
}

#[test]
fn compute_patch_bad_uuid_errors() {
    let prior = project_with_tracks(vec![]);

    let err = compute_patch(
        &prior,
        &TrackRenameArgs {
            project_id: fixture_project_id(),
            track: "not-a-uuid".to_string(),
            name: "Main Camera".to_string(),
        },
    )
    .expect_err("bad track selector must reject");

    match err {
        TrackRenameError::BadSelector { detail } => {
            assert!(detail.contains("UUID"), "{detail}");
        }
        other => panic!("expected BadSelector, got {other:?}"),
    }
}

#[test]
fn compute_patch_track_not_found_errors() {
    let prior = project_with_tracks(vec![track(TRACK_VIDEO_A, TrackKind::Video, "Video 1", false)]);

    let err = compute_patch(
        &prior,
        &TrackRenameArgs {
            project_id: fixture_project_id(),
            track: MISSING_TRACK.to_string(),
            name: "Main Camera".to_string(),
        },
    )
    .expect_err("missing track must reject");

    match err {
        TrackRenameError::TrackNotFound { track_id } => assert_eq!(track_id, MISSING_TRACK),
        other => panic!("expected TrackNotFound, got {other:?}"),
    }
}

#[test]
fn compute_patch_locked_track_errors() {
    let prior = project_with_tracks(vec![track(TRACK_VIDEO_A, TrackKind::Video, "Locked", true)]);

    let err = compute_patch(
        &prior,
        &TrackRenameArgs {
            project_id: fixture_project_id(),
            track: TRACK_VIDEO_A.to_string(),
            name: "Main Camera".to_string(),
        },
    )
    .expect_err("locked track must reject");

    match err {
        TrackRenameError::Locked {
            track_id,
            track_name,
        } => {
            assert_eq!(track_id, TRACK_VIDEO_A);
            assert_eq!(track_name, "Locked");
        }
        other => panic!("expected Locked, got {other:?}"),
    }
}

#[test]
fn compute_patch_empty_name_errors() {
    let prior = project_with_tracks(vec![track(TRACK_VIDEO_A, TrackKind::Video, "Video 1", false)]);

    let err = compute_patch(
        &prior,
        &TrackRenameArgs {
            project_id: fixture_project_id(),
            track: TRACK_VIDEO_A.to_string(),
            name: "".to_string(),
        },
    )
    .expect_err("empty names must reject");

    assert!(matches!(err, TrackRenameError::NameEmpty));
}

#[test]
fn compute_patch_129_char_name_errors() {
    let prior = project_with_tracks(vec![track(TRACK_VIDEO_A, TrackKind::Video, "Video 1", false)]);

    let err = compute_patch(
        &prior,
        &TrackRenameArgs {
            project_id: fixture_project_id(),
            track: TRACK_VIDEO_A.to_string(),
            name: "a".repeat(129),
        },
    )
    .expect_err("too-long names must reject");

    match err {
        TrackRenameError::NameTooLong { actual, max } => {
            assert_eq!(actual, 129);
            assert_eq!(max, TRACK_NAME_MAX);
        }
        other => panic!("expected NameTooLong, got {other:?}"),
    }
}

#[test]
fn compute_patch_unicode_name_chars_counted() {
    let prior = project_with_tracks(vec![track(TRACK_VIDEO_A, TrackKind::Video, "Video 1", false)]);
    let name = "界".repeat(128);
    assert_eq!(name.chars().count(), 128);
    assert!(name.len() > 128);

    let args = TrackRenameArgs {
        project_id: fixture_project_id(),
        track: TRACK_VIDEO_A.to_string(),
        name: name.clone(),
    };

    let (patch, _warnings, data) = compute_patch(&prior, &args).expect("unicode 128 chars should pass");
    assert_eq!(patch_rename_name(&patch), name);
    assert_eq!(patch.as_array().expect("patch array")[0]
        .get("path")
        .and_then(Value::as_str),
        Some("/tracks/0/name")
    );
    assert_eq!(data.name, args.name);
}

#[test]
fn compute_patch_name_conflict_within_kind_errors() {
    let prior = project_with_tracks(vec![
        track(TRACK_VIDEO_A, TrackKind::Video, "Foo", false),
        track(TRACK_VIDEO_B, TrackKind::Video, "Bar", false),
    ]);

    let err = compute_patch(
        &prior,
        &TrackRenameArgs {
            project_id: fixture_project_id(),
            track: TRACK_VIDEO_B.to_string(),
            name: "Foo".to_string(),
        },
    )
    .expect_err("same-kind conflict must reject");

    match err {
        TrackRenameError::NameConflict { name, kind } => {
            assert_eq!(name, "Foo");
            assert_eq!(kind, TrackKind::Video);
        }
        other => panic!("expected NameConflict, got {other:?}"),
    }
}

#[test]
fn compute_patch_name_conflict_excludes_self() {
    let prior = project_with_tracks(vec![track(TRACK_VIDEO_A, TrackKind::Video, "Foo", false)]);

    let (patch, warnings, data) = compute_patch(
        &prior,
        &TrackRenameArgs {
            project_id: fixture_project_id(),
            track: TRACK_VIDEO_A.to_string(),
            name: "Foo".to_string(),
        },
    )
    .expect("self-name reuse is a no-op");

    assert!(patch.as_array().expect("patch array").is_empty());
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_NOOP_CODE);
    assert_eq!(data.name, "Foo");
}

#[test]
fn compute_patch_name_no_conflict_across_kinds() {
    let prior = project_with_tracks(vec![
        track(TRACK_VIDEO_A, TrackKind::Video, "Foo", false),
        track(TRACK_AUDIO_A, TrackKind::Audio, "Audio 1", false),
    ]);

    let (patch, warnings, data) = compute_patch(
        &prior,
        &TrackRenameArgs {
            project_id: fixture_project_id(),
            track: TRACK_AUDIO_A.to_string(),
            name: "Foo".to_string(),
        },
    )
    .expect("cross-kind conflict is allowed");

    assert!(warnings.is_empty());
    assert_eq!(data.name, "Foo");
    assert_eq!(
        patch
            .as_array()
            .expect("patch array")[0]
            .get("path")
            .and_then(Value::as_str),
        Some("/tracks/1/name")
    );
}

#[test]
fn data_envelope_returns_post_state_name() {
    let mut post_state = empty_project();
    post_state.tracks = vec![track(TRACK_VIDEO_A, TrackKind::Video, "Renamed", false)];

    let args = TrackRenameArgs {
        project_id: fixture_project_id(),
        track: TRACK_VIDEO_A.to_string(),
        name: "Renamed".to_string(),
    };

    let data = data_envelope_from_post_state(&args, &post_state)
        .expect("post-state envelope should be readable");
    assert_eq!(data.track_id.to_string(), TRACK_VIDEO_A);
    assert_eq!(data.name, "Renamed");
}

#[test]
fn reconstructor_round_trip() {
    let prior = project_with_tracks(vec![track(TRACK_VIDEO_A, TrackKind::Video, "Video 1", false)]);
    let args = TrackRenameArgs {
        project_id: fixture_project_id(),
        track: TRACK_VIDEO_A.to_string(),
        name: "Renamed".to_string(),
    };

    let (patch, _warnings, _data) = compute_patch(&prior, &args).expect("compute_patch ok");
    let typed_patch: json_patch::Patch =
        serde_json::from_value(patch.clone()).expect("patch parses to typed JSON patch");
    let post_state = prior
        .apply(&typed_patch)
        .expect("track.rename patch should apply cleanly");

    let expected_data = serde_json::to_value(data_envelope_from_post_state(&args, &post_state).expect("envelope"))
        .expect("envelope serializes");

    let recorded = RecordedEvent {
        verb: "track.rename".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch,
        warnings: vec![],
        post_state,
        expected_data,
    };

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(TrackRenameVerb))
        .expect("register track.rename verb");

    let report = validate_reconstructors(&registry, std::slice::from_ref(&recorded))
        .expect("reconstructor validation should pass");
    assert_eq!(report.verbs_checked, vec!["track.rename"]);
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
    .expect("create_with_registry clears the gate and writes project.json");

    let track_id = store.project().tracks[0].id.to_string();

    let args = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "track": &track_id,
        "name": "Main Camera",
    });

    let outcome = store
        .mutate_via_verb("track.rename", args, None)
        .expect("mutate_via_verb happy path");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("happy path must return Applied, got {outcome:?}");
    };

    let data: TrackRenameData = serde_json::from_value(data).expect("track.rename data is TrackRenameData");
    assert_eq!(data.track_id.to_string(), track_id);
    assert_eq!(store.project().tracks[0].name, "Main Camera");
    assert_eq!(warnings, Vec::<Value>::new());
}

#[cfg(feature = "native")]
#[test]
fn replay_returns_same_data_envelope() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        empty_project(),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry clears the gate and writes project.json");

    let track_id = store.project().tracks[0].id.to_string();

    let args = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "track": track_id,
        "name": "Main Camera",
    });

    let first = store
        .mutate_via_verb("track.rename", args.clone(), Some("idem-track-rename".into()))
        .expect("first call should apply");
    let MutateOutcome::Applied { data: first_data, .. } = first else {
        panic!("first apply must be Applied, got {first:?}");
    };
    let first_data: TrackRenameData = serde_json::from_value(first_data)
        .expect("first result data is TrackRenameData");

    let second = store
        .mutate_via_verb("track.rename", args, Some("idem-track-rename".into()))
        .expect("replay call should be Replayed");
    let MutateOutcome::Replayed {
        data: second_data,
        warnings,
        ..
    } = second
    else {
        panic!("second call must be Replayed, got {second:?}");
    };
    let second_data: TrackRenameData = serde_json::from_value(second_data)
        .expect("replayed result data is TrackRenameData");

    assert_eq!(first_data, second_data);
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], "W_REPLAY");
}

#[test]
fn apply_chain_preserves_track_contiguity() {
    let prior = project_with_tracks(vec![
        track(TRACK_VIDEO_A, TrackKind::Video, "Video 1", false),
        track(TRACK_VIDEO_B, TrackKind::Video, "Video 2", false),
        track(TRACK_AUDIO_A, TrackKind::Audio, "Audio 1", false),
    ]);

    let args = TrackRenameArgs {
        project_id: fixture_project_id(),
        track: TRACK_VIDEO_B.to_string(),
        name: "Video 2 Renamed".to_string(),
    };

    let (patch, _warnings, _data) = compute_patch(&prior, &args).expect("compute_patch should succeed");
    let typed_patch: json_patch::Patch = serde_json::from_value(patch).expect("patch parses to RFC 6902");
    let post_state = prior
        .apply(&typed_patch)
        .expect("apply should preserve contiguity");

    assert_eq!(post_state.tracks[0].kind, TrackKind::Video);
    assert_eq!(post_state.tracks[1].kind, TrackKind::Video);
    assert_eq!(post_state.tracks[2].kind, TrackKind::Audio);
}
