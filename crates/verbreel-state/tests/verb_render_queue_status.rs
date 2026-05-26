//! Tests for `render.queue.status` (§21.3) — seventy-fourth production verb.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::render_queue_status::compute_patch;
use verbreel_state::{
    MutateOutcome, Project, RenderQueueStatusArgs, RenderQueueStatusError, RenderQueueStatusVerb,
    Verb, VerbError, VerbRegistry, default_fixtures, default_registry, validate_reconstructors,
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

fn args_default() -> RenderQueueStatusArgs {
    RenderQueueStatusArgs {
        project_id: fixture_project_id(),
        queue_job_id: A_VALID_V7.to_string(),
    }
}

// --- args deserialization ---------------------------------------------------

#[test]
fn args_deserialize_ok() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "queue_job_id": A_VALID_V7,
    });
    let typed: RenderQueueStatusArgs = serde_json::from_value(raw).expect("well-formed args parse");
    assert_eq!(typed.project_id.to_string(), FIXTURE_PROJECT_ID);
    assert_eq!(typed.queue_job_id, A_VALID_V7);
}

#[test]
fn args_missing_project_id_fails_through_verb() {
    let prior = empty_project();
    let verb = RenderQueueStatusVerb;

    let err = verb
        .compute_patch(&prior, &json!({ "queue_job_id": A_VALID_V7 }))
        .expect_err("missing project_id should map to BadArgs");

    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_missing_queue_job_id_fails_through_verb() {
    let prior = empty_project();
    let verb = RenderQueueStatusVerb;

    let err = verb
        .compute_patch(&prior, &json!({ "project_id": FIXTURE_PROJECT_ID }))
        .expect_err("missing queue_job_id should map to BadArgs");

    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_wrong_type_project_id_fails_through_verb() {
    let prior = empty_project();
    let verb = RenderQueueStatusVerb;

    let err = verb
        .compute_patch(
            &prior,
            &json!({ "project_id": 12345, "queue_job_id": A_VALID_V7 }),
        )
        .expect_err("non-string project_id should map to BadArgs");

    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_wrong_type_queue_job_id_fails_through_verb() {
    let prior = empty_project();
    let verb = RenderQueueStatusVerb;

    let err = verb
        .compute_patch(
            &prior,
            &json!({ "project_id": FIXTURE_PROJECT_ID, "queue_job_id": 42 }),
        )
        .expect_err("non-string queue_job_id should map to BadArgs");

    assert!(matches!(err, VerbError::BadArgs { .. }));
}

// --- v1 floor: every well-formed call errors -------------------------------

#[test]
fn queue_job_not_found_on_valid_uuid_v7() {
    let prior = empty_project();
    let args = args_default();
    let err = compute_patch(&prior, &args).expect_err("v1 floor: every id misses");
    let RenderQueueStatusError::QueueJobNotFound { queue_job_id } = err;
    assert_eq!(queue_job_id, A_VALID_V7);
}

#[test]
fn queue_job_not_found_on_empty_string() {
    let prior = empty_project();
    let args = RenderQueueStatusArgs {
        project_id: fixture_project_id(),
        queue_job_id: String::new(),
    };
    let err = compute_patch(&prior, &args).expect_err("empty queue_job_id still misses");
    let RenderQueueStatusError::QueueJobNotFound { queue_job_id } = err;
    assert_eq!(queue_job_id, "");
}

#[test]
fn queue_job_not_found_on_arbitrary_string() {
    let prior = empty_project();
    let args = RenderQueueStatusArgs {
        project_id: fixture_project_id(),
        queue_job_id: "not-a-uuid-at-all".to_string(),
    };
    let err = compute_patch(&prior, &args).expect_err("arbitrary id still misses");
    let RenderQueueStatusError::QueueJobNotFound { queue_job_id } = err;
    assert_eq!(queue_job_id, "not-a-uuid-at-all");
}

#[test]
fn error_details_queue_job_id_matches_input() {
    let prior = empty_project();
    let weird_id = "queued-job-xyz-123";
    let args = RenderQueueStatusArgs {
        project_id: fixture_project_id(),
        queue_job_id: weird_id.to_string(),
    };
    let err = compute_patch(&prior, &args).expect_err("v1 floor errors");
    // Confirm the error preserves the id verbatim via Display.
    let msg = err.to_string();
    assert!(
        msg.contains(weird_id),
        "error message `{msg}` should mention queue_job_id `{weird_id}`",
    );
}

#[test]
fn verb_is_project_agnostic_on_error_path() {
    let prior_a = empty_project();
    let mut prior_b = empty_project();
    prior_b.name = "different-name".to_string();
    prior_b.duration_tk = verbreel_types::Tick::new(123_456);

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
    let verb = RenderQueueStatusVerb;
    let prior = empty_project();
    let err = verb
        .compute_patch(
            &prior,
            &json!({ "project_id": FIXTURE_PROJECT_ID, "queue_job_id": A_VALID_V7 }),
        )
        .expect_err("v1 floor: verb always errors");
    // Confirm we got the mapped E_QUEUE_JOB_NOT_FOUND via BadArgs.
    let VerbError::BadArgs { detail } = err else {
        panic!("expected BadArgs, got {err:?}");
    };
    assert!(
        detail.contains("E_QUEUE_JOB_NOT_FOUND"),
        "detail `{detail}` should mention E_QUEUE_JOB_NOT_FOUND",
    );
}

#[test]
fn warnings_are_empty_on_error_path() {
    // Since compute_patch returns Err before producing warnings, the
    // warning vector is empty by construction. The strongest assertion
    // we can make is via the standalone compute_patch returning Err.
    let prior = empty_project();
    let res = compute_patch(&prior, &args_default());
    assert!(res.is_err());
}

// --- reconstructor / fixture --------------------------------------------

#[test]
fn reconstruct_returns_null_for_args_deserialize_round_trip() {
    let verb = RenderQueueStatusVerb;
    let prior = empty_project();
    let args = serde_json::to_value(args_default()).expect("args → Value");
    let data = verb
        .reconstruct(&args, &json!([]), &[], &prior)
        .expect("reconstruct succeeds for well-formed args");
    assert_eq!(data, Value::Null);
}

#[test]
fn reconstruct_rejects_malformed_args() {
    let verb = RenderQueueStatusVerb;
    let prior = empty_project();
    let err = verb
        .reconstruct(&json!({}), &json!([]), &[], &prior)
        .expect_err("malformed args should fail reconstruction");
    // Type mismatch surfaces as ReconstructError::TypeMismatch.
    let s = err.to_string();
    assert!(
        s.contains("RenderQueueStatusArgs") || s.contains("wrong type"),
        "unexpected reconstruct error: {s}",
    );
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "render.queue.status")
        .expect("default_fixtures includes render.queue.status");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(RenderQueueStatusVerb))
        .expect("register render.queue.status verb");

    let report = validate_reconstructors(&registry, &[fixture])
        .expect("render.queue.status reconstruct from fixture");
    assert_eq!(report.verbs_checked, vec!["render.queue.status"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn verb_trait_surface_lookup_via_default_registry() {
    let registry = default_registry();
    let verb = registry
        .get("render.queue.status")
        .expect("render.queue.status registered in default_registry");

    let prior = empty_project();
    let err = verb
        .compute_patch(
            &prior,
            &json!({ "project_id": FIXTURE_PROJECT_ID, "queue_job_id": A_VALID_V7 }),
        )
        .expect_err("v1 floor: always errors");

    let VerbError::BadArgs { detail } = err else {
        panic!("expected BadArgs, got {err:?}");
    };
    assert!(detail.contains("E_QUEUE_JOB_NOT_FOUND"));
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
        "render.queue.status",
        json!({ "project_id": FIXTURE_PROJECT_ID, "queue_job_id": A_VALID_V7 }),
        None,
    );

    // The verb errors at compute_patch; mutate_via_verb surfaces that as
    // an Err return rather than an Applied outcome.
    match outcome {
        Err(_) => {}
        Ok(MutateOutcome::Applied { .. }) => {
            panic!("expected mutate_via_verb to error in v1 floor, got Applied")
        }
        Ok(other) => panic!("expected Err for v1 floor, got Ok({other:?})"),
    }
}

/// v1 floor happy-path deferral marker.
///
/// In v1 the queue is always empty (no `render.queue.add` /
/// `render.start` verb exists yet) so `render.queue.status` cannot
/// resolve any id. The happy path — returning a populated `QueueEntry`
/// matching a real queued/running/completed job — lights up when the
/// queue persistence layer ships and a `VerbContext` plumbs file I/O
/// into `compute_patch`. This test is intentionally a no-op so the
/// deferral is named in the test surface rather than in a hidden TODO.
#[test]
fn happy_path_unreachable_in_v1_floor() {
    // No assertions: this test exists to document the deferral.
}
