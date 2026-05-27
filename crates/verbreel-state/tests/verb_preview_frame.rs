//! Tests for `preview.frame` (§14.1) — v1 renderer/cache floor.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::preview_frame::compute_patch;
use verbreel_state::{
    PreviewFrameArgs, PreviewFrameData, PreviewFrameError, PreviewFrameVerb, Project, Verb,
    VerbError, VerbRegistry, default_fixtures, default_registry, validate_reconstructors,
};
use verbreel_types::ProjectId;

#[cfg(feature = "native")]
use verbreel_state::{LifecycleError, ProjectStore};

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture -> Project")
}

fn fixture_project_id() -> ProjectId {
    empty_project().id
}

fn args_default() -> PreviewFrameArgs {
    PreviewFrameArgs {
        project_id: fixture_project_id(),
        at_tk: 0,
        out_path: None,
        width_px: None,
        deterministic: false,
    }
}

fn args_value() -> Value {
    json!({
        "project_id": FIXTURE_PROJECT_ID,
        "at_tk": 0,
    })
}

#[test]
fn args_deserialize_ok_with_minimal_fields() {
    let typed: PreviewFrameArgs = serde_json::from_value(args_value()).expect("args parse");
    assert_eq!(typed.project_id.to_string(), FIXTURE_PROJECT_ID);
    assert_eq!(typed.at_tk, 0);
    assert_eq!(typed.out_path, None);
    assert_eq!(typed.width_px, None);
    assert!(!typed.deterministic);
}

#[test]
fn args_deserialize_ok_with_all_optionals() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "at_tk": 1200,
        "out_path": "tmp/frame.png",
        "width_px": 1920,
        "deterministic": true,
    });
    let typed: PreviewFrameArgs = serde_json::from_value(raw).expect("args parse");
    assert_eq!(typed.project_id.to_string(), FIXTURE_PROJECT_ID);
    assert_eq!(typed.at_tk, 1200);
    assert_eq!(typed.out_path.as_deref(), Some("tmp/frame.png"));
    assert_eq!(typed.width_px, Some(1920));
    assert!(typed.deterministic);
}

#[test]
fn deterministic_omitted_defaults_false() {
    let typed: PreviewFrameArgs = serde_json::from_value(args_value()).expect("args parse");
    assert!(!typed.deterministic);
}

#[test]
fn deterministic_true_is_preserved() {
    let typed: PreviewFrameArgs = serde_json::from_value(json!({
        "project_id": FIXTURE_PROJECT_ID,
        "at_tk": 10,
        "deterministic": true,
    }))
    .expect("args parse");
    assert!(typed.deterministic);
}

#[test]
fn unknown_field_rejected_through_verb() {
    let prior = empty_project();
    let verb = PreviewFrameVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "at_tk": 0,
                "extra": true,
            }),
        )
        .expect_err("deny_unknown_fields rejects extras");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn missing_project_id_fails_through_verb() {
    let prior = empty_project();
    let verb = PreviewFrameVerb;
    let err = verb
        .compute_patch(&prior, &json!({ "at_tk": 0 }))
        .expect_err("missing project_id should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn missing_at_tk_fails_through_verb() {
    let prior = empty_project();
    let verb = PreviewFrameVerb;
    let err = verb
        .compute_patch(&prior, &json!({ "project_id": FIXTURE_PROJECT_ID }))
        .expect_err("missing at_tk should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn non_integer_at_tk_fails_through_verb() {
    let prior = empty_project();
    let verb = PreviewFrameVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({ "project_id": FIXTURE_PROJECT_ID, "at_tk": "zero" }),
        )
        .expect_err("non-integer at_tk should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn non_integer_width_px_fails_through_verb() {
    let prior = empty_project();
    let verb = PreviewFrameVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "at_tk": 0,
                "width_px": "wide",
            }),
        )
        .expect_err("non-integer width_px should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn non_boolean_deterministic_fails_through_verb() {
    let prior = empty_project();
    let verb = PreviewFrameVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "at_tk": 0,
                "deterministic": "yes",
            }),
        )
        .expect_err("non-boolean deterministic should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn at_tk_negative_returns_bad_time() {
    let prior = empty_project();
    let err = compute_patch(
        &prior,
        &PreviewFrameArgs {
            at_tk: -1,
            ..args_default()
        },
    )
    .expect_err("negative at_tk should fail");
    let PreviewFrameError::BadTime { at_tk } = err else {
        panic!("expected BadTime, got {err:?}");
    };
    assert_eq!(at_tk, -1);
}

#[test]
fn at_tk_negative_maps_to_bad_args_through_verb() {
    let prior = empty_project();
    let verb = PreviewFrameVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({ "project_id": FIXTURE_PROJECT_ID, "at_tk": -1 }),
        )
        .expect_err("negative at_tk should map to BadArgs");
    let VerbError::BadArgs { detail } = err else {
        panic!("expected BadArgs, got {err:?}");
    };
    assert!(detail.contains("E_BAD_TIME"));
}

#[test]
fn width_px_out_of_range_returns_bad_range() {
    let prior = empty_project();
    let cases = [0_i64, -1, 8193];

    for width in cases {
        let err = compute_patch(
            &prior,
            &PreviewFrameArgs {
                width_px: Some(width),
                ..args_default()
            },
        )
        .expect_err("out-of-range width_px should fail");
        let PreviewFrameError::BadRange {
            field,
            value,
            allowed,
        } = err
        else {
            panic!("expected BadRange, got {err:?}");
        };
        assert_eq!(field, "width_px");
        assert_eq!(value, width);
        assert_eq!(allowed, "[1, 8192]");
    }
}

#[test]
fn width_px_out_of_range_maps_to_bad_args_through_verb() {
    let prior = empty_project();
    let verb = PreviewFrameVerb;
    let cases = [0_i64, -1, 8193];

    for width in cases {
        let err = verb
            .compute_patch(
                &prior,
                &json!({
                    "project_id": FIXTURE_PROJECT_ID,
                    "at_tk": 0,
                    "width_px": width,
                }),
            )
            .expect_err("out-of-range width_px should map to BadArgs");
        let VerbError::BadArgs { detail } = err else {
            panic!("expected BadArgs, got {err:?}");
        };
        assert!(detail.contains("E_BAD_RANGE"));
    }
}

#[test]
fn width_px_lower_bound_reaches_io_floor() {
    let prior = empty_project();
    let err = compute_patch(
        &prior,
        &PreviewFrameArgs {
            width_px: Some(1),
            ..args_default()
        },
    )
    .expect_err("v1 floor should still return Io");
    assert!(matches!(err, PreviewFrameError::Io { .. }));
}

#[test]
fn width_px_upper_bound_reaches_io_floor() {
    let prior = empty_project();
    let err = compute_patch(
        &prior,
        &PreviewFrameArgs {
            width_px: Some(8192),
            ..args_default()
        },
    )
    .expect_err("v1 floor should still return Io");
    assert!(matches!(err, PreviewFrameError::Io { .. }));
}

#[test]
fn omitted_width_reaches_io_floor() {
    let prior = empty_project();
    let err = compute_patch(&prior, &args_default()).expect_err("v1 floor should return Io");
    assert!(matches!(err, PreviewFrameError::Io { .. }));
}

#[test]
fn runtime_io_maps_to_custom_and_includes_context() {
    let prior = empty_project();
    let verb = PreviewFrameVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "at_tk": 1440,
                "out_path": "tmp/p.png",
                "width_px": 640,
                "deterministic": true,
            }),
        )
        .expect_err("well-formed args should hit v1 Io floor");
    let VerbError::Custom(detail) = err else {
        panic!("expected Custom, got {err:?}");
    };
    assert!(detail.contains("E_IO"));
    assert!(detail.contains("at_tk 1440"));
    assert!(detail.contains("tmp/p.png"));
}

#[test]
fn future_success_data_shape_serializes_all_fields() {
    let data = PreviewFrameData {
        path: "cache/frames/a.png".to_string(),
        sha256: "deadbeef".to_string(),
        width: 1920,
        height: 1080,
        cache_hit: true,
    };
    let value = serde_json::to_value(&data).expect("data serializes");
    let obj = value.as_object().expect("data object");
    assert_eq!(obj.len(), 5);
    assert_eq!(obj.get("path"), Some(&json!("cache/frames/a.png")));
    assert_eq!(obj.get("sha256"), Some(&json!("deadbeef")));
    assert_eq!(obj.get("width"), Some(&json!(1920)));
    assert_eq!(obj.get("height"), Some(&json!(1080)));
    assert_eq!(obj.get("cache_hit"), Some(&json!(true)));
}

#[test]
fn path_escape_variant_displays_e_path_escape() {
    let msg = PreviewFrameError::PathEscape {
        path: "../escape.png".to_string(),
    }
    .to_string();
    assert!(msg.contains("E_PATH_ESCAPE"));
}

#[test]
fn reconstruct_returns_null_for_well_formed_args() {
    let verb = PreviewFrameVerb;
    let prior = empty_project();
    let data = verb
        .reconstruct(&args_value(), &json!([]), &[], &prior)
        .expect("reconstruct succeeds for well-formed args");
    assert_eq!(data, Value::Null);
}

#[test]
fn reconstruct_rejects_malformed_args() {
    let verb = PreviewFrameVerb;
    let prior = empty_project();
    let err = verb
        .reconstruct(&json!({}), &json!([]), &[], &prior)
        .expect_err("malformed args should fail reconstruction");
    let msg = err.to_string();
    assert!(
        msg.contains("PreviewFrameArgs") || msg.contains("wrong type"),
        "unexpected reconstruct error: {msg}",
    );
}

#[test]
fn default_fixture_validates_with_only_preview_frame_verb() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "preview.frame")
        .expect("default_fixtures includes preview.frame");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(PreviewFrameVerb))
        .expect("register preview.frame verb");

    let report = validate_reconstructors(&registry, &[fixture]).expect("fixture validates");
    assert_eq!(report.verbs_checked, vec!["preview.frame"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn default_fixture_has_empty_patch_warnings_and_null_expected_data() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "preview.frame")
        .expect("default_fixtures includes preview.frame");
    assert_eq!(fixture.patch, json!([]));
    assert!(fixture.warnings.is_empty());
    assert_eq!(fixture.expected_data, Value::Null);
}

#[test]
fn default_registry_contains_preview_frame() {
    let registry = default_registry();
    let verb = registry
        .get("preview.frame")
        .expect("preview.frame is in default_registry");
    let prior = empty_project();
    let err = verb
        .compute_patch(&prior, &args_value())
        .expect_err("v1 floor always errors");
    let VerbError::Custom(detail) = err else {
        panic!("expected Custom, got {err:?}");
    };
    assert!(detail.contains("E_IO"));
}

#[cfg(feature = "native")]
#[test]
fn mutate_via_verb_route_returns_custom_io() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        empty_project(),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry succeeds");

    let outcome = store.mutate_via_verb("preview.frame", args_value(), None);
    let Err(LifecycleError::VerbExecutionFailed { source, .. }) = outcome else {
        panic!("expected VerbExecutionFailed, got {outcome:?}");
    };
    let VerbError::Custom(detail) = source else {
        panic!("expected Custom source, got {source:?}");
    };
    assert!(detail.contains("E_IO"));
}
