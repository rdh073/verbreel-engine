//! Tests for `render.queue.clear` (§21.5) — seventy-third production verb.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::render_queue_clear::{compute_patch, data_envelope_from_args};
use verbreel_state::{
    CONFIRM_HINT, MutateOutcome, Project, QueueClearStateFilter, RenderQueueClearArgs,
    RenderQueueClearData, RenderQueueClearError, RenderQueueClearVerb, Verb, VerbError,
    VerbRegistry, default_fixtures, default_registry, validate_reconstructors,
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

fn args_default() -> RenderQueueClearArgs {
    RenderQueueClearArgs {
        project_id: fixture_project_id(),
        state_filter: None,
        confirm: None,
    }
}

// --- args deserialization ---------------------------------------------------

#[test]
fn args_deserialize_ok() {
    let raw = json!({ "project_id": FIXTURE_PROJECT_ID });
    let typed: RenderQueueClearArgs =
        serde_json::from_value(raw).expect("project_id alone is sufficient");
    assert_eq!(typed.project_id.to_string(), FIXTURE_PROJECT_ID);
    assert!(typed.state_filter.is_none());
    assert!(typed.confirm.is_none());
}

#[test]
fn args_missing_project_id_fails_through_verb() {
    let prior = empty_project();
    let verb = RenderQueueClearVerb;

    let err = verb
        .compute_patch(&prior, &json!({}))
        .expect_err("missing project_id should map to BadArgs");

    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_wrong_type_fails_through_verb() {
    let prior = empty_project();
    let verb = RenderQueueClearVerb;

    let err = verb
        .compute_patch(&prior, &json!({ "project_id": 12345 }))
        .expect_err("non-string project_id should map to BadArgs");

    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_state_filter_defaults_to_none_when_omitted() {
    let raw = json!({ "project_id": FIXTURE_PROJECT_ID });
    let typed: RenderQueueClearArgs =
        serde_json::from_value(raw).expect("state_filter is optional");
    assert!(typed.state_filter.is_none());
}

#[test]
fn args_confirm_defaults_to_none_when_omitted() {
    let raw = json!({ "project_id": FIXTURE_PROJECT_ID });
    let typed: RenderQueueClearArgs = serde_json::from_value(raw).expect("confirm is optional");
    assert!(typed.confirm.is_none());
}

// --- happy paths returning empty data --------------------------------------

#[test]
fn happy_path_default_filter_default_confirm() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args_default()).expect("default filter is terminal");
    assert_eq!(
        data,
        RenderQueueClearData {
            removed_queue_job_ids: vec![],
            canceled_running_job_ids: vec![],
        }
    );
}

#[test]
fn happy_path_explicit_completed_only() {
    let prior = empty_project();
    let mut a = args_default();
    a.state_filter = Some(vec![QueueClearStateFilter::Completed]);
    let (_, _, data) = compute_patch(&prior, &a).expect("completed is terminal");
    assert!(data.removed_queue_job_ids.is_empty());
    assert!(data.canceled_running_job_ids.is_empty());
}

#[test]
fn happy_path_explicit_terminal_triple() {
    let prior = empty_project();
    let mut a = args_default();
    a.state_filter = Some(vec![
        QueueClearStateFilter::Completed,
        QueueClearStateFilter::Canceled,
        QueueClearStateFilter::Failed,
    ]);
    let (_, _, data) = compute_patch(&prior, &a).expect("all terminal");
    assert!(data.removed_queue_job_ids.is_empty());
    assert!(data.canceled_running_job_ids.is_empty());
}

#[test]
fn happy_path_queued_with_confirm_true() {
    let prior = empty_project();
    let mut a = args_default();
    a.state_filter = Some(vec![QueueClearStateFilter::Queued]);
    a.confirm = Some(true);
    let (_, _, data) = compute_patch(&prior, &a).expect("queued + confirm passes gate");
    assert!(data.removed_queue_job_ids.is_empty());
    assert!(data.canceled_running_job_ids.is_empty());
}

#[test]
fn happy_path_running_with_confirm_true() {
    let prior = empty_project();
    let mut a = args_default();
    a.state_filter = Some(vec![QueueClearStateFilter::Running]);
    a.confirm = Some(true);
    let (_, _, data) = compute_patch(&prior, &a).expect("running + confirm passes gate");
    assert!(data.removed_queue_job_ids.is_empty());
    assert!(data.canceled_running_job_ids.is_empty());
}

#[test]
fn happy_path_mixed_queued_completed_with_confirm_true() {
    let prior = empty_project();
    let mut a = args_default();
    a.state_filter = Some(vec![
        QueueClearStateFilter::Queued,
        QueueClearStateFilter::Completed,
    ]);
    a.confirm = Some(true);
    let (_, _, data) = compute_patch(&prior, &a).expect("mixed + confirm passes gate");
    assert!(data.removed_queue_job_ids.is_empty());
    assert!(data.canceled_running_job_ids.is_empty());
}

// --- confirm-gate violations -----------------------------------------------

#[test]
fn confirm_gate_queued_without_confirm_rejected() {
    let prior = empty_project();
    let mut a = args_default();
    a.state_filter = Some(vec![QueueClearStateFilter::Queued]);
    let err = compute_patch(&prior, &a).expect_err("queued without confirm rejected");
    let RenderQueueClearError::ConfirmRequired {
        non_terminal_states,
        hint,
    } = err
    else {
        panic!("expected ConfirmRequired, got {err:?}");
    };
    assert_eq!(non_terminal_states, vec!["queued".to_string()]);
    assert_eq!(hint, CONFIRM_HINT);
}

#[test]
fn confirm_gate_running_without_confirm_rejected() {
    let prior = empty_project();
    let mut a = args_default();
    a.state_filter = Some(vec![QueueClearStateFilter::Running]);
    let err = compute_patch(&prior, &a).expect_err("running without confirm rejected");
    let RenderQueueClearError::ConfirmRequired {
        non_terminal_states,
        ..
    } = err
    else {
        panic!("expected ConfirmRequired, got {err:?}");
    };
    assert_eq!(non_terminal_states, vec!["running".to_string()]);
}

#[test]
fn confirm_gate_queued_and_running_without_confirm_rejected() {
    let prior = empty_project();
    let mut a = args_default();
    a.state_filter = Some(vec![
        QueueClearStateFilter::Queued,
        QueueClearStateFilter::Running,
    ]);
    let err = compute_patch(&prior, &a).expect_err("queued+running without confirm rejected");
    let RenderQueueClearError::ConfirmRequired {
        non_terminal_states,
        ..
    } = err
    else {
        panic!("expected ConfirmRequired, got {err:?}");
    };
    assert_eq!(
        non_terminal_states,
        vec!["queued".to_string(), "running".to_string()],
    );
}

#[test]
fn confirm_gate_mixed_terminal_non_terminal_without_confirm_rejected() {
    let prior = empty_project();
    let mut a = args_default();
    a.state_filter = Some(vec![
        QueueClearStateFilter::Queued,
        QueueClearStateFilter::Completed,
    ]);
    let err = compute_patch(&prior, &a).expect_err("mixed without confirm rejected");
    let RenderQueueClearError::ConfirmRequired {
        non_terminal_states,
        ..
    } = err
    else {
        panic!("expected ConfirmRequired, got {err:?}");
    };
    // Only the non-terminal state surfaces; Completed must NOT appear.
    assert_eq!(non_terminal_states, vec!["queued".to_string()]);
}

#[test]
fn confirm_gate_explicit_false_rejected() {
    let prior = empty_project();
    let mut a = args_default();
    a.state_filter = Some(vec![QueueClearStateFilter::Queued]);
    a.confirm = Some(false);
    let err = compute_patch(&prior, &a).expect_err("queued + confirm:false rejected");
    assert!(matches!(err, RenderQueueClearError::ConfirmRequired { .. }));
}

#[test]
fn confirm_gate_does_not_fire_for_pure_terminal_filter() {
    let prior = empty_project();
    let mut a = args_default();
    a.state_filter = Some(vec![
        QueueClearStateFilter::Completed,
        QueueClearStateFilter::Canceled,
        QueueClearStateFilter::Failed,
    ]);
    let (_, _, data) =
        compute_patch(&prior, &a).expect("pure terminal without confirm must succeed");
    assert!(data.removed_queue_job_ids.is_empty());
}

// --- details.non_terminal_states shape ------------------------------------

#[test]
fn non_terminal_states_lists_both_when_queued_and_running() {
    let prior = empty_project();
    let mut a = args_default();
    a.state_filter = Some(vec![
        QueueClearStateFilter::Queued,
        QueueClearStateFilter::Running,
    ]);
    let err = compute_patch(&prior, &a).expect_err("queued+running rejected");
    let RenderQueueClearError::ConfirmRequired {
        non_terminal_states,
        ..
    } = err
    else {
        panic!("expected ConfirmRequired, got {err:?}");
    };
    assert!(non_terminal_states.contains(&"queued".to_string()));
    assert!(non_terminal_states.contains(&"running".to_string()));
    assert_eq!(non_terminal_states.len(), 2);
}

// --- data shape ------------------------------------------------------------

#[test]
fn data_envelope_keys_match_v1_floor_exactly() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args_default()).expect("happy path");
    let value: Value = serde_json::to_value(&data).expect("envelope → Value");
    let obj = value.as_object().expect("envelope is a JSON object");

    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec!["canceled_running_job_ids", "removed_queue_job_ids"]
    );
}

// --- QueueClearStateFilter serialization ----------------------------------

#[test]
fn queue_clear_state_filter_serializes_to_lowercase_strings() {
    assert_eq!(
        serde_json::to_value(QueueClearStateFilter::Queued).expect("Queued → Value"),
        json!("queued")
    );
    assert_eq!(
        serde_json::to_value(QueueClearStateFilter::Running).expect("Running → Value"),
        json!("running")
    );
    assert_eq!(
        serde_json::to_value(QueueClearStateFilter::Completed).expect("Completed → Value"),
        json!("completed")
    );
    assert_eq!(
        serde_json::to_value(QueueClearStateFilter::Failed).expect("Failed → Value"),
        json!("failed")
    );
    assert_eq!(
        serde_json::to_value(QueueClearStateFilter::Canceled).expect("Canceled → Value"),
        json!("canceled")
    );
}

// --- project agnostic ------------------------------------------------------

#[test]
fn verb_is_project_agnostic() {
    let prior_a = empty_project();
    let mut prior_b = empty_project();
    prior_b.name = "different-name".to_string();
    prior_b.duration_tk = verbreel_types::Tick::new(123_456);

    let (_, _, data_a) = compute_patch(&prior_a, &args_default()).expect("a");
    let (_, _, data_b) = compute_patch(&prior_b, &args_default()).expect("b");

    assert_eq!(data_a, data_b);
}

// --- patch / warnings -----------------------------------------------------

#[test]
fn patch_is_empty() {
    let prior = empty_project();
    let (patch, _, _) = compute_patch(&prior, &args_default()).expect("happy path");
    assert_eq!(patch, json!([]));
}

#[test]
fn warnings_are_empty() {
    let prior = empty_project();
    let (_, warnings, _) = compute_patch(&prior, &args_default()).expect("happy path");
    assert!(warnings.is_empty());
}

// --- reconstructor ---------------------------------------------------------

#[test]
fn reconstruct_byte_identical_to_compute_patch() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args_default()).expect("happy path");

    let envelope = data_envelope_from_args(&args_default(), &prior).expect("envelope rebuilds");

    let lhs = serde_json::to_vec(&data).expect("forward data serializes");
    let rhs = serde_json::to_vec(&envelope).expect("reconstructed envelope serializes");
    assert_eq!(lhs, rhs);
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "render.queue.clear")
        .expect("default_fixtures includes render.queue.clear");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(RenderQueueClearVerb))
        .expect("register render.queue.clear verb");

    let report = validate_reconstructors(&registry, &[fixture])
        .expect("render.queue.clear reconstruct from fixture");
    assert_eq!(report.verbs_checked, vec!["render.queue.clear"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn verb_trait_surface_lookup_via_default_registry() {
    let registry = default_registry();
    let verb = registry
        .get("render.queue.clear")
        .expect("render.queue.clear registered in default_registry");

    let prior = empty_project();
    let (patch, data, warnings) = verb
        .compute_patch(&prior, &json!({ "project_id": FIXTURE_PROJECT_ID }))
        .expect("default_registry invocation succeeds");

    assert!(patch.is_empty());
    assert!(warnings.is_empty());
    let typed: RenderQueueClearData =
        serde_json::from_value(data).expect("envelope deserializes to RenderQueueClearData");
    assert!(typed.removed_queue_job_ids.is_empty());
    assert!(typed.canceled_running_job_ids.is_empty());
}

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb() {
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
            "render.queue.clear",
            json!({"project_id": FIXTURE_PROJECT_ID}),
            None,
        )
        .expect("render.queue.clear should route");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("expected Applied outcome from render.queue.clear");
    };
    assert!(warnings.is_empty());

    let data: RenderQueueClearData =
        serde_json::from_value(data).expect("render.queue.clear data deserializes");
    assert!(data.removed_queue_job_ids.is_empty());
    assert!(data.canceled_running_job_ids.is_empty());
}
