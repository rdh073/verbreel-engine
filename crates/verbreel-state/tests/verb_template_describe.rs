//! Tests for `template.describe` (§16.2) — v1 template not-found floor.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::template_describe::compute_patch;
use verbreel_state::{
    Project, ReconstructError, TemplateCanvasHint, TemplateDescribeArgs, TemplateDescribeData,
    TemplateDescribeError, TemplateDescribeVerb, TemplateFpsHint, TemplateSlotConstraints,
    TemplateSlotDescriptor, TemplateSlotKind, TemplateSource, Verb, VerbError, VerbRegistry,
    default_fixtures, default_registry, validate_reconstructors,
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

fn args_default() -> TemplateDescribeArgs {
    TemplateDescribeArgs {
        project_id: fixture_project_id(),
        template_id: A_VALID_TEMPLATE_ID.to_string(),
    }
}

#[test]
fn args_deserialize_ok_with_required_fields() {
    let typed: TemplateDescribeArgs = serde_json::from_value(json!({
        "project_id": FIXTURE_PROJECT_ID,
        "template_id": A_VALID_TEMPLATE_ID
    }))
    .expect("args parse");
    assert_eq!(typed.project_id.to_string(), FIXTURE_PROJECT_ID);
    assert_eq!(typed.template_id, A_VALID_TEMPLATE_ID);
}

#[test]
fn missing_required_fields_fail_through_verb() {
    let prior = empty_project();
    let verb = TemplateDescribeVerb;
    let cases = [
        json!({ "template_id": A_VALID_TEMPLATE_ID }),
        json!({ "project_id": FIXTURE_PROJECT_ID }),
    ];

    for raw in cases {
        let err = verb
            .compute_patch(&prior, &raw)
            .expect_err("missing required field should map to BadArgs");
        assert!(matches!(err, VerbError::BadArgs { .. }));
    }
}

#[test]
fn non_string_project_id_fails_through_verb() {
    let prior = empty_project();
    let verb = TemplateDescribeVerb;

    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": 1234,
                "template_id": A_VALID_TEMPLATE_ID
            }),
        )
        .expect_err("non-string project_id should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn non_string_template_id_fails_through_verb() {
    let prior = empty_project();
    let verb = TemplateDescribeVerb;

    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "template_id": 42
            }),
        )
        .expect_err("non-string template_id should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn unknown_field_rejected_through_verb() {
    let prior = empty_project();
    let verb = TemplateDescribeVerb;

    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "template_id": A_VALID_TEMPLATE_ID,
                "extra": true
            }),
        )
        .expect_err("deny_unknown_fields rejects extras");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn well_formed_template_ids_always_reach_template_not_found_floor() {
    let prior = empty_project();
    for template_id in [A_VALID_TEMPLATE_ID, "", "hero-template", "UPPER+mixed id"] {
        let err = compute_patch(
            &prior,
            &TemplateDescribeArgs {
                template_id: template_id.to_string(),
                ..args_default()
            },
        )
        .expect_err("v1 floor should miss every template id");
        let TemplateDescribeError::TemplateNotFound { template_id: id } = err;
        assert_eq!(id, template_id);
    }
}

#[test]
fn runtime_template_not_found_maps_to_custom_and_includes_template_id() {
    let prior = empty_project();
    let verb = TemplateDescribeVerb;

    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "template_id": A_VALID_TEMPLATE_ID
            }),
        )
        .expect_err("well-formed args should hit runtime floor");
    let VerbError::Custom(detail) = err else {
        panic!("expected Custom, got {err:?}");
    };
    assert!(detail.contains("E_TEMPLATE_NOT_FOUND"));
    assert!(detail.contains(A_VALID_TEMPLATE_ID));
}

#[test]
fn template_not_found_error_detail_contains_code_and_id() {
    let prior = empty_project();
    let missing_id = "custom-template-id";
    let err = compute_patch(
        &prior,
        &TemplateDescribeArgs {
            template_id: missing_id.to_string(),
            ..args_default()
        },
    )
    .expect_err("v1 floor should miss every id");
    let msg = err.to_string();
    assert!(msg.contains("E_TEMPLATE_NOT_FOUND"));
    assert!(msg.contains(missing_id));
}

#[test]
fn future_success_data_serializes_exact_spec_fields() {
    let data = TemplateDescribeData {
        id: "0190b8d3-15e3-7000-bd00-0000feedbeef".to_string(),
        name: "Hero Intro".to_string(),
        description: "desc".to_string(),
        author: "author".to_string(),
        preview_thumbnail_path: "/tmp/preview.png".to_string(),
        source: TemplateSource::Bundled,
        install_path: "/tmp/install".to_string(),
        template_schema_version: "1.0.0".to_string(),
        project_graph_schema_version: "1.1.0".to_string(),
        duration_hint_tk: 240_000,
        canvas_hint: TemplateCanvasHint {
            width: 1080,
            height: 1920,
        },
        fps_hint: TemplateFpsHint { num: 30, den: 1 },
        tags: vec!["vertical".to_string()],
        slots: vec![TemplateSlotDescriptor {
            id: "slot_headline".to_string(),
            name: "Headline".to_string(),
            description: "Main title".to_string(),
            kind: TemplateSlotKind::Text,
            required: false,
            default_value: Some("Hello".to_string()),
            constraints: Some(TemplateSlotConstraints {
                min_duration_tk: None,
                max_duration_tk: None,
                aspect_ratio_hint: None,
                max_chars: Some(64),
            }),
        }],
        embedded_asset_count: 3,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        engine_version_hint: "1.1.0".to_string(),
    };

    let value = serde_json::to_value(data).expect("TemplateDescribeData -> Value");
    let obj = value
        .as_object()
        .expect("TemplateDescribeData is an object");
    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();
    let mut expected = vec![
        "author",
        "canvas_hint",
        "created_at",
        "description",
        "duration_hint_tk",
        "embedded_asset_count",
        "engine_version_hint",
        "fps_hint",
        "id",
        "install_path",
        "name",
        "preview_thumbnail_path",
        "project_graph_schema_version",
        "slots",
        "source",
        "tags",
        "template_schema_version",
    ];
    expected.sort_unstable();
    assert_eq!(keys, expected);
}

#[test]
fn slot_descriptor_serializes_description_field() {
    let slot = TemplateSlotDescriptor {
        id: "slot_headline".to_string(),
        name: "Headline".to_string(),
        description: "Shown on first frame".to_string(),
        kind: TemplateSlotKind::Text,
        required: true,
        default_value: None,
        constraints: None,
    };

    let value = serde_json::to_value(slot).expect("TemplateSlotDescriptor -> Value");
    assert_eq!(
        value.get("description"),
        Some(&json!("Shown on first frame"))
    );
}

#[test]
fn slot_descriptor_omits_optional_default_value_and_constraints_when_absent() {
    let slot = TemplateSlotDescriptor {
        id: "slot_video".to_string(),
        name: "Video".to_string(),
        description: "Replace me".to_string(),
        kind: TemplateSlotKind::Video,
        required: true,
        default_value: None,
        constraints: None,
    };

    let value = serde_json::to_value(slot).expect("TemplateSlotDescriptor -> Value");
    let obj = value
        .as_object()
        .expect("TemplateSlotDescriptor is an object");
    assert!(!obj.contains_key("default_value"));
    assert!(!obj.contains_key("constraints"));
}

#[test]
fn slot_descriptor_includes_default_value_and_constraints_when_present() {
    let slot = TemplateSlotDescriptor {
        id: "slot_text".to_string(),
        name: "Headline".to_string(),
        description: "Replace me".to_string(),
        kind: TemplateSlotKind::Text,
        required: false,
        default_value: Some("Default".to_string()),
        constraints: Some(TemplateSlotConstraints {
            min_duration_tk: None,
            max_duration_tk: None,
            aspect_ratio_hint: None,
            max_chars: Some(24),
        }),
    };

    let value = serde_json::to_value(slot).expect("TemplateSlotDescriptor -> Value");
    let obj = value
        .as_object()
        .expect("TemplateSlotDescriptor is an object");
    assert_eq!(obj.get("default_value"), Some(&json!("Default")));
    assert!(obj.contains_key("constraints"));
}

#[test]
fn constraints_optional_members_omit_when_absent() {
    let constraints = TemplateSlotConstraints {
        min_duration_tk: None,
        max_duration_tk: None,
        aspect_ratio_hint: None,
        max_chars: None,
    };

    let value = serde_json::to_value(constraints).expect("TemplateSlotConstraints -> Value");
    let obj = value
        .as_object()
        .expect("TemplateSlotConstraints is an object");
    assert!(obj.is_empty());
}

#[test]
fn constraints_optional_members_serialize_when_present() {
    let constraints = TemplateSlotConstraints {
        min_duration_tk: Some(120_000),
        max_duration_tk: Some(480_000),
        aspect_ratio_hint: Some("9:16".to_string()),
        max_chars: Some(64),
    };

    let value = serde_json::to_value(constraints).expect("TemplateSlotConstraints -> Value");
    let obj = value
        .as_object()
        .expect("TemplateSlotConstraints is an object");
    assert_eq!(obj.get("min_duration_tk"), Some(&json!(120_000)));
    assert_eq!(obj.get("max_duration_tk"), Some(&json!(480_000)));
    assert_eq!(obj.get("aspect_ratio_hint"), Some(&json!("9:16")));
    assert_eq!(obj.get("max_chars"), Some(&json!(64)));
}

#[test]
fn template_source_wire_literals_are_lowercase() {
    assert_eq!(
        serde_json::to_value(TemplateSource::Bundled).expect("Bundled -> Value"),
        json!("bundled")
    );
    assert_eq!(
        serde_json::to_value(TemplateSource::User).expect("User -> Value"),
        json!("user")
    );
}

#[test]
fn template_slot_kind_wire_literals_are_lowercase() {
    assert_eq!(
        serde_json::to_value(TemplateSlotKind::Video).expect("Video -> Value"),
        json!("video")
    );
    assert_eq!(
        serde_json::to_value(TemplateSlotKind::Audio).expect("Audio -> Value"),
        json!("audio")
    );
    assert_eq!(
        serde_json::to_value(TemplateSlotKind::Image).expect("Image -> Value"),
        json!("image")
    );
    assert_eq!(
        serde_json::to_value(TemplateSlotKind::Text).expect("Text -> Value"),
        json!("text")
    );
}

#[test]
fn reconstruct_returns_null_for_well_formed_args() {
    let verb = TemplateDescribeVerb;
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
    let verb = TemplateDescribeVerb;
    let prior = empty_project();
    let err = verb
        .reconstruct(&json!({}), &json!([]), &[], &prior)
        .expect_err("malformed args should fail reconstruction");
    assert!(matches!(
        err,
        ReconstructError::TypeMismatch {
            expected: "TemplateDescribeArgs",
            ..
        }
    ));
}

#[test]
fn default_fixture_validates_with_only_template_describe_verb() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "template.describe")
        .expect("default_fixtures includes template.describe");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(TemplateDescribeVerb))
        .expect("register template.describe verb");

    let report = validate_reconstructors(&registry, &[fixture]).expect("fixture validates");
    assert_eq!(report.verbs_checked, vec!["template.describe"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn default_fixture_has_empty_patch_warnings_and_null_expected_data() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "template.describe")
        .expect("default_fixtures includes template.describe");
    assert_eq!(fixture.patch, json!([]));
    assert!(fixture.warnings.is_empty());
    assert_eq!(fixture.expected_data, Value::Null);
}

#[test]
fn default_registry_contains_template_describe() {
    let registry = default_registry();
    let verb = registry
        .get("template.describe")
        .expect("template.describe in default_registry");
    let prior = empty_project();
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "template_id": A_VALID_TEMPLATE_ID
            }),
        )
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

    let outcome = store.mutate_via_verb(
        "template.describe",
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "template_id": A_VALID_TEMPLATE_ID
        }),
        None,
    );
    let Err(LifecycleError::VerbExecutionFailed { source, .. }) = outcome else {
        panic!("expected VerbExecutionFailed, got {outcome:?}");
    };
    let VerbError::Custom(detail) = source else {
        panic!("expected Custom source, got {source:?}");
    };
    assert!(detail.contains("E_TEMPLATE_NOT_FOUND"));
}
