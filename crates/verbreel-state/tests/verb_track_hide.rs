//! Tests for `track.hide` (§4.10) — fourteenth production verb.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::track_hide::{
    DEFAULT_HIDDEN, W_NOOP_CODE, compute_patch, data_envelope_from_post_state,
};
use verbreel_state::{
    MutateOutcome, Project, RecordedEvent, Track, TrackHideArgs, TrackHideData, TrackHideError,
    TrackHideVerb, TrackKind, Verb, VerbError, VerbRegistry, default_fixtures, default_registry,
    validate_reconstructors,
};
use verbreel_types::ProjectId;

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const TRACK_VIDEO_A: &str = "0190b8d3-15e3-7000-bd00-0000000aa101";
const TRACK_VIDEO_B: &str = "0190b8d3-15e3-7000-bd00-0000000bb101";
const MISSING_TRACK: &str = "0190b8d3-15e3-7000-bd00-0000000cc101";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture → Project")
}

fn fixture_project_id() -> ProjectId {
    empty_project().id
}

fn track(id: &str, kind: TrackKind, name: &str, locked: bool, hidden: bool) -> Track {
    let mut track: Track = serde_json::from_value(json!({
        "id": id,
        "kind": kind,
        "name": name,
        "clips": [],
    }))
    .expect("track fixture value should deserialize");
    track.locked = locked;
    track.hidden = hidden;
    track
}

fn project_with_tracks(tracks: Vec<Track>) -> Project {
    let mut project = empty_project();
    project.tracks = tracks;
    project
}

fn patch_solo_value(patch: &Value) -> bool {
    let arr = patch.as_array().expect("patch is an array");
    assert_eq!(arr.len(), 1, "track.hide emits one replace op");
    let op = arr[0].as_object().expect("patch op is object");
    assert_eq!(op.get("op").and_then(Value::as_str), Some("replace"));
    op.get("value")
        .and_then(Value::as_bool)
        .expect("replace op value is bool")
}

#[test]
fn compute_patch_unsoloed_track_solo_true() {
    let prior = project_with_tracks(vec![track(
        TRACK_VIDEO_A,
        TrackKind::Video,
        "Video 1",
        false,
        false,
    )]);

    let args = TrackHideArgs {
        project_id: fixture_project_id(),
        track: TRACK_VIDEO_A.to_string(),
        hidden: Some(true),
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("happy-path hidden");
    assert!(patch_solo_value(&patch));
    assert!(warnings.is_empty());
    assert_eq!(data.track_id.to_string(), TRACK_VIDEO_A);
    assert!(data.hidden);
    assert_eq!(
        patch.as_array().expect("patch is array")[0]
            .get("path")
            .and_then(Value::as_str),
        Some("/tracks/0/hidden")
    );
}

#[test]
fn compute_patch_soloed_track_unsolo_false() {
    let prior = project_with_tracks(vec![track(
        TRACK_VIDEO_A,
        TrackKind::Video,
        "Video 1",
        false,
        true,
    )]);

    let args = TrackHideArgs {
        project_id: fixture_project_id(),
        track: TRACK_VIDEO_A.to_string(),
        hidden: Some(false),
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("happy-path un-hidden");
    assert!(!patch_solo_value(&patch));
    assert!(warnings.is_empty());
    assert_eq!(data.track_id.to_string(), TRACK_VIDEO_A);
    assert!(!data.hidden);
}

#[test]
fn compute_patch_defaulted_solo_none_is_true() {
    let prior = project_with_tracks(vec![track(
        TRACK_VIDEO_A,
        TrackKind::Video,
        "Video 1",
        false,
        false,
    )]);

    let args = TrackHideArgs {
        project_id: fixture_project_id(),
        track: TRACK_VIDEO_A.to_string(),
        hidden: None,
    };

    let (patch, warnings, data) =
        compute_patch(&prior, &args).expect("omitted hidden defaults true");
    assert_eq!(patch_solo_value(&patch), DEFAULT_HIDDEN);
    assert!(warnings.is_empty());
    assert_eq!(data.track_id.to_string(), TRACK_VIDEO_A);
    assert!(data.hidden);
}

#[test]
fn compute_patch_idempotent_already_soloed_emits_w_noop() {
    let prior = project_with_tracks(vec![track(
        TRACK_VIDEO_A,
        TrackKind::Video,
        "Video 1",
        false,
        true,
    )]);

    let args = TrackHideArgs {
        project_id: fixture_project_id(),
        track: TRACK_VIDEO_A.to_string(),
        hidden: Some(true),
    };

    let (patch, warnings, data) =
        compute_patch(&prior, &args).expect("same-state hidden is a no-op");
    let patch_arr = patch.as_array().expect("patch is array");
    assert!(patch_arr.is_empty(), "same-state hidden is a no-op");
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_NOOP_CODE);
    assert_eq!(warnings[0]["message"], "track hidden state unchanged");
    assert_eq!(warnings[0]["details"]["track_id"], TRACK_VIDEO_A);
    assert_eq!(warnings[0]["details"]["hidden"], true);
    assert_eq!(data.track_id.to_string(), TRACK_VIDEO_A);
    assert!(data.hidden);
}

#[test]
fn compute_patch_idempotent_already_not_soloed_emits_w_noop() {
    let prior = project_with_tracks(vec![track(
        TRACK_VIDEO_A,
        TrackKind::Video,
        "Video 1",
        false,
        false,
    )]);

    let args = TrackHideArgs {
        project_id: fixture_project_id(),
        track: TRACK_VIDEO_A.to_string(),
        hidden: Some(false),
    };

    let (patch, warnings, data) =
        compute_patch(&prior, &args).expect("same-state unsolo is a no-op");
    assert!(patch.as_array().expect("patch is array").is_empty());
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_NOOP_CODE);
    assert_eq!(warnings[0]["message"], "track hidden state unchanged");
    assert_eq!(warnings[0]["details"]["track_id"], TRACK_VIDEO_A);
    assert_eq!(warnings[0]["details"]["hidden"], false);
    assert_eq!(data.track_id.to_string(), TRACK_VIDEO_A);
    assert!(!data.hidden);
}

#[test]
fn compute_patch_defaulted_none_from_false_yields_patch_not_noop() {
    let prior = project_with_tracks(vec![track(
        TRACK_VIDEO_A,
        TrackKind::Video,
        "Video 1",
        false,
        false,
    )]);

    let args = TrackHideArgs {
        project_id: fixture_project_id(),
        track: TRACK_VIDEO_A.to_string(),
        hidden: None,
    };

    let (patch, warnings, data) =
        compute_patch(&prior, &args).expect("defaulted true should mutate");
    assert!(patch_solo_value(&patch));
    assert!(warnings.is_empty());
    assert_eq!(data.track_id.to_string(), TRACK_VIDEO_A);
    assert!(data.hidden);
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
        &TrackHideArgs {
            project_id: fixture_project_id(),
            track: TRACK_VIDEO_A.to_string(),
            hidden: Some(true),
        },
    )
    .expect_err("locked track must reject");

    match err {
        TrackHideError::Locked { track_id } => assert_eq!(track_id, TRACK_VIDEO_A),
        other => panic!("expected Locked, got {other:?}"),
    }
}

#[test]
fn compute_patch_bad_uuid_errors() {
    let prior = project_with_tracks(vec![]);

    let err = compute_patch(
        &prior,
        &TrackHideArgs {
            project_id: fixture_project_id(),
            track: "not-a-uuid".to_string(),
            hidden: Some(true),
        },
    )
    .expect_err("bad track selector must reject");

    match err {
        TrackHideError::BadSelector { detail } => {
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
        &TrackHideArgs {
            project_id: fixture_project_id(),
            track: MISSING_TRACK.to_string(),
            hidden: Some(true),
        },
    )
    .expect_err("missing track must reject");

    match err {
        TrackHideError::TrackNotFound { track_id } => assert_eq!(track_id, MISSING_TRACK),
        other => panic!("expected TrackNotFound, got {other:?}"),
    }
}

#[test]
fn data_envelope_returns_post_state_solo() {
    let mut post_state = empty_project();
    post_state.tracks = vec![track(
        TRACK_VIDEO_A,
        TrackKind::Video,
        "Video 1",
        false,
        true,
    )];

    let args = TrackHideArgs {
        project_id: fixture_project_id(),
        track: TRACK_VIDEO_A.to_string(),
        hidden: Some(false),
    };

    let data = data_envelope_from_post_state(&args, &post_state)
        .expect("post-state envelope should be readable");
    assert_eq!(data.track_id.to_string(), TRACK_VIDEO_A);
    assert!(data.hidden);
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
    let args = TrackHideArgs {
        project_id: fixture_project_id(),
        track: TRACK_VIDEO_A.to_string(),
        hidden: Some(true),
    };

    let (patch, _warnings, _data) = compute_patch(&prior, &args).expect("compute_patch ok");
    let typed_patch: json_patch::Patch =
        serde_json::from_value(patch.clone()).expect("patch parses to typed JSON patch");
    let post_state = prior
        .apply(&typed_patch)
        .expect("track.hide patch should apply cleanly");

    let expected_data =
        serde_json::to_value(data_envelope_from_post_state(&args, &post_state).expect("envelope"))
            .expect("envelope serializes");

    let recorded = RecordedEvent {
        verb: "track.hide".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch,
        warnings: vec![],
        post_state,
        expected_data,
    };

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(TrackHideVerb))
        .expect("register track.hide verb");

    let report = validate_reconstructors(&registry, std::slice::from_ref(&recorded))
        .expect("reconstructor validation should pass");
    assert_eq!(report.verbs_checked, vec!["track.hide"]);
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
    let verb = TrackHideVerb;

    let bad_selector = serde_json::to_value(TrackHideArgs {
        project_id: fixture_project_id(),
        track: "not-a-uuid".to_string(),
        hidden: Some(true),
    })
    .expect("bad selector args serialize");
    let err = verb
        .compute_patch(&prior, &bad_selector)
        .expect_err("bad selector maps");
    assert!(matches!(err, VerbError::BadArgs { .. }));

    let track_not_found = serde_json::to_value(TrackHideArgs {
        project_id: fixture_project_id(),
        track: MISSING_TRACK.to_string(),
        hidden: Some(true),
    })
    .expect("missing track args serialize");
    let err = verb
        .compute_patch(&prior, &track_not_found)
        .expect_err("missing track maps");
    assert!(matches!(err, VerbError::BadArgs { .. }));

    let locked = serde_json::to_value(TrackHideArgs {
        project_id: fixture_project_id(),
        track: TRACK_VIDEO_A.to_string(),
        hidden: Some(false),
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
        .find(|event| event.verb == "track.hide")
        .expect("default_fixtures includes track.hide");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(TrackHideVerb))
        .expect("register track.hide verb");

    let report = validate_reconstructors(&registry, &[fixture])
        .expect("track.hide reconstruct from fixture");
    assert_eq!(report.verbs_checked, vec!["track.hide"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn two_tracks_of_same_kind_can_both_be_soloed_without_error() {
    let prior = project_with_tracks(vec![
        track(TRACK_VIDEO_A, TrackKind::Video, "Video 1", false, true),
        track(TRACK_VIDEO_B, TrackKind::Video, "Video 2", false, false),
    ]);

    let args = TrackHideArgs {
        project_id: fixture_project_id(),
        track: TRACK_VIDEO_B.to_string(),
        hidden: Some(true),
    };

    let (patch, warnings, data) =
        compute_patch(&prior, &args).expect("second video track may also be soloed");
    assert!(warnings.is_empty());
    assert_eq!(data.track_id.to_string(), TRACK_VIDEO_B);
    assert!(data.hidden);
    let patch_arr = patch.as_array().expect("patch is array");
    assert_eq!(patch_arr.len(), 1);
    assert_eq!(
        patch_arr[0].get("path").and_then(Value::as_str),
        Some("/tracks/1/hidden")
    );
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
        "hidden": true,
    });

    let outcome = store
        .mutate_via_verb("track.hide", args, None)
        .expect("mutate_via_verb happy path");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("happy path must return Applied, got {outcome:?}");
    };

    let data: TrackHideData =
        serde_json::from_value(data).expect("track.hide data is TrackHideData");
    assert_eq!(data.track_id.to_string(), track_id);
    assert!(data.hidden);
    assert!(store.project().tracks[0].hidden);
    assert_eq!(warnings, Vec::<Value>::new());
}
