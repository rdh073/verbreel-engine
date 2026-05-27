//! Tests for `stock.describe` (§17.4) — v1 local stock-not-found floor.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::stock_describe::compute_patch;
use verbreel_state::{
    Project, ReconstructError, StockDescribeArgs, StockDescribeData, StockDescribeError,
    StockDescribeVerb, StockMediaKind, StockSearchDimensions, StockSearchLicense, Verb, VerbError,
    VerbRegistry, default_fixtures, default_registry, validate_reconstructors,
};

#[cfg(feature = "native")]
use verbreel_state::{LifecycleError, ProjectStore};

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture -> Project")
}

fn fixture_project_id() -> verbreel_types::ProjectId {
    empty_project().id
}

fn args_default_local() -> StockDescribeArgs {
    StockDescribeArgs {
        project_id: fixture_project_id(),
        provider_id: "local".to_string(),
        stock_id: "local:stock-123".to_string(),
    }
}

fn args_value_local_minimal() -> Value {
    json!({
        "project_id": FIXTURE_PROJECT_ID,
        "provider_id": "local",
        "stock_id": "local:stock-123",
    })
}

#[test]
fn args_deserialize_minimal() {
    let typed: StockDescribeArgs =
        serde_json::from_value(args_value_local_minimal()).expect("minimal args parse");
    assert_eq!(typed.project_id.to_string(), FIXTURE_PROJECT_ID);
    assert_eq!(typed.provider_id, "local");
    assert_eq!(typed.stock_id, "local:stock-123");
}

#[test]
fn unknown_fields_reject_through_verb_bad_args() {
    let prior = empty_project();
    let verb = StockDescribeVerb;
    let cases = [
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "provider_id": "local",
            "stock_id": "x",
            "unknown": true,
        }),
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "provider_id": "local",
            "stock_id": "x",
            "nested": { "k": "v" },
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
    let verb = StockDescribeVerb;
    let cases = [
        json!({ "provider_id": "local", "stock_id": "x" }),
        json!({ "project_id": FIXTURE_PROJECT_ID, "stock_id": "x" }),
        json!({ "project_id": FIXTURE_PROJECT_ID, "provider_id": "local" }),
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
fn non_string_required_fields_reject_through_verb_bad_args() {
    let prior = empty_project();
    let verb = StockDescribeVerb;
    let cases = [
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "provider_id": 1,
            "stock_id": "x",
        }),
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "provider_id": "local",
            "stock_id": 1,
        }),
        json!({
            "project_id": 123,
            "provider_id": "local",
            "stock_id": "x",
        }),
    ];

    for raw in cases {
        let err = verb
            .compute_patch(&prior, &raw)
            .expect_err("wrong shapes should map to BadArgs");
        assert!(matches!(err, VerbError::BadArgs { .. }));
    }
}

#[test]
fn unknown_provider_returns_provider_unknown_with_provider_id() {
    let prior = empty_project();
    let args = StockDescribeArgs {
        provider_id: "pexels".to_string(),
        ..args_default_local()
    };

    let err = compute_patch(&prior, &args).expect_err("unknown provider should fail");
    let StockDescribeError::ProviderUnknown { provider_id } = err else {
        panic!("expected ProviderUnknown");
    };
    assert_eq!(provider_id, "pexels");
}

#[test]
fn unknown_provider_maps_to_custom_through_verb() {
    let prior = empty_project();
    let verb = StockDescribeVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "provider_id": "pexels",
                "stock_id": "pexels:1",
            }),
        )
        .expect_err("unknown provider should map to Custom");
    let VerbError::Custom(detail) = err else {
        panic!("expected Custom");
    };
    assert!(detail.contains("E_STOCK_PROVIDER_UNKNOWN"));
    assert!(detail.contains("pexels"));
}

#[test]
fn provider_unknown_error_detail_contains_code_and_provider_id() {
    let prior = empty_project();
    let err = compute_patch(
        &prior,
        &StockDescribeArgs {
            provider_id: "pixabay".to_string(),
            ..args_default_local()
        },
    )
    .expect_err("unknown provider should fail");
    let msg = err.to_string();
    assert!(msg.contains("E_STOCK_PROVIDER_UNKNOWN"));
    assert!(msg.contains("pixabay"));
}

#[test]
fn local_provider_returns_stock_not_found_for_arbitrary_stock_ids() {
    let prior = empty_project();
    for stock_id in ["local:item-1", "UPPER+mixed id", ""] {
        let err = compute_patch(
            &prior,
            &StockDescribeArgs {
                stock_id: stock_id.to_string(),
                ..args_default_local()
            },
        )
        .expect_err("local provider floor should be stock-not-found");
        let StockDescribeError::StockNotFound {
            provider_id,
            stock_id: returned_stock_id,
        } = err
        else {
            panic!("expected StockNotFound");
        };
        assert_eq!(provider_id, "local");
        assert_eq!(returned_stock_id, stock_id);
    }
}

#[test]
fn stock_not_found_error_detail_contains_code_provider_and_stock_id() {
    let prior = empty_project();
    let err = compute_patch(
        &prior,
        &StockDescribeArgs {
            stock_id: "local:missing".to_string(),
            ..args_default_local()
        },
    )
    .expect_err("local provider should miss in v1 floor");
    let msg = err.to_string();
    assert!(msg.contains("E_STOCK_NOT_FOUND"));
    assert!(msg.contains("local"));
    assert!(msg.contains("local:missing"));
}

#[test]
fn local_provider_stock_not_found_maps_to_custom_through_verb() {
    let prior = empty_project();
    let verb = StockDescribeVerb;
    let err = verb
        .compute_patch(&prior, &args_value_local_minimal())
        .expect_err("local provider should map to runtime custom error");
    let VerbError::Custom(detail) = err else {
        panic!("expected Custom");
    };
    assert!(detail.contains("E_STOCK_NOT_FOUND"));
    assert!(detail.contains("local"));
    assert!(detail.contains("local:stock-123"));
}

#[test]
fn local_provider_allows_whitespace_stock_id_and_still_returns_not_found() {
    let prior = empty_project();
    let err = compute_patch(
        &prior,
        &StockDescribeArgs {
            stock_id: "  ".to_string(),
            ..args_default_local()
        },
    )
    .expect_err("well-formed string stock_id still hits v1 not-found floor");
    assert!(matches!(err, StockDescribeError::StockNotFound { .. }));
}

#[test]
fn future_success_data_serializes_exact_required_fields() {
    let data = StockDescribeData {
        stock_id: "local:future".to_string(),
        provider_id: "local".to_string(),
        kind: StockMediaKind::Video,
        title: "Sunset Clip".to_string(),
        duration_tk: None,
        dimensions: None,
        preview_url: "https://example.com/preview.jpg".to_string(),
        download_url: None,
        size_bytes: None,
        author: None,
        license: StockSearchLicense {
            spdx: "CC0-1.0".to_string(),
            attribution_required: false,
            attribution_text: None,
            source_url: "https://example.com/item".to_string(),
        },
    };

    let value = serde_json::to_value(data).expect("StockDescribeData -> Value");
    let obj = value.as_object().expect("StockDescribeData is object");
    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "kind",
            "license",
            "preview_url",
            "provider_id",
            "stock_id",
            "title"
        ]
    );
}

#[test]
fn future_success_data_serializes_optional_fields_when_present() {
    let data = StockDescribeData {
        stock_id: "local:future".to_string(),
        provider_id: "local".to_string(),
        kind: StockMediaKind::Image,
        title: "Cover".to_string(),
        duration_tk: Some(120),
        dimensions: Some(StockSearchDimensions {
            width: 1920,
            height: 1080,
        }),
        preview_url: "https://example.com/preview.jpg".to_string(),
        download_url: Some("https://example.com/download.jpg".to_string()),
        size_bytes: Some(42_000),
        author: Some("Alice".to_string()),
        license: StockSearchLicense {
            spdx: "CC-BY-4.0".to_string(),
            attribution_required: true,
            attribution_text: Some("Photo by Alice".to_string()),
            source_url: "https://example.com/item".to_string(),
        },
    };

    let value = serde_json::to_value(data).expect("StockDescribeData -> Value");
    let obj = value.as_object().expect("StockDescribeData is object");
    assert_eq!(obj.get("duration_tk"), Some(&json!(120)));
    assert_eq!(
        obj.get("dimensions"),
        Some(&json!({"width": 1920, "height": 1080}))
    );
    assert_eq!(
        obj.get("download_url"),
        Some(&json!("https://example.com/download.jpg"))
    );
    assert_eq!(obj.get("size_bytes"), Some(&json!(42_000)));
    assert_eq!(obj.get("author"), Some(&json!("Alice")));
}

#[test]
fn reserved_error_variants_display_expected_codes() {
    let cases = vec![
        (
            StockDescribeError::ProviderUnknown {
                provider_id: "missing".to_string(),
            },
            "E_STOCK_PROVIDER_UNKNOWN",
        ),
        (
            StockDescribeError::StockNotFound {
                provider_id: "local".to_string(),
                stock_id: "local:1".to_string(),
            },
            "E_STOCK_NOT_FOUND",
        ),
        (
            StockDescribeError::RateLimited {
                provider_id: "provider".to_string(),
                retry_after_s: 30,
            },
            "E_STOCK_RATE_LIMITED",
        ),
        (
            StockDescribeError::AuthRequired {
                provider_id: "provider".to_string(),
                hint: "set TOKEN".to_string(),
            },
            "E_STOCK_AUTH_REQUIRED",
        ),
    ];

    for (error, code) in cases {
        assert!(error.to_string().contains(code));
    }
}

#[test]
fn all_stock_describe_errors_map_to_custom() {
    let cases = vec![
        StockDescribeError::ProviderUnknown {
            provider_id: "missing".to_string(),
        },
        StockDescribeError::StockNotFound {
            provider_id: "local".to_string(),
            stock_id: "local:missing".to_string(),
        },
        StockDescribeError::RateLimited {
            provider_id: "provider".to_string(),
            retry_after_s: 30,
        },
        StockDescribeError::AuthRequired {
            provider_id: "provider".to_string(),
            hint: "set TOKEN".to_string(),
        },
    ];

    for case in cases {
        let mapped: VerbError = case.into();
        assert!(matches!(mapped, VerbError::Custom(_)));
    }
}

#[test]
fn reconstruct_returns_null_for_well_formed_args() {
    let verb = StockDescribeVerb;
    let prior = empty_project();
    let data = verb
        .reconstruct(
            &serde_json::to_value(args_default_local()).expect("args serialize"),
            &json!([]),
            &[],
            &prior,
        )
        .expect("reconstruct succeeds");
    assert_eq!(data, Value::Null);
}

#[test]
fn reconstruct_rejects_malformed_args() {
    let verb = StockDescribeVerb;
    let prior = empty_project();
    let err = verb
        .reconstruct(&json!({}), &json!([]), &[], &prior)
        .expect_err("malformed args should fail reconstruct");
    assert!(matches!(
        err,
        ReconstructError::TypeMismatch {
            expected: "StockDescribeArgs",
            ..
        }
    ));
}

#[test]
fn default_fixture_validates_with_only_stock_describe_verb() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "stock.describe")
        .expect("default_fixtures includes stock.describe");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(StockDescribeVerb))
        .expect("register stock.describe verb");

    let report = validate_reconstructors(&registry, &[fixture]).expect("fixture validates");
    assert_eq!(report.verbs_checked, vec!["stock.describe"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn default_fixture_has_empty_patch_warnings_and_null_expected_data() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "stock.describe")
        .expect("default_fixtures includes stock.describe");
    assert_eq!(fixture.patch, json!([]));
    assert!(fixture.warnings.is_empty());
    assert_eq!(fixture.expected_data, Value::Null);
}

#[test]
fn default_registry_contains_stock_describe() {
    let registry = default_registry();
    let verb = registry
        .get("stock.describe")
        .expect("stock.describe in default_registry");
    let prior = empty_project();
    let err = verb
        .compute_patch(&prior, &args_value_local_minimal())
        .expect_err("v1 floor always errors");
    let VerbError::Custom(detail) = err else {
        panic!("expected Custom, got {err:?}");
    };
    assert!(detail.contains("E_STOCK_NOT_FOUND"));
}

#[cfg(feature = "native")]
#[test]
fn mutate_via_verb_local_returns_stock_not_found_floor() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        empty_project(),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry succeeds");

    let outcome = store.mutate_via_verb("stock.describe", args_value_local_minimal(), None);

    let Err(LifecycleError::VerbExecutionFailed { source, .. }) = outcome else {
        panic!("expected VerbExecutionFailed, got {outcome:?}");
    };
    let VerbError::Custom(detail) = source else {
        panic!("expected Custom source, got {source:?}");
    };
    assert!(detail.contains("E_STOCK_NOT_FOUND"));
}

#[cfg(feature = "native")]
#[test]
fn mutate_via_verb_unknown_provider_returns_provider_unknown_floor() {
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
        "stock.describe",
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "provider_id": "pexels",
            "stock_id": "pexels:1",
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
