//! Tests for `stock.search` (§17.2) — v1 local empty-results floor.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::stock_search::{compute_patch, data_envelope_from_args};
use verbreel_state::{
    Project, ReconstructError, StockMediaKind, StockSearchArgs, StockSearchData,
    StockSearchDimensions, StockSearchError, StockSearchFilters, StockSearchItem,
    StockSearchLicense, StockSearchVerb, Verb, VerbError, VerbRegistry, default_fixtures,
    default_registry, validate_reconstructors,
};

#[cfg(feature = "native")]
use verbreel_state::{LifecycleError, MutateOutcome, ProjectStore};

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture -> Project")
}

fn fixture_project_id() -> verbreel_types::ProjectId {
    empty_project().id
}

fn args_value_local() -> Value {
    json!({
        "project_id": FIXTURE_PROJECT_ID,
        "provider_id": "local",
        "query": "sunset beach",
        "kind": "video",
    })
}

fn args_local() -> StockSearchArgs {
    StockSearchArgs {
        project_id: fixture_project_id(),
        provider_id: "local".to_string(),
        query: "sunset beach".to_string(),
        kind: "video".to_string(),
        limit: 25,
        filters: StockSearchFilters::default(),
    }
}

#[test]
fn args_deserialize_minimal_and_defaults_apply() {
    let typed: StockSearchArgs =
        serde_json::from_value(args_value_local()).expect("well-formed minimal args parse");
    assert_eq!(typed.project_id.to_string(), FIXTURE_PROJECT_ID);
    assert_eq!(typed.provider_id, "local");
    assert_eq!(typed.query, "sunset beach");
    assert_eq!(typed.kind, "video");
    assert_eq!(typed.limit, 25);
    assert_eq!(
        typed.filters,
        StockSearchFilters {
            duration_min_tk: None,
            duration_max_tk: None,
            aspect: "any".to_string(),
            license: "any".to_string(),
        }
    );
}

#[test]
fn args_deserialize_explicit_filters() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "provider_id": "local",
        "query": "city skyline",
        "kind": "image",
        "limit": 10,
        "filters": {
            "duration_min_tk": 100,
            "duration_max_tk": 200,
            "aspect": "16:9",
            "license": "cc0",
        }
    });

    let typed: StockSearchArgs = serde_json::from_value(raw).expect("explicit filters parse");
    assert_eq!(typed.limit, 10);
    assert_eq!(typed.filters.duration_min_tk, Some(100));
    assert_eq!(typed.filters.duration_max_tk, Some(200));
    assert_eq!(typed.filters.aspect, "16:9");
    assert_eq!(typed.filters.license, "cc0");
}

#[test]
fn unknown_fields_reject_through_verb_bad_args() {
    let prior = empty_project();
    let verb = StockSearchVerb;
    let cases = [
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "provider_id": "local",
            "query": "sunset beach",
            "kind": "video",
            "unknown": true,
        }),
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "provider_id": "local",
            "query": "sunset beach",
            "kind": "video",
            "filters": { "unknown": true },
        }),
    ];

    for raw in cases {
        let err = verb
            .compute_patch(&prior, &raw)
            .expect_err("unknown fields should map to BadArgs");
        assert!(matches!(err, VerbError::BadArgs { .. }));
    }
}

#[test]
fn missing_required_fields_reject_through_verb_bad_args() {
    let prior = empty_project();
    let verb = StockSearchVerb;
    let cases = [
        json!({ "provider_id": "local", "query": "q", "kind": "video" }),
        json!({ "project_id": FIXTURE_PROJECT_ID, "query": "q", "kind": "video" }),
        json!({ "project_id": FIXTURE_PROJECT_ID, "provider_id": "local", "kind": "video" }),
        json!({ "project_id": FIXTURE_PROJECT_ID, "provider_id": "local", "query": "q" }),
        json!({}),
    ];

    for raw in cases {
        let err = verb
            .compute_patch(&prior, &raw)
            .expect_err("missing required fields should map to BadArgs");
        assert!(matches!(err, VerbError::BadArgs { .. }));
    }
}

#[test]
fn wrong_shape_types_reject_through_verb_bad_args() {
    let prior = empty_project();
    let verb = StockSearchVerb;
    let cases = [
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "provider_id": 1,
            "query": "q",
            "kind": "video",
        }),
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "provider_id": "local",
            "query": 1,
            "kind": "video",
        }),
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "provider_id": "local",
            "query": "q",
            "kind": 1,
        }),
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "provider_id": "local",
            "query": "q",
            "kind": "video",
            "limit": 2.5,
        }),
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "provider_id": "local",
            "query": "q",
            "kind": "video",
            "filters": [],
        }),
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "provider_id": "local",
            "query": "q",
            "kind": "video",
            "filters": { "duration_min_tk": 1.25 },
        }),
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "provider_id": "local",
            "query": "q",
            "kind": "video",
            "filters": { "duration_max_tk": 1.25 },
        }),
    ];

    for (idx, raw) in cases.into_iter().enumerate() {
        let result = verb.compute_patch(&prior, &raw);
        let Err(err) = result else {
            panic!("case {idx} unexpectedly succeeded: raw={raw}");
        };
        assert!(matches!(err, VerbError::BadArgs { .. }));
    }
}

#[test]
fn schema_violations_map_to_custom() {
    let prior = empty_project();
    let verb = StockSearchVerb;
    let too_long = "x".repeat(513);
    let cases = vec![
        (
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "provider_id": "local",
                "query": "",
                "kind": "video",
            }),
            "query",
        ),
        (
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "provider_id": "local",
                "query": too_long,
                "kind": "video",
            }),
            "query",
        ),
        (
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "provider_id": "local",
                "query": "q",
                "kind": "gif",
            }),
            "kind",
        ),
        (
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "provider_id": "local",
                "query": "q",
                "kind": "video",
                "filters": { "aspect": "4:3" },
            }),
            "filters.aspect",
        ),
        (
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "provider_id": "local",
                "query": "q",
                "kind": "video",
                "filters": { "license": "commercial" },
            }),
            "filters.license",
        ),
    ];

    for (raw, field) in cases {
        let err = verb
            .compute_patch(&prior, &raw)
            .expect_err("schema failures should be runtime custom errors");
        let VerbError::Custom(detail) = err else {
            panic!("expected Custom for schema failure");
        };
        assert!(detail.contains("E_SCHEMA_VIOLATION"));
        assert!(detail.contains(field));
    }
}

#[test]
fn bad_range_failures_map_to_custom() {
    let prior = empty_project();
    let verb = StockSearchVerb;
    let cases = vec![
        (
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "provider_id": "local",
                "query": "q",
                "kind": "video",
                "limit": 0,
            }),
            "limit",
        ),
        (
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "provider_id": "local",
                "query": "q",
                "kind": "video",
                "limit": 101,
            }),
            "limit",
        ),
        (
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "provider_id": "local",
                "query": "q",
                "kind": "video",
                "filters": { "duration_min_tk": -1 },
            }),
            "filters.duration_min_tk",
        ),
        (
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "provider_id": "local",
                "query": "q",
                "kind": "video",
                "filters": { "duration_max_tk": -1 },
            }),
            "filters.duration_max_tk",
        ),
        (
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "provider_id": "local",
                "query": "q",
                "kind": "video",
                "filters": { "duration_min_tk": 200, "duration_max_tk": 200 },
            }),
            "filters.duration_max_tk",
        ),
        (
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "provider_id": "local",
                "query": "q",
                "kind": "video",
                "filters": { "duration_min_tk": 200, "duration_max_tk": 150 },
            }),
            "filters.duration_max_tk",
        ),
    ];

    for (raw, field) in cases {
        let err = verb
            .compute_patch(&prior, &raw)
            .expect_err("range failures should be runtime custom errors");
        let VerbError::Custom(detail) = err else {
            panic!("expected Custom for bad-range failure");
        };
        assert!(detail.contains("E_BAD_RANGE"));
        assert!(detail.contains(field));
    }
}

#[test]
fn unknown_provider_returns_provider_unknown_with_registered_local_only() {
    let prior = empty_project();
    let args = StockSearchArgs {
        provider_id: "pexels".to_string(),
        ..args_local()
    };

    let err = compute_patch(&prior, &args).expect_err("unknown provider should fail");
    let StockSearchError::ProviderUnknown {
        provider_id,
        registered_ids,
    } = err
    else {
        panic!("expected ProviderUnknown");
    };
    assert_eq!(provider_id, "pexels");
    assert_eq!(registered_ids, vec!["local".to_string()]);
}

#[test]
fn unknown_provider_maps_to_custom_through_verb() {
    let prior = empty_project();
    let verb = StockSearchVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "provider_id": "pexels",
                "query": "q",
                "kind": "video",
            }),
        )
        .expect_err("unknown provider should map to Custom");
    let VerbError::Custom(detail) = err else {
        panic!("expected Custom");
    };
    assert!(detail.contains("E_STOCK_PROVIDER_UNKNOWN"));
    assert!(detail.contains("local"));
}

#[test]
fn local_provider_returns_empty_patch_warnings_and_items() {
    let prior = empty_project();
    let (patch, warnings, data) = compute_patch(&prior, &args_local()).expect("local succeeds");
    assert_eq!(patch, json!([]));
    assert!(warnings.is_empty());
    assert_eq!(data, StockSearchData { items: vec![] });
}

#[test]
fn data_envelope_key_is_items_only() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args_local()).expect("local succeeds");
    let value = serde_json::to_value(&data).expect("data -> Value");
    let obj = value.as_object().expect("envelope object");
    let keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    assert_eq!(keys, vec!["items"]);
}

#[test]
fn reconstruct_byte_identical_to_compute_patch() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args_local()).expect("local succeeds");
    let envelope = data_envelope_from_args(&args_local(), &prior).expect("envelope rebuilds");
    let lhs = serde_json::to_vec(&data).expect("forward data serializes");
    let rhs = serde_json::to_vec(&envelope).expect("reconstructed data serializes");
    assert_eq!(lhs, rhs);
}

#[test]
fn future_item_serialization_omits_optional_none_fields() {
    let item = StockSearchItem {
        stock_id: "local:1".to_string(),
        provider_id: "local".to_string(),
        kind: StockMediaKind::Video,
        title: "Sample".to_string(),
        duration_tk: None,
        dimensions: None,
        preview_url: "https://example.invalid/preview.jpg".to_string(),
        license: StockSearchLicense {
            spdx: "CC0-1.0".to_string(),
            attribution_required: false,
            attribution_text: None,
            source_url: "https://example.invalid/item".to_string(),
        },
    };

    let value = serde_json::to_value(item).expect("item -> Value");
    let obj = value.as_object().expect("item object");
    assert!(!obj.contains_key("duration_tk"));
    assert!(!obj.contains_key("dimensions"));
    let license = obj
        .get("license")
        .and_then(Value::as_object)
        .expect("license object");
    assert!(!license.contains_key("attribution_text"));
}

#[test]
fn future_item_serialization_includes_optional_present_fields() {
    let item = StockSearchItem {
        stock_id: "local:2".to_string(),
        provider_id: "local".to_string(),
        kind: StockMediaKind::Image,
        title: "Sample 2".to_string(),
        duration_tk: Some(2_400_000),
        dimensions: Some(StockSearchDimensions {
            width: 1920,
            height: 1080,
        }),
        preview_url: "https://example.invalid/preview2.jpg".to_string(),
        license: StockSearchLicense {
            spdx: "CC-BY-4.0".to_string(),
            attribution_required: true,
            attribution_text: Some("Photo by Alice".to_string()),
            source_url: "https://example.invalid/item2".to_string(),
        },
    };

    let value = serde_json::to_value(item).expect("item -> Value");
    let obj = value.as_object().expect("item object");
    assert_eq!(obj.get("duration_tk"), Some(&json!(2_400_000)));
    assert_eq!(
        obj.get("dimensions"),
        Some(&json!({ "width": 1920, "height": 1080 }))
    );
    assert_eq!(
        obj.get("license")
            .and_then(Value::as_object)
            .and_then(|l| l.get("attribution_text")),
        Some(&json!("Photo by Alice"))
    );
}

#[test]
fn reserved_error_variants_display_expected_codes() {
    let cases = vec![
        (
            StockSearchError::ProviderUnknown {
                provider_id: "missing".to_string(),
                registered_ids: vec!["local".to_string()],
            },
            "E_STOCK_PROVIDER_UNKNOWN",
        ),
        (
            StockSearchError::RateLimited {
                provider_id: "provider".to_string(),
                retry_after_s: 30,
            },
            "E_STOCK_RATE_LIMITED",
        ),
        (
            StockSearchError::AuthRequired {
                provider_id: "provider".to_string(),
                hint: "set TOKEN".to_string(),
            },
            "E_STOCK_AUTH_REQUIRED",
        ),
        (
            StockSearchError::BadRange {
                field: "limit".to_string(),
                requested: "0".to_string(),
                allowed: "[1, 100]".to_string(),
            },
            "E_BAD_RANGE",
        ),
        (
            StockSearchError::SchemaViolation {
                field: "kind".to_string(),
                detail: "bad".to_string(),
            },
            "E_SCHEMA_VIOLATION",
        ),
    ];

    for (error, code) in cases {
        assert!(error.to_string().contains(code));
    }
}

#[test]
fn all_stock_search_errors_map_to_custom() {
    let cases = vec![
        StockSearchError::ProviderUnknown {
            provider_id: "missing".to_string(),
            registered_ids: vec!["local".to_string()],
        },
        StockSearchError::RateLimited {
            provider_id: "provider".to_string(),
            retry_after_s: 30,
        },
        StockSearchError::AuthRequired {
            provider_id: "provider".to_string(),
            hint: "set TOKEN".to_string(),
        },
        StockSearchError::BadRange {
            field: "limit".to_string(),
            requested: "0".to_string(),
            allowed: "[1, 100]".to_string(),
        },
        StockSearchError::SchemaViolation {
            field: "kind".to_string(),
            detail: "bad".to_string(),
        },
    ];

    for case in cases {
        let mapped: VerbError = case.into();
        assert!(matches!(mapped, VerbError::Custom(_)));
    }
}

#[test]
fn reconstruct_returns_empty_items_for_local_args() {
    let verb = StockSearchVerb;
    let prior = empty_project();
    let data = verb
        .reconstruct(
            &serde_json::to_value(args_local()).expect("args -> Value"),
            &json!([]),
            &[],
            &prior,
        )
        .expect("reconstruct succeeds");
    assert_eq!(data, json!({ "items": [] }));
}

#[test]
fn reconstruct_rejects_malformed_args() {
    let verb = StockSearchVerb;
    let prior = empty_project();
    let err = verb
        .reconstruct(&json!({}), &json!([]), &[], &prior)
        .expect_err("malformed args should fail reconstruct");
    assert!(matches!(
        err,
        ReconstructError::TypeMismatch {
            expected: "StockSearchArgs",
            ..
        }
    ));
}

#[test]
fn default_fixture_validates_with_only_stock_search_verb() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "stock.search")
        .expect("default_fixtures includes stock.search");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(StockSearchVerb))
        .expect("register stock.search verb");

    let report = validate_reconstructors(&registry, &[fixture]).expect("fixture validates");
    assert_eq!(report.verbs_checked, vec!["stock.search"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn default_fixture_has_empty_patch_warnings_and_empty_items_data() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "stock.search")
        .expect("default_fixtures includes stock.search");
    assert_eq!(fixture.patch, json!([]));
    assert!(fixture.warnings.is_empty());
    assert_eq!(fixture.expected_data, json!({ "items": [] }));
}

#[test]
fn default_registry_contains_stock_search() {
    let registry = default_registry();
    let verb = registry
        .get("stock.search")
        .expect("stock.search in default_registry");
    let prior = empty_project();

    let (patch, data, warnings) = verb
        .compute_patch(&prior, &args_value_local())
        .expect("local provider succeeds");
    assert!(patch.is_empty());
    assert!(warnings.is_empty());
    let typed: StockSearchData =
        serde_json::from_value(data).expect("data deserializes to StockSearchData");
    assert!(typed.items.is_empty());
}

#[cfg(feature = "native")]
#[test]
fn mutate_via_verb_local_returns_applied_with_empty_items() {
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
        .mutate_via_verb("stock.search", args_value_local(), None)
        .expect("stock.search local routes");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("expected Applied from stock.search");
    };
    assert!(warnings.is_empty());
    let typed: StockSearchData = serde_json::from_value(data).expect("data deserializes");
    assert!(typed.items.is_empty());
}

#[cfg(feature = "native")]
#[test]
fn mutate_via_verb_unknown_provider_returns_verb_execution_failed() {
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
        "stock.search",
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "provider_id": "pexels",
            "query": "q",
            "kind": "video",
        }),
        None,
    );

    let Err(LifecycleError::VerbExecutionFailed { source, .. }) = outcome else {
        panic!("expected VerbExecutionFailed, got {outcome:?}");
    };
    let VerbError::Custom(detail) = source else {
        panic!("expected Custom source, got {source:?}");
    };
    assert!(detail.contains("E_STOCK_PROVIDER_UNKNOWN"));
}
