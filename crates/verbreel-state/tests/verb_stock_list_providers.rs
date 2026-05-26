//! Tests for `stock.list_providers` (§17.1) — sixty-seventh production verb.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::stock_list_providers::{compute_patch, data_envelope_from_args};
use verbreel_state::{
    MutateOutcome, Project, Provider, ProviderKind, StockListProvidersArgs, StockListProvidersData,
    StockListProvidersVerb, StockMediaKind, Verb, VerbError, VerbRegistry, default_fixtures,
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

fn args() -> StockListProvidersArgs {
    StockListProvidersArgs {
        project_id: fixture_project_id(),
    }
}

#[test]
fn args_deserialize_ok() {
    let raw = json!({ "project_id": FIXTURE_PROJECT_ID });
    let typed: StockListProvidersArgs =
        serde_json::from_value(raw).expect("project_id is the only required arg field");
    assert_eq!(typed.project_id.to_string(), FIXTURE_PROJECT_ID);
}

#[test]
fn args_missing_project_id_fails_through_verb() {
    let prior = empty_project();
    let verb = StockListProvidersVerb;

    let err = verb
        .compute_patch(&prior, &json!({}))
        .expect_err("missing project_id should map to BadArgs");

    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_wrong_type_fails_through_verb() {
    let prior = empty_project();
    let verb = StockListProvidersVerb;

    let err = verb
        .compute_patch(&prior, &json!({ "project_id": 12345 }))
        .expect_err("non-string project_id should map to BadArgs");

    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn happy_path_returns_single_local_provider() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");

    assert_eq!(data.providers.len(), 1);
    let provider = &data.providers[0];
    assert_eq!(provider.id, "local");
}

#[test]
fn provider_id_is_local() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    assert_eq!(data.providers[0].id, "local");
}

#[test]
fn provider_name_is_local_catalog() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    assert_eq!(data.providers[0].name, "Local catalog");
}

#[test]
fn provider_kind_is_local_typed() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    assert_eq!(data.providers[0].kind, ProviderKind::Local);
}

#[test]
fn provider_kind_serializes_to_lowercase_string() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    let value = serde_json::to_value(&data.providers[0]).expect("Provider → Value");
    let kind = value
        .as_object()
        .and_then(|o| o.get("kind"))
        .and_then(Value::as_str)
        .expect("kind is a string");
    assert_eq!(kind, "local");
}

#[test]
fn provider_kind_enum_serdes_to_snake_case_for_compound_variants() {
    let http = serde_json::to_value(ProviderKind::HttpCatalog).expect("HttpCatalog → Value");
    assert_eq!(http, json!("http_catalog"));
    let custom = serde_json::to_value(ProviderKind::CustomCommand).expect("CustomCommand → Value");
    assert_eq!(custom, json!("custom_command"));
}

#[test]
fn kinds_supported_has_exactly_five_in_spec_order() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    let kinds = &data.providers[0].kinds_supported;
    assert_eq!(kinds.len(), 5);
    assert_eq!(
        kinds,
        &vec![
            StockMediaKind::Video,
            StockMediaKind::Audio,
            StockMediaKind::Image,
            StockMediaKind::Sticker,
            StockMediaKind::Music,
        ]
    );
}

#[test]
fn kinds_supported_serializes_to_lowercase_strings() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    let value = serde_json::to_value(&data.providers[0]).expect("Provider → Value");
    let kinds = value
        .as_object()
        .and_then(|o| o.get("kinds_supported"))
        .and_then(Value::as_array)
        .expect("kinds_supported is an array");
    let strs: Vec<&str> = kinds.iter().filter_map(Value::as_str).collect();
    assert_eq!(strs, vec!["video", "audio", "image", "sticker", "music"]);
}

#[test]
fn requires_credentials_is_false() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    assert!(!data.providers[0].requires_credentials);
}

#[test]
fn base_url_is_none_typed() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    assert!(data.providers[0].base_url.is_none());
}

#[test]
fn base_url_is_absent_from_serialized_json() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    let value = serde_json::to_value(&data.providers[0]).expect("Provider → Value");
    let obj = value.as_object().expect("Provider is a JSON object");
    assert!(
        !obj.contains_key("base_url"),
        "base_url = None must not appear in JSON output",
    );
}

#[test]
fn rate_limit_per_minute_is_none_typed() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    assert!(data.providers[0].rate_limit_per_minute.is_none());
}

#[test]
fn rate_limit_per_minute_is_absent_from_serialized_json() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    let value = serde_json::to_value(&data.providers[0]).expect("Provider → Value");
    let obj = value.as_object().expect("Provider is a JSON object");
    assert!(
        !obj.contains_key("rate_limit_per_minute"),
        "rate_limit_per_minute = None must not appear in JSON output",
    );
}

#[test]
fn provider_shape_lock_present_keys_only() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    let value = serde_json::to_value(&data.providers[0]).expect("Provider → Value");
    let obj = value.as_object().expect("Provider is a JSON object");

    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();

    let mut expected = vec![
        "id",
        "kind",
        "kinds_supported",
        "name",
        "requires_credentials",
    ];
    expected.sort_unstable();

    assert_eq!(keys, expected);
}

#[test]
fn option_fields_present_when_set() {
    let p = Provider {
        id: "shutterstock".to_string(),
        name: "Shutterstock".to_string(),
        kind: ProviderKind::HttpCatalog,
        kinds_supported: vec![StockMediaKind::Video, StockMediaKind::Image],
        requires_credentials: true,
        base_url: Some("https://example.com".to_string()),
        rate_limit_per_minute: Some(60),
    };
    let value = serde_json::to_value(&p).expect("Provider → Value");
    let obj = value.as_object().expect("Provider is a JSON object");
    assert!(obj.contains_key("base_url"));
    assert!(obj.contains_key("rate_limit_per_minute"));
    assert_eq!(obj["base_url"], json!("https://example.com"));
    assert_eq!(obj["rate_limit_per_minute"], json!(60));
}

#[test]
fn data_providers_array_has_exactly_one_entry() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    assert_eq!(data.providers.len(), 1);
}

#[test]
fn data_envelope_keys_match_v1_floor_exactly() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    let value: Value = serde_json::to_value(&data).expect("envelope → Value");
    let obj = value.as_object().expect("envelope is a JSON object");

    let keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    assert_eq!(keys, vec!["providers"]);
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
        .find(|event| event.verb == "stock.list_providers")
        .expect("default_fixtures includes stock.list_providers");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(StockListProvidersVerb))
        .expect("register stock.list_providers verb");

    let report = validate_reconstructors(&registry, &[fixture])
        .expect("stock.list_providers reconstruct from fixture");
    assert_eq!(report.verbs_checked, vec!["stock.list_providers"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn verb_trait_surface_lookup_via_default_registry() {
    let registry = default_registry();
    let verb = registry
        .get("stock.list_providers")
        .expect("stock.list_providers registered in default_registry");

    let prior = empty_project();
    let (patch, data, warnings) = verb
        .compute_patch(&prior, &json!({ "project_id": FIXTURE_PROJECT_ID }))
        .expect("default_registry invocation succeeds");

    assert!(patch.is_empty());
    assert!(warnings.is_empty());
    let typed: StockListProvidersData =
        serde_json::from_value(data).expect("envelope deserializes to StockListProvidersData");
    assert_eq!(typed.providers.len(), 1);
    assert_eq!(typed.providers[0].id, "local");
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
            "stock.list_providers",
            json!({"project_id": FIXTURE_PROJECT_ID}),
            None,
        )
        .expect("stock.list_providers should route");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("expected Applied outcome from stock.list_providers");
    };
    assert!(warnings.is_empty());

    let data: StockListProvidersData =
        serde_json::from_value(data).expect("stock.list_providers data deserializes");
    assert_eq!(data.providers.len(), 1);
    assert_eq!(data.providers[0].id, "local");
    assert_eq!(data.providers[0].kind, ProviderKind::Local);
}
