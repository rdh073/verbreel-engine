//! Tests for `clip.set_transform` (§5.9) — twenty-third production verb.

use serde_json::{Value, json};
use std::sync::Arc;

use verbreel_state::verbs::clip_set_transform::{
    W_NOOP_CODE, compute_patch, data_envelope_from_post_state,
};
use verbreel_state::{
    ClipSetTransformArgs, ClipSetTransformData, ClipSetTransformError, ClipSetTransformVerb,
    MutateOutcome, PartialTransform, Project, RecordedEvent, Track, TrackKind, Transform, Verb,
    VerbError, VerbRegistry, default_fixtures, default_registry, validate_reconstructors,
};
use verbreel_types::{ProjectId, Tick};

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const TRACK_TEXT_A: &str = "0190b8d3-15e3-7000-bd00-0000000aa101";
const TRACK_TEXT_B: &str = "0190b8d3-15e3-7000-bd00-0000000aa102";
const CLIP_TEXT_A: &str = "0190b8d3-15e3-7000-bd00-0000000bb101";
const CLIP_TEXT_B: &str = "0190b8d3-15e3-7000-bd00-0000000bb102";
const MISSING_CLIP: &str = "0190b8d3-15e3-7000-bd00-0000000cc101";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture → Project")
}

fn fixture_project_id() -> ProjectId {
    empty_project().id
}

fn text_track(
    id: &str,
    name: &str,
    track_locked: bool,
    clip_id: &str,
    clip_locked: bool,
    transform: Transform,
) -> Track {
    serde_json::from_value(json!({
        "id": id,
        "kind": TrackKind::Text,
        "name": name,
        "locked": track_locked,
        "clips": [{
            "id": clip_id,
            "name": "Text Clip",
            "asset_id": "00000000-0000-0000-0000-000000000000",
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 480_000,
            "transform": transform,
            "locked": clip_locked,
            "text": {
                "content": "Hello",
                "font_family": "Arial",
                "font_size_px": 24,
            },
        }],
    }))
    .expect("text track fixture parses")
}

fn project_with_tracks(tracks: Vec<Track>) -> Project {
    let mut project = empty_project();
    project.tracks = tracks;
    project.duration_tk = Tick::new(480_000);
    project
}

fn track_op_paths(patch: &Value) -> Vec<String> {
    patch
        .as_array()
        .expect("patch is an array")
        .iter()
        .map(|op| {
            op.get("path")
                .and_then(Value::as_str)
                .expect("patch op has path")
                .to_string()
        })
        .collect()
}

fn transform_patch_values(patch: &Value) -> Vec<Value> {
    patch
        .as_array()
        .expect("patch is an array")
        .iter()
        .map(|op| op.get("value").cloned().expect("replace op carries value"))
        .collect()
}

#[test]
fn compute_patch_change_x_only() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
        Transform::default(),
    )]);

    let args = ClipSetTransformArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_A.to_string(),
        transform: PartialTransform {
            x: Some(10.0),
            ..Default::default()
        },
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("happy-path x only");
    assert_eq!(patch.as_array().expect("patch is array").len(), 1);
    assert!(warnings.is_empty());
    assert_eq!(track_op_paths(&patch)[0], "/tracks/0/clips/0/transform/x");
    assert_eq!(transform_patch_values(&patch)[0], json!(10.0));
    assert_eq!(data.clip_id.to_string(), CLIP_TEXT_A);
    assert_eq!(data.transform.x, 10.0);
}

#[test]
fn compute_patch_change_x_and_scale_x() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
        Transform::default(),
    )]);

    let args = ClipSetTransformArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_A.to_string(),
        transform: PartialTransform {
            x: Some(10.0),
            scale_x: Some(2.0),
            ..Default::default()
        },
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("x + scale_x");
    assert_eq!(patch.as_array().expect("patch is array").len(), 2);
    assert!(warnings.is_empty());
    assert_eq!(
        track_op_paths(&patch),
        vec![
            "/tracks/0/clips/0/transform/x",
            "/tracks/0/clips/0/transform/scale_x"
        ]
    );
    assert_eq!(
        transform_patch_values(&patch),
        vec![json!(10.0), json!(2.0)]
    );
    assert_eq!(data.transform.x, 10.0);
    assert_eq!(data.transform.scale_x, 2.0);
}

#[test]
fn compute_patch_change_all_11_fields() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
        Transform::default(),
    )]);

    let args = ClipSetTransformArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_A.to_string(),
        transform: PartialTransform {
            x: Some(10.0),
            y: Some(-3.0),
            scale_x: Some(1.5),
            scale_y: Some(0.75),
            rotation_deg: Some(45.0),
            anchor_x: Some(0.25),
            anchor_y: Some(0.75),
            skew_x_deg: Some(1.5),
            skew_y_deg: Some(-2.5),
            flip_h: Some(true),
            flip_v: Some(true),
        },
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("all fields");
    assert_eq!(patch.as_array().expect("patch is array").len(), 11);
    assert!(warnings.is_empty());
    assert_eq!(
        track_op_paths(&patch),
        vec![
            "/tracks/0/clips/0/transform/x",
            "/tracks/0/clips/0/transform/y",
            "/tracks/0/clips/0/transform/scale_x",
            "/tracks/0/clips/0/transform/scale_y",
            "/tracks/0/clips/0/transform/rotation_deg",
            "/tracks/0/clips/0/transform/anchor_x",
            "/tracks/0/clips/0/transform/anchor_y",
            "/tracks/0/clips/0/transform/skew_x_deg",
            "/tracks/0/clips/0/transform/skew_y_deg",
            "/tracks/0/clips/0/transform/flip_h",
            "/tracks/0/clips/0/transform/flip_v",
        ]
    );
    assert_eq!(data.transform.x, 10.0);
    assert_eq!(data.transform.y, -3.0);
    assert_eq!(data.transform.scale_x, 1.5);
    assert_eq!(data.transform.scale_y, 0.75);
    assert_eq!(data.transform.rotation_deg, 45.0);
    assert_eq!(data.transform.anchor_x, 0.25);
    assert_eq!(data.transform.anchor_y, 0.75);
    assert_eq!(data.transform.skew_x_deg, 1.5);
    assert_eq!(data.transform.skew_y_deg, -2.5);
    assert!(data.transform.flip_h);
    assert!(data.transform.flip_v);
}

#[test]
fn compute_patch_partial_update_preserves_absent_fields() {
    let prior_transform = Transform {
        x: 10.0,
        y: 20.0,
        scale_x: 1.2,
        scale_y: 0.8,
        rotation_deg: 90.0,
        anchor_x: 0.25,
        anchor_y: 0.75,
        skew_x_deg: 12.0,
        skew_y_deg: -8.0,
        flip_h: true,
        flip_v: false,
    };
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
        prior_transform,
    )]);

    let args = ClipSetTransformArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_A.to_string(),
        transform: PartialTransform {
            x: Some(-10.0),
            ..Default::default()
        },
    };

    let (patch, _warnings, _data) = compute_patch(&prior, &args).expect("partial update");
    let typed_patch: json_patch::Patch =
        serde_json::from_value(patch.clone()).expect("replace patch parses");
    let post_state = prior
        .apply(&typed_patch)
        .expect("partial transform patch applies");
    let post_transform = &post_state.tracks[0].clips[0].transform;

    assert_eq!(post_transform.x, -10.0);
    assert_eq!(post_transform.y, prior_transform.y);
    assert_eq!(post_transform.scale_x, prior_transform.scale_x);
    assert_eq!(post_transform.scale_y, prior_transform.scale_y);
    assert_eq!(post_transform.rotation_deg, prior_transform.rotation_deg);
    assert_eq!(post_transform.anchor_x, prior_transform.anchor_x);
    assert_eq!(post_transform.anchor_y, prior_transform.anchor_y);
    assert_eq!(post_transform.skew_x_deg, prior_transform.skew_x_deg);
    assert_eq!(post_transform.skew_y_deg, prior_transform.skew_y_deg);
    assert_eq!(post_transform.flip_h, prior_transform.flip_h);
    assert_eq!(post_transform.flip_v, prior_transform.flip_v);
}

#[test]
fn compute_patch_noop_when_transform_unchanged() {
    let prior_transform = Transform {
        x: 10.0,
        y: 20.0,
        scale_x: 1.2,
        scale_y: 0.8,
        rotation_deg: 90.0,
        anchor_x: 0.25,
        anchor_y: 0.75,
        skew_x_deg: 12.0,
        skew_y_deg: -8.0,
        flip_h: true,
        flip_v: false,
    };
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
        prior_transform,
    )]);

    let args = ClipSetTransformArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_A.to_string(),
        transform: PartialTransform {
            x: Some(10.0),
            y: Some(20.0),
            scale_x: Some(1.2),
            scale_y: Some(0.8),
            rotation_deg: Some(90.0),
            anchor_x: Some(0.25),
            anchor_y: Some(0.75),
            skew_x_deg: Some(12.0),
            skew_y_deg: Some(-8.0),
            flip_h: Some(true),
            flip_v: Some(false),
        },
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("no-op");
    assert!(patch.as_array().expect("patch is array").is_empty());
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_NOOP_CODE);
    assert_eq!(warnings[0]["message"], "clip transform unchanged");
    assert_eq!(warnings[0]["details"]["clip_id"], CLIP_TEXT_A);
    assert_eq!(warnings[0]["details"]["transform"], json!(prior_transform));
    assert_eq!(data.transform, prior_transform);
}

#[test]
fn compute_patch_noop_with_empty_partial_transform() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
        Transform {
            x: 10.0,
            ..Transform::default()
        },
    )]);

    let args = ClipSetTransformArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_A.to_string(),
        transform: PartialTransform::default(),
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("empty partial");
    assert!(patch.as_array().expect("patch is array").is_empty());
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_NOOP_CODE);
    assert_eq!(warnings[0]["details"]["clip_id"], CLIP_TEXT_A);
    assert_eq!(
        warnings[0]["details"]["transform"],
        json!(Transform {
            x: 10.0,
            ..Transform::default()
        })
    );
    assert_eq!(data.clip_id.to_string(), CLIP_TEXT_A);
    assert_eq!(
        data.transform,
        Transform {
            x: 10.0,
            ..Transform::default()
        }
    );
}

#[test]
fn compute_patch_partial_diff_skips_matching_field() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
        Transform::default(),
    )]);

    let args = ClipSetTransformArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_A.to_string(),
        transform: PartialTransform {
            x: Some(0.0),
            y: Some(123.0),
            ..Default::default()
        },
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("partial diff");
    assert!(warnings.is_empty());
    assert_eq!(patch.as_array().expect("patch is array").len(), 1);
    assert_eq!(track_op_paths(&patch)[0], "/tracks/0/clips/0/transform/y");
    assert_eq!(transform_patch_values(&patch)[0], json!(123.0));
    assert_eq!(data.transform.y, 123.0);
}

#[test]
fn compute_patch_bad_value_nan_x() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
        Transform::default(),
    )]);

    let err = compute_patch(
        &prior,
        &ClipSetTransformArgs {
            project_id: fixture_project_id(),
            clip: CLIP_TEXT_A.to_string(),
            transform: PartialTransform {
                x: Some(f64::NAN),
                ..Default::default()
            },
        },
    )
    .expect_err("NaN must reject");

    match err {
        ClipSetTransformError::BadValue { field, value } => {
            assert_eq!(field, "x");
            assert!(value.is_nan())
        }
        other => panic!("expected BadValue, got {other:?}"),
    }
}

#[test]
fn compute_patch_bad_value_infinity_scale_x() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
        Transform::default(),
    )]);

    let err = compute_patch(
        &prior,
        &ClipSetTransformArgs {
            project_id: fixture_project_id(),
            clip: CLIP_TEXT_A.to_string(),
            transform: PartialTransform {
                scale_x: Some(f64::INFINITY),
                ..Default::default()
            },
        },
    )
    .expect_err("inf must reject");

    match err {
        ClipSetTransformError::BadValue { field, value } => {
            assert_eq!(field, "scale_x");
            assert!(value.is_infinite())
        }
        other => panic!("expected BadValue, got {other:?}"),
    }
}

#[test]
fn compute_patch_bad_value_neg_infinity_rotation_deg() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
        Transform::default(),
    )]);

    let err = compute_patch(
        &prior,
        &ClipSetTransformArgs {
            project_id: fixture_project_id(),
            clip: CLIP_TEXT_A.to_string(),
            transform: PartialTransform {
                rotation_deg: Some(f64::NEG_INFINITY),
                ..Default::default()
            },
        },
    )
    .expect_err("-inf must reject");

    match err {
        ClipSetTransformError::BadValue { field, value } => {
            assert_eq!(field, "rotation_deg");
            assert!(value.is_infinite())
        }
        other => panic!("expected BadValue, got {other:?}"),
    }
}

#[test]
fn compute_patch_bad_value_checks_run_after_lock_check() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        true,
        Transform::default(),
    )]);

    let err = compute_patch(
        &prior,
        &ClipSetTransformArgs {
            project_id: fixture_project_id(),
            clip: CLIP_TEXT_A.to_string(),
            transform: PartialTransform {
                x: Some(f64::NAN),
                ..Default::default()
            },
        },
    )
    .expect_err("locked should beat bad value check");

    assert!(matches!(err, ClipSetTransformError::Locked { .. }));
}

#[test]
fn compute_patch_locked_rejects() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        true,
        Transform::default(),
    )]);

    let err = compute_patch(
        &prior,
        &ClipSetTransformArgs {
            project_id: fixture_project_id(),
            clip: CLIP_TEXT_A.to_string(),
            transform: PartialTransform {
                x: Some(1.0),
                ..Default::default()
            },
        },
    )
    .expect_err("locked should reject");

    match err {
        ClipSetTransformError::Locked { clip_id } => assert_eq!(clip_id, CLIP_TEXT_A),
        other => panic!("expected Locked, got {other:?}"),
    }
}

#[test]
fn compute_patch_track_lock_does_not_block() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        true,
        CLIP_TEXT_A,
        false,
        Transform::default(),
    )]);

    let (patch, warnings, data) = compute_patch(
        &prior,
        &ClipSetTransformArgs {
            project_id: fixture_project_id(),
            clip: CLIP_TEXT_A.to_string(),
            transform: PartialTransform {
                x: Some(10.0),
                ..Default::default()
            },
        },
    )
    .expect("track lock should not block");

    assert_eq!(track_op_paths(&patch)[0], "/tracks/0/clips/0/transform/x");
    assert!(warnings.is_empty());
    assert_eq!(data.transform.x, 10.0);
}

#[test]
fn compute_patch_bad_selector() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
        Transform::default(),
    )]);

    let err = compute_patch(
        &prior,
        &ClipSetTransformArgs {
            project_id: fixture_project_id(),
            clip: "not-a-uuid".to_string(),
            transform: PartialTransform {
                x: Some(1.0),
                ..Default::default()
            },
        },
    )
    .expect_err("bad selector must reject");

    match err {
        ClipSetTransformError::BadSelector { detail } => {
            assert!(detail.contains("UUID"));
        }
        other => panic!("expected BadSelector, got {other:?}"),
    }
}

#[test]
fn compute_patch_not_found() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
        Transform::default(),
    )]);

    let err = compute_patch(
        &prior,
        &ClipSetTransformArgs {
            project_id: fixture_project_id(),
            clip: MISSING_CLIP.to_string(),
            transform: PartialTransform {
                x: Some(1.0),
                ..Default::default()
            },
        },
    )
    .expect_err("missing clip must reject");

    match err {
        ClipSetTransformError::ClipNotFound { clip_id } => assert_eq!(clip_id, MISSING_CLIP),
        other => panic!("expected ClipNotFound, got {other:?}"),
    }
}

#[test]
fn compute_patch_flip_h_and_flip_v_fields_work() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
        Transform {
            flip_h: false,
            flip_v: true,
            ..Transform::default()
        },
    )]);

    let (patch, warnings, data) = compute_patch(
        &prior,
        &ClipSetTransformArgs {
            project_id: fixture_project_id(),
            clip: CLIP_TEXT_A.to_string(),
            transform: PartialTransform {
                flip_h: Some(true),
                flip_v: Some(false),
                ..Default::default()
            },
        },
    )
    .expect("bool fields update");

    assert_eq!(patch.as_array().expect("patch is array").len(), 2);
    assert!(warnings.is_empty());
    assert_eq!(
        track_op_paths(&patch),
        vec![
            "/tracks/0/clips/0/transform/flip_h",
            "/tracks/0/clips/0/transform/flip_v",
        ]
    );
    assert_eq!(
        transform_patch_values(&patch),
        vec![json!(true), json!(false)]
    );
    assert!(data.transform.flip_h);
    assert!(!data.transform.flip_v);
}

#[test]
fn compute_patch_round_trip() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
        Transform::default(),
    )]);
    let args = ClipSetTransformArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_A.to_string(),
        transform: PartialTransform {
            x: Some(10.0),
            scale_x: Some(2.0),
            flip_h: Some(true),
            ..Default::default()
        },
    };

    let (patch, _warnings, _data) = compute_patch(&prior, &args).expect("compute_patch ok");
    let typed_patch: json_patch::Patch =
        serde_json::from_value(patch.clone()).expect("patch parses to RFC 6902");
    let post_state = prior
        .apply(&typed_patch)
        .expect("clip.set_transform patch should apply");
    let expected_data = serde_json::to_value(
        data_envelope_from_post_state(&args, &post_state).expect("envelope from post-state"),
    )
    .expect("envelope serializes");

    let recorded = RecordedEvent {
        verb: "clip.set_transform".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch,
        warnings: vec![],
        post_state,
        expected_data,
    };

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(ClipSetTransformVerb))
        .expect("register clip.set_transform verb");

    let report = validate_reconstructors(&registry, std::slice::from_ref(&recorded))
        .expect("reconstructor validation should pass");
    assert_eq!(report.verbs_checked, vec!["clip.set_transform"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn compute_patch_error_variants_map_to_bad_args() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
        Transform::default(),
    )]);
    let verb = ClipSetTransformVerb;

    let bad_selector = serde_json::to_value(ClipSetTransformArgs {
        project_id: fixture_project_id(),
        clip: "not-a-uuid".to_string(),
        transform: PartialTransform {
            x: Some(1.0),
            ..Default::default()
        },
    })
    .expect("bad selector args serialize");
    let err = verb
        .compute_patch(&prior, &bad_selector)
        .expect_err("bad selector maps");
    assert!(matches!(err, VerbError::BadArgs { .. }));

    let not_found = serde_json::to_value(ClipSetTransformArgs {
        project_id: fixture_project_id(),
        clip: MISSING_CLIP.to_string(),
        transform: PartialTransform {
            x: Some(1.0),
            ..Default::default()
        },
    })
    .expect("missing clip args serialize");
    let err = verb
        .compute_patch(&prior, &not_found)
        .expect_err("missing clip maps");
    assert!(matches!(err, VerbError::BadArgs { .. }));

    let verb_locked = serde_json::to_value(ClipSetTransformArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_A.to_string(),
        transform: PartialTransform {
            x: Some(1.0),
            ..Default::default()
        },
    })
    .expect("locked args serialize");
    let prior_locked = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        true,
        Transform::default(),
    )]);
    let err = verb
        .compute_patch(&prior_locked, &verb_locked)
        .expect_err("locked maps");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "clip.set_transform")
        .expect("default_fixtures includes clip.set_transform");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(ClipSetTransformVerb))
        .expect("register clip.set_transform verb");

    let report = validate_reconstructors(&registry, &[fixture])
        .expect("clip.set_transform reconstruct from fixture");
    assert_eq!(report.verbs_checked, vec!["clip.set_transform"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn data_envelope_from_post_state_returns_full_transform() {
    let post_state = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
        Transform {
            x: 12.0,
            y: -3.0,
            scale_x: 1.5,
            scale_y: 0.25,
            rotation_deg: 33.0,
            anchor_x: 0.2,
            anchor_y: 0.8,
            skew_x_deg: -0.5,
            skew_y_deg: 0.75,
            flip_h: true,
            flip_v: false,
        },
    )]);

    let args = ClipSetTransformArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_A.to_string(),
        transform: PartialTransform {
            y: Some(99.0),
            ..Default::default()
        },
    };

    let data = data_envelope_from_post_state(&args, &post_state)
        .expect("post-state envelope should be readable");
    let expected = &post_state.tracks[0].clips[0].transform;
    assert_eq!(data.clip_id.to_string(), CLIP_TEXT_A);
    assert_eq!(data.transform, *expected);
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
        false,
        Transform::default(),
    )]);

    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        base_project,
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry clears gate");

    let outcome = store
        .mutate_via_verb(
            "clip.set_transform",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "clip": CLIP_TEXT_A,
                "transform": {"x": 50.0}
            }),
            None,
        )
        .expect("mutate_via_verb happy path");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("happy path must return Applied, got {outcome:?}")
    };

    let data: ClipSetTransformData =
        serde_json::from_value(data).expect("clip.set_transform data is ClipSetTransformData");
    assert_eq!(data.clip_id.to_string(), CLIP_TEXT_A);
    assert_eq!(store.project().tracks[0].clips[0].transform.x, 50.0);
    assert_eq!(warnings, Vec::<Value>::new());
}

#[test]
fn multi_track_patch_uses_track_index() {
    let prior = project_with_tracks(vec![
        text_track(
            TRACK_TEXT_A,
            "Text 1",
            false,
            CLIP_TEXT_A,
            false,
            Transform::default(),
        ),
        text_track(
            TRACK_TEXT_B,
            "Text 2",
            false,
            CLIP_TEXT_B,
            false,
            Transform::default(),
        ),
    ]);

    let args = ClipSetTransformArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_B.to_string(),
        transform: PartialTransform {
            x: Some(99.0),
            ..Default::default()
        },
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("search in second track");
    assert!(warnings.is_empty());
    assert_eq!(
        track_op_paths(&patch),
        vec!["/tracks/1/clips/0/transform/x"]
    );
    assert_eq!(data.clip_id.to_string(), CLIP_TEXT_B);
    assert_eq!(data.transform.x, 99.0);
}

#[test]
fn compute_patch_returns_bad_selector_before_not_found() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
        Transform::default(),
    )]);

    let err = compute_patch(
        &prior,
        &ClipSetTransformArgs {
            project_id: fixture_project_id(),
            clip: "not-a-uuid".to_string(),
            transform: Default::default(),
        },
    )
    .expect_err("bad selector before not_found");
    assert!(matches!(err, ClipSetTransformError::BadSelector { .. }));
}
