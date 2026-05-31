//! Tests for `font.list` (§7.5) — sixty-eighth production verb.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::font_list::{compute_patch, data_envelope_from_args};
use verbreel_state::{
    FontFamilyEntry, FontListArgs, FontListData, FontListVerb, Project, RegistrySource, Verb,
    VerbError, VerbRegistry, default_fixtures, default_registry, validate_reconstructors,
};

#[cfg(feature = "native")]
use verbreel_state::{MutateOutcome, ProjectStore};

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture → Project")
}

fn fixture_project_id() -> verbreel_types::ProjectId {
    empty_project().id
}

fn args() -> FontListArgs {
    FontListArgs {
        project_id: fixture_project_id(),
    }
}

#[test]
fn args_deserialize_ok() {
    let raw = json!({ "project_id": FIXTURE_PROJECT_ID });
    let typed: FontListArgs =
        serde_json::from_value(raw).expect("project_id is the only required arg field");
    assert_eq!(typed.project_id.to_string(), FIXTURE_PROJECT_ID);
}

#[test]
fn args_missing_project_id_fails_through_verb() {
    let prior = empty_project();
    let verb = FontListVerb;

    let err = verb
        .compute_patch(&prior, &json!({}))
        .expect_err("missing project_id should map to BadArgs");

    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_wrong_type_fails_through_verb() {
    let prior = empty_project();
    let verb = FontListVerb;

    let err = verb
        .compute_patch(&prior, &json!({ "project_id": 12345 }))
        .expect_err("non-string project_id should map to BadArgs");

    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn happy_path_returns_non_empty_family_list() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    assert!(!data.families.is_empty());
}

#[test]
fn bundled_inter_exists() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    assert!(
        data.families
            .iter()
            .any(|family| { family.name == "Inter" && family.source == RegistrySource::Bundled })
    );
}

#[test]
fn data_envelope_has_exactly_one_field_named_families() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    let value: Value = serde_json::to_value(&data).expect("envelope → Value");
    let obj = value.as_object().expect("envelope is a JSON object");

    assert_eq!(obj.keys().count(), 1);
    let keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    assert_eq!(keys, vec!["families"]);
}

#[test]
fn family_entry_without_path_serializes_public_fields() {
    let entry = FontFamilyEntry {
        name: "Inter".to_string(),
        source: RegistrySource::Bundled,
        path: None,
    };
    let value = serde_json::to_value(&entry).expect("FontEntry → Value");
    let obj = value.as_object().expect("FontEntry is a JSON object");

    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();
    let mut expected = vec!["name", "source"];
    expected.sort_unstable();
    assert_eq!(keys, expected);
    assert_eq!(obj.keys().count(), 2);
}

#[test]
fn registry_output_omits_font_paths() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");

    assert!(data.families.iter().all(|family| family.path.is_none()));
}

#[test]
fn family_entry_with_path_serializes_path_field() {
    let entry = FontFamilyEntry {
        name: "Inter".to_string(),
        source: RegistrySource::System,
        path: Some("/tmp/Inter.ttf".to_string()),
    };
    let value = serde_json::to_value(entry).expect("FontFamilyEntry → Value");
    let obj = value.as_object().expect("FontFamilyEntry is a JSON object");

    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();
    let mut expected = vec!["name", "path", "source"];
    expected.sort_unstable();
    assert_eq!(keys, expected);
    assert_eq!(obj.keys().count(), 3);
}

#[test]
fn font_source_bundled_serializes_to_lowercase_string() {
    let value =
        serde_json::to_value(RegistrySource::Bundled).expect("RegistrySource::Bundled → Value");
    assert_eq!(value, json!("bundled"));
}

#[test]
fn font_source_system_serializes_to_lowercase_string() {
    let value =
        serde_json::to_value(RegistrySource::System).expect("RegistrySource::System → Value");
    assert_eq!(value, json!("system"));
}

#[test]
fn family_entry_round_trip() {
    let original = FontFamilyEntry {
        name: "Roboto".to_string(),
        source: RegistrySource::System,
        path: Some("/usr/share/fonts/Roboto.ttf".to_string()),
    };
    let value = serde_json::to_value(&original).expect("FontFamilyEntry → Value");
    let parsed: FontFamilyEntry = serde_json::from_value(value).expect("Value → FontFamilyEntry");
    assert_eq!(parsed, original);
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
fn family_list_is_sorted_case_insensitive() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    let mut sorted = data.families.clone();
    sorted.sort_by_key(|family| family.name.to_lowercase());
    assert_eq!(data.families, sorted);
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
        .find(|event| event.verb == "font.list")
        .expect("default_fixtures includes font.list");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(FontListVerb))
        .expect("register font.list verb");

    let report =
        validate_reconstructors(&registry, &[fixture]).expect("font.list reconstruct from fixture");
    assert_eq!(report.verbs_checked, vec!["font.list"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn verb_trait_surface_lookup_via_default_registry() {
    let registry = default_registry();
    let verb = registry
        .get("font.list")
        .expect("font.list registered in default_registry");

    let prior = empty_project();
    let (patch, data, warnings) = verb
        .compute_patch(&prior, &json!({ "project_id": FIXTURE_PROJECT_ID }))
        .expect("default_registry invocation succeeds");

    assert!(patch.is_empty());
    assert!(warnings.is_empty());
    let typed: FontListData =
        serde_json::from_value(data).expect("envelope deserializes to FontListData");
    assert!(!typed.families.is_empty());
    assert!(
        typed
            .families
            .iter()
            .any(|family| { family.name == "Inter" && family.source == RegistrySource::Bundled })
    );
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
        .mutate_via_verb("font.list", json!({"project_id": FIXTURE_PROJECT_ID}), None)
        .expect("font.list should route");

    let MutateOutcome::NoOp { data, warnings, .. } = outcome else {
        panic!("expected NoOp outcome from font.list");
    };
    assert!(warnings.is_empty());

    let data: FontListData = serde_json::from_value(data).expect("font.list data deserializes");
    assert!(
        data.families
            .iter()
            .any(|family| { family.name == "Inter" && family.source == RegistrySource::Bundled })
    );
}
