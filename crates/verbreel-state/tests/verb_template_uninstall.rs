//! Tests for `template.uninstall` (§16.6) — v1 template not-found floor.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::template_uninstall::compute_patch;
use verbreel_state::{
    Project, ReconstructError, TemplateUninstallArgs, TemplateUninstallData,
    TemplateUninstallError, TemplateUninstallVerb, Verb, VerbError, VerbRegistry, default_fixtures,
    default_registry, validate_reconstructors,
};

#[cfg(feature = "native")]
use verbreel_state::{LifecycleError, ProjectStore};

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const A_VALID_TEMPLATE_ID: &str = "0190b8d3-15e3-7000-bd00-0000feedbeef";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture -> Project")
}

fn fixture_project_id() -> verbreel_types::ProjectId {
    empty_project().id
}

fn args_default() -> TemplateUninstallArgs {
    TemplateUninstallArgs {
        project_id: fixture_project_id(),
        template_id: A_VALID_TEMPLATE_ID.to_string(),
    }
}

fn args_value() -> Value {
    json!({
        "project_id": FIXTURE_PROJECT_ID,
        "template_id": A_VALID_TEMPLATE_ID,
    })
}

#[test]
fn args_deserialize_ok_with_required_fields() {
    let typed: TemplateUninstallArgs = serde_json::from_value(args_value()).expect("args parse");
    assert_eq!(typed.project_id.to_string(), FIXTURE_PROJECT_ID);
    assert_eq!(typed.template_id, A_VALID_TEMPLATE_ID);
}

#[test]
fn top_level_unknown_field_rejected_through_verb() {
    let prior = empty_project();
    let verb = TemplateUninstallVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "template_id": A_VALID_TEMPLATE_ID,
                "unknown": true,
            }),
        )
        .expect_err("unknown field should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn missing_required_fields_fail_through_verb() {
    let prior = empty_project();
    let verb = TemplateUninstallVerb;
    let cases = [
        json!({ "template_id": A_VALID_TEMPLATE_ID }),
        json!({ "project_id": FIXTURE_PROJECT_ID }),
        json!({}),
    ];

    for raw in cases {
        let err = verb
            .compute_patch(&prior, &raw)
            .expect_err("missing required field should map to BadArgs");
        assert!(matches!(err, VerbError::BadArgs { .. }));
    }
}

#[test]
fn non_string_project_id_rejected_through_verb() {
    let prior = empty_project();
    let verb = TemplateUninstallVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": 123,
                "template_id": A_VALID_TEMPLATE_ID,
            }),
        )
        .expect_err("non-string project_id should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn non_string_template_id_rejected_through_verb() {
    let prior = empty_project();
    let verb = TemplateUninstallVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "template_id": 42,
            }),
        )
        .expect_err("non-string template_id should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn empty_template_id_is_shape_valid_and_hits_not_found_floor() {
    let prior = empty_project();
    let err = compute_patch(
        &prior,
        &TemplateUninstallArgs {
            template_id: String::new(),
            ..args_default()
        },
    )
    .expect_err("well-formed args should hit runtime floor");
    let TemplateUninstallError::TemplateNotFound { template_id } = err else {
        panic!("expected TemplateNotFound, got {err:?}");
    };
    assert_eq!(template_id, "");
}

#[test]
fn arbitrary_template_ids_are_shape_valid_and_hit_not_found_floor() {
    let prior = empty_project();
    for template_id in [A_VALID_TEMPLATE_ID, "hero-template", "UPPER+mixed id"] {
        let err = compute_patch(
            &prior,
            &TemplateUninstallArgs {
                template_id: template_id.to_string(),
                ..args_default()
            },
        )
        .expect_err("well-formed args should hit runtime floor");
        let TemplateUninstallError::TemplateNotFound {
            template_id: returned,
        } = err
        else {
            panic!("expected TemplateNotFound, got {err:?}");
        };
        assert_eq!(returned, template_id);
    }
}

#[test]
fn all_well_formed_calls_return_template_not_found() {
    let prior = empty_project();
    let cases = [
        args_default(),
        TemplateUninstallArgs {
            template_id: "".to_string(),
            ..args_default()
        },
    ];

    for case in cases {
        let err = compute_patch(&prior, &case).expect_err("v1 floor always errors");
        assert!(matches!(
            err,
            TemplateUninstallError::TemplateNotFound { .. }
        ));
    }
}

#[test]
fn template_not_found_error_detail_contains_code_and_template_id() {
    let prior = empty_project();
    let missing_id = "missing-template";
    let err = compute_patch(
        &prior,
        &TemplateUninstallArgs {
            template_id: missing_id.to_string(),
            ..args_default()
        },
    )
    .expect_err("v1 floor always errors");
    let msg = err.to_string();
    assert!(msg.contains("E_TEMPLATE_NOT_FOUND"));
    assert!(msg.contains(missing_id));
}

#[test]
fn template_not_found_maps_to_custom_through_verb_and_includes_template_id() {
    let prior = empty_project();
    let verb = TemplateUninstallVerb;
    let err = verb
        .compute_patch(&prior, &args_value())
        .expect_err("well-formed args should hit runtime floor");
    let VerbError::Custom(detail) = err else {
        panic!("expected Custom, got {err:?}");
    };
    assert!(detail.contains("E_TEMPLATE_NOT_FOUND"));
    assert!(detail.contains(A_VALID_TEMPLATE_ID));
}

#[test]
fn future_success_data_serializes_exact_spec_fields() {
    let data = TemplateUninstallData {
        template_id: A_VALID_TEMPLATE_ID.to_string(),
        removed_path: "/home/user/.verbreel/templates/0190b8d3-15e3-7000-bd00-0000feedbeef"
            .to_string(),
    };

    let value = serde_json::to_value(data).expect("TemplateUninstallData -> Value");
    let obj = value
        .as_object()
        .expect("TemplateUninstallData is an object");
    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();
    let mut expected = vec!["removed_path", "template_id"];
    expected.sort_unstable();
    assert_eq!(keys, expected);
}

#[test]
fn reserved_error_variants_display_expected_codes() {
    let cases = vec![
        (
            TemplateUninstallError::TemplateNotFound {
                template_id: "missing".to_string(),
            },
            "E_TEMPLATE_NOT_FOUND",
        ),
        (
            TemplateUninstallError::TemplateBundledImmutable {
                template_id: "bundled-template".to_string(),
                template_source: "bundled".to_string(),
                install_path: "/opt/verbreel/templates/bundled-template".to_string(),
            },
            "E_TEMPLATE_BUNDLED_IMMUTABLE",
        ),
        (
            TemplateUninstallError::Io {
                detail: "io failure".to_string(),
            },
            "E_IO",
        ),
    ];

    for (error, code) in cases {
        assert!(error.to_string().contains(code));
    }
}

#[test]
fn all_template_uninstall_errors_map_to_custom() {
    let cases = vec![
        TemplateUninstallError::TemplateNotFound {
            template_id: "missing".to_string(),
        },
        TemplateUninstallError::TemplateBundledImmutable {
            template_id: "bundled-template".to_string(),
            template_source: "bundled".to_string(),
            install_path: "/opt/verbreel/templates/bundled-template".to_string(),
        },
        TemplateUninstallError::Io {
            detail: "io failure".to_string(),
        },
    ];

    for case in cases {
        let mapped: VerbError = case.into();
        assert!(matches!(mapped, VerbError::Custom(_)));
    }
}

#[test]
fn reconstruct_returns_null_for_well_formed_args() {
    let verb = TemplateUninstallVerb;
    let prior = empty_project();
    let data = verb
        .reconstruct(
            &serde_json::to_value(args_default()).expect("args serialize"),
            &json!([]),
            &[],
            &prior,
        )
        .expect("reconstruct succeeds");
    assert_eq!(data, Value::Null);
}

#[test]
fn reconstruct_rejects_malformed_args() {
    let verb = TemplateUninstallVerb;
    let prior = empty_project();
    let err = verb
        .reconstruct(&json!({}), &json!([]), &[], &prior)
        .expect_err("malformed args should fail reconstruction");
    assert!(matches!(
        err,
        ReconstructError::TypeMismatch {
            expected: "TemplateUninstallArgs",
            ..
        }
    ));
}

#[test]
fn default_fixture_validates_with_only_template_uninstall_verb() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "template.uninstall")
        .expect("default_fixtures includes template.uninstall");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(TemplateUninstallVerb))
        .expect("register template.uninstall verb");

    let report = validate_reconstructors(&registry, &[fixture]).expect("fixture validates");
    assert_eq!(report.verbs_checked, vec!["template.uninstall"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn default_fixture_has_empty_patch_warnings_and_null_expected_data() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "template.uninstall")
        .expect("default_fixtures includes template.uninstall");
    assert_eq!(fixture.patch, json!([]));
    assert!(fixture.warnings.is_empty());
    assert_eq!(fixture.expected_data, Value::Null);
}

#[test]
fn default_registry_contains_template_uninstall() {
    let registry = default_registry();
    let verb = registry
        .get("template.uninstall")
        .expect("template.uninstall in default_registry");
    let prior = empty_project();
    let err = verb
        .compute_patch(&prior, &args_value())
        .expect_err("v1 floor always errors");
    let VerbError::Custom(detail) = err else {
        panic!("expected Custom, got {err:?}");
    };
    assert!(detail.contains("E_TEMPLATE_NOT_FOUND"));
}

#[cfg(feature = "native")]
#[test]
fn mutate_via_verb_returns_runtime_template_not_found_floor() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        empty_project(),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry succeeds");

    let outcome = store.mutate_via_verb("template.uninstall", args_value(), None);
    let Err(LifecycleError::VerbExecutionFailed { source, .. }) = outcome else {
        panic!("expected VerbExecutionFailed, got {outcome:?}");
    };
    let VerbError::Custom(detail) = source else {
        panic!("expected Custom source, got {source:?}");
    };
    assert!(detail.contains("E_TEMPLATE_NOT_FOUND"));
}
