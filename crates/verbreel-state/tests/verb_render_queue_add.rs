//! Tests for `render.queue.add` (§21.1) — v1 queue-enqueue floor.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::render_queue_add::compute_patch;
use verbreel_state::{
    Project, QueueJobState, RenderAudioCodec, RenderQueueAddArgs, RenderQueueAddData,
    RenderQueueAddError, RenderQueueAddJobError, RenderQueueAddVerb, RenderVideoCodec, Verb,
    VerbError, VerbRegistry, default_fixtures, default_registry, validate_reconstructors,
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

fn args_default() -> RenderQueueAddArgs {
    RenderQueueAddArgs {
        project_id: fixture_project_id(),
        preset: "youtube-1080p".to_string(),
        out_path: "exports/queue-floor.mp4".to_string(),
        from_tk: None,
        to_tk: None,
        video_codec: None,
        audio_codec: None,
        bitrate_bps: None,
        crf: None,
        deterministic: false,
        keep_temp: false,
        overwrite: false,
        priority: 0,
        wait: false,
    }
}

fn args_value_default() -> Value {
    serde_json::to_value(args_default()).expect("args serialize")
}

fn bad_args_detail(err: VerbError) -> String {
    match err {
        VerbError::BadArgs { detail } => detail,
        other => panic!("expected BadArgs, got {other:?}"),
    }
}

fn custom_detail(err: VerbError) -> String {
    match err {
        VerbError::Custom(detail) => detail,
        other => panic!("expected Custom, got {other:?}"),
    }
}

#[test]
fn args_deserialize_minimal() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "preset": "youtube-1080p",
        "out_path": "exports/min.mp4",
    });
    let typed: RenderQueueAddArgs = serde_json::from_value(raw).expect("minimal args parse");
    assert_eq!(typed.project_id.to_string(), FIXTURE_PROJECT_ID);
    assert_eq!(typed.preset, "youtube-1080p");
    assert_eq!(typed.out_path, "exports/min.mp4");
    assert!(!typed.deterministic);
    assert!(!typed.keep_temp);
    assert!(!typed.overwrite);
    assert_eq!(typed.priority, 0);
    assert!(!typed.wait);
}

#[test]
fn args_deserialize_all_fields() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "preset": "web-vp9-1080p",
        "out_path": "exports/all.webm",
        "from_tk": 10,
        "to_tk": 100,
        "video_codec": "vp9",
        "audio_codec": "opus",
        "bitrate_bps": 4_000_000,
        "crf": 32,
        "deterministic": true,
        "keep_temp": true,
        "overwrite": true,
        "priority": -5,
        "wait": true,
    });
    let typed: RenderQueueAddArgs = serde_json::from_value(raw).expect("full args parse");
    assert_eq!(typed.project_id.to_string(), FIXTURE_PROJECT_ID);
    assert_eq!(typed.preset, "web-vp9-1080p");
    assert_eq!(typed.out_path, "exports/all.webm");
    assert_eq!(typed.from_tk, Some(10));
    assert_eq!(typed.to_tk, Some(100));
    assert_eq!(typed.video_codec, Some(RenderVideoCodec::Vp9));
    assert_eq!(typed.audio_codec, Some(RenderAudioCodec::Opus));
    assert_eq!(typed.bitrate_bps, Some(4_000_000));
    assert_eq!(typed.crf, Some(32));
    assert!(typed.deterministic);
    assert!(typed.keep_temp);
    assert!(typed.overwrite);
    assert_eq!(typed.priority, -5);
    assert!(typed.wait);
}

#[test]
fn args_unknown_field_rejected_through_verb() {
    let prior = empty_project();
    let verb = RenderQueueAddVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "preset": "youtube-1080p",
                "out_path": "exports/min.mp4",
                "extra": true,
            }),
        )
        .expect_err("unknown field must reject");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_missing_project_id_fails_through_verb() {
    let prior = empty_project();
    let verb = RenderQueueAddVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({"preset":"youtube-1080p","out_path":"exports/min.mp4"}),
        )
        .expect_err("missing project_id should reject");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_missing_preset_fails_through_verb() {
    let prior = empty_project();
    let verb = RenderQueueAddVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({"project_id":FIXTURE_PROJECT_ID,"out_path":"exports/min.mp4"}),
        )
        .expect_err("missing preset should reject");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_missing_out_path_fails_through_verb() {
    let prior = empty_project();
    let verb = RenderQueueAddVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({"project_id":FIXTURE_PROJECT_ID,"preset":"youtube-1080p"}),
        )
        .expect_err("missing out_path should reject");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn omitted_booleans_default_false() {
    let typed: RenderQueueAddArgs = serde_json::from_value(args_value_default()).expect("parse");
    assert!(!typed.deterministic);
    assert!(!typed.keep_temp);
    assert!(!typed.overwrite);
    assert!(!typed.wait);
}

#[test]
fn omitted_priority_defaults_zero() {
    let typed: RenderQueueAddArgs = serde_json::from_value(args_value_default()).expect("parse");
    assert_eq!(typed.priority, 0);
}

#[test]
fn codec_enums_accept_all_literals() {
    let video_cases = [
        ("h264", RenderVideoCodec::H264),
        ("h265", RenderVideoCodec::H265),
        ("prores", RenderVideoCodec::Prores),
        ("vp9", RenderVideoCodec::Vp9),
        ("av1", RenderVideoCodec::Av1),
    ];
    for (wire, expected) in video_cases {
        let parsed: RenderVideoCodec = serde_json::from_value(json!(wire)).expect("video parse");
        assert_eq!(parsed, expected);
        assert_eq!(
            serde_json::to_value(parsed).expect("video serialize"),
            json!(wire)
        );
    }

    let audio_cases = [
        ("aac", RenderAudioCodec::Aac),
        ("opus", RenderAudioCodec::Opus),
        ("pcm_s16le", RenderAudioCodec::PcmS16Le),
    ];
    for (wire, expected) in audio_cases {
        let parsed: RenderAudioCodec = serde_json::from_value(json!(wire)).expect("audio parse");
        assert_eq!(parsed, expected);
        assert_eq!(
            serde_json::to_value(parsed).expect("audio serialize"),
            json!(wire)
        );
    }
}

#[test]
fn unknown_preset_maps_to_bad_args_with_unknown_code() {
    let prior = empty_project();
    let verb = RenderQueueAddVerb;
    let detail = bad_args_detail(
        verb.compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "preset": "not-a-preset",
                "out_path": "exports/min.mp4",
            }),
        )
        .expect_err("unknown preset should fail"),
    );
    assert!(detail.contains("E_RENDER_PRESET_UNKNOWN"));
}

#[test]
fn negative_from_tk_maps_to_bad_args_with_bad_time() {
    let prior = empty_project();
    let verb = RenderQueueAddVerb;
    let detail = bad_args_detail(
        verb.compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "preset": "youtube-1080p",
                "out_path": "exports/min.mp4",
                "from_tk": -1,
            }),
        )
        .expect_err("negative from_tk should fail"),
    );
    assert!(detail.contains("E_BAD_TIME"));
}

#[test]
fn empty_explicit_range_maps_to_bad_args_with_empty_range() {
    let prior = empty_project();
    let verb = RenderQueueAddVerb;
    let detail = bad_args_detail(
        verb.compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "preset": "youtube-1080p",
                "out_path": "exports/min.mp4",
                "from_tk": 100,
                "to_tk": 100,
            }),
        )
        .expect_err("empty range should fail"),
    );
    assert!(detail.contains("E_RENDER_EMPTY_RANGE"));
}

#[test]
fn bitrate_below_range_maps_to_bad_args_with_bad_range() {
    let prior = empty_project();
    let verb = RenderQueueAddVerb;
    let detail = bad_args_detail(
        verb.compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "preset": "youtube-1080p",
                "out_path": "exports/min.mp4",
                "bitrate_bps": 0,
            }),
        )
        .expect_err("bitrate below min should fail"),
    );
    assert!(detail.contains("E_BAD_RANGE"));
}

#[test]
fn bitrate_above_range_maps_to_bad_args_with_bad_range() {
    let prior = empty_project();
    let verb = RenderQueueAddVerb;
    let detail = bad_args_detail(
        verb.compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "preset": "youtube-1080p",
                "out_path": "exports/min.mp4",
                "bitrate_bps": 1_000_000_001_i64,
            }),
        )
        .expect_err("bitrate above max should fail"),
    );
    assert!(detail.contains("E_BAD_RANGE"));
}

#[test]
fn crf_below_range_maps_to_bad_args_with_bad_range() {
    let prior = empty_project();
    let verb = RenderQueueAddVerb;
    let detail = bad_args_detail(
        verb.compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "preset": "youtube-1080p",
                "out_path": "exports/min.mp4",
                "video_codec": "h264",
                "crf": -1,
            }),
        )
        .expect_err("crf below min should fail"),
    );
    assert!(detail.contains("E_BAD_RANGE"));
}

#[test]
fn crf_above_range_maps_to_bad_args_with_bad_range() {
    let prior = empty_project();
    let verb = RenderQueueAddVerb;
    let detail = bad_args_detail(
        verb.compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "preset": "youtube-1080p",
                "out_path": "exports/min.mp4",
                "video_codec": "h264",
                "crf": 52,
            }),
        )
        .expect_err("crf above max should fail"),
    );
    assert!(detail.contains("E_BAD_RANGE"));
}

#[test]
fn prores_with_crf_maps_to_bad_args_with_args_incompatible() {
    let prior = empty_project();
    let verb = RenderQueueAddVerb;
    let detail = bad_args_detail(
        verb.compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "preset": "prores-master",
                "out_path": "exports/min.mov",
                "crf": 10,
            }),
        )
        .expect_err("prores+crf should fail"),
    );
    assert!(detail.contains("E_ARGS_INCOMPATIBLE"));
}

#[test]
fn crf_and_bitrate_together_map_to_bad_args_with_args_incompatible() {
    let prior = empty_project();
    let verb = RenderQueueAddVerb;
    let detail = bad_args_detail(
        verb.compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "preset": "youtube-1080p",
                "out_path": "exports/min.mp4",
                "crf": 23,
                "bitrate_bps": 5_000_000,
            }),
        )
        .expect_err("crf+bitrate should fail"),
    );
    assert!(detail.contains("E_ARGS_INCOMPATIBLE"));
}

#[test]
fn valid_h264_crf_reaches_queue_full_custom() {
    let prior = empty_project();
    let verb = RenderQueueAddVerb;
    let detail = custom_detail(
        verb.compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "preset": "youtube-1080p",
                "out_path": "exports/min.mp4",
                "video_codec": "h264",
                "crf": 23,
            }),
        )
        .expect_err("valid request should hit v1 floor"),
    );
    assert!(detail.contains("E_QUEUE_FULL"));
}

#[test]
fn valid_h265_crf_reaches_queue_full_custom() {
    let prior = empty_project();
    let verb = RenderQueueAddVerb;
    let detail = custom_detail(
        verb.compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "preset": "youtube-1080p",
                "out_path": "exports/min.mp4",
                "video_codec": "h265",
                "crf": 23,
            }),
        )
        .expect_err("valid request should hit v1 floor"),
    );
    assert!(detail.contains("E_QUEUE_FULL"));
}

#[test]
fn valid_vp9_crf_reaches_queue_full_custom() {
    let prior = empty_project();
    let verb = RenderQueueAddVerb;
    let detail = custom_detail(
        verb.compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "preset": "web-vp9-1080p",
                "out_path": "exports/min.webm",
                "video_codec": "vp9",
                "crf": 32,
            }),
        )
        .expect_err("valid request should hit v1 floor"),
    );
    assert!(detail.contains("E_QUEUE_FULL"));
}

#[test]
fn valid_av1_crf_reaches_queue_full_custom() {
    let prior = empty_project();
    let verb = RenderQueueAddVerb;
    let detail = custom_detail(
        verb.compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "preset": "web-vp9-1080p",
                "out_path": "exports/min.webm",
                "video_codec": "av1",
                "crf": 40,
            }),
        )
        .expect_err("valid request should hit v1 floor"),
    );
    assert!(detail.contains("E_QUEUE_FULL"));
}

#[test]
fn wait_true_still_reaches_queue_full_custom() {
    let prior = empty_project();
    let verb = RenderQueueAddVerb;
    let detail = custom_detail(
        verb.compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "preset": "youtube-1080p",
                "out_path": "exports/min.mp4",
                "wait": true,
            }),
        )
        .expect_err("wait=true still hits v1 floor"),
    );
    assert!(detail.contains("E_QUEUE_FULL"));
}

#[test]
fn future_data_serializes_non_wait_fields() {
    let data = RenderQueueAddData {
        queue_job_id: "0190b8d3-15e3-7000-bd00-0000feedbeef".to_string(),
        project_id: FIXTURE_PROJECT_ID.to_string(),
        position_in_queue: 0,
        state: QueueJobState::Queued,
        added_at: "2026-05-28T00:00:00Z".to_string(),
        started_at: None,
        finished_at: None,
        output_path: None,
        partial_path: None,
        error: None,
    };
    let value = serde_json::to_value(&data).expect("data serializes");
    let obj = value.as_object().expect("object");
    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "added_at",
            "position_in_queue",
            "project_id",
            "queue_job_id",
            "state"
        ]
    );
}

#[test]
fn terminal_error_payload_serializes_code_message_and_details() {
    let payload = RenderQueueAddJobError {
        code: "E_RENDER_FAIL".to_string(),
        message: "encoder crashed".to_string(),
        details: Some(json!({"stage":"encoding"})),
    };
    let value = serde_json::to_value(&payload).expect("payload serializes");
    let obj = value.as_object().expect("object");
    assert_eq!(obj.get("code"), Some(&json!("E_RENDER_FAIL")));
    assert_eq!(obj.get("message"), Some(&json!("encoder crashed")));
    assert_eq!(obj.get("details"), Some(&json!({"stage":"encoding"})));
}

#[test]
fn reserved_error_variants_display_codes_and_map_as_briefed() {
    let static_cases = vec![
        RenderQueueAddError::RenderPresetUnknown {
            preset: "missing".to_string(),
        },
        RenderQueueAddError::BadRange {
            field: "crf".to_string(),
            value: "99".to_string(),
            allowed: "[0, 51]".to_string(),
        },
        RenderQueueAddError::BadTime { from_tk: -1 },
        RenderQueueAddError::RenderEmptyRange {
            from_tk: 100,
            to_tk: 100,
        },
        RenderQueueAddError::ArgsIncompatible {
            detail: "crf with prores".to_string(),
            hint: "remove crf".to_string(),
        },
    ];
    for error in static_cases {
        let detail = error.to_string();
        assert!(detail.contains("E_"));
        let mapped = VerbError::from(error);
        assert!(matches!(mapped, VerbError::BadArgs { .. }));
    }

    let runtime_cases = vec![
        RenderQueueAddError::PathEscape {
            path: "../escape.mp4".to_string(),
        },
        RenderQueueAddError::QueueFull {
            project_id: FIXTURE_PROJECT_ID.to_string(),
            cap: 0,
            current_length: 0,
        },
        RenderQueueAddError::Busy {
            detail: "idempotency in progress".to_string(),
        },
    ];
    for error in runtime_cases {
        let detail = error.to_string();
        assert!(detail.contains("E_"));
        let mapped = VerbError::from(error);
        assert!(matches!(mapped, VerbError::Custom(_)));
    }
}

#[test]
fn reconstruct_returns_null_for_well_formed_args() {
    let verb = RenderQueueAddVerb;
    let prior = empty_project();
    let value = verb
        .reconstruct(&args_value_default(), &json!([]), &[], &prior)
        .expect("reconstruct should succeed");
    assert_eq!(value, Value::Null);
}

#[test]
fn reconstruct_rejects_malformed_args() {
    let verb = RenderQueueAddVerb;
    let prior = empty_project();
    let err = verb
        .reconstruct(&json!({}), &json!([]), &[], &prior)
        .expect_err("malformed args should reject");
    assert!(err.to_string().contains("RenderQueueAddArgs"));
}

#[test]
fn default_fixture_validates_with_single_verb_registry() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "render.queue.add")
        .expect("fixture present");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(RenderQueueAddVerb))
        .expect("register verb");

    let report = validate_reconstructors(&registry, &[fixture]).expect("validate fixture");
    assert_eq!(report.verbs_checked, vec!["render.queue.add"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn default_fixture_has_empty_patch_warnings_and_null_expected_data() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "render.queue.add")
        .expect("fixture present");
    assert_eq!(fixture.patch, json!([]));
    assert_eq!(fixture.warnings, Vec::<Value>::new());
    assert_eq!(fixture.expected_data, Value::Null);
}

#[test]
fn default_registry_contains_render_queue_add() {
    let registry = default_registry();
    assert!(
        registry.get("render.queue.add").is_some(),
        "render.queue.add should be registered"
    );
}

#[test]
fn default_registry_lookup_returns_queue_full_for_valid_request() {
    let registry = default_registry();
    let verb = registry
        .get("render.queue.add")
        .expect("render.queue.add in registry");
    let prior = empty_project();
    let detail = custom_detail(
        verb.compute_patch(&prior, &args_value_default())
            .expect_err("v1 floor should fail"),
    );
    assert!(detail.contains("E_QUEUE_FULL"));
}

#[cfg(feature = "native")]
#[test]
fn native_mutate_via_verb_returns_queue_full_for_valid_request() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        empty_project(),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry");

    let outcome = store.mutate_via_verb("render.queue.add", args_value_default(), None);
    let Err(LifecycleError::VerbExecutionFailed { source, .. }) = outcome else {
        panic!("expected VerbExecutionFailed, got {outcome:?}");
    };
    let VerbError::Custom(detail) = source else {
        panic!("expected Custom, got {source:?}");
    };
    assert!(detail.contains("E_QUEUE_FULL"));
}

#[cfg(feature = "native")]
#[test]
fn native_mutate_via_verb_returns_bad_args_for_static_validation_failure() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        empty_project(),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry");

    let outcome = store.mutate_via_verb(
        "render.queue.add",
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "preset": "youtube-1080p",
            "out_path": "exports/min.mp4",
            "from_tk": -1,
        }),
        None,
    );
    let Err(LifecycleError::VerbExecutionFailed { source, .. }) = outcome else {
        panic!("expected VerbExecutionFailed, got {outcome:?}");
    };
    let VerbError::BadArgs { detail } = source else {
        panic!("expected BadArgs, got {source:?}");
    };
    assert!(detail.contains("E_BAD_TIME"));
}

#[test]
fn compute_patch_helper_returns_queue_full_for_valid_request() {
    let prior = empty_project();
    let err = compute_patch(&prior, &args_default()).expect_err("v1 floor should error");
    assert!(matches!(err, RenderQueueAddError::QueueFull { .. }));
}
