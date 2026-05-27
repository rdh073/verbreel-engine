//! Tests for `timeline.diff` (§12.2) — v1 event-log range floor.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::timeline_diff::compute_patch;
use verbreel_state::{
    MutateOutcome, Project, TimelineDiffArgs, TimelineDiffData, TimelineDiffError,
    TimelineDiffEvent, TimelineDiffVerb, Verb, VerbError, VerbRegistry, default_fixtures,
    default_registry, validate_reconstructors,
};

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const A_VALID_V7: &str = "0190b8d3-15e3-7000-bd00-0000feedbeef";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture → Project")
}

fn fixture_project_id() -> verbreel_types::ProjectId {
    empty_project().id
}

fn args_default() -> TimelineDiffArgs {
    TimelineDiffArgs {
        project_id: fixture_project_id(),
        since: A_VALID_V7.to_string(),
        until: None,
    }
}

// --- args deserialization ---------------------------------------------------

#[test]
fn args_deserialize_happy_paths_table() {
    struct Case {
        name: &'static str,
        raw: Value,
        since: &'static str,
        until: Option<&'static str>,
    }

    let cases = vec![
        Case {
            name: "since only",
            raw: json!({
                "project_id": FIXTURE_PROJECT_ID,
                "since": A_VALID_V7,
            }),
            since: A_VALID_V7,
            until: None,
        },
        Case {
            name: "since and until",
            raw: json!({
                "project_id": FIXTURE_PROJECT_ID,
                "since": A_VALID_V7,
                "until": "0190b8d3-15e3-7000-bd00-0000abcdd001",
            }),
            since: A_VALID_V7,
            until: Some("0190b8d3-15e3-7000-bd00-0000abcdd001"),
        },
        Case {
            name: "since empty sentinel",
            raw: json!({
                "project_id": FIXTURE_PROJECT_ID,
                "since": "empty",
            }),
            since: "empty",
            until: None,
        },
        Case {
            name: "opaque non-uuid strings are accepted",
            raw: json!({
                "project_id": FIXTURE_PROJECT_ID,
                "since": "not-a-uuid",
                "until": "also-not-a-uuid",
            }),
            since: "not-a-uuid",
            until: Some("also-not-a-uuid"),
        },
        Case {
            name: "until empty sentinel accepted at args layer",
            raw: json!({
                "project_id": FIXTURE_PROJECT_ID,
                "since": A_VALID_V7,
                "until": "empty",
            }),
            since: A_VALID_V7,
            until: Some("empty"),
        },
    ];

    for case in cases {
        let args: TimelineDiffArgs =
            serde_json::from_value(case.raw).unwrap_or_else(|_| panic!("{}", case.name));
        assert_eq!(
            args.project_id.to_string(),
            FIXTURE_PROJECT_ID,
            "{}",
            case.name
        );
        assert_eq!(args.since, case.since, "{}", case.name);
        assert_eq!(args.until.as_deref(), case.until, "{}", case.name);
    }
}

#[test]
fn args_unknown_field_rejected_by_deny_unknown_fields() {
    let err = serde_json::from_value::<TimelineDiffArgs>(json!({
        "project_id": FIXTURE_PROJECT_ID,
        "since": A_VALID_V7,
        "extra": 1,
    }))
    .expect_err("unknown field must fail");
    let msg = err.to_string();
    assert!(msg.contains("unknown field"), "unexpected error: {msg}");
}

#[test]
fn args_missing_project_id_is_bad_args_through_verb() {
    let prior = empty_project();
    let verb = TimelineDiffVerb;
    let err = verb
        .compute_patch(&prior, &json!({ "since": A_VALID_V7 }))
        .expect_err("missing project_id should fail");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_missing_since_is_bad_args_through_verb() {
    let prior = empty_project();
    let verb = TimelineDiffVerb;
    let err = verb
        .compute_patch(&prior, &json!({ "project_id": FIXTURE_PROJECT_ID }))
        .expect_err("missing since should fail");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_wrong_type_until_is_bad_args_through_verb() {
    let prior = empty_project();
    let verb = TimelineDiffVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "since": A_VALID_V7,
                "until": 7,
            }),
        )
        .expect_err("non-string until should fail");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

// --- v1 floor: every well-formed call errors -------------------------------

#[test]
fn compute_patch_always_returns_event_not_found_table() {
    struct Case {
        name: &'static str,
        args: TimelineDiffArgs,
    }

    let cases = vec![
        Case {
            name: "since only",
            args: args_default(),
        },
        Case {
            name: "since empty",
            args: TimelineDiffArgs {
                project_id: fixture_project_id(),
                since: "empty".to_string(),
                until: None,
            },
        },
        Case {
            name: "until empty",
            args: TimelineDiffArgs {
                project_id: fixture_project_id(),
                since: A_VALID_V7.to_string(),
                until: Some("empty".to_string()),
            },
        },
    ];

    let prior = empty_project();
    for case in cases {
        let err = compute_patch(&prior, &case.args).expect_err(case.name);
        let TimelineDiffError::EventNotFound { detail } = err;
        assert!(
            detail.contains(&case.args.since),
            "{} detail must include since token",
            case.name
        );
    }
}

#[test]
fn until_empty_routes_to_runtime_error_not_bad_args() {
    let prior = empty_project();
    let verb = TimelineDiffVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "since": A_VALID_V7,
                "until": "empty",
            }),
        )
        .expect_err("v1 floor should error");
    assert!(matches!(err, VerbError::Custom(_)));
}

#[test]
fn since_empty_routes_to_runtime_error_not_bad_args() {
    let prior = empty_project();
    let verb = TimelineDiffVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "since": "empty",
            }),
        )
        .expect_err("v1 floor should error");
    assert!(matches!(err, VerbError::Custom(_)));
}

#[test]
fn error_maps_to_custom_and_includes_e_event_not_found() {
    let prior = empty_project();
    let verb = TimelineDiffVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "since": A_VALID_V7,
            }),
        )
        .expect_err("v1 floor should error");
    let VerbError::Custom(detail) = err else {
        panic!("expected Custom, got {err:?}");
    };
    assert!(
        detail.contains("E_EVENT_NOT_FOUND"),
        "detail should include E_EVENT_NOT_FOUND: {detail}"
    );
}

#[test]
fn error_path_is_project_agnostic() {
    let prior_a = empty_project();
    let mut prior_b = empty_project();
    prior_b.name = "another-name".to_string();
    prior_b.duration_tk = verbreel_types::Tick::new(123_456);
    let args = args_default();
    let err_a = compute_patch(&prior_a, &args).expect_err("a errors");
    let err_b = compute_patch(&prior_b, &args).expect_err("b errors");
    assert_eq!(err_a, err_b);
}

#[test]
fn timeline_diff_error_into_verb_error_is_custom() {
    let err = TimelineDiffError::EventNotFound {
        detail: "missing".to_string(),
    };
    let mapped: VerbError = err.into();
    assert!(matches!(mapped, VerbError::Custom(_)));
}

// --- future success data shape ---------------------------------------------

#[test]
fn future_success_data_serializes_empty_arrays() {
    let data = TimelineDiffData {
        patches: vec![],
        events: vec![],
    };
    let value = serde_json::to_value(&data).expect("serialize");
    assert_eq!(value, json!({ "patches": [], "events": [] }));
}

#[test]
fn future_success_data_serializes_patch_and_event_row() {
    let data = TimelineDiffData {
        patches: vec![json!([
            {"op":"replace","path":"/name","value":"renamed"}
        ])],
        events: vec![TimelineDiffEvent {
            id: "0190b8d3-15e3-7000-bd00-0000e0e0aa01".to_string(),
            verb: "project.rename".to_string(),
            ts: "2026-05-27T10:00:00Z".to_string(),
        }],
    };
    let value = serde_json::to_value(&data).expect("serialize");
    assert_eq!(
        value,
        json!({
            "patches": [[{"op":"replace","path":"/name","value":"renamed"}]],
            "events": [{
                "id":"0190b8d3-15e3-7000-bd00-0000e0e0aa01",
                "verb":"project.rename",
                "ts":"2026-05-27T10:00:00Z"
            }]
        })
    );
}

// --- reconstructor / fixtures / registry -----------------------------------

#[test]
fn reconstruct_returns_null_for_well_formed_args() {
    let verb = TimelineDiffVerb;
    let prior = empty_project();
    let args = serde_json::to_value(args_default()).expect("args → Value");
    let data = verb
        .reconstruct(&args, &json!([]), &[], &prior)
        .expect("reconstruct should succeed");
    assert_eq!(data, Value::Null);
}

#[test]
fn reconstruct_rejects_malformed_args() {
    let verb = TimelineDiffVerb;
    let prior = empty_project();
    let err = verb
        .reconstruct(&json!({}), &json!([]), &[], &prior)
        .expect_err("malformed args should fail");
    let msg = err.to_string();
    assert!(
        msg.contains("TimelineDiffArgs") || msg.contains("wrong type"),
        "unexpected reconstruct error: {msg}"
    );
}

#[test]
fn default_fixture_has_empty_patch_warnings_and_null_expected_data() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "timeline.diff")
        .expect("default_fixtures includes timeline.diff");

    assert_eq!(fixture.patch, json!([]));
    assert!(fixture.warnings.is_empty());
    assert_eq!(fixture.expected_data, Value::Null);
}

#[test]
fn default_fixture_validates_with_timeline_diff_only_registry() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "timeline.diff")
        .expect("default_fixtures includes timeline.diff");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(TimelineDiffVerb))
        .expect("register timeline.diff");

    let report = validate_reconstructors(&registry, &[fixture]).expect("validation passes");
    assert_eq!(report.verbs_checked, vec!["timeline.diff"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn default_registry_contains_timeline_diff() {
    let registry = default_registry();
    let verb = registry
        .get("timeline.diff")
        .expect("timeline.diff must be in default_registry");
    assert_eq!(verb.verb(), "timeline.diff");
}

#[test]
fn default_registry_route_returns_custom_on_v1_floor() {
    let registry = default_registry();
    let verb = registry
        .get("timeline.diff")
        .expect("timeline.diff must be in default_registry");
    let prior = empty_project();
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "since": A_VALID_V7,
            }),
        )
        .expect_err("v1 floor should error");
    assert!(matches!(err, VerbError::Custom(_)));
}

#[cfg(feature = "native")]
#[test]
fn mutate_via_verb_returns_error_on_v1_floor() {
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
        "timeline.diff",
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "since": A_VALID_V7,
        }),
        None,
    );

    match outcome {
        Err(err) => {
            let msg = err.to_string();
            assert!(
                msg.contains("E_EVENT_NOT_FOUND"),
                "error should include E_EVENT_NOT_FOUND: {msg}"
            );
        }
        Ok(MutateOutcome::Applied { .. }) => {
            panic!("expected mutate_via_verb to error in v1 floor, got Applied")
        }
        Ok(other) => panic!("expected Err for v1 floor, got Ok({other:?})"),
    }
}
