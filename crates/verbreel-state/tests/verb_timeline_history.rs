//! Tests for `timeline.history` (§12.6) — v1 event-ring floor.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::timeline_history::{compute_patch, data_envelope_from_args};
use verbreel_state::{
    MutateOutcome, Project, TimelineHistoryArgs, TimelineHistoryData, TimelineHistoryEvent,
    TimelineHistoryEventKind, TimelineHistoryVerb, Verb, VerbError, VerbRegistry, default_fixtures,
    default_registry, validate_reconstructors,
};

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture → Project")
}

fn fixture_project_id() -> verbreel_types::ProjectId {
    empty_project().id
}

fn args_default() -> TimelineHistoryArgs {
    TimelineHistoryArgs {
        project_id: fixture_project_id(),
        limit: None,
        since: None,
        include_undone: None,
    }
}

#[test]
fn args_deserialize_happy_paths_table() {
    struct Case {
        name: &'static str,
        raw: Value,
        limit: Option<i64>,
        since: Option<&'static str>,
        include_undone: Option<bool>,
    }

    let cases = vec![
        Case {
            name: "project_id only",
            raw: json!({ "project_id": FIXTURE_PROJECT_ID }),
            limit: None,
            since: None,
            include_undone: None,
        },
        Case {
            name: "with limit",
            raw: json!({
                "project_id": FIXTURE_PROJECT_ID,
                "limit": 25,
            }),
            limit: Some(25),
            since: None,
            include_undone: None,
        },
        Case {
            name: "with since",
            raw: json!({
                "project_id": FIXTURE_PROJECT_ID,
                "since": "0190b8d3-15e3-7000-bd00-0000abcdd001",
            }),
            limit: None,
            since: Some("0190b8d3-15e3-7000-bd00-0000abcdd001"),
            include_undone: None,
        },
        Case {
            name: "with since empty sentinel",
            raw: json!({
                "project_id": FIXTURE_PROJECT_ID,
                "since": "empty",
            }),
            limit: None,
            since: Some("empty"),
            include_undone: None,
        },
        Case {
            name: "with include_undone",
            raw: json!({
                "project_id": FIXTURE_PROJECT_ID,
                "include_undone": true,
            }),
            limit: None,
            since: None,
            include_undone: Some(true),
        },
    ];

    for case in cases {
        let args: TimelineHistoryArgs =
            serde_json::from_value(case.raw).unwrap_or_else(|_| panic!("{}", case.name));
        assert_eq!(
            args.project_id.to_string(),
            FIXTURE_PROJECT_ID,
            "{}",
            case.name
        );
        assert_eq!(args.limit, case.limit, "{}", case.name);
        assert_eq!(args.since.as_deref(), case.since, "{}", case.name);
        assert_eq!(args.include_undone, case.include_undone, "{}", case.name);
    }
}

#[test]
fn args_unknown_field_rejected_by_deny_unknown_fields() {
    let err = serde_json::from_value::<TimelineHistoryArgs>(json!({
        "project_id": FIXTURE_PROJECT_ID,
        "extra": 1,
    }))
    .expect_err("unknown field must fail");
    let msg = err.to_string();
    assert!(msg.contains("unknown field"), "unexpected error: {msg}");
}

#[test]
fn args_missing_project_id_is_bad_args_through_verb() {
    let prior = empty_project();
    let verb = TimelineHistoryVerb;
    let err = verb
        .compute_patch(&prior, &json!({}))
        .expect_err("missing project_id should fail");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_non_integer_limit_is_bad_args_through_verb() {
    let prior = empty_project();
    let verb = TimelineHistoryVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "limit": "ten",
            }),
        )
        .expect_err("string limit should fail");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_non_boolean_include_undone_is_bad_args_through_verb() {
    let prior = empty_project();
    let verb = TimelineHistoryVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "include_undone": "yes",
            }),
        )
        .expect_err("non-bool include_undone should fail");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn compute_patch_returns_empty_patch_warnings_and_events() {
    let prior = empty_project();
    let (patch, warnings, data) = compute_patch(&prior, &args_default()).expect("should succeed");
    assert_eq!(patch, json!([]));
    assert!(warnings.is_empty());
    assert!(data.events.is_empty());
}

#[test]
fn compute_patch_accepts_since_empty() {
    let prior = empty_project();
    let args = TimelineHistoryArgs {
        project_id: fixture_project_id(),
        limit: None,
        since: Some("empty".to_string()),
        include_undone: None,
    };
    let (patch, warnings, data) = compute_patch(&prior, &args).expect("empty sentinel accepted");
    assert_eq!(patch, json!([]));
    assert!(warnings.is_empty());
    assert_eq!(data, TimelineHistoryData { events: vec![] });
}

#[test]
fn compute_patch_accepts_zero_and_negative_limit_without_runtime_error() {
    let prior = empty_project();
    let cases = vec![0_i64, -1_i64, -25_i64];

    for limit in cases {
        let args = TimelineHistoryArgs {
            project_id: fixture_project_id(),
            limit: Some(limit),
            since: None,
            include_undone: None,
        };
        let (patch, warnings, data) =
            compute_patch(&prior, &args).expect("limit shape should be accepted");
        assert_eq!(patch, json!([]));
        assert!(warnings.is_empty());
        assert_eq!(data.events, Vec::<TimelineHistoryEvent>::new());
    }
}

#[test]
fn event_kind_serializes_to_lowercase_wire_literals() {
    assert_eq!(
        serde_json::to_value(TimelineHistoryEventKind::Apply).expect("apply serialize"),
        json!("apply")
    );
    assert_eq!(
        serde_json::to_value(TimelineHistoryEventKind::Undo).expect("undo serialize"),
        json!("undo")
    );
    assert_eq!(
        serde_json::to_value(TimelineHistoryEventKind::Redo).expect("redo serialize"),
        json!("redo")
    );
}

#[test]
fn future_event_row_serializes_without_parent_event_id_when_none() {
    let row = TimelineHistoryEvent {
        id: "0190b8d3-15e3-7000-bd00-0000e0e0aa01".to_string(),
        verb: "project.rename".to_string(),
        args: json!({ "name": "new-name" }),
        ts: "2026-05-27T10:00:00Z".to_string(),
        kind: TimelineHistoryEventKind::Apply,
        parent_event_id: None,
        effectively_undone: false,
    };
    let value = serde_json::to_value(row).expect("serialize");
    assert_eq!(
        value,
        json!({
            "id":"0190b8d3-15e3-7000-bd00-0000e0e0aa01",
            "verb":"project.rename",
            "args":{"name":"new-name"},
            "ts":"2026-05-27T10:00:00Z",
            "kind":"apply",
            "effectively_undone":false
        })
    );
}

#[test]
fn future_event_row_serializes_with_parent_event_id_when_present() {
    let row = TimelineHistoryEvent {
        id: "0190b8d3-15e3-7000-bd00-0000e0e0aa02".to_string(),
        verb: "timeline.redo".to_string(),
        args: json!({ "steps": 1 }),
        ts: "2026-05-27T10:00:01Z".to_string(),
        kind: TimelineHistoryEventKind::Redo,
        parent_event_id: Some("0190b8d3-15e3-7000-bd00-0000e0e0aa00".to_string()),
        effectively_undone: false,
    };
    let value = serde_json::to_value(row).expect("serialize");
    assert_eq!(
        value,
        json!({
            "id":"0190b8d3-15e3-7000-bd00-0000e0e0aa02",
            "verb":"timeline.redo",
            "args":{"steps":1},
            "ts":"2026-05-27T10:00:01Z",
            "kind":"redo",
            "parent_event_id":"0190b8d3-15e3-7000-bd00-0000e0e0aa00",
            "effectively_undone":false
        })
    );
}

#[test]
fn reconstruct_returns_empty_events_envelope_for_well_formed_args() {
    let verb = TimelineHistoryVerb;
    let prior = empty_project();
    let args = serde_json::to_value(args_default()).expect("args serialize");
    let data = verb
        .reconstruct(&args, &json!([]), &[], &prior)
        .expect("reconstruct should succeed");
    assert_eq!(data, json!({ "events": [] }));
}

#[test]
fn reconstruct_rejects_malformed_args() {
    let verb = TimelineHistoryVerb;
    let prior = empty_project();
    let err = verb
        .reconstruct(&json!({}), &json!([]), &[], &prior)
        .expect_err("malformed args should fail");
    let msg = err.to_string();
    assert!(
        msg.contains("TimelineHistoryArgs") || msg.contains("wrong type"),
        "unexpected reconstruct error: {msg}"
    );
}

#[test]
fn data_envelope_from_args_matches_compute_patch_data() {
    let prior = empty_project();
    let args = args_default();
    let (_, _, expected) = compute_patch(&prior, &args).expect("compute_patch succeeds");
    let actual = data_envelope_from_args(&args, &prior).expect("rebuild succeeds");
    assert_eq!(actual, expected);
}

#[test]
fn default_fixture_validates_with_timeline_history_only_registry() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "timeline.history")
        .expect("default_fixtures includes timeline.history");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(TimelineHistoryVerb))
        .expect("register timeline.history");

    let report = validate_reconstructors(&registry, &[fixture]).expect("validation passes");
    assert_eq!(report.verbs_checked, vec!["timeline.history"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn default_fixture_has_empty_patch_warnings_and_events_expected_data() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "timeline.history")
        .expect("default_fixtures includes timeline.history");

    assert_eq!(fixture.patch, json!([]));
    assert!(fixture.warnings.is_empty());
    assert_eq!(fixture.expected_data, json!({ "events": [] }));
}

#[test]
fn default_registry_contains_timeline_history() {
    let registry = default_registry();
    let verb = registry
        .get("timeline.history")
        .expect("timeline.history must be in default_registry");
    assert_eq!(verb.verb(), "timeline.history");
}

#[test]
fn default_registry_route_returns_empty_history_data() {
    let registry = default_registry();
    let verb = registry
        .get("timeline.history")
        .expect("timeline.history must be in default_registry");
    let prior = empty_project();
    let (patch, data, warnings) = verb
        .compute_patch(&prior, &json!({ "project_id": FIXTURE_PROJECT_ID }))
        .expect("v1 floor succeeds");

    assert!(patch.is_empty());
    assert!(warnings.is_empty());
    let typed: TimelineHistoryData =
        serde_json::from_value(data).expect("envelope deserializes to TimelineHistoryData");
    assert!(typed.events.is_empty());
}

#[cfg(feature = "native")]
#[test]
fn mutate_via_verb_returns_applied_with_empty_history_data() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        empty_project(),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry succeeds");

    let outcome = store
        .mutate_via_verb(
            "timeline.history",
            json!({ "project_id": FIXTURE_PROJECT_ID }),
            None,
        )
        .expect("timeline.history should route");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("expected Applied outcome from timeline.history");
    };
    assert!(warnings.is_empty());

    let data: TimelineHistoryData =
        serde_json::from_value(data).expect("timeline.history data deserializes");
    assert!(data.events.is_empty());
}
