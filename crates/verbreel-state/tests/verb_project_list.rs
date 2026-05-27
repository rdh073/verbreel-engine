//! Tests for `project.list` (§2.6) — eighty-first production verb.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::project_list::{compute_patch, data_envelope_from_args};
use verbreel_state::{
    MutateOutcome, Project, ProjectEntry, ProjectListArgs, ProjectListData, ProjectListState,
    ProjectListVerb, Verb, VerbError, VerbRegistry, default_fixtures, default_registry,
    validate_reconstructors,
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

fn args() -> ProjectListArgs {
    ProjectListArgs {
        project_id: fixture_project_id(),
    }
}

#[test]
fn args_deserialize_ok() {
    let raw = json!({ "project_id": FIXTURE_PROJECT_ID });
    let typed: ProjectListArgs =
        serde_json::from_value(raw).expect("project_id alone is sufficient");
    assert_eq!(typed.project_id.to_string(), FIXTURE_PROJECT_ID);
}

#[test]
fn args_missing_project_id_fails_through_verb() {
    let prior = empty_project();
    let verb = ProjectListVerb;

    let err = verb
        .compute_patch(&prior, &json!({}))
        .expect_err("missing project_id should map to BadArgs");

    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_wrong_type_fails_through_verb() {
    let prior = empty_project();
    let verb = ProjectListVerb;

    let err = verb
        .compute_patch(&prior, &json!({ "project_id": 12345 }))
        .expect_err("non-string project_id should map to BadArgs");

    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn happy_path_returns_empty_projects_list() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    assert_eq!(data, ProjectListData { projects: vec![] });
}

#[test]
fn projects_is_empty() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    assert!(data.projects.is_empty());
}

#[test]
fn data_envelope_keys_match_v1_floor_exactly() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    let value: Value = serde_json::to_value(&data).expect("envelope → Value");
    let obj = value.as_object().expect("envelope is a JSON object");

    let keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    assert_eq!(keys, vec!["projects"]);
}

#[test]
fn project_list_state_open_serializes_lowercase() {
    assert_eq!(
        serde_json::to_value(ProjectListState::Open).expect("Open → Value"),
        json!("open")
    );
}

#[test]
fn project_list_state_closed_serializes_lowercase() {
    assert_eq!(
        serde_json::to_value(ProjectListState::Closed).expect("Closed → Value"),
        json!("closed")
    );
}

#[test]
fn project_entry_has_five_keys() {
    let entry = ProjectEntry {
        id: FIXTURE_PROJECT_ID.to_string(),
        name: "Demo".to_string(),
        path: "/tmp/demo".to_string(),
        state: ProjectListState::Closed,
        last_opened_at: "2026-05-27T00:00:00Z".to_string(),
    };
    let value = serde_json::to_value(&entry).expect("ProjectEntry → Value");
    let obj = value.as_object().expect("ProjectEntry is a JSON object");

    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();

    let mut expected = vec!["id", "last_opened_at", "name", "path", "state"];
    expected.sort_unstable();

    assert_eq!(keys, expected);
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
fn verb_is_project_agnostic() {
    let prior_a = empty_project();
    let mut prior_b = empty_project();
    prior_b.name = "different-name".to_string();
    prior_b.duration_tk = verbreel_types::Tick::new(987_654);

    let (_, _, data_a) = compute_patch(&prior_a, &args()).expect("happy path a");
    let (_, _, data_b) = compute_patch(&prior_b, &args()).expect("happy path b");

    assert_eq!(data_a, data_b);
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
        .find(|event| event.verb == "project.list")
        .expect("default_fixtures includes project.list");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(ProjectListVerb))
        .expect("register project.list verb");

    let report = validate_reconstructors(&registry, &[fixture])
        .expect("project.list reconstruct from fixture");
    assert_eq!(report.verbs_checked, vec!["project.list"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn verb_trait_surface_lookup_via_default_registry() {
    let registry = default_registry();
    let verb = registry
        .get("project.list")
        .expect("project.list registered in default_registry");

    let prior = empty_project();
    let (patch, data, warnings) = verb
        .compute_patch(&prior, &json!({ "project_id": FIXTURE_PROJECT_ID }))
        .expect("default_registry invocation succeeds");

    assert!(patch.is_empty());
    assert!(warnings.is_empty());
    let typed: ProjectListData =
        serde_json::from_value(data).expect("envelope deserializes to ProjectListData");
    assert!(typed.projects.is_empty());
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
            "project.list",
            json!({"project_id": FIXTURE_PROJECT_ID}),
            None,
        )
        .expect("project.list should route");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("expected Applied outcome from project.list");
    };
    assert!(warnings.is_empty());

    let data: ProjectListData =
        serde_json::from_value(data).expect("project.list data deserializes");
    assert!(data.projects.is_empty());
}
