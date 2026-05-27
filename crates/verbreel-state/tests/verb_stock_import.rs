//! Tests for `stock.import` (§17.3) — v1 local stock-not-found floor.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::stock_import::compute_patch;
use verbreel_state::{
    Project, ReconstructError, StockImportArgs, StockImportData, StockImportError,
    StockImportLicenseRecorded, StockImportMode, StockImportVerb, Verb, VerbError, VerbRegistry,
    default_fixtures, default_registry, validate_reconstructors,
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

fn args_default_local() -> StockImportArgs {
    StockImportArgs {
        project_id: fixture_project_id(),
        provider_id: "local".to_string(),
        stock_id: "local:stock-123".to_string(),
        mode: StockImportMode::Copy,
        accept_license_unknown: false,
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
fn args_deserialize_minimal_and_defaults_apply() {
    let typed: StockImportArgs =
        serde_json::from_value(args_value_local_minimal()).expect("minimal args parse");
    assert_eq!(typed.project_id.to_string(), FIXTURE_PROJECT_ID);
    assert_eq!(typed.provider_id, "local");
    assert_eq!(typed.stock_id, "local:stock-123");
    assert_eq!(typed.mode, StockImportMode::Copy);
    assert!(!typed.accept_license_unknown);
}

#[test]
fn args_deserialize_explicit_mode_values() {
    let copy_raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "provider_id": "local",
        "stock_id": "local:copy",
        "mode": "copy",
    });
    let link_raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "provider_id": "local",
        "stock_id": "local:link",
        "mode": "link",
    });

    let copy_typed: StockImportArgs = serde_json::from_value(copy_raw).expect("copy mode parses");
    let link_typed: StockImportArgs = serde_json::from_value(link_raw).expect("link mode parses");
    assert_eq!(copy_typed.mode, StockImportMode::Copy);
    assert_eq!(link_typed.mode, StockImportMode::Link);
}

#[test]
fn accept_license_unknown_explicit_true_and_false_parse() {
    let true_raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "provider_id": "local",
        "stock_id": "local:true",
        "accept_license_unknown": true,
    });
    let false_raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "provider_id": "local",
        "stock_id": "local:false",
        "accept_license_unknown": false,
    });

    let true_typed: StockImportArgs = serde_json::from_value(true_raw).expect("true bool parses");
    let false_typed: StockImportArgs =
        serde_json::from_value(false_raw).expect("false bool parses");
    assert!(true_typed.accept_license_unknown);
    assert!(!false_typed.accept_license_unknown);
}

#[test]
fn unknown_fields_reject_through_verb_bad_args() {
    let prior = empty_project();
    let verb = StockImportVerb;
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
            "mode": "copy",
            "extra": "field",
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
    let verb = StockImportVerb;
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
    let verb = StockImportVerb;
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
fn invalid_mode_literal_rejects_through_verb_bad_args() {
    let prior = empty_project();
    let verb = StockImportVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "provider_id": "local",
                "stock_id": "x",
                "mode": "hardlink",
            }),
        )
        .expect_err("invalid mode should fail at arg-shape stage");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn non_boolean_accept_license_unknown_rejects_through_verb_bad_args() {
    let prior = empty_project();
    let verb = StockImportVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "provider_id": "local",
                "stock_id": "x",
                "accept_license_unknown": "true",
            }),
        )
        .expect_err("non-boolean flag should fail at arg-shape stage");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn unknown_provider_returns_provider_unknown_with_provider_id() {
    let prior = empty_project();
    let args = StockImportArgs {
        provider_id: "pexels".to_string(),
        ..args_default_local()
    };

    let err = compute_patch(&prior, &args).expect_err("unknown provider should fail");
    let StockImportError::ProviderUnknown { provider_id } = err else {
        panic!("expected ProviderUnknown");
    };
    assert_eq!(provider_id, "pexels");
}

#[test]
fn unknown_provider_maps_to_custom_through_verb() {
    let prior = empty_project();
    let verb = StockImportVerb;
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
fn local_provider_returns_stock_not_found_for_arbitrary_stock_ids() {
    let prior = empty_project();
    for stock_id in ["local:item-1", "UPPER+mixed id", ""] {
        let err = compute_patch(
            &prior,
            &StockImportArgs {
                stock_id: stock_id.to_string(),
                ..args_default_local()
            },
        )
        .expect_err("local provider floor should be stock-not-found");
        let StockImportError::StockNotFound {
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
fn local_provider_returns_stock_not_found_for_copy_and_link_modes() {
    let prior = empty_project();
    for mode in [StockImportMode::Copy, StockImportMode::Link] {
        let err = compute_patch(
            &prior,
            &StockImportArgs {
                mode,
                ..args_default_local()
            },
        )
        .expect_err("local provider floor should be stock-not-found");
        assert!(matches!(err, StockImportError::StockNotFound { .. }));
    }
}

#[test]
fn local_provider_stock_not_found_maps_to_custom_through_verb() {
    let prior = empty_project();
    let verb = StockImportVerb;
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
fn future_success_data_serializes_exact_spec_fields() {
    let data = StockImportData {
        asset_id: "0190b8d3-15e3-7000-bd00-0000feed0001".to_string(),
        stock_id: "local:future".to_string(),
        provider_id: "local".to_string(),
        license_recorded: StockImportLicenseRecorded {
            spdx: "CC0-1.0".to_string(),
            attribution_text: Some("Photo by Alice".to_string()),
        },
        bytes_downloaded: 1234,
        dedup_hit: false,
        mode_used: StockImportMode::Copy,
    };

    let value = serde_json::to_value(data).expect("StockImportData -> Value");
    let obj = value.as_object().expect("StockImportData is object");
    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "asset_id",
            "bytes_downloaded",
            "dedup_hit",
            "license_recorded",
            "mode_used",
            "provider_id",
            "stock_id",
        ]
    );
}

#[test]
fn license_recorded_omits_attribution_text_when_none() {
    let value = serde_json::to_value(StockImportLicenseRecorded {
        spdx: "unknown".to_string(),
        attribution_text: None,
    })
    .expect("license -> Value");
    let obj = value.as_object().expect("license object");
    assert_eq!(obj.get("spdx"), Some(&json!("unknown")));
    assert!(!obj.contains_key("attribution_text"));
}

#[test]
fn license_recorded_includes_attribution_text_when_present() {
    let value = serde_json::to_value(StockImportLicenseRecorded {
        spdx: "CC-BY-4.0".to_string(),
        attribution_text: Some("Photo by Bob".to_string()),
    })
    .expect("license -> Value");
    let obj = value.as_object().expect("license object");
    assert_eq!(obj.get("spdx"), Some(&json!("CC-BY-4.0")));
    assert_eq!(obj.get("attribution_text"), Some(&json!("Photo by Bob")));
}

#[test]
fn reserved_error_variants_display_expected_codes() {
    let cases = vec![
        (
            StockImportError::ProviderUnknown {
                provider_id: "missing".to_string(),
            },
            "E_STOCK_PROVIDER_UNKNOWN",
        ),
        (
            StockImportError::StockNotFound {
                provider_id: "local".to_string(),
                stock_id: "local:1".to_string(),
            },
            "E_STOCK_NOT_FOUND",
        ),
        (
            StockImportError::RateLimited {
                provider_id: "provider".to_string(),
                retry_after_s: 30,
            },
            "E_STOCK_RATE_LIMITED",
        ),
        (
            StockImportError::AuthRequired {
                provider_id: "provider".to_string(),
                hint: "set TOKEN".to_string(),
            },
            "E_STOCK_AUTH_REQUIRED",
        ),
        (
            StockImportError::LicenseUnknown {
                provider_id: "provider".to_string(),
                stock_id: "provider:id".to_string(),
                hint: "pass accept_license_unknown: true".to_string(),
            },
            "E_STOCK_LICENSE_UNKNOWN",
        ),
        (
            StockImportError::FetchFailed {
                provider_id: "provider".to_string(),
                stock_id: "provider:id".to_string(),
                upstream_status: "timeout".to_string(),
                elapsed_s: 120,
            },
            "E_STOCK_FETCH_FAILED",
        ),
        (
            StockImportError::ArgsIncompatible {
                hint: "link mode is only valid for local providers".to_string(),
            },
            "E_ARGS_INCOMPATIBLE",
        ),
        (
            StockImportError::AssetUnsupportedKind {
                detail: "ffprobe could not classify".to_string(),
            },
            "E_ASSET_UNSUPPORTED_KIND",
        ),
        (
            StockImportError::AssetUnreadable {
                detail: "permission denied".to_string(),
            },
            "E_ASSET_UNREADABLE",
        ),
        (
            StockImportError::AssetProbeTimeout {
                detail: "probe exceeded timeout".to_string(),
            },
            "E_ASSET_PROBE_TIMEOUT",
        ),
        (
            StockImportError::Io {
                detail: "io failed".to_string(),
            },
            "E_IO",
        ),
        (
            StockImportError::SchemaViolation {
                field: "kind".to_string(),
                detail: "invalid".to_string(),
            },
            "E_SCHEMA_VIOLATION",
        ),
    ];

    for (error, code) in cases {
        assert!(error.to_string().contains(code));
    }
}

#[test]
fn all_stock_import_errors_map_to_custom() {
    let cases = vec![
        StockImportError::ProviderUnknown {
            provider_id: "missing".to_string(),
        },
        StockImportError::StockNotFound {
            provider_id: "local".to_string(),
            stock_id: "local:missing".to_string(),
        },
        StockImportError::RateLimited {
            provider_id: "provider".to_string(),
            retry_after_s: 30,
        },
        StockImportError::AuthRequired {
            provider_id: "provider".to_string(),
            hint: "set TOKEN".to_string(),
        },
        StockImportError::LicenseUnknown {
            provider_id: "provider".to_string(),
            stock_id: "provider:id".to_string(),
            hint: "set accept_license_unknown".to_string(),
        },
        StockImportError::FetchFailed {
            provider_id: "provider".to_string(),
            stock_id: "provider:id".to_string(),
            upstream_status: "timeout".to_string(),
            elapsed_s: 120,
        },
        StockImportError::ArgsIncompatible {
            hint: "link mode is only valid for local providers".to_string(),
        },
        StockImportError::AssetUnsupportedKind {
            detail: "unsupported".to_string(),
        },
        StockImportError::AssetUnreadable {
            detail: "unreadable".to_string(),
        },
        StockImportError::AssetProbeTimeout {
            detail: "timeout".to_string(),
        },
        StockImportError::Io {
            detail: "io".to_string(),
        },
        StockImportError::SchemaViolation {
            field: "field".to_string(),
            detail: "detail".to_string(),
        },
    ];

    for case in cases {
        let mapped: VerbError = case.into();
        assert!(matches!(mapped, VerbError::Custom(_)));
    }
}

#[test]
fn reconstruct_returns_null_for_well_formed_args() {
    let verb = StockImportVerb;
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
    let verb = StockImportVerb;
    let prior = empty_project();
    let err = verb
        .reconstruct(&json!({}), &json!([]), &[], &prior)
        .expect_err("malformed args should fail reconstruct");
    assert!(matches!(
        err,
        ReconstructError::TypeMismatch {
            expected: "StockImportArgs",
            ..
        }
    ));
}

#[test]
fn default_fixture_validates_with_only_stock_import_verb() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "stock.import")
        .expect("default_fixtures includes stock.import");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(StockImportVerb))
        .expect("register stock.import verb");

    let report = validate_reconstructors(&registry, &[fixture]).expect("fixture validates");
    assert_eq!(report.verbs_checked, vec!["stock.import"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn default_fixture_has_empty_patch_warnings_and_null_expected_data() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "stock.import")
        .expect("default_fixtures includes stock.import");
    assert_eq!(fixture.patch, json!([]));
    assert!(fixture.warnings.is_empty());
    assert_eq!(fixture.expected_data, Value::Null);
}

#[test]
fn default_registry_contains_stock_import() {
    let registry = default_registry();
    let verb = registry
        .get("stock.import")
        .expect("stock.import in default_registry");
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

    let outcome = store.mutate_via_verb("stock.import", args_value_local_minimal(), None);

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
        "stock.import",
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
