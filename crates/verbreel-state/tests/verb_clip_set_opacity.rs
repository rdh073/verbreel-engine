//! Tests for `clip.set_opacity` (§5.10) — twentieth production verb.

use serde_json::{Value, json};
use std::sync::Arc;

use verbreel_state::verbs::clip_set_opacity::{
    W_NOOP_CODE, compute_patch, data_envelope_from_post_state,
};
use verbreel_state::{
    ClipSetOpacityArgs, ClipSetOpacityData, ClipSetOpacityError, ClipSetOpacityVerb, MutateOutcome,
    Project, RecordedEvent, Track, TrackKind, Verb, VerbError, VerbRegistry, default_fixtures,
    default_registry, validate_reconstructors,
};
use verbreel_types::{ProjectId, Tick};

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const TRACK_TEXT_A: &str = "0190b8d3-15e3-7000-bd00-0000000aa101";
const TRACK_TEXT_B: &str = "0190b8d3-15e3-7000-bd00-0000000aa102";
const TRACK_AUDIO_A: &str = "0190b8d3-15e3-7000-bd00-0000000aa201";
const TRACK_VIDEO_A: &str = "0190b8d3-15e3-7000-bd00-0000000aa301";
const CLIP_TEXT_A: &str = "0190b8d3-15e3-7000-bd00-0000000bb101";
const CLIP_TEXT_B: &str = "0190b8d3-15e3-7000-bd00-0000000bb102";
const CLIP_AUDIO_A: &str = "0190b8d3-15e3-7000-bd00-0000000bb201";
const CLIP_VIDEO_A: &str = "0190b8d3-15e3-7000-bd00-0000000bb301";
const MISSING_CLIP: &str = "0190b8d3-15e3-7000-bd00-0000000cc101";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture → Project")
}

fn fixture_project_id() -> ProjectId {
    empty_project().id
}

fn text_track(
    id: &str,
    track_name: &str,
    track_locked: bool,
    clip_id: &str,
    clip_name: &str,
    clip_locked: bool,
    opacity: f64,
) -> Track {
    serde_json::from_value(json!({
        "id": id,
        "kind": TrackKind::Text,
        "name": track_name,
        "locked": track_locked,
        "clips": [{
            "id": clip_id,
            "name": clip_name,
            "asset_id": "00000000-0000-0000-0000-000000000000",
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 480_000,
            "opacity": opacity,
            "locked": clip_locked,
            "text": {
                "content": "Hello",
                "font_family": "Arial",
                "font_size_px": 24,
            },
        }],
    }))
    .expect("text track fixture value should deserialize")
}

fn audio_track(
    id: &str,
    track_name: &str,
    track_locked: bool,
    clip_id: &str,
    clip_locked: bool,
    opacity: f64,
) -> Track {
    serde_json::from_value(json!({
        "id": id,
        "kind": TrackKind::Audio,
        "name": track_name,
        "locked": track_locked,
        "clips": [{
            "id": clip_id,
            "name": "Audio Clip",
            "asset_id": "01900000-0000-7000-8000-0000000cd201",
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 480_000,
            "opacity": opacity,
            "locked": clip_locked,
        }],
    }))
    .expect("audio track fixture value should deserialize")
}

fn video_track(
    id: &str,
    track_name: &str,
    track_locked: bool,
    clip_id: &str,
    clip_locked: bool,
    opacity: f64,
) -> Track {
    serde_json::from_value(json!({
        "id": id,
        "kind": TrackKind::Video,
        "name": track_name,
        "locked": track_locked,
        "clips": [{
            "id": clip_id,
            "name": "Video Clip",
            "asset_id": "01900000-0000-7000-8000-0000000ce201",
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 480_000,
            "opacity": opacity,
            "locked": clip_locked,
        }],
    }))
    .expect("video track fixture value should deserialize")
}

fn project_with_tracks(tracks: Vec<Track>) -> Project {
    let mut project = empty_project();
    project.tracks = tracks;
    project.duration_tk = Tick::new(480_000);
    project
}

fn patch_opacity_value(patch: &Value) -> f64 {
    let arr = patch.as_array().expect("patch is an array");
    assert_eq!(arr.len(), 1, "clip.set_opacity emits one replace op");
    let op = arr[0].as_object().expect("patch op is object");
    assert_eq!(op.get("op").and_then(Value::as_str), Some("replace"));
    op.get("value")
        .and_then(Value::as_f64)
        .expect("replace op value is f64")
}

#[test]
fn compute_patch_text_clip_set_opacity_0_5() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        "Clip 1",
        false,
        1.0,
    )]);

    let args = ClipSetOpacityArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_A.to_string(),
        opacity: 0.5,
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("happy-path opacity");
    assert_eq!(patch_opacity_value(&patch), 0.5);
    assert!(warnings.is_empty());
    assert_eq!(data.clip_id.to_string(), CLIP_TEXT_A);
    assert_eq!(data.opacity, 0.5);
    assert_eq!(
        patch.as_array().expect("patch is array")[0]
            .get("path")
            .and_then(Value::as_str),
        Some("/tracks/0/clips/0/opacity")
    );
}

#[test]
fn compute_patch_set_opacity_boundary_low() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        "Clip 1",
        false,
        1.0,
    )]);

    let args = ClipSetOpacityArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_A.to_string(),
        opacity: 0.0,
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("boundary low");
    assert_eq!(patch_opacity_value(&patch), 0.0);
    assert!(warnings.is_empty());
    assert_eq!(data.clip_id.to_string(), CLIP_TEXT_A);
    assert_eq!(data.opacity, 0.0);
}

#[test]
fn compute_patch_set_opacity_boundary_high() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        "Clip 1",
        false,
        0.2,
    )]);

    let args = ClipSetOpacityArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_A.to_string(),
        opacity: 1.0,
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("boundary high");
    assert_eq!(patch_opacity_value(&patch), 1.0);
    assert!(warnings.is_empty());
    assert_eq!(data.clip_id.to_string(), CLIP_TEXT_A);
    assert_eq!(data.opacity, 1.0);
}

#[test]
fn compute_patch_set_opacity_noop_emits_w_noop() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        "Clip 1",
        false,
        0.5,
    )]);

    let args = ClipSetOpacityArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_A.to_string(),
        opacity: 0.5,
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("same opacity is a no-op");
    assert!(patch.as_array().expect("patch is array").is_empty());
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_NOOP_CODE);
    assert_eq!(warnings[0]["message"], "clip opacity unchanged");
    assert_eq!(warnings[0]["details"]["clip_id"], CLIP_TEXT_A);
    assert_eq!(warnings[0]["details"]["opacity"], 0.5);
    assert_eq!(data.clip_id.to_string(), CLIP_TEXT_A);
    assert_eq!(data.opacity, 0.5);
}

#[test]
fn compute_patch_set_opacity_bad_range_below_zero() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        "Clip 1",
        false,
        1.0,
    )]);

    let err = compute_patch(
        &prior,
        &ClipSetOpacityArgs {
            project_id: fixture_project_id(),
            clip: CLIP_TEXT_A.to_string(),
            opacity: -0.1,
        },
    )
    .expect_err("below-range must reject");

    match err {
        ClipSetOpacityError::BadRange { value } => assert_eq!(value, -0.1),
        other => panic!("expected BadRange, got {other:?}"),
    }
}

#[test]
fn compute_patch_set_opacity_bad_range_above_one() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        "Clip 1",
        false,
        1.0,
    )]);

    let err = compute_patch(
        &prior,
        &ClipSetOpacityArgs {
            project_id: fixture_project_id(),
            clip: CLIP_TEXT_A.to_string(),
            opacity: 1.1,
        },
    )
    .expect_err("above-range must reject");

    match err {
        ClipSetOpacityError::BadRange { value } => assert_eq!(value, 1.1),
        other => panic!("expected BadRange, got {other:?}"),
    }
}

#[test]
fn compute_patch_set_opacity_bad_range_nan() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        "Clip 1",
        false,
        1.0,
    )]);

    let err = compute_patch(
        &prior,
        &ClipSetOpacityArgs {
            project_id: fixture_project_id(),
            clip: CLIP_TEXT_A.to_string(),
            opacity: f64::NAN,
        },
    )
    .expect_err("NaN must reject");

    match err {
        ClipSetOpacityError::BadRange { value } => assert!(value.is_nan()),
        other => panic!("expected BadRange, got {other:?}"),
    }
}

#[test]
fn compute_patch_set_opacity_bad_range_infinity() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        "Clip 1",
        false,
        1.0,
    )]);

    let err = compute_patch(
        &prior,
        &ClipSetOpacityArgs {
            project_id: fixture_project_id(),
            clip: CLIP_TEXT_A.to_string(),
            opacity: f64::INFINITY,
        },
    )
    .expect_err("infinity must reject");

    match err {
        ClipSetOpacityError::BadRange { value } => {
            assert!(value.is_infinite() && value.is_sign_positive())
        }
        other => panic!("expected BadRange, got {other:?}"),
    }
}

#[test]
fn compute_patch_set_opacity_bad_range_neg_infinity() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        "Clip 1",
        false,
        1.0,
    )]);

    let err = compute_patch(
        &prior,
        &ClipSetOpacityArgs {
            project_id: fixture_project_id(),
            clip: CLIP_TEXT_A.to_string(),
            opacity: f64::NEG_INFINITY,
        },
    )
    .expect_err("-infinity must reject");

    match err {
        ClipSetOpacityError::BadRange { value } => {
            assert!(value.is_infinite() && value.is_sign_negative())
        }
        other => panic!("expected BadRange, got {other:?}"),
    }
}

#[test]
fn compute_patch_set_opacity_locked_rejects() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        "Clip 1",
        true,
        1.0,
    )]);

    let err = compute_patch(
        &prior,
        &ClipSetOpacityArgs {
            project_id: fixture_project_id(),
            clip: CLIP_TEXT_A.to_string(),
            opacity: 0.5,
        },
    )
    .expect_err("locked clip must reject");

    match err {
        ClipSetOpacityError::Locked { clip_id } => assert_eq!(clip_id, CLIP_TEXT_A),
        other => panic!("expected Locked, got {other:?}"),
    }
}

#[test]
fn compute_patch_set_opacity_locked_beats_bad_range_check() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        "Clip 1",
        true,
        1.0,
    )]);

    let err = compute_patch(
        &prior,
        &ClipSetOpacityArgs {
            project_id: fixture_project_id(),
            clip: CLIP_TEXT_A.to_string(),
            opacity: 1.5,
        },
    )
    .expect_err("locked must beat range check");

    assert!(matches!(err, ClipSetOpacityError::Locked { .. }));
}

#[test]
fn compute_patch_set_opacity_track_lock_does_not_block() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        true,
        CLIP_TEXT_A,
        "Clip 1",
        false,
        1.0,
    )]);

    let args = ClipSetOpacityArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_A.to_string(),
        opacity: 0.5,
    };

    let (patch, warnings, data) =
        compute_patch(&prior, &args).expect("track lock should not block");
    assert_eq!(patch_opacity_value(&patch), 0.5);
    assert!(warnings.is_empty());
    assert_eq!(data.clip_id.to_string(), CLIP_TEXT_A);
    assert_eq!(data.opacity, 0.5);
}

#[test]
fn compute_patch_set_opacity_bad_selector_errors() {
    let prior = project_with_tracks(vec![]);

    let err = compute_patch(
        &prior,
        &ClipSetOpacityArgs {
            project_id: fixture_project_id(),
            clip: "not-a-uuid".to_string(),
            opacity: 0.5,
        },
    )
    .expect_err("bad clip selector must reject");

    match err {
        ClipSetOpacityError::BadSelector { detail } => {
            assert!(detail.contains("UUID"), "{detail}");
        }
        other => panic!("expected BadSelector, got {other:?}"),
    }
}

#[test]
fn compute_patch_set_opacity_clip_not_found_errors() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        "Clip 1",
        false,
        1.0,
    )]);

    let err = compute_patch(
        &prior,
        &ClipSetOpacityArgs {
            project_id: fixture_project_id(),
            clip: MISSING_CLIP.to_string(),
            opacity: 0.5,
        },
    )
    .expect_err("missing clip must reject");

    match err {
        ClipSetOpacityError::ClipNotFound { clip_id } => assert_eq!(clip_id, MISSING_CLIP),
        other => panic!("expected ClipNotFound, got {other:?}"),
    }
}

#[test]
fn compute_patch_set_opacity_reconstructor_round_trip() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        "Clip 1",
        false,
        1.0,
    )]);
    let args = ClipSetOpacityArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_A.to_string(),
        opacity: 0.5,
    };

    let (patch, _warnings, _data) = compute_patch(&prior, &args).expect("compute_patch ok");
    let typed_patch: json_patch::Patch =
        serde_json::from_value(patch.clone()).expect("patch parses to RFC 6902");
    let post_state = prior
        .apply(&typed_patch)
        .expect("clip.set_opacity patch should apply cleanly");

    let expected_data = serde_json::to_value(
        data_envelope_from_post_state(&args, &post_state)
            .expect("envelope from post-state should be readable"),
    )
    .expect("envelope serializes");

    let recorded = RecordedEvent {
        verb: "clip.set_opacity".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch,
        warnings: vec![],
        post_state,
        expected_data,
    };

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(ClipSetOpacityVerb))
        .expect("register clip.set_opacity verb");

    let report = validate_reconstructors(&registry, std::slice::from_ref(&recorded))
        .expect("reconstructor validation should pass");
    assert_eq!(report.verbs_checked, vec!["clip.set_opacity"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn compute_patch_error_variants_map_to_bad_args() {
    let prior = project_with_tracks(vec![
        text_track(
            TRACK_TEXT_A,
            "Text 1",
            false,
            CLIP_TEXT_A,
            "Clip 1",
            false,
            1.0,
        ),
        audio_track(TRACK_AUDIO_A, "Audio 1", false, CLIP_AUDIO_A, false, 1.0),
    ]);
    let verb = ClipSetOpacityVerb;

    let bad_selector = serde_json::to_value(ClipSetOpacityArgs {
        project_id: fixture_project_id(),
        clip: "not-a-uuid".to_string(),
        opacity: 0.5,
    })
    .expect("bad selector args serialize");
    let err = verb
        .compute_patch(&prior, &bad_selector)
        .expect_err("bad selector maps");
    assert!(matches!(err, VerbError::BadArgs { .. }));

    let clip_not_found = serde_json::to_value(ClipSetOpacityArgs {
        project_id: fixture_project_id(),
        clip: MISSING_CLIP.to_string(),
        opacity: 0.5,
    })
    .expect("missing clip args serialize");
    let err = verb
        .compute_patch(&prior, &clip_not_found)
        .expect_err("missing clip maps");
    assert!(matches!(err, VerbError::BadArgs { .. }));

    let locked = serde_json::to_value(ClipSetOpacityArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_A.to_string(),
        opacity: 0.5,
    })
    .expect("locked clip args serialize");
    let locked_state = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        "Clip 1",
        true,
        1.0,
    )]);
    let err = verb
        .compute_patch(&locked_state, &locked)
        .expect_err("locked clip maps");
    assert!(matches!(err, VerbError::BadArgs { .. }));

    let bad_range = serde_json::to_value(ClipSetOpacityArgs {
        project_id: fixture_project_id(),
        clip: CLIP_AUDIO_A.to_string(),
        opacity: 1.1,
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
        .find(|event| event.verb == "clip.set_opacity")
        .expect("default_fixtures includes clip.set_opacity");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(ClipSetOpacityVerb))
        .expect("register clip.set_opacity verb");

    let report = validate_reconstructors(&registry, &[fixture])
        .expect("clip.set_opacity reconstruct from fixture");
    assert_eq!(report.verbs_checked, vec!["clip.set_opacity"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn data_envelope_from_post_state_returns_post_opacity() {
    let post_state = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        "Clip 1",
        false,
        0.75,
    )]);
    let args = ClipSetOpacityArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_A.to_string(),
        opacity: 0.5,
    };

    let data = data_envelope_from_post_state(&args, &post_state)
        .expect("post-state envelope should be readable");
    assert_eq!(data.clip_id.to_string(), CLIP_TEXT_A);
    assert_eq!(data.opacity, 0.75);
}

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let base_project = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        "Clip 1",
        false,
        1.0,
    )]);
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        base_project,
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry clears the gate and writes project.json");

    let outcome = store
        .mutate_via_verb(
            "clip.set_opacity",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "clip": CLIP_TEXT_A,
                "opacity": 0.5,
            }),
            None,
        )
        .expect("mutate_via_verb happy path");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("happy path must return Applied, got {outcome:?}");
    };

    let data: ClipSetOpacityData =
        serde_json::from_value(data).expect("clip.set_opacity data is ClipSetOpacityData");
    assert_eq!(data.clip_id.to_string(), CLIP_TEXT_A);
    assert_eq!(store.project().tracks[0].clips[0].opacity, 0.5);
    assert_eq!(data.opacity, 0.5);
    assert_eq!(warnings, Vec::<Value>::new());
}

#[test]
fn multi_track_clip_resolution_uses_track_index() {
    let prior = project_with_tracks(vec![
        text_track(
            TRACK_TEXT_A,
            "Text 1",
            false,
            CLIP_TEXT_A,
            "Clip 1",
            false,
            1.0,
        ),
        text_track(
            TRACK_TEXT_B,
            "Text 2",
            false,
            CLIP_TEXT_B,
            "Clip 2",
            false,
            1.0,
        ),
    ]);
    let args = ClipSetOpacityArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_B.to_string(),
        opacity: 0.5,
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("search in second track");
    assert_eq!(patch_opacity_value(&patch), 0.5);
    assert!(warnings.is_empty());
    assert_eq!(data.clip_id.to_string(), CLIP_TEXT_B);
    assert_eq!(data.opacity, 0.5);
    assert_eq!(
        patch.as_array().expect("patch is array")[0]
            .get("path")
            .and_then(Value::as_str),
        Some("/tracks/1/clips/0/opacity")
    );
}

#[test]
fn compute_patch_set_opacity_no_kind_guard() {
    let prior = project_with_tracks(vec![
        text_track(
            TRACK_TEXT_A,
            "Text 1",
            false,
            CLIP_TEXT_A,
            "Text Clip",
            false,
            1.0,
        ),
        audio_track(TRACK_AUDIO_A, "Audio 1", false, CLIP_AUDIO_A, false, 1.0),
        video_track(TRACK_VIDEO_A, "Video 1", false, CLIP_VIDEO_A, false, 1.0),
    ]);

    let text_case = compute_patch(
        &prior,
        &ClipSetOpacityArgs {
            project_id: fixture_project_id(),
            clip: CLIP_TEXT_A.to_string(),
            opacity: 0.6,
        },
    )
    .expect("text clip should pass");
    assert_eq!(patch_opacity_value(&text_case.0), 0.6);

    let audio_case = compute_patch(
        &prior,
        &ClipSetOpacityArgs {
            project_id: fixture_project_id(),
            clip: CLIP_AUDIO_A.to_string(),
            opacity: 0.6,
        },
    )
    .expect("audio clip should pass");
    assert_eq!(patch_opacity_value(&audio_case.0), 0.6);

    let video_case = compute_patch(
        &prior,
        &ClipSetOpacityArgs {
            project_id: fixture_project_id(),
            clip: CLIP_VIDEO_A.to_string(),
            opacity: 0.6,
        },
    )
    .expect("video clip should pass");
    assert_eq!(patch_opacity_value(&video_case.0), 0.6);
}
