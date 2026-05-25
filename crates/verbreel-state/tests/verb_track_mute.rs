//! Tests for `track.mute` (§4.4) — twelfth production verb.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::track_mute::{
    DEFAULT_MUTED, W_NOOP_CODE, compute_patch, data_envelope_from_post_state,
};
use verbreel_state::{
    MutateOutcome, Project, RecordedEvent, Track, TrackKind, TrackMuteArgs, TrackMuteData,
    TrackMuteError, TrackMuteVerb, Verb, VerbError, VerbRegistry, default_fixtures,
    default_registry, validate_reconstructors,
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

fn track(id: &str, kind: TrackKind, name: &str, locked: bool, muted: bool) -> Track {
    let mut track: Track = serde_json::from_value(json!({
        "id": id,
        "kind": kind,
        "name": name,
        "clips": [],
    }))
    .expect("track fixture value should deserialize");
    track.locked = locked;
    track.muted = muted;
    track
}

fn project_with_tracks(tracks: Vec<Track>) -> Project {
    let mut project = empty_project();
    project.tracks = tracks;
    project
}

fn patch_mute_value(patch: &Value) -> bool {
    let arr = patch.as_array().expect("patch is an array");
    assert_eq!(arr.len(), 1, "track.mute emits one replace op");
    let op = arr[0].as_object().expect("patch op is object");
    assert_eq!(op.get("op").and_then(Value::as_str), Some("replace"));
    op.get("value")
        .and_then(Value::as_bool)
        .expect("replace op value is bool")
}

#[test]
fn compute_patch_unmuted_track_mute_true() {
    let prior = project_with_tracks(vec![track(
        TRACK_VIDEO_A,
        TrackKind::Video,
        "Video 1",
        false,
        false,
    )]);

    let args = TrackMuteArgs {
        project_id: fixture_project_id(),
        track: TRACK_VIDEO_A.to_string(),
        muted: Some(true),
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("happy-path mute");
    assert!(patch_mute_value(&patch));
    assert!(warnings.is_empty());
    assert_eq!(data.track_id.to_string(), TRACK_VIDEO_A);
    assert!(data.muted);
    assert_eq!(
        patch.as_array().expect("patch is array")[0]
            .get("path")
            .and_then(Value::as_str),
        Some("/tracks/0/muted")
    );
}

#[test]
fn compute_patch_muted_track_unmute_false() {
    let prior = project_with_tracks(vec![track(
        TRACK_VIDEO_A,
        TrackKind::Video,
        "Video 1",
        false,
        true,
    )]);

    let args = TrackMuteArgs {
        project_id: fixture_project_id(),
        track: TRACK_VIDEO_A.to_string(),
        muted: Some(false),
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("happy-path unmute");
    assert!(!patch_mute_value(&patch));
    assert!(warnings.is_empty());
    assert_eq!(data.track_id.to_string(), TRACK_VIDEO_A);
    assert!(!data.muted);
}

#[test]
fn compute_patch_defaulted_muted_none_is_true() {
    let prior = project_with_tracks(vec![track(
        TRACK_VIDEO_A,
        TrackKind::Video,
        "Video 1",
        false,
        false,
    )]);

    let args = TrackMuteArgs {
        project_id: fixture_project_id(),
        track: TRACK_VIDEO_A.to_string(),
        muted: None,
    };

    let (patch, warnings, data) =
        compute_patch(&prior, &args).expect("omitted muted defaults true");
    assert_eq!(patch_mute_value(&patch), DEFAULT_MUTED);
    assert!(warnings.is_empty());
    assert_eq!(data.track_id.to_string(), TRACK_VIDEO_A);
    assert!(data.muted);
}

#[test]
fn compute_patch_idempotent_already_muted_emits_w_noop() {
    let prior = project_with_tracks(vec![track(
        TRACK_VIDEO_A,
        TrackKind::Video,
        "Video 1",
        false,
        true,
    )]);

    let args = TrackMuteArgs {
        project_id: fixture_project_id(),
        track: TRACK_VIDEO_A.to_string(),
        muted: Some(true),
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("same-state mute is a no-op");
    let patch_arr = patch.as_array().expect("patch is array");
    assert!(patch_arr.is_empty(), "same-state mute is a no-op");
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_NOOP_CODE);
    assert_eq!(warnings[0]["message"], "track mute state unchanged");
    assert_eq!(warnings[0]["details"]["track_id"], TRACK_VIDEO_A);
    assert_eq!(warnings[0]["details"]["muted"], true);
    assert_eq!(data.track_id.to_string(), TRACK_VIDEO_A);
    assert!(data.muted);
}

#[test]
fn compute_patch_idempotent_already_unmuted_emits_w_noop() {
    let prior = project_with_tracks(vec![track(
        TRACK_VIDEO_A,
        TrackKind::Video,
        "Video 1",
        false,
        false,
    )]);

    let args = TrackMuteArgs {
        project_id: fixture_project_id(),
        track: TRACK_VIDEO_A.to_string(),
        muted: Some(false),
    };

    let (patch, warnings, data) =
        compute_patch(&prior, &args).expect("same-state unmute is a no-op");
    assert!(patch.as_array().expect("patch is array").is_empty());
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_NOOP_CODE);
    assert_eq!(warnings[0]["message"], "track mute state unchanged");
    assert_eq!(warnings[0]["details"]["track_id"], TRACK_VIDEO_A);
    assert_eq!(warnings[0]["details"]["muted"], false);
    assert_eq!(data.track_id.to_string(), TRACK_VIDEO_A);
    assert!(!data.muted);
}

#[test]
fn compute_patch_muted_none_from_false_yields_patch_not_noop() {
    let prior = project_with_tracks(vec![track(
        TRACK_VIDEO_A,
        TrackKind::Video,
        "Video 1",
        false,
        false,
    )]);

    let args = TrackMuteArgs {
        project_id: fixture_project_id(),
        track: TRACK_VIDEO_A.to_string(),
        muted: None,
    };

    let (patch, warnings, data) =
        compute_patch(&prior, &args).expect("defaulted true should mutate");
    assert!(patch_mute_value(&patch));
    assert!(warnings.is_empty());
    assert_eq!(data.track_id.to_string(), TRACK_VIDEO_A);
    assert!(data.muted);
}

#[test]
fn compute_patch_locked_track_rejects_with_e_locked() {
    let prior = project_with_tracks(vec![track(
        TRACK_VIDEO_A,
        TrackKind::Video,
        "Video 1",
        true,
        false,
    )]);

    let err = compute_patch(
        &prior,
        &TrackMuteArgs {
            project_id: fixture_project_id(),
            track: TRACK_VIDEO_A.to_string(),
            muted: Some(true),
        },
    )
    .expect_err("locked track must reject");

    match err {
        TrackMuteError::Locked { track_id } => assert_eq!(track_id, TRACK_VIDEO_A),
        other => panic!("expected Locked, got {other:?}"),
    }
}

#[test]
fn compute_patch_bad_uuid_errors() {
    let prior = project_with_tracks(vec![]);

    let err = compute_patch(
        &prior,
        &TrackMuteArgs {
            project_id: fixture_project_id(),
            track: "not-a-uuid".to_string(),
            muted: Some(true),
        },
    )
    .expect_err("bad track selector must reject");

    match err {
        TrackMuteError::BadSelector { detail } => {
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
        true,
    )]);

    let err = compute_patch(
        &prior,
        &TrackMuteArgs {
            project_id: fixture_project_id(),
            track: MISSING_TRACK.to_string(),
            muted: Some(true),
        },
    )
    .expect_err("missing track must reject");

    match err {
        TrackMuteError::TrackNotFound { track_id } => assert_eq!(track_id, MISSING_TRACK),
        other => panic!("expected TrackNotFound, got {other:?}"),
    }
}

#[test]
fn data_envelope_returns_post_state_muted() {
    let mut post_state = empty_project();
    post_state.tracks = vec![track(
        TRACK_VIDEO_A,
        TrackKind::Video,
        "Video 1",
        false,
        true,
    )];

    let args = TrackMuteArgs {
        project_id: fixture_project_id(),
        track: TRACK_VIDEO_A.to_string(),
        muted: Some(false),
    };

    let data = data_envelope_from_post_state(&args, &post_state)
        .expect("post-state envelope should be readable");
    assert_eq!(data.track_id.to_string(), TRACK_VIDEO_A);
    assert!(data.muted);
}

#[test]
fn reconstructor_round_trip() {
    let prior = project_with_tracks(vec![track(
        TRACK_VIDEO_A,
        TrackKind::Video,
        "Video 1",
        false,
        false,
    )]);
    let args = TrackMuteArgs {
        project_id: fixture_project_id(),
        track: TRACK_VIDEO_A.to_string(),
        muted: Some(true),
    };

    let (patch, _warnings, _data) = compute_patch(&prior, &args).expect("compute_patch ok");
    let typed_patch: json_patch::Patch =
        serde_json::from_value(patch.clone()).expect("patch parses to typed JSON patch");
    let post_state = prior
        .apply(&typed_patch)
        .expect("track.mute patch should apply cleanly");

    let expected_data =
        serde_json::to_value(data_envelope_from_post_state(&args, &post_state).expect("envelope"))
            .expect("envelope serializes");

    let recorded = RecordedEvent {
        verb: "track.mute".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch,
        warnings: vec![],
        post_state,
        expected_data,
    };

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(TrackMuteVerb))
        .expect("register track.mute verb");

    let report = validate_reconstructors(&registry, std::slice::from_ref(&recorded))
        .expect("reconstructor validation should pass");
    assert_eq!(report.verbs_checked, vec!["track.mute"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn compute_patch_error_variants_map_to_bad_args() {
    let prior = project_with_tracks(vec![track(
        TRACK_VIDEO_A,
        TrackKind::Video,
        "Video 1",
        false,
        false,
    )]);
    let verb = TrackMuteVerb;

    let bad_selector = serde_json::to_value(TrackMuteArgs {
        project_id: fixture_project_id(),
        track: "not-a-uuid".to_string(),
        muted: Some(true),
    })
    .expect("bad selector args serialize");
    let err = verb
        .compute_patch(&prior, &bad_selector)
        .expect_err("bad selector maps");
    assert!(matches!(err, VerbError::BadArgs { .. }));

    let track_not_found = serde_json::to_value(TrackMuteArgs {
        project_id: fixture_project_id(),
        track: MISSING_TRACK.to_string(),
        muted: Some(true),
    })
    .expect("missing track args serialize");
    let err = verb
        .compute_patch(&prior, &track_not_found)
        .expect_err("missing track maps");
    assert!(matches!(err, VerbError::BadArgs { .. }));

    let locked = serde_json::to_value(TrackMuteArgs {
        project_id: fixture_project_id(),
        track: TRACK_VIDEO_A.to_string(),
        muted: Some(false),
    })
    .expect("locked args serialize");
    let prior_locked = project_with_tracks(vec![track(
        TRACK_VIDEO_A,
        TrackKind::Video,
        "Video 1",
        true,
        false,
    )]);
    let err = verb
        .compute_patch(&prior_locked, &locked)
        .expect_err("locked maps");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "track.mute")
        .expect("default_fixtures includes track.mute");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(TrackMuteVerb))
        .expect("register track.mute verb");

    let report = validate_reconstructors(&registry, &[fixture])
        .expect("track.mute reconstruct from fixture");
    assert_eq!(report.verbs_checked, vec!["track.mute"]);
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
        "muted": true,
    });

    let outcome = store
        .mutate_via_verb("track.mute", args, None)
        .expect("mutate_via_verb happy path");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("happy path must return Applied, got {outcome:?}");
    };

    let data: TrackMuteData =
        serde_json::from_value(data).expect("track.mute data is TrackMuteData");
    assert_eq!(data.track_id.to_string(), track_id);
    assert!(data.muted);
    assert!(store.project().tracks[0].muted);
    assert_eq!(warnings, Vec::<Value>::new());
}
