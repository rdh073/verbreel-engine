//! Tests for `timeline.undo` (§12.3) — v1 undo-stack floor.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::timeline_undo::{compute_patch, resolved_steps};
use verbreel_state::{
    MutateOutcome, Project, TimelineUndoArgs, TimelineUndoData, TimelineUndoError,
    TimelineUndoVerb, Verb, VerbError, VerbRegistry, default_fixtures, default_registry,
    validate_reconstructors,
};

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture -> Project")
}

fn fixture_project_id() -> verbreel_types::ProjectId {
    empty_project().id
}

fn args_default() -> TimelineUndoArgs {
    TimelineUndoArgs {
        project_id: fixture_project_id(),
        steps: None,
    }
}

// --- args deserialization ---------------------------------------------------

#[test]
fn args_deserialize_happy_paths_table() {
    struct Case {
        name: &'static str,
        raw: Value,
        expected_steps: Option<i64>,
    }

    let cases = vec![
        Case {
            name: "steps omitted",
            raw: json!({
                "project_id": FIXTURE_PROJECT_ID,
            }),
            expected_steps: None,
        },
        Case {
            name: "steps explicit positive",
            raw: json!({
                "project_id": FIXTURE_PROJECT_ID,
                "steps": 3,
            }),
            expected_steps: Some(3),
        },
    ];

    for case in cases {
        let args: TimelineUndoArgs =
            serde_json::from_value(case.raw).unwrap_or_else(|_| panic!("{}", case.name));
        assert_eq!(
            args.project_id.to_string(),
            FIXTURE_PROJECT_ID,
            "{}",
            case.name
        );
        assert_eq!(args.steps, case.expected_steps, "{}", case.name);
    }
}

#[test]
fn resolved_steps_defaults_to_one() {
    let args = args_default();
    assert_eq!(resolved_steps(&args), 1);
}

#[test]
fn resolved_steps_preserves_explicit_positive_steps() {
    let args = TimelineUndoArgs {
        project_id: fixture_project_id(),
        steps: Some(7),
    };
    assert_eq!(resolved_steps(&args), 7);
}

#[test]
fn args_unknown_field_rejected_by_deny_unknown_fields() {
    let err = serde_json::from_value::<TimelineUndoArgs>(json!({
        "project_id": FIXTURE_PROJECT_ID,
        "steps": 1,
        "extra": true,
    }))
    .expect_err("unknown field should fail");
    assert!(
        err.to_string().contains("unknown field"),
        "unexpected error: {err}",
    );
}

#[test]
fn args_missing_project_id_is_bad_args_through_verb() {
    let prior = empty_project();
    let verb = TimelineUndoVerb;
    let err = verb
        .compute_patch(&prior, &json!({ "steps": 1 }))
        .expect_err("missing project_id should fail");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_non_integer_steps_is_bad_args_through_verb() {
    let prior = empty_project();
    let verb = TimelineUndoVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "steps": "2",
            }),
        )
        .expect_err("non-integer steps should fail args parse");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

// --- local schema floor: steps < 1 => E_SCHEMA_VIOLATION ------------------

#[test]
fn compute_patch_zero_steps_is_schema_violation() {
    let prior = empty_project();
    let args = TimelineUndoArgs {
        project_id: fixture_project_id(),
        steps: Some(0),
    };
    let err = compute_patch(&prior, &args).expect_err("steps=0 should fail schema");
    let TimelineUndoError::SchemaViolation { detail } = err else {
        panic!("expected SchemaViolation");
    };
    assert!(
        detail.contains("steps"),
        "detail should mention steps: {detail}"
    );
}

#[test]
fn compute_patch_negative_steps_is_schema_violation() {
    let prior = empty_project();
    let args = TimelineUndoArgs {
        project_id: fixture_project_id(),
        steps: Some(-5),
    };
    let err = compute_patch(&prior, &args).expect_err("negative steps should fail schema");
    let TimelineUndoError::SchemaViolation { detail } = err else {
        panic!("expected SchemaViolation");
    };
    assert!(
        detail.contains("-5"),
        "detail should include offending value: {detail}"
    );
}

#[test]
fn schema_violation_maps_to_bad_args_through_verb() {
    let prior = empty_project();
    let verb = TimelineUndoVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "steps": 0,
            }),
        )
        .expect_err("steps=0 should map to BadArgs");
    let VerbError::BadArgs { detail } = err else {
        panic!("expected BadArgs, got {err:?}");
    };
    assert!(
        detail.contains("E_SCHEMA_VIOLATION"),
        "detail should include schema code: {detail}",
    );
}

#[test]
fn timeline_undo_error_into_verb_error_bad_args_for_schema_violation() {
    let err = TimelineUndoError::SchemaViolation {
        detail: "steps < 1".to_string(),
    };
    let mapped: VerbError = err.into();
    assert!(matches!(mapped, VerbError::BadArgs { .. }));
}

// --- v1 runtime floor: well-formed args => E_NOTHING_TO_UNDO --------------

#[test]
fn compute_patch_positive_steps_returns_nothing_to_undo() {
    let prior = empty_project();
    let args = TimelineUndoArgs {
        project_id: fixture_project_id(),
        steps: Some(2),
    };
    let err = compute_patch(&prior, &args).expect_err("v1 floor should runtime-error");
    let TimelineUndoError::NothingToUndo { detail } = err else {
        panic!("expected NothingToUndo");
    };
    assert!(detail.contains("requested_steps=2"), "detail: {detail}");
}

#[test]
fn compute_patch_omitted_steps_returns_nothing_to_undo() {
    let prior = empty_project();
    let err = compute_patch(&prior, &args_default()).expect_err("v1 floor should runtime-error");
    let TimelineUndoError::NothingToUndo { detail } = err else {
        panic!("expected NothingToUndo");
    };
    assert!(detail.contains("requested_steps=1"), "detail: {detail}");
}

#[test]
fn runtime_error_maps_to_custom_not_bad_args() {
    let prior = empty_project();
    let verb = TimelineUndoVerb;
    let err = verb
        .compute_patch(&prior, &json!({ "project_id": FIXTURE_PROJECT_ID }))
        .expect_err("v1 floor should runtime-error");
    assert!(matches!(err, VerbError::Custom(_)));
}

#[test]
fn error_text_includes_e_nothing_to_undo() {
    let prior = empty_project();
    let verb = TimelineUndoVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "steps": 3,
            }),
        )
        .expect_err("v1 floor should runtime-error");
    let VerbError::Custom(detail) = err else {
        panic!("expected Custom");
    };
    assert!(
        detail.contains("E_NOTHING_TO_UNDO"),
        "detail should include E_NOTHING_TO_UNDO: {detail}",
    );
}

#[test]
fn timeline_undo_error_into_verb_error_custom_for_nothing_to_undo() {
    let err = TimelineUndoError::NothingToUndo {
        detail: "empty history pointer".to_string(),
    };
    let mapped: VerbError = err.into();
    assert!(matches!(mapped, VerbError::Custom(_)));
}

#[test]
fn error_path_is_project_agnostic() {
    let prior_a = empty_project();
    let mut prior_b = empty_project();
    prior_b.name = "different-project".to_string();
    prior_b.duration_tk = verbreel_types::Tick::new(321_000);

    let args = TimelineUndoArgs {
        project_id: fixture_project_id(),
        steps: Some(2),
    };

    let err_a = compute_patch(&prior_a, &args).expect_err("a errors");
    let err_b = compute_patch(&prior_b, &args).expect_err("b errors");
    assert_eq!(err_a, err_b);
}

// --- future success data shape ---------------------------------------------

#[test]
fn future_success_data_serializes_full_shape() {
    let data = TimelineUndoData {
        undone_event_ids: vec![
            "0190b8d3-15e3-7000-bd00-0000aa000001".to_string(),
            "0190b8d3-15e3-7000-bd00-0000aa000002".to_string(),
        ],
        current_event_id: "0190b8d3-15e3-7000-bd00-0000aa000003".to_string(),
        requested_steps: 3,
        actual_steps: 2,
    };
    let value = serde_json::to_value(&data).expect("serialize");
    assert_eq!(
        value,
        json!({
            "undone_event_ids": [
                "0190b8d3-15e3-7000-bd00-0000aa000001",
                "0190b8d3-15e3-7000-bd00-0000aa000002"
            ],
            "current_event_id": "0190b8d3-15e3-7000-bd00-0000aa000003",
            "requested_steps": 3,
            "actual_steps": 2
        })
    );
}

// --- reconstructor / fixtures / registry -----------------------------------

#[test]
fn reconstruct_returns_null_for_well_formed_args() {
    let verb = TimelineUndoVerb;
    let prior = empty_project();
    let args = serde_json::to_value(args_default()).expect("args -> Value");
    let data = verb
        .reconstruct(&args, &json!([]), &[], &prior)
        .expect("reconstruct should succeed");
    assert_eq!(data, Value::Null);
}

#[test]
fn reconstruct_rejects_malformed_args() {
    let verb = TimelineUndoVerb;
    let prior = empty_project();
    let err = verb
        .reconstruct(&json!({}), &json!([]), &[], &prior)
        .expect_err("malformed args should fail");
    let msg = err.to_string();
    assert!(
        msg.contains("TimelineUndoArgs") || msg.contains("wrong type"),
        "unexpected reconstruct error: {msg}",
    );
}

#[test]
fn default_fixture_has_empty_patch_warnings_and_null_expected_data() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "timeline.undo")
        .expect("default_fixtures includes timeline.undo");

    assert_eq!(fixture.patch, json!([]));
    assert!(fixture.warnings.is_empty());
    assert_eq!(fixture.expected_data, Value::Null);
}

#[test]
fn default_fixture_validates_with_timeline_undo_only_registry() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "timeline.undo")
        .expect("default_fixtures includes timeline.undo");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(TimelineUndoVerb))
        .expect("register timeline.undo");

    let report = validate_reconstructors(&registry, &[fixture]).expect("validation should pass");
    assert_eq!(report.verbs_checked, vec!["timeline.undo"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn default_registry_contains_timeline_undo() {
    let registry = default_registry();
    let verb = registry
        .get("timeline.undo")
        .expect("timeline.undo must be in default_registry");
    assert_eq!(verb.verb(), "timeline.undo");
}

#[test]
fn default_registry_route_returns_custom_on_v1_floor() {
    let registry = default_registry();
    let verb = registry
        .get("timeline.undo")
        .expect("timeline.undo must be in default_registry");
    let prior = empty_project();
    let err = verb
        .compute_patch(&prior, &json!({ "project_id": FIXTURE_PROJECT_ID }))
        .expect_err("v1 floor should runtime-error");
    assert!(matches!(err, VerbError::Custom(_)));
}

#[cfg(feature = "native")]
#[test]
fn mutate_via_verb_returns_custom_on_v1_floor() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        empty_project(),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry succeeds");

    let outcome = store.mutate_via_verb(
        "timeline.undo",
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "steps": 1,
        }),
        None,
    );

    match outcome {
        Err(err) => {
            let msg = err.to_string();
            assert!(
                msg.contains("E_NOTHING_TO_UNDO"),
                "error should include E_NOTHING_TO_UNDO: {msg}",
            );
        }
        Ok(MutateOutcome::Applied { .. }) => {
            panic!("expected mutate_via_verb to error in v1 floor, got Applied")
        }
        Ok(other) => panic!("expected Err for v1 floor, got Ok({other:?})"),
    }
}
