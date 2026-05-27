//! Tests for `preview.session.pause` (§15.4) — v1 session not-found floor.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::preview_session_pause::compute_patch;
use verbreel_state::{
    PreviewSessionPauseArgs, PreviewSessionPauseData, PreviewSessionPauseError,
    PreviewSessionPauseVerb, Project, Verb, VerbError, VerbRegistry, default_fixtures,
    default_registry, validate_reconstructors,
};

#[cfg(feature = "native")]
use verbreel_state::{LifecycleError, ProjectStore};

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const A_VALID_SESSION_ID: &str = "0190b8d3-15e3-7000-bd00-0000feedbeef";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture -> Project")
}

fn fixture_project_id() -> verbreel_types::ProjectId {
    empty_project().id
}

fn args_default() -> PreviewSessionPauseArgs {
    PreviewSessionPauseArgs {
        project_id: fixture_project_id(),
        session_id: A_VALID_SESSION_ID.to_string(),
    }
}

#[test]
fn args_deserialize_ok_with_required_fields() {
    let typed: PreviewSessionPauseArgs = serde_json::from_value(json!({
        "project_id": FIXTURE_PROJECT_ID,
        "session_id": A_VALID_SESSION_ID,
    }))
    .expect("args parse");
    assert_eq!(typed.project_id.to_string(), FIXTURE_PROJECT_ID);
    assert_eq!(typed.session_id, A_VALID_SESSION_ID);
}

#[test]
fn unknown_field_rejected_through_verb() {
    let prior = empty_project();
    let verb = PreviewSessionPauseVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "session_id": A_VALID_SESSION_ID,
                "extra": true
            }),
        )
        .expect_err("deny_unknown_fields rejects extras");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn missing_required_fields_fail_through_verb() {
    let prior = empty_project();
    let verb = PreviewSessionPauseVerb;
    let cases = [
        json!({ "session_id": A_VALID_SESSION_ID }),
        json!({ "project_id": FIXTURE_PROJECT_ID }),
    ];

    for raw in cases {
        let err = verb
            .compute_patch(&prior, &raw)
            .expect_err("missing required field should map to BadArgs");
        assert!(matches!(err, VerbError::BadArgs { .. }));
    }
}

#[test]
fn non_string_project_id_fails_through_verb() {
    let prior = empty_project();
    let verb = PreviewSessionPauseVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": 1234,
                "session_id": A_VALID_SESSION_ID
            }),
        )
        .expect_err("non-string project_id should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn non_string_session_id_fails_through_verb() {
    let prior = empty_project();
    let verb = PreviewSessionPauseVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "session_id": 42
            }),
        )
        .expect_err("non-string session_id should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn valid_args_always_reach_session_not_found_floor() {
    let prior = empty_project();
    for session_id in [A_VALID_SESSION_ID, "", "preview-session-abc"] {
        let err = compute_patch(
            &prior,
            &PreviewSessionPauseArgs {
                session_id: session_id.to_string(),
                ..args_default()
            },
        )
        .expect_err("v1 floor should always miss session");
        let PreviewSessionPauseError::SessionNotFound { session_id: id } = err else {
            panic!("expected SessionNotFound, got {err:?}");
        };
        assert_eq!(id, session_id);
    }
}

#[test]
fn compute_patch_is_project_agnostic_on_error_path() {
    let prior_a = empty_project();
    let mut prior_b = empty_project();
    prior_b.name = "other".to_string();
    prior_b.duration_tk = verbreel_types::Tick::new(987_654);

    let args = args_default();
    let err_a = compute_patch(&prior_a, &args).expect_err("a errors");
    let err_b = compute_patch(&prior_b, &args).expect_err("b errors");
    assert_eq!(err_a, err_b);
}

#[test]
fn runtime_session_not_found_maps_to_custom_and_includes_session_id() {
    let prior = empty_project();
    let verb = PreviewSessionPauseVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "session_id": A_VALID_SESSION_ID
            }),
        )
        .expect_err("well-formed args should hit runtime floor");
    let VerbError::Custom(detail) = err else {
        panic!("expected Custom, got {err:?}");
    };
    assert!(detail.contains("E_PREVIEW_SESSION_NOT_FOUND"));
    assert!(detail.contains(A_VALID_SESSION_ID));
}

#[test]
fn session_not_found_error_detail_contains_code_and_id() {
    let prior = empty_project();
    let missing_id = "preview-session-xyz-789";
    let err = compute_patch(
        &prior,
        &PreviewSessionPauseArgs {
            session_id: missing_id.to_string(),
            ..args_default()
        },
    )
    .expect_err("v1 floor should miss every id");
    let msg = err.to_string();
    assert!(msg.contains("E_PREVIEW_SESSION_NOT_FOUND"));
    assert!(msg.contains(missing_id));
}

#[test]
fn future_success_data_shape_serializes_was_playing_and_at_tk() {
    let data = PreviewSessionPauseData {
        was_playing: false,
        at_tk: 240_000,
    };
    let value = serde_json::to_value(&data).expect("data serializes");
    let obj = value.as_object().expect("data object");
    assert_eq!(obj.len(), 2);
    assert_eq!(obj.get("was_playing"), Some(&json!(false)));
    assert_eq!(obj.get("at_tk"), Some(&json!(240_000)));
}

#[test]
fn reserved_error_variant_displays_expected_code() {
    let not_found = PreviewSessionPauseError::ProjectNotFound {
        project_id: FIXTURE_PROJECT_ID.to_string(),
    }
    .to_string();
    assert!(not_found.contains("E_PROJECT_NOT_FOUND"));
}

#[test]
fn reconstruct_returns_null_for_well_formed_args() {
    let verb = PreviewSessionPauseVerb;
    let prior = empty_project();
    let data = verb
        .reconstruct(
            &serde_json::to_value(args_default()).expect("args serialize"),
            &json!([]),
            &[],
            &prior,
        )
        .expect("reconstruct succeeds");
    assert_eq!(data, Value::Null);
}

#[test]
fn reconstruct_rejects_malformed_args() {
    let verb = PreviewSessionPauseVerb;
    let prior = empty_project();
    let err = verb
        .reconstruct(&json!({}), &json!([]), &[], &prior)
        .expect_err("malformed args should fail reconstruction");
    let msg = err.to_string();
    assert!(
        msg.contains("PreviewSessionPauseArgs") || msg.contains("wrong type"),
        "unexpected reconstruct error: {msg}",
    );
}

#[test]
fn default_fixture_validates_with_only_preview_session_pause_verb() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "preview.session.pause")
        .expect("default_fixtures includes preview.session.pause");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(PreviewSessionPauseVerb))
        .expect("register preview.session.pause verb");

    let report = validate_reconstructors(&registry, &[fixture]).expect("fixture validates");
    assert_eq!(report.verbs_checked, vec!["preview.session.pause"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn default_fixture_has_empty_patch_warnings_and_null_expected_data() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "preview.session.pause")
        .expect("default_fixtures includes preview.session.pause");
    assert_eq!(fixture.patch, json!([]));
    assert!(fixture.warnings.is_empty());
    assert_eq!(fixture.expected_data, Value::Null);
}

#[test]
fn default_registry_contains_preview_session_pause() {
    let registry = default_registry();
    let verb = registry
        .get("preview.session.pause")
        .expect("preview.session.pause in default_registry");
    let prior = empty_project();
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "session_id": A_VALID_SESSION_ID
            }),
        )
        .expect_err("v1 floor always errors");
    let VerbError::Custom(detail) = err else {
        panic!("expected Custom, got {err:?}");
    };
    assert!(detail.contains("E_PREVIEW_SESSION_NOT_FOUND"));
}

#[cfg(feature = "native")]
#[test]
fn mutate_via_verb_route_returns_runtime_session_not_found_floor() {
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
        "preview.session.pause",
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "session_id": A_VALID_SESSION_ID
        }),
        None,
    );
    let Err(LifecycleError::VerbExecutionFailed { source, .. }) = outcome else {
        panic!("expected VerbExecutionFailed, got {outcome:?}");
    };
    let VerbError::Custom(detail) = source else {
        panic!("expected Custom source, got {source:?}");
    };
    assert!(detail.contains("E_PREVIEW_SESSION_NOT_FOUND"));
}
