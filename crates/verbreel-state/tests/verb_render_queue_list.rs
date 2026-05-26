//! Tests for `render.queue.list` (§21.2) — seventy-second production verb.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::render_queue_list::{compute_patch, data_envelope_from_args};
use verbreel_state::{
    MutateOutcome, Project, QueueEntry, QueueJobState, QueueStateFilter, RenderQueueListArgs,
    RenderQueueListData, RenderQueueListVerb, Verb, VerbError, VerbRegistry, default_fixtures,
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

fn args() -> RenderQueueListArgs {
    RenderQueueListArgs {
        project_id: fixture_project_id(),
        state_filter: None,
    }
}

#[test]
fn args_deserialize_ok() {
    let raw = json!({ "project_id": FIXTURE_PROJECT_ID });
    let typed: RenderQueueListArgs =
        serde_json::from_value(raw).expect("project_id alone is sufficient");
    assert_eq!(typed.project_id.to_string(), FIXTURE_PROJECT_ID);
    assert!(typed.state_filter.is_none());
}

#[test]
fn args_missing_project_id_fails_through_verb() {
    let prior = empty_project();
    let verb = RenderQueueListVerb;

    let err = verb
        .compute_patch(&prior, &json!({}))
        .expect_err("missing project_id should map to BadArgs");

    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_wrong_type_fails_through_verb() {
    let prior = empty_project();
    let verb = RenderQueueListVerb;

    let err = verb
        .compute_patch(&prior, &json!({ "project_id": 12345 }))
        .expect_err("non-string project_id should map to BadArgs");

    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_state_filter_defaults_to_none_when_omitted() {
    let raw = json!({ "project_id": FIXTURE_PROJECT_ID });
    let typed: RenderQueueListArgs = serde_json::from_value(raw).expect("state_filter is optional");
    assert!(typed.state_filter.is_none());
}

#[test]
fn args_state_filter_accepts_queued() {
    let raw = json!({ "project_id": FIXTURE_PROJECT_ID, "state_filter": "queued" });
    let typed: RenderQueueListArgs = serde_json::from_value(raw).expect("queued deserializes");
    assert_eq!(typed.state_filter, Some(QueueStateFilter::Queued));
}

#[test]
fn args_state_filter_accepts_running() {
    let raw = json!({ "project_id": FIXTURE_PROJECT_ID, "state_filter": "running" });
    let typed: RenderQueueListArgs = serde_json::from_value(raw).expect("running deserializes");
    assert_eq!(typed.state_filter, Some(QueueStateFilter::Running));
}

#[test]
fn args_state_filter_accepts_completed() {
    let raw = json!({ "project_id": FIXTURE_PROJECT_ID, "state_filter": "completed" });
    let typed: RenderQueueListArgs = serde_json::from_value(raw).expect("completed deserializes");
    assert_eq!(typed.state_filter, Some(QueueStateFilter::Completed));
}

#[test]
fn args_state_filter_accepts_failed() {
    let raw = json!({ "project_id": FIXTURE_PROJECT_ID, "state_filter": "failed" });
    let typed: RenderQueueListArgs = serde_json::from_value(raw).expect("failed deserializes");
    assert_eq!(typed.state_filter, Some(QueueStateFilter::Failed));
}

#[test]
fn args_state_filter_accepts_canceled() {
    let raw = json!({ "project_id": FIXTURE_PROJECT_ID, "state_filter": "canceled" });
    let typed: RenderQueueListArgs = serde_json::from_value(raw).expect("canceled deserializes");
    assert_eq!(typed.state_filter, Some(QueueStateFilter::Canceled));
}

#[test]
fn args_state_filter_accepts_all() {
    let raw = json!({ "project_id": FIXTURE_PROJECT_ID, "state_filter": "all" });
    let typed: RenderQueueListArgs = serde_json::from_value(raw).expect("all deserializes");
    assert_eq!(typed.state_filter, Some(QueueStateFilter::All));
}

#[test]
fn happy_path_returns_empty_items_list() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    assert_eq!(data, RenderQueueListData { items: vec![] });
}

#[test]
fn items_is_empty() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    assert!(data.items.is_empty());
}

#[test]
fn data_envelope_keys_match_v1_floor_exactly() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    let value: Value = serde_json::to_value(&data).expect("envelope → Value");
    let obj = value.as_object().expect("envelope is a JSON object");

    let keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    assert_eq!(keys, vec!["items"]);
}

#[test]
fn queue_job_state_serializes_to_lowercase_strings() {
    assert_eq!(
        serde_json::to_value(QueueJobState::Queued).expect("Queued → Value"),
        json!("queued")
    );
    assert_eq!(
        serde_json::to_value(QueueJobState::Running).expect("Running → Value"),
        json!("running")
    );
    assert_eq!(
        serde_json::to_value(QueueJobState::Completed).expect("Completed → Value"),
        json!("completed")
    );
    assert_eq!(
        serde_json::to_value(QueueJobState::Failed).expect("Failed → Value"),
        json!("failed")
    );
    assert_eq!(
        serde_json::to_value(QueueJobState::Canceled).expect("Canceled → Value"),
        json!("canceled")
    );
}

#[test]
fn queue_state_filter_serializes_to_lowercase_strings() {
    assert_eq!(
        serde_json::to_value(QueueStateFilter::Queued).expect("Queued → Value"),
        json!("queued")
    );
    assert_eq!(
        serde_json::to_value(QueueStateFilter::Running).expect("Running → Value"),
        json!("running")
    );
    assert_eq!(
        serde_json::to_value(QueueStateFilter::Completed).expect("Completed → Value"),
        json!("completed")
    );
    assert_eq!(
        serde_json::to_value(QueueStateFilter::Failed).expect("Failed → Value"),
        json!("failed")
    );
    assert_eq!(
        serde_json::to_value(QueueStateFilter::Canceled).expect("Canceled → Value"),
        json!("canceled")
    );
    assert_eq!(
        serde_json::to_value(QueueStateFilter::All).expect("All → Value"),
        json!("all")
    );
}

#[test]
fn queue_entry_serialization_includes_started_at_when_some() {
    let entry = QueueEntry {
        queue_job_id: "qj-1".to_string(),
        project_id: FIXTURE_PROJECT_ID.to_string(),
        state: QueueJobState::Running,
        preset: "h264-1080p".to_string(),
        out_path: "/tmp/out.mp4".to_string(),
        priority: 10,
        added_at: "2026-05-27T00:00:00Z".to_string(),
        started_at: Some("2026-05-27T00:00:01Z".to_string()),
    };
    let value = serde_json::to_value(&entry).expect("QueueEntry → Value");
    let obj = value.as_object().expect("QueueEntry is a JSON object");

    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();

    let mut expected = vec![
        "added_at",
        "out_path",
        "preset",
        "priority",
        "project_id",
        "queue_job_id",
        "started_at",
        "state",
    ];
    expected.sort_unstable();

    assert_eq!(keys, expected);
    assert_eq!(obj["started_at"], json!("2026-05-27T00:00:01Z"));
}

#[test]
fn queue_entry_serialization_omits_started_at_when_none() {
    let entry = QueueEntry {
        queue_job_id: "qj-2".to_string(),
        project_id: FIXTURE_PROJECT_ID.to_string(),
        state: QueueJobState::Queued,
        preset: "h264-1080p".to_string(),
        out_path: "/tmp/out.mp4".to_string(),
        priority: 0,
        added_at: "2026-05-27T00:00:00Z".to_string(),
        started_at: None,
    };
    let value = serde_json::to_value(&entry).expect("QueueEntry → Value");
    let obj = value.as_object().expect("QueueEntry is a JSON object");

    assert!(
        !obj.contains_key("started_at"),
        "started_at = None must not appear in JSON output",
    );

    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();
    let mut expected = vec![
        "added_at",
        "out_path",
        "preset",
        "priority",
        "project_id",
        "queue_job_id",
        "state",
    ];
    expected.sort_unstable();
    assert_eq!(keys, expected);
}

#[test]
fn verb_is_project_agnostic() {
    let prior_a = empty_project();
    let mut prior_b = empty_project();
    prior_b.name = "different-name".to_string();
    prior_b.duration_tk = verbreel_types::Tick::new(123_456);

    let (_, _, data_a) = compute_patch(&prior_a, &args()).expect("happy path a");
    let (_, _, data_b) = compute_patch(&prior_b, &args()).expect("happy path b");

    assert_eq!(data_a, data_b);
}

#[test]
fn verb_ignores_state_filter_in_v1() {
    let prior = empty_project();
    let mut args_all = args();
    args_all.state_filter = Some(QueueStateFilter::All);
    let mut args_queued = args();
    args_queued.state_filter = Some(QueueStateFilter::Queued);

    let (_, _, data_all) = compute_patch(&prior, &args_all).expect("all");
    let (_, _, data_queued) = compute_patch(&prior, &args_queued).expect("queued");

    assert_eq!(data_all, data_queued);
    assert!(data_all.items.is_empty());
}

#[test]
fn patch_is_empty() {
    let prior = empty_project();
    let (patch, _, _) = compute_patch(&prior, &args()).expect("happy path");
    assert_eq!(patch, json!([]));
}

#[test]
fn warnings_are_empty() {
    let prior = empty_project();
    let (_, warnings, _) = compute_patch(&prior, &args()).expect("happy path");
    assert!(warnings.is_empty());
}

#[test]
fn reconstruct_byte_identical_to_compute_patch() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");

    let envelope = data_envelope_from_args(&args(), &prior).expect("envelope rebuilds");

    let lhs = serde_json::to_vec(&data).expect("forward data serializes");
    let rhs = serde_json::to_vec(&envelope).expect("reconstructed envelope serializes");
    assert_eq!(lhs, rhs);
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "render.queue.list")
        .expect("default_fixtures includes render.queue.list");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(RenderQueueListVerb))
        .expect("register render.queue.list verb");

    let report = validate_reconstructors(&registry, &[fixture])
        .expect("render.queue.list reconstruct from fixture");
    assert_eq!(report.verbs_checked, vec!["render.queue.list"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn verb_trait_surface_lookup_via_default_registry() {
    let registry = default_registry();
    let verb = registry
        .get("render.queue.list")
        .expect("render.queue.list registered in default_registry");

    let prior = empty_project();
    let (patch, data, warnings) = verb
        .compute_patch(&prior, &json!({ "project_id": FIXTURE_PROJECT_ID }))
        .expect("default_registry invocation succeeds");

    assert!(patch.is_empty());
    assert!(warnings.is_empty());
    let typed: RenderQueueListData =
        serde_json::from_value(data).expect("envelope deserializes to RenderQueueListData");
    assert!(typed.items.is_empty());
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
            "render.queue.list",
            json!({"project_id": FIXTURE_PROJECT_ID}),
            None,
        )
        .expect("render.queue.list should route");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("expected Applied outcome from render.queue.list");
    };
    assert!(warnings.is_empty());

    let data: RenderQueueListData =
        serde_json::from_value(data).expect("render.queue.list data deserializes");
    assert!(data.items.is_empty());
}
