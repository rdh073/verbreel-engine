//! Tests for `render.start` (§11.1) — v1 always-error renderer floor.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::render_start::compute_patch;
use verbreel_state::{
    Project, RenderAudioCodec, RenderStartArgs, RenderStartData, RenderStartError, RenderStartVerb,
    RenderVideoCodec, Verb, VerbError, VerbRegistry, default_fixtures, default_registry,
    render_start_result_warning, validate_reconstructors,
};
use verbreel_types::ProjectId;

#[cfg(feature = "native")]
use verbreel_state::{LifecycleError, ProjectStore};

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture -> Project")
}

fn fixture_project_id() -> ProjectId {
    empty_project().id
}

fn args_default() -> RenderStartArgs {
    RenderStartArgs {
        project_id: fixture_project_id(),
        preset: "youtube-1080p".to_string(),
        out_path: "exports/floor.mp4".to_string(),
        from_tk: None,
        to_tk: None,
        video_codec: None,
        audio_codec: None,
        bitrate_bps: None,
        crf: None,
        deterministic: false,
        keep_temp: false,
        overwrite: false,
    }
}

fn args_value() -> Value {
    json!({
        "project_id": FIXTURE_PROJECT_ID,
        "preset": "youtube-1080p",
        "out_path": "exports/floor.mp4",
    })
}

#[test]
fn args_deserialize_ok_with_all_fields() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "preset": "youtube-1080p",
        "out_path": "exports/all.mp4",
        "from_tk": 10,
        "to_tk": 100,
        "video_codec": "h264",
        "audio_codec": "opus",
        "bitrate_bps": 5_000_000,
        "crf": 23,
        "deterministic": true,
        "keep_temp": true,
        "overwrite": true,
    });

    let typed: RenderStartArgs = serde_json::from_value(raw).expect("args parse");
    assert_eq!(typed.project_id.to_string(), FIXTURE_PROJECT_ID);
    assert_eq!(typed.preset, "youtube-1080p");
    assert_eq!(typed.out_path, "exports/all.mp4");
    assert_eq!(typed.from_tk, Some(10));
    assert_eq!(typed.to_tk, Some(100));
    assert_eq!(typed.video_codec, Some(RenderVideoCodec::H264));
    assert_eq!(typed.audio_codec, Some(RenderAudioCodec::Opus));
    assert_eq!(typed.bitrate_bps, Some(5_000_000));
    assert_eq!(typed.crf, Some(23));
    assert!(typed.deterministic);
    assert!(typed.keep_temp);
    assert!(typed.overwrite);
}

#[test]
fn args_deserialize_ok_with_omitted_optionals() {
    let typed: RenderStartArgs = serde_json::from_value(args_value()).expect("args parse");
    assert_eq!(typed.project_id.to_string(), FIXTURE_PROJECT_ID);
    assert_eq!(typed.preset, "youtube-1080p");
    assert_eq!(typed.out_path, "exports/floor.mp4");
    assert_eq!(typed.from_tk, None);
    assert_eq!(typed.to_tk, None);
    assert_eq!(typed.video_codec, None);
    assert_eq!(typed.audio_codec, None);
    assert_eq!(typed.bitrate_bps, None);
    assert_eq!(typed.crf, None);
    assert!(!typed.deterministic);
    assert!(!typed.keep_temp);
    assert!(!typed.overwrite);
}

#[test]
fn omitted_booleans_default_false() {
    let typed: RenderStartArgs = serde_json::from_value(args_value()).expect("args parse");
    assert!(!typed.deterministic);
    assert!(!typed.keep_temp);
    assert!(!typed.overwrite);
}

#[test]
fn video_codec_serde_accepts_all_literals() {
    let cases = [
        ("h264", RenderVideoCodec::H264),
        ("h265", RenderVideoCodec::H265),
        ("prores", RenderVideoCodec::Prores),
        ("vp9", RenderVideoCodec::Vp9),
        ("av1", RenderVideoCodec::Av1),
    ];

    for (wire, expected) in cases {
        let parsed: RenderVideoCodec =
            serde_json::from_value(json!(wire)).expect("video codec literal parses");
        assert_eq!(parsed, expected);
        assert_eq!(
            serde_json::to_value(parsed).expect("codec serializes"),
            json!(wire)
        );
    }
}

#[test]
fn audio_codec_serde_accepts_all_literals() {
    let cases = [
        ("aac", RenderAudioCodec::Aac),
        ("opus", RenderAudioCodec::Opus),
        ("pcm_s16le", RenderAudioCodec::PcmS16Le),
    ];

    for (wire, expected) in cases {
        let parsed: RenderAudioCodec =
            serde_json::from_value(json!(wire)).expect("audio codec literal parses");
        assert_eq!(parsed, expected);
        assert_eq!(
            serde_json::to_value(parsed).expect("codec serializes"),
            json!(wire)
        );
    }
}

#[test]
fn invalid_video_codec_rejected_as_bad_args_through_verb() {
    let prior = empty_project();
    let verb = RenderStartVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "preset": "youtube-1080p",
                "out_path": "exports/floor.mp4",
                "video_codec": "mpeg2",
            }),
        )
        .expect_err("invalid video codec should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn invalid_audio_codec_rejected_as_bad_args_through_verb() {
    let prior = empty_project();
    let verb = RenderStartVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "preset": "youtube-1080p",
                "out_path": "exports/floor.mp4",
                "audio_codec": "flac",
            }),
        )
        .expect_err("invalid audio codec should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn bitrate_bps_negative_integer_parses_at_args_layer() {
    let typed: RenderStartArgs = serde_json::from_value(json!({
        "project_id": FIXTURE_PROJECT_ID,
        "preset": "youtube-1080p",
        "out_path": "exports/floor.mp4",
        "bitrate_bps": -1,
    }))
    .expect("signed bitrate accepts negatives at args layer");
    assert_eq!(typed.bitrate_bps, Some(-1));
}

#[test]
fn crf_negative_integer_parses_at_args_layer() {
    let typed: RenderStartArgs = serde_json::from_value(json!({
        "project_id": FIXTURE_PROJECT_ID,
        "preset": "youtube-1080p",
        "out_path": "exports/floor.mp4",
        "crf": -5,
    }))
    .expect("signed crf accepts negatives at args layer");
    assert_eq!(typed.crf, Some(-5));
}

#[test]
fn unknown_field_rejected_through_verb() {
    let prior = empty_project();
    let verb = RenderStartVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "preset": "youtube-1080p",
                "out_path": "exports/floor.mp4",
                "extra": true,
            }),
        )
        .expect_err("deny_unknown_fields rejects extras");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn missing_project_id_fails_through_verb() {
    let prior = empty_project();
    let verb = RenderStartVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({ "preset": "youtube-1080p", "out_path": "exports/floor.mp4" }),
        )
        .expect_err("missing project_id should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn missing_preset_fails_through_verb() {
    let prior = empty_project();
    let verb = RenderStartVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({ "project_id": FIXTURE_PROJECT_ID, "out_path": "exports/floor.mp4" }),
        )
        .expect_err("missing preset should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn missing_out_path_fails_through_verb() {
    let prior = empty_project();
    let verb = RenderStartVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({ "project_id": FIXTURE_PROJECT_ID, "preset": "youtube-1080p" }),
        )
        .expect_err("missing out_path should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn future_success_data_shape_serializes() {
    let data = RenderStartData {
        job_id: "0190b8d3-15e3-7000-bd00-0000feedbeef".to_string(),
        output_path: "exports/done.mp4".to_string(),
        duration_tk: 240_000,
    };
    let value = serde_json::to_value(&data).expect("data serializes");
    let obj = value.as_object().expect("data is object");
    assert_eq!(obj.len(), 3);
    assert_eq!(
        obj.get("job_id"),
        Some(&json!("0190b8d3-15e3-7000-bd00-0000feedbeef"))
    );
    assert_eq!(obj.get("output_path"), Some(&json!("exports/done.mp4")));
    assert_eq!(obj.get("duration_tk"), Some(&json!(240_000)));
}

#[test]
fn compute_patch_always_returns_render_fail_for_well_formed_args() {
    let prior = empty_project();
    let err = compute_patch(&prior, &args_default()).expect_err("v1 floor always errors");
    assert!(matches!(err, RenderStartError::RenderFail { .. }));
}

#[test]
fn error_maps_to_custom_not_bad_args() {
    let prior = empty_project();
    let verb = RenderStartVerb;
    let err = verb
        .compute_patch(&prior, &args_value())
        .expect_err("v1 floor always errors");
    assert!(matches!(err, VerbError::Custom(_)));
}

#[test]
fn error_text_contains_e_render_fail() {
    let prior = empty_project();
    let err = compute_patch(&prior, &args_default()).expect_err("v1 floor always errors");
    assert!(err.to_string().contains("E_RENDER_FAIL"));
}

#[test]
fn verb_custom_error_detail_contains_e_render_fail() {
    let prior = empty_project();
    let verb = RenderStartVerb;
    let err = verb
        .compute_patch(&prior, &args_value())
        .expect_err("v1 floor always errors");
    let VerbError::Custom(detail) = err else {
        panic!("expected Custom, got {err:?}");
    };
    assert!(detail.contains("E_RENDER_FAIL"));
}

#[test]
fn error_path_is_project_agnostic() {
    let prior_a = empty_project();
    let mut prior_b = empty_project();
    prior_b.name = "different-name".to_string();
    prior_b.duration_tk = verbreel_types::Tick::new(777_000);

    let args = args_default();
    let err_a = compute_patch(&prior_a, &args).expect_err("a errors");
    let err_b = compute_patch(&prior_b, &args).expect_err("b errors");
    assert_eq!(err_a, err_b);
}

#[test]
fn reconstruct_returns_null_for_well_formed_args() {
    let verb = RenderStartVerb;
    let prior = empty_project();
    let data = verb
        .reconstruct(&args_value(), &json!([]), &[], &prior)
        .expect("reconstruct succeeds for well-formed args");
    assert_eq!(data, Value::Null);
}

#[test]
fn reconstruct_rejects_malformed_args() {
    let verb = RenderStartVerb;
    let prior = empty_project();
    let err = verb
        .reconstruct(&json!({}), &json!([]), &[], &prior)
        .expect_err("malformed args should fail reconstruction");
    let msg = err.to_string();
    assert!(
        msg.contains("RenderStartArgs") || msg.contains("wrong type"),
        "unexpected reconstruct error: {msg}",
    );
}

#[test]
fn default_fixture_validates_with_single_verb_registry() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "render.start")
        .expect("default_fixtures includes render.start");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(RenderStartVerb))
        .expect("register render.start verb");

    let report = validate_reconstructors(&registry, &[fixture]).expect("fixture validates");
    assert_eq!(report.verbs_checked, vec!["render.start"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn default_fixture_has_empty_patch_warnings_and_null_expected_data() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "render.start")
        .expect("default_fixtures includes render.start");
    assert_eq!(fixture.patch, json!([]));
    assert!(fixture.warnings.is_empty());
    assert_eq!(fixture.expected_data, Value::Null);
}

#[test]
fn reconstruct_recovers_runtime_recorded_result_data() {
    let verb = RenderStartVerb;
    let expected = RenderStartData {
        job_id: "0190b8d3-15e3-7000-bd00-000000000099".to_string(),
        output_path: "/tmp/out.mp4".to_string(),
        duration_tk: 120_000,
    };
    let data = verb
        .reconstruct(
            &args_value(),
            &json!([]),
            &[render_start_result_warning(&expected)],
            &empty_project(),
        )
        .expect("recorded runtime warning reconstructs");
    let actual: RenderStartData = serde_json::from_value(data).expect("data shape");
    assert_eq!(actual, expected);
}

#[test]
fn default_registry_contains_render_start() {
    let registry = default_registry();
    let verb = registry
        .get("render.start")
        .expect("render.start is in default_registry");
    let prior = empty_project();
    let err = verb
        .compute_patch(&prior, &args_value())
        .expect_err("v1 floor always errors");
    let VerbError::Custom(detail) = err else {
        panic!("expected Custom, got {err:?}");
    };
    assert!(detail.contains("E_RENDER_FAIL"));
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

    let outcome = store.mutate_via_verb("render.start", args_value(), None);
    let Err(LifecycleError::VerbExecutionFailed { source, .. }) = outcome else {
        panic!("expected VerbExecutionFailed, got {outcome:?}");
    };
    let VerbError::Custom(detail) = source else {
        panic!("expected Custom source, got {source:?}");
    };
    assert!(detail.contains("E_RENDER_FAIL"));
}
