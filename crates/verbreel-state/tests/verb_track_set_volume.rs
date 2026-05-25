//! Tests for `track.set_volume` (§4.8) — fifteenth production verb.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::track_set_volume::{
    W_NOOP_CODE, compute_patch, data_envelope_from_post_state,
};
use verbreel_state::{
    MutateOutcome, Project, RecordedEvent, Track, TrackAddData, TrackKind, TrackSetVolumeArgs,
    TrackSetVolumeData, TrackSetVolumeError, TrackSetVolumeVerb, VerbError, VerbRegistry,
    Verb, default_fixtures, default_registry, validate_reconstructors,
};
use verbreel_types::ProjectId;

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const TRACK_AUDIO_A: &str = "0190b8d3-15e3-7000-bd00-0000000aa101";
const TRACK_VIDEO_A: &str = "0190b8d3-15e3-7000-bd00-0000000bb101";
const TRACK_TEXT_A: &str = "0190b8d3-15e3-7000-bd00-0000000cc101";
const TRACK_EFFECT_A: &str = "0190b8d3-15e3-7000-bd00-0000000dd101";
const MISSING_TRACK: &str = "0190b8d3-15e3-7000-bd00-0000000ee101";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture → Project")
}

fn fixture_project_id() -> ProjectId {
    empty_project().id
}

fn audio_track(id: &str, name: &str, locked: bool, volume: f64) -> Track {
    let mut track: Track = serde_json::from_value(json!({
        "id": id,
        "kind": "audio",
        "name": name,
        "clips": [],
    }))
    .expect("audio track fixture value should deserialize");
    track.locked = locked;
    track.volume = volume;
    track
}

fn video_track(id: &str, name: &str, locked: bool) -> Track {
    let mut track: Track = serde_json::from_value(json!({
        "id": id,
        "kind": "video",
        "name": name,
        "clips": [],
    }))
    .expect("video track fixture value should deserialize");
    track.locked = locked;
    track
}

fn text_track(id: &str, name: &str, locked: bool) -> Track {
    let mut track: Track = serde_json::from_value(json!({
        "id": id,
        "kind": "text",
        "name": name,
        "clips": [],
    }))
    .expect("text track fixture value should deserialize");
    track.locked = locked;
    track
}

fn effect_track(id: &str, name: &str, locked: bool) -> Track {
    let mut track: Track = serde_json::from_value(json!({
        "id": id,
        "kind": "effect",
        "name": name,
        "clips": [],
    }))
    .expect("effect track fixture value should deserialize");
    track.locked = locked;
    track
}

fn project_with_tracks(tracks: Vec<Track>) -> Project {
    let mut project = empty_project();
    project.tracks = tracks;
    project
}

fn patch_volume_value(patch: &Value) -> f64 {
    let arr = patch.as_array().expect("patch is an array");
    assert_eq!(arr.len(), 1, "track.set_volume emits one replace op");
    let op = arr[0].as_object().expect("patch op is object");
    assert_eq!(op.get("op").and_then(Value::as_str), Some("replace"));
    op.get("value")
        .and_then(Value::as_f64)
        .expect("replace op value is f64")
}

#[test]
fn compute_patch_audio_track_set_volume_1_5() {
    let prior = project_with_tracks(vec![audio_track(TRACK_AUDIO_A, "Audio 1", false, 1.0)]);

    let args = TrackSetVolumeArgs {
        project_id: fixture_project_id(),
        track: TRACK_AUDIO_A.to_string(),
        volume: 1.5,
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("happy-path volume");
    assert_eq!(patch_volume_value(&patch), 1.5);
    assert!(warnings.is_empty());
    assert_eq!(data.track_id.to_string(), TRACK_AUDIO_A);
    assert_eq!(data.volume, 1.5);
    assert_eq!(
        patch.as_array().expect("patch is array")[0]
            .get("path")
            .and_then(Value::as_str),
        Some("/tracks/0/volume")
    );
}

#[test]
fn compute_patch_audio_track_set_volume_boundary_low() {
    let prior = project_with_tracks(vec![audio_track(TRACK_AUDIO_A, "Audio 1", false, 1.0)]);

    let args = TrackSetVolumeArgs {
        project_id: fixture_project_id(),
        track: TRACK_AUDIO_A.to_string(),
        volume: 0.0,
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("boundary low");
    assert_eq!(patch_volume_value(&patch), 0.0);
    assert!(warnings.is_empty());
    assert_eq!(data.track_id.to_string(), TRACK_AUDIO_A);
    assert_eq!(data.volume, 0.0);
}

#[test]
fn compute_patch_audio_track_set_volume_boundary_high() {
    let prior = project_with_tracks(vec![audio_track(TRACK_AUDIO_A, "Audio 1", false, 1.0)]);

    let args = TrackSetVolumeArgs {
        project_id: fixture_project_id(),
        track: TRACK_AUDIO_A.to_string(),
        volume: 4.0,
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("boundary high");
    assert_eq!(patch_volume_value(&patch), 4.0);
    assert!(warnings.is_empty());
    assert_eq!(data.track_id.to_string(), TRACK_AUDIO_A);
    assert_eq!(data.volume, 4.0);
}

#[test]
fn compute_patch_audio_track_set_volume_default_noop() {
    let prior = project_with_tracks(vec![audio_track(TRACK_AUDIO_A, "Audio 1", false, 1.0)]);

    let args = TrackSetVolumeArgs {
        project_id: fixture_project_id(),
        track: TRACK_AUDIO_A.to_string(),
        volume: 1.0,
    };

    let (patch, warnings, data) =
        compute_patch(&prior, &args).expect("default volume should be a no-op");
    let patch_arr = patch.as_array().expect("patch is array");
    assert!(patch_arr.is_empty(), "same-state volume is a no-op");
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_NOOP_CODE);
    assert_eq!(warnings[0]["message"], "track volume unchanged");
    assert_eq!(warnings[0]["details"]["track_id"], TRACK_AUDIO_A);
    assert_eq!(warnings[0]["details"]["volume"], 1.0);
    assert_eq!(data.track_id.to_string(), TRACK_AUDIO_A);
    assert_eq!(data.volume, 1.0);
}

#[test]
fn compute_patch_set_volume_bad_range_below_zero() {
    let prior = project_with_tracks(vec![audio_track(TRACK_AUDIO_A, "Audio 1", false, 1.0)]);

    let err = compute_patch(
        &prior,
        &TrackSetVolumeArgs {
            project_id: fixture_project_id(),
            track: TRACK_AUDIO_A.to_string(),
            volume: -0.1,
        },
    )
    .expect_err("below-range must reject");

    match err {
        TrackSetVolumeError::BadRange { value } => assert_eq!(value, -0.1),
        other => panic!("expected BadRange, got {other:?}"),
    }
}

#[test]
fn compute_patch_set_volume_bad_range_above_four() {
    let prior = project_with_tracks(vec![audio_track(TRACK_AUDIO_A, "Audio 1", false, 1.0)]);

    let err = compute_patch(
        &prior,
        &TrackSetVolumeArgs {
            project_id: fixture_project_id(),
            track: TRACK_AUDIO_A.to_string(),
            volume: 4.1,
        },
    )
    .expect_err("above-range must reject");

    match err {
        TrackSetVolumeError::BadRange { value } => assert_eq!(value, 4.1),
        other => panic!("expected BadRange, got {other:?}"),
    }
}

#[test]
fn compute_patch_set_volume_bad_range_nan() {
    let prior = project_with_tracks(vec![audio_track(TRACK_AUDIO_A, "Audio 1", false, 1.0)]);

    let err = compute_patch(
        &prior,
        &TrackSetVolumeArgs {
            project_id: fixture_project_id(),
            track: TRACK_AUDIO_A.to_string(),
            volume: f64::NAN,
        },
    )
    .expect_err("NaN must reject");

    match err {
        TrackSetVolumeError::BadRange { value } => assert!(value.is_nan()),
        other => panic!("expected BadRange, got {other:?}"),
    }
}

#[test]
fn compute_patch_set_volume_bad_range_infinity() {
    let prior = project_with_tracks(vec![audio_track(TRACK_AUDIO_A, "Audio 1", false, 1.0)]);

    let err = compute_patch(
        &prior,
        &TrackSetVolumeArgs {
            project_id: fixture_project_id(),
            track: TRACK_AUDIO_A.to_string(),
            volume: f64::INFINITY,
        },
    )
    .expect_err("infinity must reject");

    match err {
        TrackSetVolumeError::BadRange { value } => {
            assert!(value.is_infinite() && value.is_sign_positive())
        }
        other => panic!("expected BadRange, got {other:?}"),
    }
}

#[test]
fn compute_patch_set_volume_bad_range_neg_infinity() {
    let prior = project_with_tracks(vec![audio_track(TRACK_AUDIO_A, "Audio 1", false, 1.0)]);

    let err = compute_patch(
        &prior,
        &TrackSetVolumeArgs {
            project_id: fixture_project_id(),
            track: TRACK_AUDIO_A.to_string(),
            volume: f64::NEG_INFINITY,
        },
    )
    .expect_err("-infinity must reject");

    match err {
        TrackSetVolumeError::BadRange { value } => {
            assert!(value.is_infinite() && value.is_sign_negative())
        }
        other => panic!("expected BadRange, got {other:?}"),
    }
}

#[test]
fn compute_patch_set_volume_rejects_video_tracks() {
    let prior = project_with_tracks(vec![video_track(TRACK_VIDEO_A, "Video 1", false)]);

    let err = compute_patch(
        &prior,
        &TrackSetVolumeArgs {
            project_id: fixture_project_id(),
            track: TRACK_VIDEO_A.to_string(),
            volume: 1.5,
        },
    )
    .expect_err("video track should reject");

    match err {
        TrackSetVolumeError::KindMismatch {
            track_id,
            found_kind,
        } => {
            assert_eq!(track_id, TRACK_VIDEO_A);
            assert_eq!(found_kind, TrackKind::Video);
        }
        other => panic!("expected KindMismatch, got {other:?}"),
    }
}

#[test]
fn compute_patch_set_volume_rejects_text_tracks() {
    let prior = project_with_tracks(vec![text_track(TRACK_TEXT_A, "Text 1", false)]);

    let err = compute_patch(
        &prior,
        &TrackSetVolumeArgs {
            project_id: fixture_project_id(),
            track: TRACK_TEXT_A.to_string(),
            volume: 1.5,
        },
    )
    .expect_err("text track should reject");

    match err {
        TrackSetVolumeError::KindMismatch {
            track_id,
            found_kind,
        } => {
            assert_eq!(track_id, TRACK_TEXT_A);
            assert_eq!(found_kind, TrackKind::Text);
        }
        other => panic!("expected KindMismatch, got {other:?}"),
    }
}

#[test]
fn compute_patch_set_volume_rejects_effect_tracks() {
    let prior = project_with_tracks(vec![effect_track(TRACK_EFFECT_A, "Effect 1", false)]);

    let err = compute_patch(
        &prior,
        &TrackSetVolumeArgs {
            project_id: fixture_project_id(),
            track: TRACK_EFFECT_A.to_string(),
            volume: 1.5,
        },
    )
    .expect_err("effect track should reject");

    match err {
        TrackSetVolumeError::KindMismatch {
            track_id,
            found_kind,
        } => {
            assert_eq!(track_id, TRACK_EFFECT_A);
            assert_eq!(found_kind, TrackKind::Effect);
        }
        other => panic!("expected KindMismatch, got {other:?}"),
    }
}

#[test]
fn compute_patch_set_volume_locked_before_bad_range_check() {
    let prior = project_with_tracks(vec![audio_track(TRACK_AUDIO_A, "Audio 1", true, 1.0)]);

    let err = compute_patch(
        &prior,
        &TrackSetVolumeArgs {
            project_id: fixture_project_id(),
            track: TRACK_AUDIO_A.to_string(),
            volume: 9.0,
        },
    )
    .expect_err("locked should beat range check");

    assert!(matches!(err, TrackSetVolumeError::Locked { .. }));
}

#[test]
fn compute_patch_set_volume_bad_selector() {
    let prior = project_with_tracks(vec![]);

    let err = compute_patch(
        &prior,
        &TrackSetVolumeArgs {
            project_id: fixture_project_id(),
            track: "not-a-uuid".to_string(),
            volume: 1.5,
        },
    )
    .expect_err("bad track selector must reject");

    match err {
        TrackSetVolumeError::BadSelector { detail } => {
            assert!(detail.contains("UUID"), "{detail}");
        }
        other => panic!("expected BadSelector, got {other:?}"),
    }
}

#[test]
fn compute_patch_set_volume_not_found() {
    let prior = project_with_tracks(vec![audio_track(TRACK_AUDIO_A, "Audio 1", false, 1.0)]);

    let err = compute_patch(
        &prior,
        &TrackSetVolumeArgs {
            project_id: fixture_project_id(),
            track: MISSING_TRACK.to_string(),
            volume: 1.5,
        },
    )
    .expect_err("missing track must reject");

    match err {
        TrackSetVolumeError::TrackNotFound { track_id } => assert_eq!(track_id, MISSING_TRACK),
        other => panic!("expected TrackNotFound, got {other:?}"),
    }
}

#[test]
fn compute_patch_set_volume_idempotent_emits_w_noop() {
    let prior = project_with_tracks(vec![audio_track(TRACK_AUDIO_A, "Audio 1", false, 1.5)]);

    let args = TrackSetVolumeArgs {
        project_id: fixture_project_id(),
        track: TRACK_AUDIO_A.to_string(),
        volume: 1.5,
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("same-state should succeed");
    let patch_arr = patch.as_array().expect("patch is array");
    assert!(patch_arr.is_empty(), "same-state volume is a no-op");
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_NOOP_CODE);
    assert_eq!(warnings[0]["message"], "track volume unchanged");
    assert_eq!(warnings[0]["details"]["track_id"], TRACK_AUDIO_A);
    assert_eq!(warnings[0]["details"]["volume"], 1.5);
    assert_eq!(data.track_id.to_string(), TRACK_AUDIO_A);
    assert_eq!(data.volume, 1.5);
}

#[test]
fn reconstructor_round_trip() {
    let prior = project_with_tracks(vec![audio_track(TRACK_AUDIO_A, "Audio 1", false, 1.0)]);
    let args = TrackSetVolumeArgs {
        project_id: fixture_project_id(),
        track: TRACK_AUDIO_A.to_string(),
        volume: 1.5,
    };

    let (patch, _warnings, _data) = compute_patch(&prior, &args).expect("compute_patch ok");
    let typed_patch: json_patch::Patch =
        serde_json::from_value(patch.clone()).expect("patch parses to typed JSON patch");
    let post_state = prior
        .apply(&typed_patch)
        .expect("track.set_volume patch should apply cleanly");

    let expected_data =
        serde_json::to_value(data_envelope_from_post_state(&args, &post_state).expect("envelope"))
            .expect("envelope serializes");

    let recorded = RecordedEvent {
        verb: "track.set_volume".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch,
        warnings: vec![],
        post_state,
        expected_data,
    };

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(TrackSetVolumeVerb))
        .expect("register track.set_volume verb");

    let report = validate_reconstructors(&registry, std::slice::from_ref(&recorded))
        .expect("reconstructor validation should pass");
    assert_eq!(report.verbs_checked, vec!["track.set_volume"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn compute_patch_error_variants_map_to_bad_args() {
    let prior = project_with_tracks(vec![audio_track(TRACK_AUDIO_A, "Audio 1", false, 1.0)]);

    let verb = TrackSetVolumeVerb;

    let bad_selector = serde_json::to_value(TrackSetVolumeArgs {
        project_id: fixture_project_id(),
        track: "not-a-uuid".to_string(),
        volume: 1.5,
    })
    .expect("bad selector args serialize");
    let err = verb
        .compute_patch(&prior, &bad_selector)
        .expect_err("bad selector maps");
    assert!(matches!(err, VerbError::BadArgs { .. }));

    let track_not_found = serde_json::to_value(TrackSetVolumeArgs {
        project_id: fixture_project_id(),
        track: MISSING_TRACK.to_string(),
        volume: 1.5,
    })
    .expect("missing track args serialize");
    let err = verb
        .compute_patch(&prior, &track_not_found)
        .expect_err("missing track maps");
    assert!(matches!(err, VerbError::BadArgs { .. }));

    let kind_mismatch = serde_json::to_value(TrackSetVolumeArgs {
        project_id: fixture_project_id(),
        track: TRACK_VIDEO_A.to_string(),
        volume: 1.5,
    })
    .expect("kind mismatch args serialize");
    let prior_video = project_with_tracks(vec![video_track(TRACK_VIDEO_A, "Video 1", false)]);
    let err = verb
        .compute_patch(&prior_video, &kind_mismatch)
        .expect_err("kind mismatch maps");
    assert!(matches!(err, VerbError::BadArgs { .. }));

    let locked = serde_json::to_value(TrackSetVolumeArgs {
        project_id: fixture_project_id(),
        track: TRACK_AUDIO_A.to_string(),
        volume: 1.5,
    })
    .expect("locked args serialize");
    let prior_locked = project_with_tracks(vec![audio_track(TRACK_AUDIO_A, "Audio 1", true, 1.0)]);
    let err = verb
        .compute_patch(&prior_locked, &locked)
        .expect_err("locked maps");
    assert!(matches!(err, VerbError::BadArgs { .. }));

    let bad_range = serde_json::to_value(TrackSetVolumeArgs {
        project_id: fixture_project_id(),
        track: TRACK_AUDIO_A.to_string(),
        volume: 4.1,
    })
    .expect("bad range args serialize");
    let err = verb
        .compute_patch(&prior, &bad_range)
        .expect_err("bad range maps");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "track.set_volume")
        .expect("default_fixtures includes track.set_volume");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(TrackSetVolumeVerb))
        .expect("register track.set_volume verb");

    let report = validate_reconstructors(&registry, &[fixture])
        .expect("track.set_volume reconstruct from fixture");
    assert_eq!(report.verbs_checked, vec!["track.set_volume"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn data_envelope_returns_post_state_volume() {
    let mut post_state = empty_project();
    post_state.tracks = vec![audio_track(TRACK_AUDIO_A, "Audio 1", false, 1.75)];

    let args = TrackSetVolumeArgs {
        project_id: fixture_project_id(),
        track: TRACK_AUDIO_A.to_string(),
        volume: 0.5,
    };

    let data = data_envelope_from_post_state(&args, &post_state)
        .expect("post-state envelope should be readable");
    assert_eq!(data.track_id.to_string(), TRACK_AUDIO_A);
    assert_eq!(data.volume, 1.75);
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

    let track_add_outcome = store
        .mutate_via_verb(
            "track.add",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "kind": "audio",
            }),
            None,
        )
        .expect("create audio track");

    let MutateOutcome::Applied { data: add_data, .. } = track_add_outcome else {
        panic!("track.add must be applied, got {track_add_outcome:?}");
    };

    let add_data: TrackAddData =
        serde_json::from_value(add_data).expect("track.add data is TrackAddData");
    let track_id = add_data.track_id.to_string();

    let outcome = store
        .mutate_via_verb(
            "track.set_volume",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "track": track_id,
                "volume": 0.5,
            }),
            None,
        )
        .expect("set_volume happy path");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("happy path must return Applied, got {outcome:?}");
    };

    let data: TrackSetVolumeData =
        serde_json::from_value(data).expect("track.set_volume data is TrackSetVolumeData");
    assert_eq!(data.volume, 0.5);
    assert_eq!(warnings, Vec::<Value>::new());

    let track = store
        .project()
        .tracks
        .iter()
        .find(|t| t.id == data.track_id)
        .expect("track must exist after set_volume");
    assert_eq!(track.volume, 0.5);
}
