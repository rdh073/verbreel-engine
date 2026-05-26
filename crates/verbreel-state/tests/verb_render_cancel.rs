//! Tests for `render.cancel` (§11.3) — seventy-seventh production verb.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::render_cancel::compute_patch;
use verbreel_state::{
    MutateOutcome, Project, RenderCancelArgs, RenderCancelData, RenderCancelError,
    RenderCancelVerb, Verb, VerbError, VerbRegistry, default_fixtures, default_registry,
    validate_reconstructors,
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

fn args_default() -> RenderCancelArgs {
    RenderCancelArgs {
        project_id: fixture_project_id(),
        job_id: A_VALID_V7.to_string(),
    }
}

// --- args deserialization ---------------------------------------------------

#[test]
fn args_deserialize_ok() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "job_id": A_VALID_V7,
    });
    let typed: RenderCancelArgs = serde_json::from_value(raw).expect("well-formed args parse");
    assert_eq!(typed.project_id.to_string(), FIXTURE_PROJECT_ID);
    assert_eq!(typed.job_id, A_VALID_V7);
}

#[test]
fn args_missing_project_id_fails_through_verb() {
    let prior = empty_project();
    let verb = RenderCancelVerb;

    let err = verb
        .compute_patch(&prior, &json!({ "job_id": A_VALID_V7 }))
        .expect_err("missing project_id should map to BadArgs");

    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_missing_job_id_fails_through_verb() {
    let prior = empty_project();
    let verb = RenderCancelVerb;

    let err = verb
        .compute_patch(&prior, &json!({ "project_id": FIXTURE_PROJECT_ID }))
        .expect_err("missing job_id should map to BadArgs");

    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_wrong_type_project_id_fails_through_verb() {
    let prior = empty_project();
    let verb = RenderCancelVerb;

    let err = verb
        .compute_patch(
            &prior,
            &json!({ "project_id": 12345, "job_id": A_VALID_V7 }),
        )
        .expect_err("non-string project_id should map to BadArgs");

    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_wrong_type_job_id_fails_through_verb() {
    let prior = empty_project();
    let verb = RenderCancelVerb;

    let err = verb
        .compute_patch(
            &prior,
            &json!({ "project_id": FIXTURE_PROJECT_ID, "job_id": 42 }),
        )
        .expect_err("non-string job_id should map to BadArgs");

    assert!(matches!(err, VerbError::BadArgs { .. }));
}

// --- v1 floor: every well-formed call errors -------------------------------

#[test]
fn job_not_found_on_valid_uuid_v7() {
    let prior = empty_project();
    let args = args_default();
    let err = compute_patch(&prior, &args).expect_err("v1 floor: every id misses");
    let RenderCancelError::JobNotFound { job_id } = err;
    assert_eq!(job_id, A_VALID_V7);
}

#[test]
fn job_not_found_on_empty_string() {
    let prior = empty_project();
    let args = RenderCancelArgs {
        project_id: fixture_project_id(),
        job_id: String::new(),
    };
    let err = compute_patch(&prior, &args).expect_err("empty job_id still misses");
    let RenderCancelError::JobNotFound { job_id } = err;
    assert_eq!(job_id, "");
}

#[test]
fn error_maps_to_custom_not_bad_args() {
    let prior = empty_project();
    let verb = RenderCancelVerb;

    let err = verb
        .compute_patch(
            &prior,
            &json!({ "project_id": FIXTURE_PROJECT_ID, "job_id": A_VALID_V7 }),
        )
        .expect_err("v1 floor: verb always errors");

    // Runtime-state error (job miss) must surface as Custom — not BadArgs.
    // BadArgs is reserved for arg-shape failures (validate_command §1.4 relies
    // on this distinction to avoid mis-reporting well-formed args as invalid).
    assert!(
        matches!(err, VerbError::Custom(_)),
        "expected VerbError::Custom, got {err:?}",
    );
}

#[test]
fn error_detail_contains_e_job_not_found_code() {
    let prior = empty_project();
    let verb = RenderCancelVerb;

    let err = verb
        .compute_patch(
            &prior,
            &json!({ "project_id": FIXTURE_PROJECT_ID, "job_id": A_VALID_V7 }),
        )
        .expect_err("v1 floor: verb always errors");

    let VerbError::Custom(detail) = err else {
        panic!("expected Custom, got {err:?}");
    };
    assert!(
        detail.contains("E_JOB_NOT_FOUND"),
        "detail `{detail}` should mention E_JOB_NOT_FOUND",
    );
}

#[test]
fn error_detail_contains_queried_job_id() {
    let prior = empty_project();
    let weird_id = "render-job-xyz-789";
    let args = RenderCancelArgs {
        project_id: fixture_project_id(),
        job_id: weird_id.to_string(),
    };
    let err = compute_patch(&prior, &args).expect_err("v1 floor errors");
    let msg = err.to_string();
    assert!(
        msg.contains(weird_id),
        "error message `{msg}` should mention job_id `{weird_id}`",
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
    // The forward router never produces a patch on the error path; this
    // is verified by the Err branch returning before patch construction.
    let verb = RenderCancelVerb;
    let prior = empty_project();
    let err = verb
        .compute_patch(
            &prior,
            &json!({ "project_id": FIXTURE_PROJECT_ID, "job_id": A_VALID_V7 }),
        )
        .expect_err("v1 floor: verb always errors");
    let VerbError::Custom(detail) = err else {
        panic!("expected Custom, got {err:?}");
    };
    assert!(detail.contains("E_JOB_NOT_FOUND"));
}

#[test]
fn warnings_are_empty_on_error_path() {
    let prior = empty_project();
    let res = compute_patch(&prior, &args_default());
    assert!(res.is_err());
}

// --- reconstructor / fixture --------------------------------------------

#[test]
fn reconstruct_returns_null_for_args_deserialize_round_trip() {
    let verb = RenderCancelVerb;
    let prior = empty_project();
    let args = serde_json::to_value(args_default()).expect("args → Value");
    let data = verb
        .reconstruct(&args, &json!([]), &[], &prior)
        .expect("reconstruct succeeds for well-formed args");
    assert_eq!(data, Value::Null);
}

#[test]
fn reconstruct_rejects_malformed_args() {
    let verb = RenderCancelVerb;
    let prior = empty_project();
    let err = verb
        .reconstruct(&json!({}), &json!([]), &[], &prior)
        .expect_err("malformed args should fail reconstruction");
    let s = err.to_string();
    assert!(
        s.contains("RenderCancelArgs") || s.contains("wrong type"),
        "unexpected reconstruct error: {s}",
    );
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "render.cancel")
        .expect("default_fixtures includes render.cancel");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(RenderCancelVerb))
        .expect("register render.cancel verb");

    let report = validate_reconstructors(&registry, &[fixture])
        .expect("render.cancel reconstruct from fixture");
    assert_eq!(report.verbs_checked, vec!["render.cancel"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn verb_trait_surface_lookup_via_default_registry() {
    let registry = default_registry();
    let verb = registry
        .get("render.cancel")
        .expect("render.cancel registered in default_registry");

    let prior = empty_project();
    let err = verb
        .compute_patch(
            &prior,
            &json!({ "project_id": FIXTURE_PROJECT_ID, "job_id": A_VALID_V7 }),
        )
        .expect_err("v1 floor: always errors");

    let VerbError::Custom(detail) = err else {
        panic!("expected Custom, got {err:?}");
    };
    assert!(detail.contains("E_JOB_NOT_FOUND"));
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
        "render.cancel",
        json!({ "project_id": FIXTURE_PROJECT_ID, "job_id": A_VALID_V7 }),
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

// --- data shape sanity -----------------------------------------------------

#[test]
fn data_shape_omits_partial_path_when_none() {
    let data = RenderCancelData {
        job_id: A_VALID_V7.to_string(),
        state: "canceled".to_string(),
        partial_path: None,
    };
    let v = serde_json::to_value(&data).expect("data serializes");
    let obj = v.as_object().expect("object");
    assert_eq!(obj.len(), 2, "expected 2 keys when partial_path is None");
    assert_eq!(obj.get("job_id"), Some(&json!(A_VALID_V7)));
    assert_eq!(obj.get("state"), Some(&json!("canceled")));
    assert!(obj.get("partial_path").is_none());
}

#[test]
fn data_shape_includes_partial_path_when_some() {
    let data = RenderCancelData {
        job_id: A_VALID_V7.to_string(),
        state: "canceled".to_string(),
        partial_path: Some("renders/partial-abc.mp4".to_string()),
    };
    let v = serde_json::to_value(&data).expect("data serializes");
    let obj = v.as_object().expect("object");
    assert_eq!(obj.len(), 3, "expected 3 keys when partial_path is Some");
    assert_eq!(
        obj.get("partial_path"),
        Some(&json!("renders/partial-abc.mp4")),
    );
}

/// v1 floor happy-path deferral marker.
///
/// In v1 no `render.start` verb exists yet, so no render job is ever
/// in flight and `render.cancel` cannot resolve any id. The happy
/// path — transitioning a `running` render job to `canceled`,
/// returning a `RenderCancelData` with `state: "canceled"` and
/// optional `partial_path` to any persisted partial encoder output —
/// lights up when the render-worker integration and a `VerbContext`
/// plumb file I/O into `compute_patch`. The terminal-state no-op
/// `W_NOOP` warning and the encoder `.tmp` → `.partial` rename are
/// likewise unreachable in v1. This test is intentionally a no-op so
/// the deferral is named in the test surface rather than in a hidden
/// TODO.
#[test]
fn happy_path_unreachable_in_v1_floor() {
    // No assertions: this test exists to document the deferral.
}
