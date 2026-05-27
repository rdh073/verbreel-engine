//! Tests for `template.install` (§16.5) — v1 file-installer unavailable floor.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::template_install::compute_patch;
use verbreel_state::{
    Project, ReconstructError, TemplateInstallArgs, TemplateInstallData, TemplateInstallError,
    TemplateInstallVerb, Verb, VerbError, VerbRegistry, default_fixtures, default_registry,
    validate_reconstructors,
};

#[cfg(feature = "native")]
use verbreel_state::{LifecycleError, ProjectStore};

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const TEMPLATE_PATH: &str = "/tmp/template.v1.verbreel-template";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture -> Project")
}

fn fixture_project_id() -> verbreel_types::ProjectId {
    empty_project().id
}

fn args_default() -> TemplateInstallArgs {
    TemplateInstallArgs {
        project_id: fixture_project_id(),
        path: TEMPLATE_PATH.to_string(),
        overwrite: false,
    }
}

fn args_value() -> Value {
    json!({
        "project_id": FIXTURE_PROJECT_ID,
        "path": TEMPLATE_PATH,
    })
}

#[test]
fn args_deserialize_ok_with_required_fields() {
    let typed: TemplateInstallArgs = serde_json::from_value(args_value()).expect("args parse");
    assert_eq!(typed.project_id.to_string(), FIXTURE_PROJECT_ID);
    assert_eq!(typed.path, TEMPLATE_PATH);
    assert!(!typed.overwrite);
}

#[test]
fn overwrite_omitted_defaults_to_false() {
    let typed: TemplateInstallArgs = serde_json::from_value(args_value()).expect("args parse");
    assert!(!typed.overwrite);
}

#[test]
fn overwrite_true_parses() {
    let typed: TemplateInstallArgs = serde_json::from_value(json!({
        "project_id": FIXTURE_PROJECT_ID,
        "path": TEMPLATE_PATH,
        "overwrite": true,
    }))
    .expect("args parse");
    assert!(typed.overwrite);
}

#[test]
fn overwrite_false_parses() {
    let typed: TemplateInstallArgs = serde_json::from_value(json!({
        "project_id": FIXTURE_PROJECT_ID,
        "path": TEMPLATE_PATH,
        "overwrite": false,
    }))
    .expect("args parse");
    assert!(!typed.overwrite);
}

#[test]
fn top_level_unknown_field_rejected_through_verb() {
    let prior = empty_project();
    let verb = TemplateInstallVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "path": TEMPLATE_PATH,
                "unknown": true,
            }),
        )
        .expect_err("unknown field should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn missing_required_fields_fail_through_verb() {
    let prior = empty_project();
    let verb = TemplateInstallVerb;
    let cases = vec![
        json!({ "path": TEMPLATE_PATH }),
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
    let verb = TemplateInstallVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": 123,
                "path": TEMPLATE_PATH,
            }),
        )
        .expect_err("non-string project_id should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn non_string_path_rejected_through_verb() {
    let prior = empty_project();
    let verb = TemplateInstallVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "path": 55,
            }),
        )
        .expect_err("non-string path should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn non_boolean_overwrite_rejected_through_verb() {
    let prior = empty_project();
    let verb = TemplateInstallVerb;
    let cases = vec![
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "path": TEMPLATE_PATH,
            "overwrite": "true",
        }),
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "path": TEMPLATE_PATH,
            "overwrite": 1,
        }),
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "path": TEMPLATE_PATH,
            "overwrite": null,
        }),
    ];

    for raw in cases {
        let err = verb
            .compute_patch(&prior, &raw)
            .expect_err("non-boolean overwrite should map to BadArgs");
        assert!(matches!(err, VerbError::BadArgs { .. }));
    }
}

#[test]
fn arbitrary_path_strings_are_shape_valid_and_hit_io_floor() {
    let prior = empty_project();
    let cases = vec![
        TemplateInstallArgs {
            path: "".to_string(),
            ..args_default()
        },
        TemplateInstallArgs {
            path: "relative/template.verbreel-template".to_string(),
            ..args_default()
        },
        TemplateInstallArgs {
            path: "../outside/template.verbreel-template".to_string(),
            ..args_default()
        },
    ];

    for case in cases {
        let err = compute_patch(&prior, &case).expect_err("v1 floor should always error");
        assert!(matches!(err, TemplateInstallError::Io { .. }));
    }
}

#[test]
fn all_well_formed_calls_return_e_io() {
    let prior = empty_project();
    let cases = vec![
        args_default(),
        TemplateInstallArgs {
            overwrite: true,
            ..args_default()
        },
    ];

    for case in cases {
        let err = compute_patch(&prior, &case).expect_err("v1 floor always errors");
        assert!(matches!(err, TemplateInstallError::Io { .. }));
    }
}

#[test]
fn io_error_text_contains_code_and_path() {
    let prior = empty_project();
    let args = TemplateInstallArgs {
        path: "/tmp/install-floor-check.verbreel-template".to_string(),
        ..args_default()
    };
    let err = compute_patch(&prior, &args).expect_err("v1 floor always errors");
    let msg = err.to_string();
    assert!(msg.contains("E_IO"));
    assert!(msg.contains("/tmp/install-floor-check.verbreel-template"));
}

#[test]
fn io_floor_maps_to_custom_through_verb_and_includes_path() {
    let prior = empty_project();
    let verb = TemplateInstallVerb;
    let err = verb
        .compute_patch(&prior, &args_value())
        .expect_err("well-formed args should hit runtime floor");
    let VerbError::Custom(detail) = err else {
        panic!("expected Custom, got {err:?}");
    };
    assert!(detail.contains("E_IO"));
    assert!(detail.contains(TEMPLATE_PATH));
}

#[test]
fn future_success_data_serializes_exact_spec_fields() {
    let data = TemplateInstallData {
        template_id: "0190b8d3-15e3-7000-bd00-0000feedbeef".to_string(),
        install_path: "/home/user/.verbreel/templates/0190b8d3-15e3-7000-bd00-0000feedbeef"
            .to_string(),
        would_overwrite: None,
    };

    let value = serde_json::to_value(data).expect("TemplateInstallData -> Value");
    let obj = value.as_object().expect("TemplateInstallData is an object");
    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();
    let mut expected = vec!["install_path", "template_id"];
    expected.sort_unstable();
    assert_eq!(keys, expected);
    assert!(!obj.contains_key("would_overwrite"));
}

#[test]
fn future_success_data_includes_would_overwrite_when_present() {
    let data = TemplateInstallData {
        template_id: "0190b8d3-15e3-7000-bd00-0000feedbeef".to_string(),
        install_path: "/tmp/install-target".to_string(),
        would_overwrite: Some(true),
    };

    let value = serde_json::to_value(data).expect("TemplateInstallData -> Value");
    let obj = value.as_object().expect("TemplateInstallData is an object");
    assert_eq!(obj.get("would_overwrite"), Some(&json!(true)));
}

#[test]
fn reserved_error_variants_display_expected_codes() {
    let cases = vec![
        (
            TemplateInstallError::PathEscape {
                detail: "escaped".to_string(),
            },
            "E_PATH_ESCAPE",
        ),
        (
            TemplateInstallError::TemplateSchemaViolation {
                detail: "invalid template".to_string(),
            },
            "E_TEMPLATE_SCHEMA_VIOLATION",
        ),
        (
            TemplateInstallError::Io {
                detail: "runtime unavailable".to_string(),
            },
            "E_IO",
        ),
    ];

    for (error, code) in cases {
        assert!(error.to_string().contains(code));
    }
}

#[test]
fn all_template_install_errors_map_to_custom() {
    let cases = vec![
        TemplateInstallError::PathEscape {
            detail: "escape".to_string(),
        },
        TemplateInstallError::TemplateSchemaViolation {
            detail: "schema".to_string(),
        },
        TemplateInstallError::Io {
            detail: "io".to_string(),
        },
    ];

    for case in cases {
        let mapped: VerbError = case.into();
        assert!(matches!(mapped, VerbError::Custom(_)));
    }
}

#[test]
fn reconstruct_returns_null_for_well_formed_args() {
    let verb = TemplateInstallVerb;
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
    let verb = TemplateInstallVerb;
    let prior = empty_project();
    let err = verb
        .reconstruct(&json!({}), &json!([]), &[], &prior)
        .expect_err("malformed args should fail reconstruction");
    assert!(matches!(
        err,
        ReconstructError::TypeMismatch {
            expected: "TemplateInstallArgs",
            ..
        }
    ));
}

#[test]
fn default_fixture_validates_with_only_template_install_verb() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "template.install")
        .expect("default_fixtures includes template.install");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(TemplateInstallVerb))
        .expect("register template.install verb");

    let report = validate_reconstructors(&registry, &[fixture]).expect("fixture validates");
    assert_eq!(report.verbs_checked, vec!["template.install"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn default_fixture_has_empty_patch_warnings_and_null_expected_data() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "template.install")
        .expect("default_fixtures includes template.install");
    assert_eq!(fixture.patch, json!([]));
    assert!(fixture.warnings.is_empty());
    assert_eq!(fixture.expected_data, Value::Null);
}

#[test]
fn default_registry_contains_template_install() {
    let registry = default_registry();
    let verb = registry
        .get("template.install")
        .expect("template.install in default_registry");
    let prior = empty_project();
    let err = verb
        .compute_patch(&prior, &args_value())
        .expect_err("v1 floor always errors");
    let VerbError::Custom(detail) = err else {
        panic!("expected Custom, got {err:?}");
    };
    assert!(detail.contains("E_IO"));
}

#[cfg(feature = "native")]
#[test]
fn mutate_via_verb_returns_runtime_e_io_floor() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        empty_project(),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry succeeds");

    let outcome = store.mutate_via_verb("template.install", args_value(), None);
    let Err(LifecycleError::VerbExecutionFailed { source, .. }) = outcome else {
        panic!("expected VerbExecutionFailed, got {outcome:?}");
    };
    let VerbError::Custom(detail) = source else {
        panic!("expected Custom source, got {source:?}");
    };
    assert!(detail.contains("E_IO"));
}
