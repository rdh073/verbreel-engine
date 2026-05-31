//! Tests for `schema` (§1.2) — sixty-sixth production verb (meta arc final).

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::schema::{compute_patch, data_envelope_from_post_state};
use verbreel_state::{
    MutateOutcome, Project, SchemaArgs, SchemaData, SchemaTarget, SchemaVerb, SchemaVerbError,
    Verb, VerbError, VerbRegistry, default_fixtures, default_registry, validate_reconstructors,
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

fn args(target: SchemaTarget, name: Option<&str>) -> SchemaArgs {
    SchemaArgs {
        project_id: fixture_project_id(),
        target,
        name: name.map(str::to_string),
    }
}

// ------- arg deserialization -------

#[test]
fn args_deserialize_ok() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "target": "project",
    });
    let typed: SchemaArgs = serde_json::from_value(raw).expect("ok");
    assert_eq!(typed.project_id.to_string(), FIXTURE_PROJECT_ID);
    assert_eq!(typed.target, SchemaTarget::Project);
    assert!(typed.name.is_none());
}

#[test]
fn args_missing_target_fails_through_verb() {
    let prior = empty_project();
    let verb = SchemaVerb;
    let err = verb
        .compute_patch(&prior, &json!({ "project_id": FIXTURE_PROJECT_ID }))
        .expect_err("missing target → BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_missing_project_id_fails_through_verb() {
    let prior = empty_project();
    let verb = SchemaVerb;
    let err = verb
        .compute_patch(&prior, &json!({ "target": "project" }))
        .expect_err("missing project_id → BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_bad_target_value_surfaces_unknown_target() {
    let prior = empty_project();
    let verb = SchemaVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "target": "bogus",
            }),
        )
        .expect_err("bogus target → BadArgs");
    let detail = match err {
        VerbError::BadArgs { detail } => detail,
        other => panic!("expected BadArgs, got {other:?}"),
    };
    assert!(
        detail.contains("E_UNKNOWN_TARGET"),
        "detail should embed E_UNKNOWN_TARGET, got `{detail}`",
    );
}

#[test]
fn args_wrong_type_for_name_fails_through_verb() {
    let prior = empty_project();
    let verb = SchemaVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "target": "command",
                "name": 12345,
            }),
        )
        .expect_err("non-string name → BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

// ------- target=Project happy path -------

#[test]
fn project_target_returns_parsed_schema_object() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args(SchemaTarget::Project, None)).expect("ok");
    assert!(data.schema.is_object(), "schema must be a JSON object");
    assert_eq!(data.schema_id, "verbreel:project:v1");
    let title = data
        .schema
        .get("title")
        .and_then(Value::as_str)
        .expect("schema has title");
    assert_eq!(title, "Verbreel Project");
}

#[test]
fn project_target_with_name_provided_is_ignored() {
    let prior = empty_project();
    let (_, _, with_none) = compute_patch(&prior, &args(SchemaTarget::Project, None)).expect("ok");
    let (_, _, with_name) =
        compute_patch(&prior, &args(SchemaTarget::Project, Some("foo"))).expect("ok");
    assert_eq!(with_none, with_name);
}

// ------- target=Command happy paths -------

#[test]
fn command_target_with_known_verb_returns_vacuous_schema() {
    let prior = empty_project();
    let (_, _, data) =
        compute_patch(&prior, &args(SchemaTarget::Command, Some("clip.add"))).expect("ok");
    assert_eq!(data.schema_id, "verbreel:command:clip.add:v1");
    let obj = data.schema.as_object().expect("schema is object");
    assert_eq!(obj.get("type"), Some(&json!("object")));
    assert_eq!(obj.get("additionalProperties"), Some(&json!(true)));
    assert_eq!(obj.get("title"), Some(&json!("clip.add args")));
}

#[test]
fn command_target_missing_name_returns_missing_name() {
    let prior = empty_project();
    let err = compute_patch(&prior, &args(SchemaTarget::Command, None))
        .expect_err("missing name → MissingName");
    assert!(
        matches!(err, SchemaVerbError::MissingName { ref target } if target == "command"),
        "got {err:?}"
    );
}

#[test]
fn command_target_unknown_verb_returns_unknown_kind() {
    let prior = empty_project();
    let err = compute_patch(&prior, &args(SchemaTarget::Command, Some("no.such")))
        .expect_err("unknown verb → UnknownKind");
    assert!(
        matches!(err, SchemaVerbError::UnknownKind { ref kind } if kind == "no.such"),
        "got {err:?}"
    );
}

#[test]
fn command_target_unknown_verb_maps_to_bad_args_at_verb_surface() {
    let prior = empty_project();
    let verb = SchemaVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "target": "command",
                "name": "no.such",
            }),
        )
        .expect_err("unknown verb → BadArgs");
    let detail = match err {
        VerbError::BadArgs { detail } => detail,
        other => panic!("expected BadArgs, got {other:?}"),
    };
    assert!(detail.contains("E_UNKNOWN_KIND"));
    assert!(detail.contains("no.such"));
}

// ------- target=Effect happy paths -------

#[test]
fn effect_target_with_known_kind_returns_vacuous_schema() {
    let prior = empty_project();
    let (_, _, data) =
        compute_patch(&prior, &args(SchemaTarget::Effect, Some("blur"))).expect("ok");
    assert_eq!(data.schema_id, "verbreel:effect:blur:v1");
    let obj = data.schema.as_object().expect("schema is object");
    assert_eq!(obj.get("type"), Some(&json!("object")));
    assert_eq!(obj.get("additionalProperties"), Some(&json!(true)));
    assert_eq!(obj.get("title"), Some(&json!("blur params")));
}

#[test]
fn effect_target_missing_name_returns_missing_name() {
    let prior = empty_project();
    let err = compute_patch(&prior, &args(SchemaTarget::Effect, None))
        .expect_err("missing name → MissingName");
    assert!(
        matches!(err, SchemaVerbError::MissingName { ref target } if target == "effect"),
        "got {err:?}"
    );
}

#[test]
fn effect_target_unknown_kind_returns_unknown_kind() {
    let prior = empty_project();
    let err = compute_patch(&prior, &args(SchemaTarget::Effect, Some("no_such_fx")))
        .expect_err("unknown effect → UnknownKind");
    assert!(
        matches!(err, SchemaVerbError::UnknownKind { ref kind } if kind == "no_such_fx"),
        "got {err:?}"
    );
}

// ------- schema_id format checks -------

#[test]
fn schema_id_format_for_project() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args(SchemaTarget::Project, None)).expect("ok");
    assert_eq!(data.schema_id, "verbreel:project:v1");
}

#[test]
fn schema_id_format_for_command() {
    let prior = empty_project();
    let (_, _, data) =
        compute_patch(&prior, &args(SchemaTarget::Command, Some("marker.add"))).expect("ok");
    assert_eq!(data.schema_id, "verbreel:command:marker.add:v1");
}

#[test]
fn schema_id_format_for_effect() {
    let prior = empty_project();
    let (_, _, data) =
        compute_patch(&prior, &args(SchemaTarget::Effect, Some("blur"))).expect("ok");
    assert_eq!(data.schema_id, "verbreel:effect:blur:v1");
}

// ------- vacuous schema shape -------

#[test]
fn vacuous_command_schema_has_exactly_three_keys() {
    let prior = empty_project();
    let (_, _, data) =
        compute_patch(&prior, &args(SchemaTarget::Command, Some("clip.add"))).expect("ok");
    let obj = data.schema.as_object().expect("object");
    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, vec!["additionalProperties", "title", "type"]);
}

#[test]
fn vacuous_effect_schema_has_exactly_three_keys() {
    let prior = empty_project();
    let (_, _, data) =
        compute_patch(&prior, &args(SchemaTarget::Effect, Some("blur"))).expect("ok");
    let obj = data.schema.as_object().expect("object");
    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, vec!["additionalProperties", "title", "type"]);
}

// ------- patch / warnings invariants -------

#[test]
fn patch_is_empty() {
    let prior = empty_project();
    let (patch, _, _) = compute_patch(&prior, &args(SchemaTarget::Project, None)).expect("ok");
    assert_eq!(patch, json!([]));
}

#[test]
fn warnings_are_empty() {
    let prior = empty_project();
    let (_, warnings, _) = compute_patch(&prior, &args(SchemaTarget::Project, None)).expect("ok");
    assert!(warnings.is_empty());
}

// ------- project-agnostic invariant -------

#[test]
fn verb_is_project_agnostic() {
    let prior_a = empty_project();
    let mut prior_b = empty_project();
    prior_b.name = "different-name".to_string();
    prior_b.duration_tk = verbreel_types::Tick::new(123_456);

    let inputs = args(SchemaTarget::Command, Some("clip.add"));
    let (_, _, data_a) = compute_patch(&prior_a, &inputs).expect("a");
    let (_, _, data_b) = compute_patch(&prior_b, &inputs).expect("b");
    assert_eq!(data_a, data_b);
}

// ------- reconstructor round-trip -------

#[test]
fn reconstruct_byte_identical_to_compute_patch() {
    let prior = empty_project();
    let inputs = args(SchemaTarget::Project, None);
    let (_, _, data) = compute_patch(&prior, &inputs).expect("ok");
    let envelope = data_envelope_from_post_state(&inputs, &prior).expect("envelope");
    let lhs = serde_json::to_vec(&data).expect("forward serializes");
    let rhs = serde_json::to_vec(&envelope).expect("rebuilt serializes");
    assert_eq!(lhs, rhs);
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "schema")
        .expect("default_fixtures includes schema");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(SchemaVerb))
        .expect("register schema verb");

    let report =
        validate_reconstructors(&registry, &[fixture]).expect("schema reconstruct from fixture");
    assert_eq!(report.verbs_checked, vec!["schema"]);
    assert_eq!(report.fixtures_run, 1);
}

// ------- registry / trait surface -------

#[test]
fn verb_trait_surface_lookup_via_default_registry() {
    let registry = default_registry();
    let verb = registry
        .get("schema")
        .expect("schema registered in default_registry");

    let prior = empty_project();
    let (patch, data, warnings) = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "target": "project",
            }),
        )
        .expect("default_registry invocation succeeds");

    assert!(patch.is_empty());
    assert!(warnings.is_empty());
    let typed: SchemaData = serde_json::from_value(data).expect("envelope deserializes");
    assert_eq!(typed.schema_id, "verbreel:project:v1");
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
            "schema",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "target": "effect",
                "name": "blur",
            }),
            None,
        )
        .expect("schema should route");

    let MutateOutcome::NoOp { data, warnings, .. } = outcome else {
        panic!("expected NoOp outcome from schema");
    };
    assert!(warnings.is_empty());

    let data: SchemaData = serde_json::from_value(data).expect("data deserializes");
    assert_eq!(data.schema_id, "verbreel:effect:blur:v1");
}

// ------- serialization shape -------

#[test]
fn schema_data_serializes_to_exactly_two_fields() {
    let data = SchemaData {
        schema: json!({"type": "object"}),
        schema_id: "verbreel:project:v1".to_string(),
    };
    let value = serde_json::to_value(&data).expect("serializes");
    let obj = value.as_object().expect("object");
    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, vec!["schema", "schema_id"]);
}

#[test]
fn schema_data_roundtrips_through_serde() {
    let original = SchemaData {
        schema: json!({"type": "object", "title": "T"}),
        schema_id: "verbreel:command:foo:v1".to_string(),
    };
    let value = serde_json::to_value(&original).expect("serialize");
    let restored: SchemaData = serde_json::from_value(value).expect("deserialize");
    assert_eq!(original, restored);
}
