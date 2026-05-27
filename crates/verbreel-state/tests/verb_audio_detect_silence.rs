//! Tests for `audio.detect_silence` (§19.3) — v1 analysis-runtime floor.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::audio_detect_silence::{
    compute_patch, resolved_min_silence_tk, resolved_threshold_db,
};
use verbreel_state::{
    AudioAnalysisTargetKind, AudioDetectSilenceArgs, AudioDetectSilenceData,
    AudioDetectSilenceError, AudioDetectSilenceStage, AudioDetectSilenceVerb, AudioSilenceInterval,
    Project, Verb, VerbError, VerbRegistry, default_fixtures, default_registry,
    validate_reconstructors,
};
use verbreel_types::ProjectId;

#[cfg(feature = "native")]
use verbreel_state::{LifecycleError, ProjectStore};

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const CLIP_ID: &str = "0190b8d3-15e3-7000-bd00-0000000bb910";
const TRACK_ID: &str = "0190b8d3-15e3-7000-bd00-0000000aa910";
const ASSET_ID: &str = "0190b8d3-15e3-7000-bd00-0000000cc910";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture -> Project")
}

fn fixture_project_id() -> ProjectId {
    empty_project().id
}

fn args_default() -> AudioDetectSilenceArgs {
    AudioDetectSilenceArgs {
        project_id: fixture_project_id(),
        target: format!("clip:{CLIP_ID}"),
        min_silence_tk: None,
        threshold_db: None,
        from_tk: None,
        to_tk: None,
    }
}

fn args_value_default() -> Value {
    serde_json::to_value(args_default()).expect("args serialize")
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
        "target": format!("clip:{CLIP_ID}"),
    });
    let parsed: AudioDetectSilenceArgs = serde_json::from_value(raw).expect("minimal args parse");
    assert_eq!(parsed.project_id.to_string(), FIXTURE_PROJECT_ID);
    assert_eq!(parsed.target, format!("clip:{CLIP_ID}"));
    assert_eq!(parsed.min_silence_tk, None);
    assert_eq!(parsed.threshold_db, None);
    assert_eq!(parsed.from_tk, None);
    assert_eq!(parsed.to_tk, None);
}

#[test]
fn args_deserialize_all_optional_fields() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "target": format!("track:{TRACK_ID}"),
        "min_silence_tk": 240_000,
        "threshold_db": -25.5,
        "from_tk": 100,
        "to_tk": 200,
    });
    let parsed: AudioDetectSilenceArgs = serde_json::from_value(raw).expect("args parse");
    assert_eq!(parsed.target, format!("track:{TRACK_ID}"));
    assert_eq!(parsed.min_silence_tk, Some(240_000));
    assert_eq!(parsed.threshold_db, Some(-25.5));
    assert_eq!(parsed.from_tk, Some(100));
    assert_eq!(parsed.to_tk, Some(200));
}

#[test]
fn args_unknown_field_rejected() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "target": format!("clip:{CLIP_ID}"),
        "extra": true,
    });
    let err =
        serde_json::from_value::<AudioDetectSilenceArgs>(raw).expect_err("unknown field rejects");
    assert!(err.to_string().contains("unknown field"));
}

#[test]
fn args_missing_project_id_fails_through_verb() {
    let prior = empty_project();
    let verb = AudioDetectSilenceVerb;
    let err = verb
        .compute_patch(&prior, &json!({ "target": format!("clip:{CLIP_ID}") }))
        .expect_err("missing project_id should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_missing_target_fails_through_verb() {
    let prior = empty_project();
    let verb = AudioDetectSilenceVerb;
    let err = verb
        .compute_patch(&prior, &json!({ "project_id": FIXTURE_PROJECT_ID }))
        .expect_err("missing target should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_non_string_target_rejected() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "target": 42,
    });
    let parsed = serde_json::from_value::<AudioDetectSilenceArgs>(raw);
    assert!(parsed.is_err(), "non-string target should reject");
}

#[test]
fn args_non_integer_min_silence_rejected() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "target": format!("clip:{CLIP_ID}"),
        "min_silence_tk": 0.5,
    });
    let parsed = serde_json::from_value::<AudioDetectSilenceArgs>(raw);
    assert!(parsed.is_err(), "non-integer min_silence_tk should reject");
}

#[test]
fn args_non_number_threshold_rejected() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "target": format!("clip:{CLIP_ID}"),
        "threshold_db": "-40",
    });
    let parsed = serde_json::from_value::<AudioDetectSilenceArgs>(raw);
    assert!(parsed.is_err(), "non-number threshold_db should reject");
}

#[test]
fn args_non_integer_tick_fields_rejected() {
    let bad = vec![
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "target": format!("clip:{CLIP_ID}"),
            "from_tk": 1.5,
        }),
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "target": format!("clip:{CLIP_ID}"),
            "to_tk": "100",
        }),
    ];
    for raw in bad {
        let parsed = serde_json::from_value::<AudioDetectSilenceArgs>(raw);
        assert!(parsed.is_err(), "non-integer tick field should reject");
    }
}

#[test]
fn omitted_min_silence_defaults_to_120000() {
    let args = args_default();
    assert_eq!(
        resolved_min_silence_tk(&args).expect("default min silence"),
        120_000
    );
}

#[test]
fn zero_or_negative_min_silence_maps_to_custom_e_bad_time() {
    let prior = empty_project();
    let verb = AudioDetectSilenceVerb;

    for value in [0_i64, -1_i64] {
        let mut raw = args_value_default();
        raw["min_silence_tk"] = json!(value);
        let detail = custom_detail(
            verb.compute_patch(&prior, &raw)
                .expect_err("non-positive min_silence_tk should fail"),
        );
        assert!(detail.contains("E_BAD_TIME"));
        assert!(detail.contains("min_silence_tk"));
    }
}

#[test]
fn omitted_threshold_defaults_to_minus_40() {
    let args = args_default();
    assert_eq!(
        resolved_threshold_db(&args).expect("default threshold"),
        -40.0
    );
}

#[test]
fn threshold_boundaries_are_accepted() {
    for value in [-90.0, 0.0] {
        let mut args = args_default();
        args.threshold_db = Some(value);
        assert_eq!(
            resolved_threshold_db(&args).expect("boundary should pass"),
            value
        );
    }
}

#[test]
fn threshold_out_of_range_maps_to_custom_e_bad_range() {
    let prior = empty_project();
    let verb = AudioDetectSilenceVerb;
    for value in [-90.1, 0.1] {
        let mut raw = args_value_default();
        raw["threshold_db"] = json!(value);
        let detail = custom_detail(
            verb.compute_patch(&prior, &raw)
                .expect_err("out-of-range threshold_db should fail"),
        );
        assert!(detail.contains("E_BAD_RANGE"));
        assert!(detail.contains("threshold_db"));
    }
}

#[test]
fn threshold_non_finite_maps_bad_range_in_helper() {
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut args = args_default();
        args.threshold_db = Some(value);
        let err = resolved_threshold_db(&args).expect_err("non-finite should fail");
        assert!(matches!(err, AudioDetectSilenceError::BadRange { .. }));
        assert!(err.to_string().contains("E_BAD_RANGE"));
    }
}

#[test]
fn qualified_clip_target_reaches_analysis_failed() {
    let prior = empty_project();
    let verb = AudioDetectSilenceVerb;
    let detail = custom_detail(
        verb.compute_patch(&prior, &args_value_default())
            .expect_err("clip target should fail with analysis floor"),
    );
    assert!(detail.contains("E_ANALYSIS_FAILED"));
    assert!(detail.contains("algorithm_step"));
}

#[test]
fn qualified_track_target_reaches_analysis_failed() {
    let prior = empty_project();
    let verb = AudioDetectSilenceVerb;
    let mut raw = args_value_default();
    raw["target"] = json!(format!("track:{TRACK_ID}"));
    let detail = custom_detail(
        verb.compute_patch(&prior, &raw)
            .expect_err("track target should fail with analysis floor"),
    );
    assert!(detail.contains("E_ANALYSIS_FAILED"));
}

#[test]
fn qualified_asset_target_reaches_analysis_failed() {
    let prior = empty_project();
    let verb = AudioDetectSilenceVerb;
    let mut raw = args_value_default();
    raw["target"] = json!(format!("asset:{ASSET_ID}"));
    let detail = custom_detail(
        verb.compute_patch(&prior, &raw)
            .expect_err("asset target should fail with analysis floor"),
    );
    assert!(detail.contains("E_ANALYSIS_FAILED"));
}

#[test]
fn bare_selector_rejected_as_bad_selector() {
    let prior = empty_project();
    let verb = AudioDetectSilenceVerb;
    let mut raw = args_value_default();
    raw["target"] = json!(CLIP_ID);
    let detail = custom_detail(
        verb.compute_patch(&prior, &raw)
            .expect_err("bare UUID should reject"),
    );
    assert!(detail.contains("E_BAD_SELECTOR"));
}

#[test]
fn malformed_uuid_body_rejected_as_bad_selector() {
    let prior = empty_project();
    let verb = AudioDetectSilenceVerb;
    let mut raw = args_value_default();
    raw["target"] = json!("clip:not-a-uuid");
    let detail = custom_detail(
        verb.compute_patch(&prior, &raw)
            .expect_err("bad clip UUID should reject"),
    );
    assert!(detail.contains("E_BAD_SELECTOR"));
}

#[test]
fn unsupported_prefix_rejected_as_selector_kind_mismatch() {
    let prior = empty_project();
    let verb = AudioDetectSilenceVerb;
    let mut raw = args_value_default();
    raw["target"] = json!("effect:0190b8d3-15e3-7000-bd00-0000000ee910");
    let detail = custom_detail(
        verb.compute_patch(&prior, &raw)
            .expect_err("unknown prefix should reject"),
    );
    assert!(detail.contains("E_SELECTOR_KIND_MISMATCH"));
}

#[test]
fn negative_from_tk_rejected_as_bad_time() {
    let prior = empty_project();
    let verb = AudioDetectSilenceVerb;
    let mut raw = args_value_default();
    raw["from_tk"] = json!(-1);
    let detail = custom_detail(
        verb.compute_patch(&prior, &raw)
            .expect_err("negative from_tk should reject"),
    );
    assert!(detail.contains("E_BAD_TIME"));
    assert!(detail.contains("from_tk"));
}

#[test]
fn negative_to_tk_rejected_as_bad_time() {
    let prior = empty_project();
    let verb = AudioDetectSilenceVerb;
    let mut raw = args_value_default();
    raw["to_tk"] = json!(-1);
    let detail = custom_detail(
        verb.compute_patch(&prior, &raw)
            .expect_err("negative to_tk should reject"),
    );
    assert!(detail.contains("E_BAD_TIME"));
    assert!(detail.contains("to_tk"));
}

#[test]
fn to_tk_less_or_equal_from_tk_rejected_as_bad_time() {
    let prior = empty_project();
    let verb = AudioDetectSilenceVerb;
    for (from_tk, to_tk) in [(10_i64, 10_i64), (10_i64, 9_i64)] {
        let mut raw = args_value_default();
        raw["from_tk"] = json!(from_tk);
        raw["to_tk"] = json!(to_tk);
        let detail = custom_detail(
            verb.compute_patch(&prior, &raw)
                .expect_err("to_tk <= from_tk should reject"),
        );
        assert!(detail.contains("E_BAD_TIME"));
    }
}

#[test]
fn future_data_serializes_exact_fields() {
    let data = AudioDetectSilenceData {
        target_id: CLIP_ID.to_string(),
        target_kind: AudioAnalysisTargetKind::Clip,
        silences: vec![
            AudioSilenceInterval {
                start_tk: 0,
                end_tk: 120_000,
            },
            AudioSilenceInterval {
                start_tk: 240_000,
                end_tk: 360_000,
            },
        ],
        total_silence_tk: 240_000,
        cache_path: "/tmp/cache/audio_analysis/clip.json".to_string(),
        cache_hit: false,
    };

    let value = serde_json::to_value(&data).expect("data serializes");
    let obj = value.as_object().expect("data object");
    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "cache_hit",
            "cache_path",
            "silences",
            "target_id",
            "target_kind",
            "total_silence_tk",
        ]
    );
}

#[test]
fn reserved_error_variants_display_spec_codes_and_map_to_custom() {
    let cases = vec![
        AudioDetectSilenceError::AssetNoAudio {
            target_id: CLIP_ID.to_string(),
            target_kind: AudioAnalysisTargetKind::Clip,
        },
        AudioDetectSilenceError::AssetUnsupportedKind {
            asset_kind: "image".to_string(),
        },
        AudioDetectSilenceError::NoMatch {
            selector: "clip:audio[name=\"missing\"][0]".to_string(),
        },
        AudioDetectSilenceError::NotFound {
            target_kind: "clip".to_string(),
            target_id: CLIP_ID.to_string(),
        },
        AudioDetectSilenceError::TrackKindMismatch {
            track_id: TRACK_ID.to_string(),
            actual_kind: "video".to_string(),
        },
        AudioDetectSilenceError::ClipKindMismatch {
            clip_id: CLIP_ID.to_string(),
            actual_kind: "video".to_string(),
        },
    ];

    for error in cases {
        let detail = error.to_string();
        assert!(
            detail.contains("E_"),
            "detail must include spec code: {detail}"
        );
        let mapped = VerbError::from(error);
        let VerbError::Custom(custom) = mapped else {
            panic!("expected Custom");
        };
        assert!(custom.contains("E_"), "custom detail must include code");
    }
}

#[test]
fn stage_serde_accepts_spec_literals() {
    let cases = vec![
        ("decoder_init", AudioDetectSilenceStage::DecoderInit),
        ("audio_decode", AudioDetectSilenceStage::AudioDecode),
        ("algorithm_step", AudioDetectSilenceStage::AlgorithmStep),
        ("cache_write", AudioDetectSilenceStage::CacheWrite),
    ];
    for (literal, expected) in cases {
        let parsed: AudioDetectSilenceStage =
            serde_json::from_value(json!(literal)).expect("stage parses");
        assert_eq!(parsed, expected);
        assert_eq!(
            serde_json::to_value(expected).expect("stage serializes"),
            json!(literal)
        );
    }
}

#[test]
fn reconstruct_returns_null_for_well_formed_args() {
    let verb = AudioDetectSilenceVerb;
    let prior = empty_project();
    let value = verb
        .reconstruct(&args_value_default(), &json!([]), &[], &prior)
        .expect("reconstruct should succeed");
    assert_eq!(value, Value::Null);
}

#[test]
fn reconstruct_rejects_malformed_args() {
    let verb = AudioDetectSilenceVerb;
    let prior = empty_project();
    let err = verb
        .reconstruct(&json!({}), &json!([]), &[], &prior)
        .expect_err("malformed args should reject");
    assert!(err.to_string().contains("AudioDetectSilenceArgs"));
}

#[test]
fn reconstruct_from_default_fixture_with_single_verb_registry() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "audio.detect_silence")
        .expect("fixture present");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(AudioDetectSilenceVerb))
        .expect("register verb");

    let report = validate_reconstructors(&registry, &[fixture]).expect("validate fixture");
    assert_eq!(report.verbs_checked, vec!["audio.detect_silence"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn default_fixture_has_empty_patch_warnings_and_null_expected_data() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "audio.detect_silence")
        .expect("fixture present");
    assert_eq!(fixture.patch, json!([]));
    assert_eq!(fixture.warnings, Vec::<Value>::new());
    assert_eq!(fixture.expected_data, Value::Null);
}

#[test]
fn default_registry_contains_audio_detect_silence() {
    let registry = default_registry();
    assert!(
        registry.get("audio.detect_silence").is_some(),
        "audio.detect_silence should be registered"
    );
}

#[test]
fn verb_trait_lookup_via_default_registry_returns_analysis_failed() {
    let registry = default_registry();
    let verb = registry
        .get("audio.detect_silence")
        .expect("audio.detect_silence in registry");
    let prior = empty_project();
    let detail = custom_detail(
        verb.compute_patch(&prior, &args_value_default())
            .expect_err("v1 floor should fail"),
    );
    assert!(detail.contains("E_ANALYSIS_FAILED"));
}

#[cfg(feature = "native")]
#[test]
fn native_mutate_via_verb_returns_analysis_failed_for_accepted_targets() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        empty_project(),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry");

    for target in [
        format!("clip:{CLIP_ID}"),
        format!("track:{TRACK_ID}"),
        format!("asset:{ASSET_ID}"),
    ] {
        let raw = json!({
            "project_id": FIXTURE_PROJECT_ID,
            "target": target,
        });
        let outcome = store.mutate_via_verb("audio.detect_silence", raw, None);
        let Err(LifecycleError::VerbExecutionFailed { source, .. }) = outcome else {
            panic!("expected VerbExecutionFailed, got {outcome:?}");
        };
        let VerbError::Custom(detail) = source else {
            panic!("expected Custom, got {source:?}");
        };
        assert!(detail.contains("E_ANALYSIS_FAILED"));
    }
}

#[cfg(feature = "native")]
#[test]
fn native_mutate_via_verb_returns_bad_range_for_bad_threshold() {
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
        "audio.detect_silence",
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "target": format!("clip:{CLIP_ID}"),
            "threshold_db": -95.0,
        }),
        None,
    );

    let Err(LifecycleError::VerbExecutionFailed { source, .. }) = outcome else {
        panic!("expected VerbExecutionFailed, got {outcome:?}");
    };
    let VerbError::Custom(detail) = source else {
        panic!("expected Custom source, got {source:?}");
    };
    assert!(detail.contains("E_BAD_RANGE"));
}

#[test]
fn compute_patch_helper_returns_analysis_failed_for_accepted_target() {
    let prior = empty_project();
    let args = args_default();
    let err = compute_patch(&prior, &args).expect_err("v1 floor should error");
    let AudioDetectSilenceError::AnalysisFailed { stage, .. } = err else {
        panic!("expected AnalysisFailed");
    };
    assert_eq!(stage, AudioDetectSilenceStage::AlgorithmStep);
}
