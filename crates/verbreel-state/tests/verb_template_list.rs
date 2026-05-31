//! Tests for `template.list` (§16.1) — eighty-fourth production verb.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::template_list::{compute_patch, data_envelope_from_args};
use verbreel_state::{
    Project, ReconstructError, TemplateCanvasHint, TemplateFpsHint, TemplateListArgs,
    TemplateListData, TemplateListEntry, TemplateListError, TemplateListVerb, TemplateSlotKind,
    TemplateSlotSummary, TemplateSource, Verb, VerbError, VerbRegistry, default_fixtures,
    default_registry, validate_reconstructors,
};

#[cfg(feature = "native")]
use verbreel_state::{MutateOutcome, ProjectStore};

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture -> Project")
}

fn fixture_project_id() -> verbreel_types::ProjectId {
    empty_project().id
}

fn args() -> TemplateListArgs {
    TemplateListArgs {
        project_id: fixture_project_id(),
        source: None,
        kind: None,
    }
}

#[test]
fn args_deserialize_ok_with_only_project_id() {
    let raw = json!({ "project_id": FIXTURE_PROJECT_ID });
    let typed: TemplateListArgs =
        serde_json::from_value(raw).expect("project_id alone is sufficient");
    assert_eq!(typed.project_id.to_string(), FIXTURE_PROJECT_ID);
    assert_eq!(typed.source, None);
    assert_eq!(typed.kind, None);
}

#[test]
fn args_deserialize_ok_with_bundled_filter_and_kind() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "source": "bundled",
        "kind": "vertical",
    });
    let typed: TemplateListArgs = serde_json::from_value(raw).expect("source + kind should parse");
    assert_eq!(typed.source, Some(TemplateSource::Bundled));
    assert_eq!(typed.kind.as_deref(), Some("vertical"));
}

#[test]
fn args_deserialize_ok_with_user_filter_and_empty_kind() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "source": "user",
        "kind": "",
    });
    let typed: TemplateListArgs =
        serde_json::from_value(raw).expect("source + empty kind should parse");
    assert_eq!(typed.source, Some(TemplateSource::User));
    assert_eq!(typed.kind.as_deref(), Some(""));
}

#[test]
fn args_missing_project_id_fails_through_verb() {
    let prior = empty_project();
    let verb = TemplateListVerb;

    let err = verb
        .compute_patch(&prior, &json!({}))
        .expect_err("missing project_id should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_non_string_project_id_fails_through_verb() {
    let prior = empty_project();
    let verb = TemplateListVerb;

    let err = verb
        .compute_patch(&prior, &json!({"project_id": 12345}))
        .expect_err("non-string project_id should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_invalid_source_string_maps_to_bad_args() {
    let prior = empty_project();
    let verb = TemplateListVerb;

    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "source": "system",
            }),
        )
        .expect_err("invalid source should map to BadArgs via serde");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_non_string_kind_maps_to_bad_args() {
    let prior = empty_project();
    let verb = TemplateListVerb;

    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "kind": 12,
            }),
        )
        .expect_err("non-string kind should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_unknown_field_rejected_via_deny_unknown_fields() {
    let prior = empty_project();
    let verb = TemplateListVerb;

    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "unexpected": true,
            }),
        )
        .expect_err("unknown fields must be rejected");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn all_well_formed_filter_combinations_return_empty_templates() {
    let prior = empty_project();
    let cases = vec![
        TemplateListArgs {
            project_id: fixture_project_id(),
            source: None,
            kind: None,
        },
        TemplateListArgs {
            project_id: fixture_project_id(),
            source: Some(TemplateSource::Bundled),
            kind: None,
        },
        TemplateListArgs {
            project_id: fixture_project_id(),
            source: Some(TemplateSource::User),
            kind: None,
        },
        TemplateListArgs {
            project_id: fixture_project_id(),
            source: None,
            kind: Some("vertical".to_string()),
        },
        TemplateListArgs {
            project_id: fixture_project_id(),
            source: Some(TemplateSource::Bundled),
            kind: Some("talking-head".to_string()),
        },
        TemplateListArgs {
            project_id: fixture_project_id(),
            source: Some(TemplateSource::User),
            kind: Some(String::new()),
        },
    ];

    for case in cases {
        let (patch, warnings, data) = compute_patch(&prior, &case).expect("well-formed args");
        assert_eq!(patch, json!([]));
        assert!(warnings.is_empty());
        assert!(data.templates.is_empty());
    }
}

#[test]
fn kind_accepts_arbitrary_string() {
    let prior = empty_project();
    let typed = TemplateListArgs {
        project_id: fixture_project_id(),
        source: None,
        kind: Some("THIS Is Any Tag_123".to_string()),
    };
    let (_, _, data) = compute_patch(&prior, &typed).expect("arbitrary kind should be accepted");
    assert!(data.templates.is_empty());
}

#[test]
fn kind_accepts_empty_string() {
    let prior = empty_project();
    let typed = TemplateListArgs {
        project_id: fixture_project_id(),
        source: None,
        kind: Some(String::new()),
    };
    let (_, _, data) = compute_patch(&prior, &typed).expect("empty kind should be accepted");
    assert!(data.templates.is_empty());
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
fn data_envelope_has_only_templates_field() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    let value: Value = serde_json::to_value(data).expect("TemplateListData -> Value");
    let obj = value.as_object().expect("data envelope is an object");
    let keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    assert_eq!(keys, vec!["templates"]);
}

#[test]
fn template_source_wire_literals_are_lowercase() {
    assert_eq!(
        serde_json::to_value(TemplateSource::Bundled).expect("TemplateSource::Bundled -> Value"),
        json!("bundled")
    );
    assert_eq!(
        serde_json::to_value(TemplateSource::User).expect("TemplateSource::User -> Value"),
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
fn template_slot_summary_shape_omits_constraints_and_default_value() {
    let slot = TemplateSlotSummary {
        id: "slot-hero".to_string(),
        name: "Hero".to_string(),
        kind: TemplateSlotKind::Video,
        required: true,
    };
    let value = serde_json::to_value(slot).expect("TemplateSlotSummary -> Value");
    let obj = value.as_object().expect("TemplateSlotSummary is an object");

    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();
    let mut expected = vec!["id", "kind", "name", "required"];
    expected.sort_unstable();
    assert_eq!(keys, expected);
    assert!(!obj.contains_key("constraints"));
    assert!(!obj.contains_key("default_value"));
}

#[test]
fn template_list_entry_serializes_exact_compact_fields() {
    let entry = TemplateListEntry {
        id: "0190b8d3-15e3-7000-bd00-000000000001".to_string(),
        name: "Template".to_string(),
        description: "desc".to_string(),
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
        slots: vec![TemplateSlotSummary {
            id: "slot".to_string(),
            name: "Slot".to_string(),
            kind: TemplateSlotKind::Image,
            required: false,
        }],
    };

    let value = serde_json::to_value(entry).expect("TemplateListEntry -> Value");
    let obj = value.as_object().expect("TemplateListEntry is an object");

    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();
    let mut expected = vec![
        "canvas_hint",
        "description",
        "duration_hint_tk",
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
fn canvas_and_fps_hint_shapes_are_exact() {
    let canvas = serde_json::to_value(TemplateCanvasHint {
        width: 1920,
        height: 1080,
    })
    .expect("TemplateCanvasHint -> Value");
    let canvas_obj = canvas.as_object().expect("canvas_hint is object");
    let canvas_keys: Vec<&str> = canvas_obj.keys().map(String::as_str).collect();
    assert_eq!(canvas_keys, vec!["width", "height"]);

    let fps = serde_json::to_value(TemplateFpsHint {
        num: 30000,
        den: 1001,
    })
    .expect("TemplateFpsHint -> Value");
    let fps_obj = fps.as_object().expect("fps_hint is object");
    let fps_keys: Vec<&str> = fps_obj.keys().map(String::as_str).collect();
    assert_eq!(fps_keys, vec!["num", "den"]);
}

#[test]
fn reserved_eio_display_contains_error_code() {
    let err = TemplateListError::Io {
        detail: "filesystem unavailable".to_string(),
    };
    let message = err.to_string();
    assert!(message.contains("E_IO"));
    assert!(message.contains("filesystem unavailable"));
}

#[test]
fn reserved_eio_maps_to_custom_verb_error() {
    let err = TemplateListError::Io {
        detail: "reserved floor".to_string(),
    };
    let mapped: VerbError = err.into();
    assert!(matches!(mapped, VerbError::Custom(_)));
}

#[test]
fn reconstruct_byte_identical_to_compute_data() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    let rebuilt = data_envelope_from_args(&args(), &prior).expect("rebuild data envelope");

    let lhs = serde_json::to_vec(&data).expect("forward data serializes");
    let rhs = serde_json::to_vec(&rebuilt).expect("reconstructed data serializes");
    assert_eq!(lhs, rhs);
}

#[test]
fn reconstruct_rejects_malformed_args() {
    let prior = empty_project();
    let verb = TemplateListVerb;
    let err = verb
        .reconstruct(&json!({"source":"bundled"}), &json!([]), &[], &prior)
        .expect_err("missing project_id should fail reconstruct args decode");
    assert!(matches!(
        err,
        ReconstructError::TypeMismatch {
            name: "args",
            expected: "TemplateListArgs",
        }
    ));
}

#[test]
fn default_fixture_validates_with_only_template_list_verb() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "template.list")
        .expect("default_fixtures includes template.list");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(TemplateListVerb))
        .expect("register template.list verb");

    let report =
        validate_reconstructors(&registry, &[fixture]).expect("template.list fixture validates");
    assert_eq!(report.verbs_checked, vec!["template.list"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn default_fixture_has_empty_patch_warnings_and_expected_data() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "template.list")
        .expect("default_fixtures includes template.list");
    assert_eq!(fixture.patch, json!([]));
    assert!(fixture.warnings.is_empty());
    assert_eq!(fixture.expected_data, json!({ "templates": [] }));
}

#[test]
fn default_registry_contains_template_list() {
    let registry = default_registry();
    let verb = registry
        .get("template.list")
        .expect("template.list registered in default_registry");
    assert_eq!(verb.verb(), "template.list");

    let prior = empty_project();
    let (patch, data, warnings) = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "source": "bundled",
                "kind": "vertical",
            }),
        )
        .expect("registry route succeeds");
    assert!(patch.is_empty());
    assert!(warnings.is_empty());

    let typed: TemplateListData =
        serde_json::from_value(data).expect("data deserializes to TemplateListData");
    assert!(typed.templates.is_empty());
}

#[cfg(feature = "native")]
#[test]
fn mutate_via_verb_returns_noop_empty_list_envelope() {
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
            "template.list",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "source": "user",
                "kind": "talking-head",
            }),
            None,
        )
        .expect("template.list should route");

    // template.list is read-only (empty patch) → NoOp, no event line.
    let MutateOutcome::NoOp { data, warnings, .. } = outcome else {
        panic!("expected NoOp outcome from read-only template.list, got {outcome:?}");
    };
    assert!(warnings.is_empty());

    let typed: TemplateListData =
        serde_json::from_value(data).expect("template.list data deserializes");
    assert!(typed.templates.is_empty());
}
