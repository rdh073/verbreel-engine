//! Tests for `template.from_project` (§16.4) — v1 file-writer unavailable floor.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::template_from_project::compute_patch;
use verbreel_state::{
    Project, ReconstructError, TemplateFromProjectArgs, TemplateFromProjectData,
    TemplateFromProjectError, TemplateFromProjectVerb, TemplateSlotClipArg, TemplateSlotTextArg,
    Verb, VerbError, VerbRegistry, default_fixtures, default_registry, validate_reconstructors,
};

#[cfg(feature = "native")]
use verbreel_state::{LifecycleError, ProjectStore};

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const OUT_PATH: &str = "/tmp/template.v1.verbreel-template";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture -> Project")
}

fn fixture_project_id() -> verbreel_types::ProjectId {
    empty_project().id
}

fn args_default() -> TemplateFromProjectArgs {
    TemplateFromProjectArgs {
        project_id: fixture_project_id(),
        out_path: OUT_PATH.to_string(),
        name: "Template Name".to_string(),
        description: String::new(),
        author: String::new(),
        slot_clips: Vec::new(),
        slot_texts: Vec::new(),
        include_slot_defaults: false,
        from_tk: None,
        to_tk: None,
        preview_png: None,
        tags: Vec::new(),
    }
}

fn args_value() -> Value {
    json!({
        "project_id": FIXTURE_PROJECT_ID,
        "out_path": OUT_PATH,
        "name": "Template Name",
    })
}

#[test]
fn args_deserialize_ok_with_required_fields() {
    let typed: TemplateFromProjectArgs = serde_json::from_value(args_value()).expect("args parse");
    assert_eq!(typed.project_id.to_string(), FIXTURE_PROJECT_ID);
    assert_eq!(typed.out_path, OUT_PATH);
    assert_eq!(typed.name, "Template Name");
}

#[test]
fn omitted_optionals_materialize_spec_defaults() {
    let typed: TemplateFromProjectArgs = serde_json::from_value(args_value()).expect("args parse");
    assert_eq!(typed.description, "");
    assert_eq!(typed.author, "");
    assert!(typed.slot_clips.is_empty());
    assert!(typed.slot_texts.is_empty());
    assert!(!typed.include_slot_defaults);
    assert_eq!(typed.from_tk, None);
    assert_eq!(typed.to_tk, None);
    assert_eq!(typed.preview_png, None);
    assert!(typed.tags.is_empty());
}

#[test]
fn args_deserialize_ok_with_all_optionals() {
    let typed: TemplateFromProjectArgs = serde_json::from_value(json!({
        "project_id": FIXTURE_PROJECT_ID,
        "out_path": "/tmp/explicit-template.verbreel-template",
        "name": "Explicit",
        "description": "desc",
        "author": "author",
        "slot_clips": [{ "clip_id": "clip-a", "slot_id": "slot-a", "slot_name": "Slot A" }],
        "slot_texts": [{ "clip_id": "clip-t", "slot_id": "slot-t", "slot_name": "Slot T" }],
        "include_slot_defaults": true,
        "from_tk": -11,
        "to_tk": -9,
        "preview_png": "/tmp/preview.png",
        "tags": ["vertical", "intro"],
    }))
    .expect("args parse");

    assert_eq!(typed.description, "desc");
    assert_eq!(typed.author, "author");
    assert_eq!(typed.slot_clips.len(), 1);
    assert_eq!(typed.slot_texts.len(), 1);
    assert!(typed.include_slot_defaults);
    assert_eq!(typed.from_tk, Some(-11));
    assert_eq!(typed.to_tk, Some(-9));
    assert_eq!(typed.preview_png.as_deref(), Some("/tmp/preview.png"));
    assert_eq!(typed.tags, vec!["vertical", "intro"]);
}

#[test]
fn slot_clips_nested_unknown_field_rejected_through_verb() {
    let prior = empty_project();
    let verb = TemplateFromProjectVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "out_path": OUT_PATH,
                "name": "Template Name",
                "slot_clips": [{
                    "clip_id": "clip-a",
                    "slot_id": "slot-a",
                    "slot_name": "Slot A",
                    "extra": true
                }],
            }),
        )
        .expect_err("nested unknown field should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn slot_texts_nested_unknown_field_rejected_through_verb() {
    let prior = empty_project();
    let verb = TemplateFromProjectVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "out_path": OUT_PATH,
                "name": "Template Name",
                "slot_texts": [{
                    "clip_id": "clip-t",
                    "slot_id": "slot-t",
                    "slot_name": "Slot T",
                    "extra": true
                }],
            }),
        )
        .expect_err("nested unknown field should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn top_level_unknown_field_rejected_through_verb() {
    let prior = empty_project();
    let verb = TemplateFromProjectVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "out_path": OUT_PATH,
                "name": "Template Name",
                "unexpected": true,
            }),
        )
        .expect_err("top-level unknown field should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn missing_required_fields_fail_through_verb() {
    let prior = empty_project();
    let verb = TemplateFromProjectVerb;
    let cases = vec![
        json!({
            "out_path": OUT_PATH,
            "name": "Template Name",
        }),
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "name": "Template Name",
        }),
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "out_path": OUT_PATH,
        }),
    ];

    for raw in cases {
        let err = verb
            .compute_patch(&prior, &raw)
            .expect_err("missing required field should map to BadArgs");
        assert!(matches!(err, VerbError::BadArgs { .. }));
    }
}

#[test]
fn non_string_required_fields_fail_through_verb() {
    let prior = empty_project();
    let verb = TemplateFromProjectVerb;
    let cases = vec![
        json!({
            "project_id": 7,
            "out_path": OUT_PATH,
            "name": "Template Name",
        }),
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "out_path": 88,
            "name": "Template Name",
        }),
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "out_path": OUT_PATH,
            "name": false,
        }),
    ];

    for raw in cases {
        let err = verb
            .compute_patch(&prior, &raw)
            .expect_err("non-string required field should map to BadArgs");
        assert!(matches!(err, VerbError::BadArgs { .. }));
    }
}

#[test]
fn non_array_slot_and_tag_fields_fail_through_verb() {
    let prior = empty_project();
    let verb = TemplateFromProjectVerb;
    let cases = vec![
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "out_path": OUT_PATH,
            "name": "Template Name",
            "slot_clips": {},
        }),
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "out_path": OUT_PATH,
            "name": "Template Name",
            "slot_texts": "not-array",
        }),
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "out_path": OUT_PATH,
            "name": "Template Name",
            "tags": {"bad": true},
        }),
    ];

    for raw in cases {
        let err = verb
            .compute_patch(&prior, &raw)
            .expect_err("non-array slot/tags field should map to BadArgs");
        assert!(matches!(err, VerbError::BadArgs { .. }));
    }
}

#[test]
fn non_string_slot_marker_fields_fail_through_verb() {
    let prior = empty_project();
    let verb = TemplateFromProjectVerb;
    let cases = vec![
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "out_path": OUT_PATH,
            "name": "Template Name",
            "slot_clips": [{ "clip_id": 1, "slot_id": "slot-a", "slot_name": "Slot A" }],
        }),
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "out_path": OUT_PATH,
            "name": "Template Name",
            "slot_clips": [{ "clip_id": "clip-a", "slot_id": 2, "slot_name": "Slot A" }],
        }),
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "out_path": OUT_PATH,
            "name": "Template Name",
            "slot_clips": [{ "clip_id": "clip-a", "slot_id": "slot-a", "slot_name": 3 }],
        }),
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "out_path": OUT_PATH,
            "name": "Template Name",
            "slot_texts": [{ "clip_id": 4, "slot_id": "slot-t", "slot_name": "Slot T" }],
        }),
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "out_path": OUT_PATH,
            "name": "Template Name",
            "slot_texts": [{ "clip_id": "clip-t", "slot_id": 5, "slot_name": "Slot T" }],
        }),
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "out_path": OUT_PATH,
            "name": "Template Name",
            "slot_texts": [{ "clip_id": "clip-t", "slot_id": "slot-t", "slot_name": 6 }],
        }),
    ];

    for raw in cases {
        let err = verb
            .compute_patch(&prior, &raw)
            .expect_err("non-string slot marker field should map to BadArgs");
        assert!(matches!(err, VerbError::BadArgs { .. }));
    }
}

#[test]
fn non_bool_include_slot_defaults_rejected_through_verb() {
    let prior = empty_project();
    let verb = TemplateFromProjectVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "out_path": OUT_PATH,
                "name": "Template Name",
                "include_slot_defaults": "true",
            }),
        )
        .expect_err("non-bool include_slot_defaults should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn non_integer_from_to_ticks_rejected_through_verb() {
    let prior = empty_project();
    let verb = TemplateFromProjectVerb;
    let cases = vec![
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "out_path": OUT_PATH,
            "name": "Template Name",
            "from_tk": "0",
        }),
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "out_path": OUT_PATH,
            "name": "Template Name",
            "to_tk": 1.25,
        }),
    ];

    for raw in cases {
        let err = verb
            .compute_patch(&prior, &raw)
            .expect_err("non-integer ticks should map to BadArgs");
        assert!(matches!(err, VerbError::BadArgs { .. }));
    }
}

#[test]
fn all_well_formed_calls_return_e_io_floor() {
    let prior = empty_project();
    let cases = vec![
        args_default(),
        TemplateFromProjectArgs {
            out_path: "/tmp/opaque-path-value".to_string(),
            name: String::new(),
            ..args_default()
        },
        TemplateFromProjectArgs {
            slot_clips: vec![TemplateSlotClipArg {
                clip_id: String::new(),
                slot_id: "UPPER slot:id".to_string(),
                slot_name: "Slot With Spaces".to_string(),
            }],
            slot_texts: vec![TemplateSlotTextArg {
                clip_id: "text-clip".to_string(),
                slot_id: "slot:text".to_string(),
                slot_name: String::new(),
            }],
            include_slot_defaults: true,
            from_tk: Some(-999),
            to_tk: Some(-1000),
            preview_png: Some("/tmp/preview.png".to_string()),
            tags: vec!["one".to_string(), "two".to_string()],
            ..args_default()
        },
    ];

    for case in cases {
        let err = compute_patch(&prior, &case).expect_err("v1 floor should always error");
        assert!(matches!(err, TemplateFromProjectError::Io { .. }));
    }
}

#[test]
fn io_error_text_contains_code_and_out_path() {
    let prior = empty_project();
    let args = TemplateFromProjectArgs {
        out_path: "/tmp/floor-check.verbreel-template".to_string(),
        ..args_default()
    };
    let err = compute_patch(&prior, &args).expect_err("v1 floor always errors");
    let msg = err.to_string();
    assert!(msg.contains("E_IO"));
    assert!(msg.contains("/tmp/floor-check.verbreel-template"));
}

#[test]
fn io_floor_maps_to_custom_error_through_verb() {
    let prior = empty_project();
    let verb = TemplateFromProjectVerb;
    let err = verb
        .compute_patch(&prior, &args_value())
        .expect_err("well-formed args should hit runtime floor");
    let VerbError::Custom(detail) = err else {
        panic!("expected Custom, got {err:?}");
    };
    assert!(detail.contains("E_IO"));
    assert!(detail.contains(OUT_PATH));
}

#[test]
fn future_success_data_serializes_exact_spec_fields() {
    let data = TemplateFromProjectData {
        template_id: "0190b8d3-15e3-7000-bd00-0000feedbeef".to_string(),
        out_path: "/tmp/output.verbreel-template".to_string(),
        slot_count: 3,
        embedded_asset_count: 4,
        embedded_asset_bytes: 1024,
        bytes_written: 2048,
    };

    let value = serde_json::to_value(data).expect("TemplateFromProjectData -> Value");
    let obj = value
        .as_object()
        .expect("TemplateFromProjectData is an object");
    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();
    let mut expected = vec![
        "bytes_written",
        "embedded_asset_bytes",
        "embedded_asset_count",
        "out_path",
        "slot_count",
        "template_id",
    ];
    expected.sort_unstable();
    assert_eq!(keys, expected);
}

#[test]
fn reserved_error_variants_display_expected_codes() {
    let cases = vec![
        (
            TemplateFromProjectError::PathEscape {
                detail: "escaped".to_string(),
            },
            "E_PATH_ESCAPE",
        ),
        (
            TemplateFromProjectError::OutPathExists {
                out_path: "/tmp/existing.verbreel-template".to_string(),
            },
            "E_OUT_PATH_EXISTS",
        ),
        (
            TemplateFromProjectError::TemplateSchemaViolation {
                detail: "duplicate slot".to_string(),
            },
            "E_TEMPLATE_SCHEMA_VIOLATION",
        ),
        (
            TemplateFromProjectError::NotFound {
                detail: "clip not found".to_string(),
            },
            "E_NOT_FOUND",
        ),
        (
            TemplateFromProjectError::BadTime {
                detail: "to_tk <= from_tk".to_string(),
            },
            "E_BAD_TIME",
        ),
        (
            TemplateFromProjectError::Io {
                detail: "writer unavailable".to_string(),
            },
            "E_IO",
        ),
    ];

    for (error, code) in cases {
        assert!(error.to_string().contains(code));
    }
}

#[test]
fn all_template_from_project_errors_map_to_custom() {
    let cases = vec![
        TemplateFromProjectError::PathEscape {
            detail: "escape".to_string(),
        },
        TemplateFromProjectError::OutPathExists {
            out_path: "/tmp/existing.verbreel-template".to_string(),
        },
        TemplateFromProjectError::TemplateSchemaViolation {
            detail: "schema".to_string(),
        },
        TemplateFromProjectError::NotFound {
            detail: "missing".to_string(),
        },
        TemplateFromProjectError::BadTime {
            detail: "bad time".to_string(),
        },
        TemplateFromProjectError::Io {
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
    let verb = TemplateFromProjectVerb;
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
    let verb = TemplateFromProjectVerb;
    let prior = empty_project();
    let err = verb
        .reconstruct(&json!({}), &json!([]), &[], &prior)
        .expect_err("malformed args should fail reconstruction");
    assert!(matches!(
        err,
        ReconstructError::TypeMismatch {
            expected: "TemplateFromProjectArgs",
            ..
        }
    ));
}

#[test]
fn default_fixture_validates_with_only_template_from_project_verb() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "template.from_project")
        .expect("default_fixtures includes template.from_project");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(TemplateFromProjectVerb))
        .expect("register template.from_project verb");

    let report = validate_reconstructors(&registry, &[fixture]).expect("fixture validates");
    assert_eq!(report.verbs_checked, vec!["template.from_project"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn default_fixture_has_empty_patch_warnings_and_null_expected_data() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "template.from_project")
        .expect("default_fixtures includes template.from_project");
    assert_eq!(fixture.patch, json!([]));
    assert!(fixture.warnings.is_empty());
    assert_eq!(fixture.expected_data, Value::Null);
}

#[test]
fn default_registry_contains_template_from_project() {
    let registry = default_registry();
    let verb = registry
        .get("template.from_project")
        .expect("template.from_project in default_registry");
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

    let outcome = store.mutate_via_verb("template.from_project", args_value(), None);
    let Err(LifecycleError::VerbExecutionFailed { source, .. }) = outcome else {
        panic!("expected VerbExecutionFailed, got {outcome:?}");
    };
    let VerbError::Custom(detail) = source else {
        panic!("expected Custom source, got {source:?}");
    };
    assert!(detail.contains("E_IO"));
}
