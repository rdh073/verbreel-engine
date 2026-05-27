//! Tests for `caption.export` (§10.6) — ninety-third production verb.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::caption_export::{compute_patch, resolved_format};
use verbreel_state::{
    CaptionExportArgs, CaptionExportData, CaptionExportError, CaptionExportFormat,
    CaptionExportVerb, Project, Verb, VerbError, VerbRegistry, default_fixtures, default_registry,
    validate_reconstructors,
};
use verbreel_types::ProjectId;

#[cfg(feature = "native")]
use verbreel_state::{LifecycleError, ProjectStore};

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const TEXT_TRACK_SELECTOR: &str = "track:text[0]";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture -> Project")
}

fn fixture_project_id() -> ProjectId {
    empty_project().id
}

fn args_default() -> CaptionExportArgs {
    CaptionExportArgs {
        project_id: fixture_project_id(),
        text_track: TEXT_TRACK_SELECTOR.to_string(),
        out_path: "captions.srt".to_string(),
        format: None,
        overwrite: false,
    }
}

fn args_value() -> Value {
    json!({
        "project_id": FIXTURE_PROJECT_ID,
        "text_track": TEXT_TRACK_SELECTOR,
        "out_path": "captions.srt",
    })
}

#[test]
fn args_deserialize_ok_with_all_fields() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "text_track": TEXT_TRACK_SELECTOR,
        "out_path": "captions.vtt",
        "format": "ass",
        "overwrite": true,
    });

    let typed: CaptionExportArgs = serde_json::from_value(raw).expect("args parse");
    assert_eq!(typed.project_id.to_string(), FIXTURE_PROJECT_ID);
    assert_eq!(typed.text_track, TEXT_TRACK_SELECTOR);
    assert_eq!(typed.out_path, "captions.vtt");
    assert_eq!(typed.format, Some(CaptionExportFormat::Ass));
    assert!(typed.overwrite);
}

#[test]
fn args_deserialize_ok_with_omitted_optionals() {
    let typed: CaptionExportArgs = serde_json::from_value(args_value()).expect("args parse");
    assert_eq!(typed.project_id.to_string(), FIXTURE_PROJECT_ID);
    assert_eq!(typed.text_track, TEXT_TRACK_SELECTOR);
    assert_eq!(typed.out_path, "captions.srt");
    assert_eq!(typed.format, None);
    assert!(!typed.overwrite);
}

#[test]
fn overwrite_omitted_resolves_false() {
    let typed: CaptionExportArgs = serde_json::from_value(args_value()).expect("args parse");
    assert!(!typed.overwrite);
}

#[test]
fn overwrite_true_parses() {
    let typed: CaptionExportArgs = serde_json::from_value(json!({
        "project_id": FIXTURE_PROJECT_ID,
        "text_track": TEXT_TRACK_SELECTOR,
        "out_path": "captions.srt",
        "overwrite": true,
    }))
    .expect("args parse");
    assert!(typed.overwrite);
}

#[test]
fn format_serde_accepts_srt() {
    let format: CaptionExportFormat =
        serde_json::from_value(json!("srt")).expect("srt parses as format");
    assert_eq!(format, CaptionExportFormat::Srt);
    assert_eq!(
        serde_json::to_value(format).expect("format serializes"),
        json!("srt")
    );
}

#[test]
fn format_serde_accepts_vtt() {
    let format: CaptionExportFormat =
        serde_json::from_value(json!("vtt")).expect("vtt parses as format");
    assert_eq!(format, CaptionExportFormat::Vtt);
    assert_eq!(
        serde_json::to_value(format).expect("format serializes"),
        json!("vtt")
    );
}

#[test]
fn format_serde_accepts_ass() {
    let format: CaptionExportFormat =
        serde_json::from_value(json!("ass")).expect("ass parses as format");
    assert_eq!(format, CaptionExportFormat::Ass);
    assert_eq!(
        serde_json::to_value(format).expect("format serializes"),
        json!("ass")
    );
}

#[test]
fn format_serde_rejects_unknown_literal() {
    let err = serde_json::from_value::<CaptionExportFormat>(json!("ssa"))
        .expect_err("unknown literal rejects");
    let msg = err.to_string();
    assert!(
        msg.contains("unknown variant") || msg.contains("ssa"),
        "unexpected format parse error: {msg}",
    );
}

#[test]
fn invalid_format_rejected_as_bad_args_through_verb() {
    let prior = empty_project();
    let verb = CaptionExportVerb;

    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "text_track": TEXT_TRACK_SELECTOR,
                "out_path": "captions.srt",
                "format": "ssa",
            }),
        )
        .expect_err("invalid format should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_unknown_field_rejected_through_verb() {
    let prior = empty_project();
    let verb = CaptionExportVerb;

    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "text_track": TEXT_TRACK_SELECTOR,
                "out_path": "captions.srt",
                "extra": true,
            }),
        )
        .expect_err("deny_unknown_fields rejects extra key");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_missing_project_id_fails_through_verb() {
    let prior = empty_project();
    let verb = CaptionExportVerb;

    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "text_track": TEXT_TRACK_SELECTOR,
                "out_path": "captions.srt",
            }),
        )
        .expect_err("missing project_id should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_missing_text_track_fails_through_verb() {
    let prior = empty_project();
    let verb = CaptionExportVerb;

    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "out_path": "captions.srt",
            }),
        )
        .expect_err("missing text_track should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_missing_out_path_fails_through_verb() {
    let prior = empty_project();
    let verb = CaptionExportVerb;

    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "text_track": TEXT_TRACK_SELECTOR,
            }),
        )
        .expect_err("missing out_path should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn resolved_format_prefers_explicit_over_extension() {
    let mut args = args_default();
    args.out_path = "captions.vtt".to_string();
    args.format = Some(CaptionExportFormat::Ass);
    assert_eq!(resolved_format(&args), CaptionExportFormat::Ass);
}

#[test]
fn resolved_format_infers_srt_extension() {
    let mut args = args_default();
    args.out_path = "captions.srt".to_string();
    args.format = None;
    assert_eq!(resolved_format(&args), CaptionExportFormat::Srt);
}

#[test]
fn resolved_format_infers_vtt_extension() {
    let mut args = args_default();
    args.out_path = "captions.vtt".to_string();
    args.format = None;
    assert_eq!(resolved_format(&args), CaptionExportFormat::Vtt);
}

#[test]
fn resolved_format_infers_ass_extension() {
    let mut args = args_default();
    args.out_path = "captions.ass".to_string();
    args.format = None;
    assert_eq!(resolved_format(&args), CaptionExportFormat::Ass);
}

#[test]
fn resolved_format_unknown_extension_defaults_to_srt() {
    let mut args = args_default();
    args.out_path = "captions.txt".to_string();
    args.format = None;
    assert_eq!(resolved_format(&args), CaptionExportFormat::Srt);
}

#[test]
fn resolved_format_no_extension_defaults_to_srt() {
    let mut args = args_default();
    args.out_path = "captions".to_string();
    args.format = None;
    assert_eq!(resolved_format(&args), CaptionExportFormat::Srt);
}

#[test]
fn success_data_shape_omits_dropped_style_fields_when_none() {
    let data = CaptionExportData {
        out_path: "captions.srt".to_string(),
        format: CaptionExportFormat::Srt,
        segment_count: 3,
        dropped_style_fields: None,
    };

    let value = serde_json::to_value(&data).expect("data serializes");
    let obj = value.as_object().expect("data is object");
    assert_eq!(obj.get("out_path"), Some(&json!("captions.srt")));
    assert_eq!(obj.get("format"), Some(&json!("srt")));
    assert_eq!(obj.get("segment_count"), Some(&json!(3)));
    assert!(!obj.contains_key("dropped_style_fields"));
}

#[test]
fn success_data_shape_includes_dropped_style_fields_when_some() {
    let data = CaptionExportData {
        out_path: "captions.vtt".to_string(),
        format: CaptionExportFormat::Vtt,
        segment_count: 7,
        dropped_style_fields: Some(vec![
            "text.shadow".to_string(),
            "text.stroke_px".to_string(),
        ]),
    };

    let value = serde_json::to_value(&data).expect("data serializes");
    assert_eq!(value["out_path"], json!("captions.vtt"));
    assert_eq!(value["format"], json!("vtt"));
    assert_eq!(value["segment_count"], json!(7));
    assert_eq!(
        value["dropped_style_fields"],
        json!(["text.shadow", "text.stroke_px"])
    );
}

#[test]
fn compute_patch_always_returns_io_for_well_formed_args() {
    let prior = empty_project();
    let args = args_default();
    let err = compute_patch(&prior, &args).expect_err("v1 floor always errors");
    assert!(matches!(err, CaptionExportError::Io { .. }));
}

#[test]
fn error_maps_to_custom_not_bad_args() {
    let prior = empty_project();
    let verb = CaptionExportVerb;
    let err = verb
        .compute_patch(&prior, &args_value())
        .expect_err("v1 floor always errors");
    assert!(matches!(err, VerbError::Custom(_)));
}

#[test]
fn error_text_contains_e_io() {
    let prior = empty_project();
    let args = args_default();
    let err = compute_patch(&prior, &args).expect_err("v1 floor always errors");
    assert!(err.to_string().contains("E_IO"));
}

#[test]
fn verb_custom_error_detail_contains_e_io() {
    let prior = empty_project();
    let verb = CaptionExportVerb;
    let err = verb
        .compute_patch(&prior, &args_value())
        .expect_err("v1 floor always errors");

    let VerbError::Custom(detail) = err else {
        panic!("expected Custom, got {err:?}");
    };
    assert!(detail.contains("E_IO"));
}

#[test]
fn error_path_is_project_agnostic() {
    let prior_a = empty_project();
    let mut prior_b = empty_project();
    prior_b.name = "different-name".to_string();
    prior_b.duration_tk = verbreel_types::Tick::new(123_456);

    let args = args_default();
    let err_a = compute_patch(&prior_a, &args).expect_err("a errors");
    let err_b = compute_patch(&prior_b, &args).expect_err("b errors");
    assert_eq!(err_a, err_b);
}

#[test]
fn reconstruct_returns_null_for_well_formed_args() {
    let verb = CaptionExportVerb;
    let prior = empty_project();

    let data = verb
        .reconstruct(&args_value(), &json!([]), &[], &prior)
        .expect("reconstruct succeeds for typed args");
    assert_eq!(data, Value::Null);
}

#[test]
fn reconstruct_rejects_malformed_args() {
    let verb = CaptionExportVerb;
    let prior = empty_project();
    let err = verb
        .reconstruct(&json!({}), &json!([]), &[], &prior)
        .expect_err("malformed args fail");
    let msg = err.to_string();
    assert!(
        msg.contains("CaptionExportArgs") || msg.contains("wrong type"),
        "unexpected reconstruct error: {msg}",
    );
}

#[test]
fn default_fixture_validates_with_single_verb_registry() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "caption.export")
        .expect("default_fixtures includes caption.export");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(CaptionExportVerb))
        .expect("register caption.export verb");

    let report = validate_reconstructors(&registry, &[fixture]).expect("fixture validates");
    assert_eq!(report.verbs_checked, vec!["caption.export"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn default_fixture_has_empty_patch_warnings_and_null_expected_data() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "caption.export")
        .expect("default_fixtures includes caption.export");

    assert_eq!(fixture.patch, json!([]));
    assert!(fixture.warnings.is_empty());
    assert_eq!(fixture.expected_data, Value::Null);
}

#[test]
fn default_registry_contains_caption_export() {
    let registry = default_registry();
    let verb = registry
        .get("caption.export")
        .expect("caption.export in default_registry");

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
fn mutate_via_verb_route_returns_custom() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        empty_project(),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry succeeds");

    let outcome = store.mutate_via_verb("caption.export", args_value(), None);
    let Err(LifecycleError::VerbExecutionFailed { source, .. }) = outcome else {
        panic!("expected VerbExecutionFailed, got {outcome:?}");
    };

    let VerbError::Custom(detail) = source else {
        panic!("expected Custom source, got {source:?}");
    };
    assert!(detail.contains("E_IO"));
}
