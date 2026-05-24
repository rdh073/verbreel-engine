//! Tests for `track.lock` (§4.6) — eleventh production verb.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::track_lock::{
    DEFAULT_LOCKED, W_NOOP_CODE, compute_patch, data_envelope_from_post_state,
};
use verbreel_state::{
    MutateOutcome, Project, RecordedEvent, Track, TrackKind, TrackLockArgs, TrackLockData,
    TrackLockError, TrackLockVerb, VerbRegistry, default_fixtures, default_registry,
    validate_reconstructors,
};
use verbreel_types::ProjectId;

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const TRACK_VIDEO_A: &str = "0190b8d3-15e3-7000-bd00-0000000aa101";
const MISSING_TRACK: &str = "0190b8d3-15e3-7000-bd00-0000000cc101";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture → Project")
}

fn fixture_project_id() -> ProjectId {
    empty_project().id
}

fn track(id: &str, kind: TrackKind, name: &str, locked: bool) -> Track {
    let mut track: Track = serde_json::from_value(json!({
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

fn patch_lock_value(patch: &Value) -> bool {
    let arr = patch.as_array().expect("patch is an array");
    assert_eq!(arr.len(), 1, "track.lock emits one replace op");
    let op = arr[0].as_object().expect("patch op is object");
    assert_eq!(op.get("op").and_then(Value::as_str), Some("replace"));
    op.get("value")
        .and_then(Value::as_bool)
        .expect("replace op value is bool")
}

#[test]
fn compute_patch_lock_unlocked_track() {
    let prior = project_with_tracks(vec![track(
        TRACK_VIDEO_A,
        TrackKind::Video,
        "Video 1",
        false,
    )]);

    let args = TrackLockArgs {
        project_id: fixture_project_id(),
        track: TRACK_VIDEO_A.to_string(),
        locked: Some(true),
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("happy-path lock");
    assert!(patch_lock_value(&patch));
    assert!(warnings.is_empty());
    assert_eq!(data.track_id.to_string(), TRACK_VIDEO_A);
    assert!(data.locked);
    assert_eq!(
        patch.as_array().expect("patch is array")[0]
            .get("path")
            .and_then(Value::as_str),
        Some("/tracks/0/locked")
    );
}

#[test]
fn compute_patch_unlock_locked_track() {
    let prior = project_with_tracks(vec![track(
        TRACK_VIDEO_A,
        TrackKind::Video,
        "Video 1",
        true,
    )]);

    let args = TrackLockArgs {
        project_id: fixture_project_id(),
        track: TRACK_VIDEO_A.to_string(),
        locked: Some(false),
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("happy-path unlock");
    assert!(!patch_lock_value(&patch));
    assert!(warnings.is_empty());
    assert_eq!(data.track_id.to_string(), TRACK_VIDEO_A);
    assert!(!data.locked);
}

#[test]
fn compute_patch_locked_omitted_defaults_to_true() {
    let prior = project_with_tracks(vec![track(
        TRACK_VIDEO_A,
        TrackKind::Video,
        "Video 1",
        false,
    )]);

    let args = TrackLockArgs {
        project_id: fixture_project_id(),
        track: TRACK_VIDEO_A.to_string(),
        locked: None,
    };

    let (patch, warnings, data) =
        compute_patch(&prior, &args).expect("omitted locked defaults true");
    assert_eq!(patch_lock_value(&patch), DEFAULT_LOCKED);
    assert!(warnings.is_empty());
    assert_eq!(data.track_id.to_string(), TRACK_VIDEO_A);
    assert!(data.locked);
}

#[test]
fn compute_patch_idempotent_already_locked_emits_w_noop() {
    let prior = project_with_tracks(vec![track(
        TRACK_VIDEO_A,
        TrackKind::Video,
        "Video 1",
        true,
    )]);

    let args = TrackLockArgs {
        project_id: fixture_project_id(),
        track: TRACK_VIDEO_A.to_string(),
        locked: Some(true),
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("idempotent lock");
    let patch_arr = patch.as_array().expect("patch is array");
    assert!(patch_arr.is_empty(), "same-state lock is a no-op");
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_NOOP_CODE);
    assert_eq!(warnings[0]["message"], "track lock state unchanged");
    assert_eq!(warnings[0]["details"]["track_id"], TRACK_VIDEO_A);
    assert_eq!(warnings[0]["details"]["locked"], true);
    assert_eq!(data.track_id.to_string(), TRACK_VIDEO_A);
    assert!(data.locked);
}

#[test]
fn compute_patch_idempotent_already_unlocked_emits_w_noop() {
    let prior = project_with_tracks(vec![track(
        TRACK_VIDEO_A,
        TrackKind::Video,
        "Video 1",
        false,
    )]);

    let args = TrackLockArgs {
        project_id: fixture_project_id(),
        track: TRACK_VIDEO_A.to_string(),
        locked: Some(false),
    };

    let (patch, warnings, data) =
        compute_patch(&prior, &args).expect("idempotent unlock is a no-op");
    assert!(patch.as_array().expect("patch is array").is_empty());
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_NOOP_CODE);
    assert_eq!(warnings[0]["message"], "track lock state unchanged");
    assert_eq!(warnings[0]["details"]["track_id"], TRACK_VIDEO_A);
    assert_eq!(warnings[0]["details"]["locked"], false);
    assert_eq!(data.track_id.to_string(), TRACK_VIDEO_A);
    assert!(!data.locked);
}

#[test]
fn compute_patch_bad_uuid_errors() {
    let prior = project_with_tracks(vec![]);

    let err = compute_patch(
        &prior,
        &TrackLockArgs {
            project_id: fixture_project_id(),
            track: "not-a-uuid".to_string(),
            locked: Some(true),
        },
    )
    .expect_err("bad track selector must reject");

    match err {
        TrackLockError::BadSelector { detail } => {
            assert!(detail.contains("UUID"), "{detail}");
        }
        other => panic!("expected BadSelector, got {other:?}"),
    }
}

#[test]
fn compute_patch_track_not_found_errors() {
    let prior = project_with_tracks(vec![track(
        TRACK_VIDEO_A,
        TrackKind::Video,
        "Video 1",
        false,
    )]);

    let err = compute_patch(
        &prior,
        &TrackLockArgs {
            project_id: fixture_project_id(),
            track: MISSING_TRACK.to_string(),
            locked: Some(true),
        },
    )
    .expect_err("missing track must reject");

    match err {
        TrackLockError::TrackNotFound { track_id } => assert_eq!(track_id, MISSING_TRACK),
        other => panic!("expected TrackNotFound, got {other:?}"),
    }
}

#[test]
fn compute_patch_locked_explicit_false_unlocks() {
    let prior = project_with_tracks(vec![track(
        TRACK_VIDEO_A,
        TrackKind::Video,
        "Video 1",
        true,
    )]);

    let args = TrackLockArgs {
        project_id: fixture_project_id(),
        track: TRACK_VIDEO_A.to_string(),
        locked: Some(false),
    };

    let (patch, warnings, data) =
        compute_patch(&prior, &args).expect("explicit false should unlock");
    assert!(!patch_lock_value(&patch));
    assert!(warnings.is_empty());
    assert!(!data.locked);
}

#[test]
fn data_envelope_returns_post_state_locked() {
    let mut post_state = empty_project();
    post_state.tracks = vec![track(TRACK_VIDEO_A, TrackKind::Video, "Video 1", true)];

    let args = TrackLockArgs {
        project_id: fixture_project_id(),
        track: TRACK_VIDEO_A.to_string(),
        locked: Some(false),
    };

    let data = data_envelope_from_post_state(&args, &post_state)
        .expect("post-state envelope should be readable");
    assert_eq!(data.track_id.to_string(), TRACK_VIDEO_A);
    assert!(data.locked);
}

#[test]
fn reconstructor_round_trip() {
    let prior = project_with_tracks(vec![track(
        TRACK_VIDEO_A,
        TrackKind::Video,
        "Video 1",
        false,
    )]);
    let args = TrackLockArgs {
        project_id: fixture_project_id(),
        track: TRACK_VIDEO_A.to_string(),
        locked: Some(true),
    };

    let (patch, _warnings, _data) = compute_patch(&prior, &args).expect("compute_patch ok");
    let typed_patch: json_patch::Patch =
        serde_json::from_value(patch.clone()).expect("patch parses to typed JSON patch");
    let post_state = prior
        .apply(&typed_patch)
        .expect("track.lock patch should apply cleanly");

    let expected_data =
        serde_json::to_value(data_envelope_from_post_state(&args, &post_state).expect("envelope"))
            .expect("envelope serializes");

    let recorded = RecordedEvent {
        verb: "track.lock".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch,
        warnings: vec![],
        post_state,
        expected_data,
    };

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(TrackLockVerb))
        .expect("register track.lock verb");

    let report = validate_reconstructors(&registry, std::slice::from_ref(&recorded))
        .expect("reconstructor validation should pass");
    assert_eq!(report.verbs_checked, vec!["track.lock"]);
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
        "locked": true,
    });

    let outcome = store
        .mutate_via_verb("track.lock", args, None)
        .expect("mutate_via_verb happy path");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("happy path must return Applied, got {outcome:?}");
    };

    let data: TrackLockData =
        serde_json::from_value(data).expect("track.lock data is TrackLockData");
    assert_eq!(data.track_id.to_string(), track_id);
    assert!(store.project().tracks[0].locked);
    assert!(data.locked);
    assert_eq!(warnings, Vec::<Value>::new());
}

#[cfg(feature = "native")]
#[test]
fn replay_returns_same_locked_state() {
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
        "locked": true,
    });

    let first = store
        .mutate_via_verb("track.lock", args.clone(), Some("idem-track-lock".into()))
        .expect("first call should apply");
    let MutateOutcome::Applied {
        data: first_data, ..
    } = first
    else {
        panic!("first apply must be Applied, got {first:?}");
    };
    let first_data: TrackLockData =
        serde_json::from_value(first_data).expect("first result data is TrackLockData");

    let second = store
        .mutate_via_verb("track.lock", args, Some("idem-track-lock".into()))
        .expect("replay call should be Replayed");
    let MutateOutcome::Replayed {
        data: second_data,
        warnings,
        ..
    } = second
    else {
        panic!("second call must be Replayed, got {second:?}");
    };
    let second_data: TrackLockData =
        serde_json::from_value(second_data).expect("replayed result data is TrackLockData");

    assert_eq!(first_data, second_data);
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], "W_REPLAY");
}

#[cfg(feature = "native")]
#[test]
fn lock_then_unlock_round_trip() {
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

    let lock = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "track": &track_id,
        "locked": true,
    });

    let first = store
        .mutate_via_verb("track.lock", lock, None)
        .expect("first call should lock");
    let MutateOutcome::Applied { .. } = first else {
        panic!("first call must apply");
    };
    assert!(store.project().tracks[0].locked);

    let unlock = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "track": track_id,
        "locked": false,
    });

    let outcome = store
        .mutate_via_verb("track.lock", unlock, None)
        .expect("unlock call should apply due carve-out");
    let MutateOutcome::Applied { data, .. } = outcome else {
        panic!("unlock must apply");
    };

    let data: TrackLockData = serde_json::from_value(data).expect("unlock data is TrackLockData");
    assert_eq!(data.track_id.to_string(), track_id);
    assert!(!data.locked);
}
