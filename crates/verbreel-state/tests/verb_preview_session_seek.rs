//! Tests for `preview.session.seek` (§15.2) — v1 session not-found floor.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::preview_session_seek::compute_patch;
use verbreel_state::{
    PreviewSessionSeekArgs, PreviewSessionSeekData, PreviewSessionSeekError,
    PreviewSessionSeekVerb, Project, Verb, VerbError, VerbRegistry, default_fixtures,
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

fn args_default() -> PreviewSessionSeekArgs {
    PreviewSessionSeekArgs {
        project_id: fixture_project_id(),
        session_id: A_VALID_SESSION_ID.to_string(),
        at_tk: 0,
    }
}

#[test]
fn args_deserialize_ok_with_required_fields() {
    let typed: PreviewSessionSeekArgs = serde_json::from_value(json!({
        "project_id": FIXTURE_PROJECT_ID,
        "session_id": A_VALID_SESSION_ID,
        "at_tk": 1234
    }))
    .expect("args parse");
    assert_eq!(typed.project_id.to_string(), FIXTURE_PROJECT_ID);
    assert_eq!(typed.session_id, A_VALID_SESSION_ID);
    assert_eq!(typed.at_tk, 1234);
}

#[test]
fn unknown_field_rejected_through_verb() {
    let prior = empty_project();
    let verb = PreviewSessionSeekVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "session_id": A_VALID_SESSION_ID,
                "at_tk": 0,
                "extra": true
            }),
        )
        .expect_err("deny_unknown_fields rejects extras");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn missing_required_fields_fail_through_verb() {
    let prior = empty_project();
    let verb = PreviewSessionSeekVerb;
    let cases = [
        json!({ "session_id": A_VALID_SESSION_ID, "at_tk": 0 }),
        json!({ "project_id": FIXTURE_PROJECT_ID, "at_tk": 0 }),
        json!({ "project_id": FIXTURE_PROJECT_ID, "session_id": A_VALID_SESSION_ID }),
    ];

    for raw in cases {
        let err = verb
            .compute_patch(&prior, &raw)
            .expect_err("missing required field should map to BadArgs");
        assert!(matches!(err, VerbError::BadArgs { .. }));
    }
}

#[test]
fn non_integer_at_tk_fails_through_verb() {
    let prior = empty_project();
    let verb = PreviewSessionSeekVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "session_id": A_VALID_SESSION_ID,
                "at_tk": "zero"
            }),
        )
        .expect_err("non-integer at_tk should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn non_string_project_id_fails_through_verb() {
    let prior = empty_project();
    let verb = PreviewSessionSeekVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": 1234,
                "session_id": A_VALID_SESSION_ID,
                "at_tk": 0
            }),
        )
        .expect_err("non-string project_id should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn non_string_session_id_fails_through_verb() {
    let prior = empty_project();
    let verb = PreviewSessionSeekVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "session_id": 42,
                "at_tk": 0
            }),
        )
        .expect_err("non-string session_id should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn negative_at_tk_returns_bad_time() {
    let prior = empty_project();
    let err = compute_patch(
        &prior,
        &PreviewSessionSeekArgs {
            at_tk: -1,
            ..args_default()
        },
    )
    .expect_err("negative at_tk should fail");
    let PreviewSessionSeekError::BadTime { at_tk } = err else {
        panic!("expected BadTime, got {err:?}");
    };
    assert_eq!(at_tk, -1);
}

#[test]
fn negative_at_tk_maps_to_bad_args_through_verb() {
    let prior = empty_project();
    let verb = PreviewSessionSeekVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "session_id": A_VALID_SESSION_ID,
                "at_tk": -1
            }),
        )
        .expect_err("negative at_tk should map to BadArgs");
    let VerbError::BadArgs { detail } = err else {
        panic!("expected BadArgs, got {err:?}");
    };
    assert!(detail.contains("E_BAD_TIME"));
}

#[test]
fn non_negative_at_tk_reaches_session_not_found_floor() {
    let prior = empty_project();
    for at_tk in [0_i64, 1_i64, 240_000_i64, i64::MAX / 2] {
        let err = compute_patch(
            &prior,
            &PreviewSessionSeekArgs {
                at_tk,
                ..args_default()
            },
        )
        .expect_err("v1 floor should always miss session");
        let PreviewSessionSeekError::SessionNotFound { session_id } = err else {
            panic!("expected SessionNotFound, got {err:?}");
        };
        assert_eq!(session_id, A_VALID_SESSION_ID);
    }
}

#[test]
fn runtime_session_not_found_maps_to_custom_and_includes_session_id() {
    let prior = empty_project();
    let verb = PreviewSessionSeekVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "session_id": A_VALID_SESSION_ID,
                "at_tk": 123
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
    let missing_id = "preview-session-abc-123";
    let err = compute_patch(
        &prior,
        &PreviewSessionSeekArgs {
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
fn future_success_data_shape_serializes_all_fields() {
    let data = PreviewSessionSeekData {
        path: "cache/frames/f.png".to_string(),
        sha256: "deadbeef".to_string(),
        width: 1920,
        height: 1080,
        cache_hit: true,
        at_tk: 240_000,
    };
    let value = serde_json::to_value(&data).expect("data serializes");
    let obj = value.as_object().expect("data object");
    assert_eq!(obj.len(), 6);
    assert_eq!(obj.get("path"), Some(&json!("cache/frames/f.png")));
    assert_eq!(obj.get("sha256"), Some(&json!("deadbeef")));
    assert_eq!(obj.get("width"), Some(&json!(1920)));
    assert_eq!(obj.get("height"), Some(&json!(1080)));
    assert_eq!(obj.get("cache_hit"), Some(&json!(true)));
    assert_eq!(obj.get("at_tk"), Some(&json!(240_000)));
}

#[test]
fn reserved_error_variants_display_expected_codes() {
    let not_found = PreviewSessionSeekError::ProjectNotFound {
        project_id: FIXTURE_PROJECT_ID.to_string(),
    }
    .to_string();
    assert!(not_found.contains("E_PROJECT_NOT_FOUND"));

    let decoder_failed = PreviewSessionSeekError::DecoderFailed {
        session_id: A_VALID_SESSION_ID.to_string(),
        decoder_error: "decode died".to_string(),
    }
    .to_string();
    assert!(decoder_failed.contains("E_PREVIEW_SESSION_DECODER_FAILED"));
}

#[test]
fn reconstruct_returns_null_for_well_formed_args() {
    let verb = PreviewSessionSeekVerb;
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
    let verb = PreviewSessionSeekVerb;
    let prior = empty_project();
    let err = verb
        .reconstruct(&json!({}), &json!([]), &[], &prior)
        .expect_err("malformed args should fail reconstruction");
    let msg = err.to_string();
    assert!(
        msg.contains("PreviewSessionSeekArgs") || msg.contains("wrong type"),
        "unexpected reconstruct error: {msg}",
    );
}

#[test]
fn default_fixture_validates_with_only_preview_session_seek_verb() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "preview.session.seek")
        .expect("default_fixtures includes preview.session.seek");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(PreviewSessionSeekVerb))
        .expect("register preview.session.seek verb");

    let report = validate_reconstructors(&registry, &[fixture]).expect("fixture validates");
    assert_eq!(report.verbs_checked, vec!["preview.session.seek"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn default_fixture_has_empty_patch_warnings_and_null_expected_data() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "preview.session.seek")
        .expect("default_fixtures includes preview.session.seek");
    assert_eq!(fixture.patch, json!([]));
    assert!(fixture.warnings.is_empty());
    assert_eq!(fixture.expected_data, Value::Null);
}

#[test]
fn default_registry_contains_preview_session_seek() {
    let registry = default_registry();
    let verb = registry
        .get("preview.session.seek")
        .expect("preview.session.seek in default_registry");
    let prior = empty_project();
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "session_id": A_VALID_SESSION_ID,
                "at_tk": 0
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
        "preview.session.seek",
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "session_id": A_VALID_SESSION_ID,
            "at_tk": 0
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
