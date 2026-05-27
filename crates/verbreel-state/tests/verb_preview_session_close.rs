//! Tests for `preview.session.close` (§15.5) — seventy-ninth production
//! verb. Opens the preview-session arc (0/6 → 1/6).

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::preview_session_close::compute_patch;
use verbreel_state::{
    MutateOutcome, PreviewSessionCloseArgs, PreviewSessionCloseData, PreviewSessionCloseError,
    PreviewSessionCloseVerb, Project, Verb, VerbError, VerbRegistry, default_fixtures,
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

fn args_default() -> PreviewSessionCloseArgs {
    PreviewSessionCloseArgs {
        project_id: fixture_project_id(),
        session_id: A_VALID_V7.to_string(),
    }
}

// --- args deserialization --------------------------------------------------

#[test]
fn args_deserialize_ok() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "session_id": A_VALID_V7,
    });
    let typed: PreviewSessionCloseArgs =
        serde_json::from_value(raw).expect("well-formed args parse");
    assert_eq!(typed.project_id.to_string(), FIXTURE_PROJECT_ID);
    assert_eq!(typed.session_id, A_VALID_V7);
}

#[test]
fn args_missing_project_id_fails_through_verb() {
    let prior = empty_project();
    let verb = PreviewSessionCloseVerb;

    let err = verb
        .compute_patch(&prior, &json!({ "session_id": A_VALID_V7 }))
        .expect_err("missing project_id should map to BadArgs");

    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_missing_session_id_fails_through_verb() {
    let prior = empty_project();
    let verb = PreviewSessionCloseVerb;

    let err = verb
        .compute_patch(&prior, &json!({ "project_id": FIXTURE_PROJECT_ID }))
        .expect_err("missing session_id should map to BadArgs");

    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_wrong_type_project_id_fails_through_verb() {
    let prior = empty_project();
    let verb = PreviewSessionCloseVerb;

    let err = verb
        .compute_patch(
            &prior,
            &json!({ "project_id": 12345, "session_id": A_VALID_V7 }),
        )
        .expect_err("non-string project_id should map to BadArgs");

    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_wrong_type_session_id_fails_through_verb() {
    let prior = empty_project();
    let verb = PreviewSessionCloseVerb;

    let err = verb
        .compute_patch(
            &prior,
            &json!({ "project_id": FIXTURE_PROJECT_ID, "session_id": 42 }),
        )
        .expect_err("non-string session_id should map to BadArgs");

    assert!(matches!(err, VerbError::BadArgs { .. }));
}

// --- v1 floor: every well-formed call errors -------------------------------

#[test]
fn session_not_found_on_valid_uuid_v7() {
    let prior = empty_project();
    let args = args_default();
    let err = compute_patch(&prior, &args).expect_err("v1 floor: every id misses");
    let PreviewSessionCloseError::SessionNotFound { session_id } = err;
    assert_eq!(session_id, A_VALID_V7);
}

#[test]
fn session_not_found_on_empty_string() {
    let prior = empty_project();
    let args = PreviewSessionCloseArgs {
        project_id: fixture_project_id(),
        session_id: String::new(),
    };
    let err = compute_patch(&prior, &args).expect_err("empty session_id still misses");
    let PreviewSessionCloseError::SessionNotFound { session_id } = err;
    assert_eq!(session_id, "");
}

#[test]
fn error_maps_to_custom_not_bad_args() {
    let prior = empty_project();
    let verb = PreviewSessionCloseVerb;

    let err = verb
        .compute_patch(
            &prior,
            &json!({ "project_id": FIXTURE_PROJECT_ID, "session_id": A_VALID_V7 }),
        )
        .expect_err("v1 floor: verb always errors");

    // Runtime-state error (session miss) must surface as Custom — not
    // BadArgs. BadArgs is reserved for arg-shape failures
    // (validate_command §1.4 relies on this distinction to avoid
    // mis-reporting well-formed args as invalid).
    assert!(
        matches!(err, VerbError::Custom(_)),
        "expected VerbError::Custom, got {err:?}",
    );
}

#[test]
fn error_detail_contains_e_preview_session_not_found_code() {
    let prior = empty_project();
    let verb = PreviewSessionCloseVerb;

    let err = verb
        .compute_patch(
            &prior,
            &json!({ "project_id": FIXTURE_PROJECT_ID, "session_id": A_VALID_V7 }),
        )
        .expect_err("v1 floor: verb always errors");

    let VerbError::Custom(detail) = err else {
        panic!("expected Custom, got {err:?}");
    };
    assert!(
        detail.contains("E_PREVIEW_SESSION_NOT_FOUND"),
        "detail `{detail}` should mention E_PREVIEW_SESSION_NOT_FOUND",
    );
}

#[test]
fn error_detail_contains_queried_session_id() {
    let prior = empty_project();
    let weird_id = "preview-session-xyz-789";
    let args = PreviewSessionCloseArgs {
        project_id: fixture_project_id(),
        session_id: weird_id.to_string(),
    };
    let err = compute_patch(&prior, &args).expect_err("v1 floor errors");
    let msg = err.to_string();
    assert!(
        msg.contains(weird_id),
        "error message `{msg}` should mention session_id `{weird_id}`",
    );
}

#[test]
fn verb_is_project_agnostic_on_error_path() {
    let prior_a = empty_project();
    let mut prior_b = empty_project();
    prior_b.name = "different-name".to_string();
    prior_b.duration_tk = verbreel_types::Tick::new(987_654);

    let args = args_default();
    let err_a = compute_patch(&prior_a, &args).expect_err("a errors");
    let err_b = compute_patch(&prior_b, &args).expect_err("b errors");

    assert_eq!(err_a, err_b);
}

// --- empty patch / warnings even on error path -----------------------------

#[test]
fn patch_is_empty_via_verb_trait_when_compute_patch_errors() {
    // The verb trait returns Result<(Patch, data, warnings), VerbError>.
    // On the error path the Ok tuple — including any prospective patch —
    // is structurally unreachable: an Err return carries no patch payload
    // at all. We assert both the Err discriminant and that the Result
    // contains no Ok value (so any future refactor that started building
    // a patch before erroring would have to change the return shape and
    // would fail this match).
    let verb = PreviewSessionCloseVerb;
    let prior = empty_project();
    let result = verb.compute_patch(
        &prior,
        &json!({ "project_id": FIXTURE_PROJECT_ID, "session_id": A_VALID_V7 }),
    );
    let Err(err) = result else {
        panic!("v1 floor must error; got Ok with patch payload");
    };
    let VerbError::Custom(detail) = err else {
        panic!("expected Custom, got {err:?}");
    };
    assert!(detail.contains("E_PREVIEW_SESSION_NOT_FOUND"));
}

#[test]
fn warnings_are_empty_on_error_path() {
    // compute_patch returns Result<(Value, Vec<Value>, Value),
    // PreviewSessionCloseError>. Err structurally carries no warnings
    // vector, so the warnings collection is unreachable on the error
    // path. We assert the discriminant explicitly so any future refactor
    // that tried to attach warnings to the error variant (e.g. by
    // changing PreviewSessionCloseError to carry a Vec<Value>) would
    // have to change PreviewSessionCloseError's shape and break this
    // test.
    let prior = empty_project();
    let result = compute_patch(&prior, &args_default());
    let Err(err) = result else {
        panic!("v1 floor must error; Ok branch would carry warnings");
    };
    // The error variant carries only `session_id` — no warnings field.
    let PreviewSessionCloseError::SessionNotFound { session_id } = err;
    assert_eq!(session_id, A_VALID_V7);
}

// --- reconstructor / fixture -----------------------------------------------

#[test]
fn reconstruct_returns_null_for_args_deserialize_round_trip() {
    let verb = PreviewSessionCloseVerb;
    let prior = empty_project();
    let args = serde_json::to_value(args_default()).expect("args → Value");
    let data = verb
        .reconstruct(&args, &json!([]), &[], &prior)
        .expect("reconstruct succeeds for well-formed args");
    assert_eq!(data, Value::Null);
}

#[test]
fn reconstruct_rejects_malformed_args() {
    let verb = PreviewSessionCloseVerb;
    let prior = empty_project();
    let err = verb
        .reconstruct(&json!({}), &json!([]), &[], &prior)
        .expect_err("malformed args should fail reconstruction");
    let s = err.to_string();
    assert!(
        s.contains("PreviewSessionCloseArgs") || s.contains("wrong type"),
        "unexpected reconstruct error: {s}",
    );
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "preview.session.close")
        .expect("default_fixtures includes preview.session.close");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(PreviewSessionCloseVerb))
        .expect("register preview.session.close verb");

    let report = validate_reconstructors(&registry, &[fixture])
        .expect("preview.session.close reconstruct from fixture");
    assert_eq!(report.verbs_checked, vec!["preview.session.close"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn verb_trait_surface_lookup_via_default_registry() {
    let registry = default_registry();
    let verb = registry
        .get("preview.session.close")
        .expect("preview.session.close registered in default_registry");

    let prior = empty_project();
    let err = verb
        .compute_patch(
            &prior,
            &json!({ "project_id": FIXTURE_PROJECT_ID, "session_id": A_VALID_V7 }),
        )
        .expect_err("v1 floor: always errors");

    let VerbError::Custom(detail) = err else {
        panic!("expected Custom, got {err:?}");
    };
    assert!(detail.contains("E_PREVIEW_SESSION_NOT_FOUND"));
}

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb_and_errors() {
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
        "preview.session.close",
        json!({ "project_id": FIXTURE_PROJECT_ID, "session_id": A_VALID_V7 }),
        None,
    );

    match outcome {
        Err(_) => {}
        Ok(MutateOutcome::Applied { .. }) => {
            panic!("expected mutate_via_verb to error in v1 floor, got Applied")
        }
        Ok(other) => panic!("expected Err for v1 floor, got Ok({other:?})"),
    }
}

// --- data-shape sanity -----------------------------------------------------

#[test]
fn data_shape_has_exactly_four_keys() {
    // All 4 fields are required (no Option), so a constructed
    // PreviewSessionCloseData must serialize to exactly 4 keys. Any
    // future refactor adding an Option field would break this — that's
    // the point: any new field has to be a deliberate spec change, not
    // a quiet drift.
    let data = PreviewSessionCloseData {
        closed: true,
        forced: false,
        final_at_tk: 120_000,
        dropped_frames: 3,
    };
    let v = serde_json::to_value(&data).expect("data serializes");
    let obj = v.as_object().expect("object");
    assert_eq!(
        obj.len(),
        4,
        "PreviewSessionCloseData must have exactly {{closed, forced, final_at_tk, dropped_frames}}",
    );
    assert_eq!(obj.get("closed"), Some(&json!(true)));
    assert_eq!(obj.get("forced"), Some(&json!(false)));
    assert_eq!(obj.get("final_at_tk"), Some(&json!(120_000)));
    assert_eq!(obj.get("dropped_frames"), Some(&json!(3)));
}

#[test]
fn data_shape_forced_termination_serializes() {
    // Forced-termination shape: worker did not respond to cooperative
    // cancel within timeout. final_at_tk reflects where the playhead
    // was when the engine forcibly tore the worker down.
    let data = PreviewSessionCloseData {
        closed: true,
        forced: true,
        final_at_tk: 0,
        dropped_frames: 0,
    };
    let v = serde_json::to_value(&data).expect("data serializes");
    let obj = v.as_object().expect("object");
    assert_eq!(obj.len(), 4);
    assert_eq!(obj.get("forced"), Some(&json!(true)));
    assert_eq!(obj.get("final_at_tk"), Some(&json!(0)));
}

/// v1 floor happy-path deferral marker.
///
/// In v1 no `preview.session.create` verb exists yet, so no preview
/// session is ever in flight and `preview.session.close` cannot
/// resolve any id. The happy path — returning a populated
/// `PreviewSessionCloseData` reflecting actual cooperative-cancel
/// teardown, drop-frame accounting since the most recent
/// `preview.play`, and the final playhead position — lights up when
/// the preview-session worker integration and a `VerbContext` plumb
/// session-state polling into `compute_patch`. This test is
/// intentionally a no-op so the deferral is named in the test surface
/// rather than in a hidden TODO.
#[test]
fn happy_path_unreachable_in_v1_floor() {
    // No assertions: this test exists to document the deferral.
}
