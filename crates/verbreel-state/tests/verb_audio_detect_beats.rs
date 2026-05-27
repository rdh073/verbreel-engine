//! Tests for `audio.detect_beats` (§19.1) — v1 analysis-runtime floor.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::audio_detect_beats::{
    compute_patch, registered_algorithms, resolved_algorithm, resolved_create_markers,
    resolved_min_confidence,
};
use verbreel_state::{
    AudioAnalysisTargetKind, AudioDetectBeatsAlgorithm, AudioDetectBeatsArgs, AudioDetectBeatsData,
    AudioDetectBeatsError, AudioDetectBeatsStage, AudioDetectBeatsVerb, Project, Verb, VerbError,
    VerbRegistry, default_fixtures, default_registry, validate_reconstructors,
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

fn args_default() -> AudioDetectBeatsArgs {
    AudioDetectBeatsArgs {
        project_id: fixture_project_id(),
        target: format!("clip:{CLIP_ID}"),
        algorithm: None,
        min_confidence: None,
        create_markers: None,
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
    let parsed: AudioDetectBeatsArgs = serde_json::from_value(raw).expect("minimal args parse");
    assert_eq!(parsed.project_id.to_string(), FIXTURE_PROJECT_ID);
    assert_eq!(parsed.target, format!("clip:{CLIP_ID}"));
    assert_eq!(parsed.algorithm, None);
    assert_eq!(parsed.min_confidence, None);
    assert_eq!(parsed.create_markers, None);
    assert_eq!(parsed.from_tk, None);
    assert_eq!(parsed.to_tk, None);
}

#[test]
fn args_deserialize_all_optional_fields() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "target": format!("track:{TRACK_ID}"),
        "algorithm": "tempo",
        "min_confidence": 0.8,
        "create_markers": false,
        "from_tk": 100,
        "to_tk": 200,
    });
    let parsed: AudioDetectBeatsArgs = serde_json::from_value(raw).expect("args parse");
    assert_eq!(parsed.target, format!("track:{TRACK_ID}"));
    assert_eq!(parsed.algorithm.as_deref(), Some("tempo"));
    assert_eq!(parsed.min_confidence, Some(0.8));
    assert_eq!(parsed.create_markers, Some(false));
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
        serde_json::from_value::<AudioDetectBeatsArgs>(raw).expect_err("unknown field rejects");
    assert!(err.to_string().contains("unknown field"));
}

#[test]
fn args_missing_project_id_fails_through_verb() {
    let prior = empty_project();
    let verb = AudioDetectBeatsVerb;
    let err = verb
        .compute_patch(&prior, &json!({ "target": format!("clip:{CLIP_ID}") }))
        .expect_err("missing project_id should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_missing_target_fails_through_verb() {
    let prior = empty_project();
    let verb = AudioDetectBeatsVerb;
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
    let parsed = serde_json::from_value::<AudioDetectBeatsArgs>(raw);
    assert!(parsed.is_err(), "non-string target should reject");
}

#[test]
fn args_non_string_algorithm_rejected() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "target": format!("clip:{CLIP_ID}"),
        "algorithm": 42,
    });
    let parsed = serde_json::from_value::<AudioDetectBeatsArgs>(raw);
    assert!(parsed.is_err(), "non-string algorithm should reject");
}

#[test]
fn args_non_number_min_confidence_rejected() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "target": format!("clip:{CLIP_ID}"),
        "min_confidence": "0.5",
    });
    let parsed = serde_json::from_value::<AudioDetectBeatsArgs>(raw);
    assert!(parsed.is_err(), "non-number min_confidence should reject");
}

#[test]
fn args_non_boolean_create_markers_rejected() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "target": format!("clip:{CLIP_ID}"),
        "create_markers": 1,
    });
    let parsed = serde_json::from_value::<AudioDetectBeatsArgs>(raw);
    assert!(parsed.is_err(), "non-boolean create_markers should reject");
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
        let parsed = serde_json::from_value::<AudioDetectBeatsArgs>(raw);
        assert!(parsed.is_err(), "non-integer tick field should reject");
    }
}

#[test]
fn registered_algorithms_helper_returns_spec_literals() {
    assert_eq!(registered_algorithms(), ["onset", "tempo", "librosa"]);
}

#[test]
fn omitted_algorithm_defaults_to_onset() {
    let args = args_default();
    let algorithm = resolved_algorithm(&args).expect("default resolves");
    assert_eq!(algorithm, AudioDetectBeatsAlgorithm::Onset);
}

#[test]
fn each_registered_algorithm_literal_resolves_and_serializes() {
    let cases = vec![
        ("onset", AudioDetectBeatsAlgorithm::Onset),
        ("tempo", AudioDetectBeatsAlgorithm::Tempo),
        ("librosa", AudioDetectBeatsAlgorithm::Librosa),
    ];

    for (literal, expected) in cases {
        let mut args = args_default();
        args.algorithm = Some(literal.to_string());
        assert_eq!(
            resolved_algorithm(&args).expect("algorithm resolves"),
            expected
        );
        assert_eq!(
            serde_json::to_value(expected).expect("algorithm serializes"),
            json!(literal)
        );
    }
}

#[test]
fn unknown_algorithm_maps_to_custom_with_registered_list() {
    let prior = empty_project();
    let verb = AudioDetectBeatsVerb;
    let mut raw = args_value_default();
    raw["algorithm"] = json!("madmom");

    let detail = custom_detail(
        verb.compute_patch(&prior, &raw)
            .expect_err("unknown algorithm should fail"),
    );

    assert!(detail.contains("E_ANALYSIS_UNKNOWN_ALGORITHM"));
    assert!(detail.contains("madmom"));
    assert!(detail.contains("onset"));
    assert!(detail.contains("tempo"));
    assert!(detail.contains("librosa"));
}

#[test]
fn omitted_min_confidence_defaults_to_half() {
    let args = args_default();
    assert_eq!(
        resolved_min_confidence(&args).expect("default min confidence"),
        0.5
    );
}

#[test]
fn min_confidence_boundaries_are_accepted() {
    for value in [0.0, 1.0] {
        let mut args = args_default();
        args.min_confidence = Some(value);
        assert_eq!(
            resolved_min_confidence(&args).expect("boundary should pass"),
            value
        );
    }
}

#[test]
fn min_confidence_out_of_range_maps_to_custom_e_bad_range() {
    let prior = empty_project();
    let verb = AudioDetectBeatsVerb;
    for value in [-0.01, 1.01] {
        let mut raw = args_value_default();
        raw["min_confidence"] = json!(value);
        let detail = custom_detail(
            verb.compute_patch(&prior, &raw)
                .expect_err("out-of-range min_confidence should fail"),
        );
        assert!(detail.contains("E_BAD_RANGE"));
        assert!(detail.contains("min_confidence"));
    }
}

#[test]
fn min_confidence_non_finite_maps_bad_range_in_helper() {
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut args = args_default();
        args.min_confidence = Some(value);
        let err = resolved_min_confidence(&args).expect_err("non-finite should fail");
        assert!(matches!(err, AudioDetectBeatsError::BadRange { .. }));
        assert!(err.to_string().contains("E_BAD_RANGE"));
    }
}

#[test]
fn omitted_create_markers_defaults_true() {
    let args = args_default();
    assert!(resolved_create_markers(&args));
}

#[test]
fn qualified_clip_target_reaches_analysis_failed() {
    let prior = empty_project();
    let verb = AudioDetectBeatsVerb;
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
    let verb = AudioDetectBeatsVerb;
    let mut raw = args_value_default();
    raw["target"] = json!(format!("track:{TRACK_ID}"));
    let detail = custom_detail(
        verb.compute_patch(&prior, &raw)
            .expect_err("track target should fail with analysis floor"),
    );
    assert!(detail.contains("E_ANALYSIS_FAILED"));
}

#[test]
fn qualified_asset_target_with_markers_false_reaches_analysis_failed() {
    let prior = empty_project();
    let verb = AudioDetectBeatsVerb;
    let mut raw = args_value_default();
    raw["target"] = json!(format!("asset:{ASSET_ID}"));
    raw["create_markers"] = json!(false);
    let detail = custom_detail(
        verb.compute_patch(&prior, &raw)
            .expect_err("asset+no_markers should reach analysis floor"),
    );
    assert!(detail.contains("E_ANALYSIS_FAILED"));
}

#[test]
fn bare_selector_rejected_as_bad_selector() {
    let prior = empty_project();
    let verb = AudioDetectBeatsVerb;
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
    let verb = AudioDetectBeatsVerb;
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
    let verb = AudioDetectBeatsVerb;
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
    let verb = AudioDetectBeatsVerb;
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
    let verb = AudioDetectBeatsVerb;
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
    let verb = AudioDetectBeatsVerb;
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
fn asset_target_with_default_markers_rejected_as_args_incompatible() {
    let prior = empty_project();
    let verb = AudioDetectBeatsVerb;
    let mut raw = args_value_default();
    raw["target"] = json!(format!("asset:{ASSET_ID}"));
    let detail = custom_detail(
        verb.compute_patch(&prior, &raw)
            .expect_err("asset target with default markers should reject"),
    );
    assert!(detail.contains("E_ARGS_INCOMPATIBLE"));
}

#[test]
fn asset_target_with_explicit_true_markers_rejected_as_args_incompatible() {
    let prior = empty_project();
    let verb = AudioDetectBeatsVerb;
    let mut raw = args_value_default();
    raw["target"] = json!(format!("asset:{ASSET_ID}"));
    raw["create_markers"] = json!(true);
    let detail = custom_detail(
        verb.compute_patch(&prior, &raw)
            .expect_err("asset target with markers should reject"),
    );
    assert!(detail.contains("E_ARGS_INCOMPATIBLE"));
}

#[test]
fn future_data_serializes_exact_fields_and_omits_created_marker_ids_when_none() {
    let data = AudioDetectBeatsData {
        target_id: CLIP_ID.to_string(),
        target_kind: AudioAnalysisTargetKind::Clip,
        algorithm: AudioDetectBeatsAlgorithm::Onset,
        tempo_bpm: 128.0,
        beats_tk: vec![240_000, 360_000],
        confidence: vec![0.8, 0.9],
        mean_confidence_pre_filter: 0.7,
        kept_beat_count: 2,
        dropped_beat_count: 1,
        cache_path: "/tmp/cache/audio_analysis/clip.json".to_string(),
        cache_hit: false,
        created_marker_ids: None,
        removed_marker_ids: vec![],
    };

    let value = serde_json::to_value(&data).expect("data serializes");
    let obj = value.as_object().expect("data object");
    assert!(!obj.contains_key("created_marker_ids"));
    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "algorithm",
            "beats_tk",
            "cache_hit",
            "cache_path",
            "confidence",
            "dropped_beat_count",
            "kept_beat_count",
            "mean_confidence_pre_filter",
            "removed_marker_ids",
            "target_id",
            "target_kind",
            "tempo_bpm",
        ]
    );
}

#[test]
fn future_data_includes_created_marker_ids_when_present() {
    let data = AudioDetectBeatsData {
        target_id: TRACK_ID.to_string(),
        target_kind: AudioAnalysisTargetKind::Track,
        algorithm: AudioDetectBeatsAlgorithm::Tempo,
        tempo_bpm: 120.0,
        beats_tk: vec![1, 2],
        confidence: vec![0.5, 0.6],
        mean_confidence_pre_filter: 0.55,
        kept_beat_count: 2,
        dropped_beat_count: 0,
        cache_path: "/tmp/cache/audio_analysis/track.json".to_string(),
        cache_hit: true,
        created_marker_ids: Some(vec![
            "0190b8d3-15e3-7000-bd00-0000000dd910".to_string(),
            "0190b8d3-15e3-7000-bd00-0000000dd911".to_string(),
        ]),
        removed_marker_ids: vec![],
    };
    let value = serde_json::to_value(&data).expect("data serializes");
    assert!(value.get("created_marker_ids").is_some());
}

#[test]
fn reserved_error_variants_display_spec_codes_and_map_to_custom() {
    let cases = vec![
        AudioDetectBeatsError::AssetNoAudio {
            target_id: CLIP_ID.to_string(),
            target_kind: AudioAnalysisTargetKind::Clip,
        },
        AudioDetectBeatsError::AssetUnsupportedKind {
            asset_kind: "image".to_string(),
        },
        AudioDetectBeatsError::NoMatch {
            selector: "clip:audio[name=\"missing\"][0]".to_string(),
        },
        AudioDetectBeatsError::NotFound {
            target_kind: "clip".to_string(),
            target_id: CLIP_ID.to_string(),
        },
        AudioDetectBeatsError::TrackKindMismatch {
            track_id: TRACK_ID.to_string(),
            actual_kind: "video".to_string(),
        },
        AudioDetectBeatsError::ClipKindMismatch {
            clip_id: CLIP_ID.to_string(),
            actual_kind: "video".to_string(),
        },
        AudioDetectBeatsError::Locked {
            kind: "track".to_string(),
            id: TRACK_ID.to_string(),
        },
        AudioDetectBeatsError::SchemaViolation {
            field: "patch_size".to_string(),
            size_bytes: 2_000_000,
            cap_bytes: 1_048_576,
            hint: "reduce emitted markers".to_string(),
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
        ("decoder_init", AudioDetectBeatsStage::DecoderInit),
        ("audio_decode", AudioDetectBeatsStage::AudioDecode),
        ("algorithm_step", AudioDetectBeatsStage::AlgorithmStep),
        ("cache_write", AudioDetectBeatsStage::CacheWrite),
    ];
    for (literal, expected) in cases {
        let parsed: AudioDetectBeatsStage =
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
    let verb = AudioDetectBeatsVerb;
    let prior = empty_project();
    let value = verb
        .reconstruct(&args_value_default(), &json!([]), &[], &prior)
        .expect("reconstruct should succeed");
    assert_eq!(value, Value::Null);
}

#[test]
fn reconstruct_rejects_malformed_args() {
    let verb = AudioDetectBeatsVerb;
    let prior = empty_project();
    let err = verb
        .reconstruct(&json!({}), &json!([]), &[], &prior)
        .expect_err("malformed args should reject");
    assert!(err.to_string().contains("AudioDetectBeatsArgs"));
}

#[test]
fn reconstruct_from_default_fixture_with_single_verb_registry() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "audio.detect_beats")
        .expect("fixture present");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(AudioDetectBeatsVerb))
        .expect("register verb");

    let report = validate_reconstructors(&registry, &[fixture]).expect("validate fixture");
    assert_eq!(report.verbs_checked, vec!["audio.detect_beats"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn default_fixture_has_empty_patch_warnings_and_null_expected_data() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "audio.detect_beats")
        .expect("fixture present");
    assert_eq!(fixture.patch, json!([]));
    assert_eq!(fixture.warnings, Vec::<Value>::new());
    assert_eq!(fixture.expected_data, Value::Null);
}

#[test]
fn default_registry_contains_audio_detect_beats() {
    let registry = default_registry();
    assert!(
        registry.get("audio.detect_beats").is_some(),
        "audio.detect_beats should be registered"
    );
}

#[test]
fn verb_trait_lookup_via_default_registry_returns_analysis_failed() {
    let registry = default_registry();
    let verb = registry
        .get("audio.detect_beats")
        .expect("audio.detect_beats in registry");
    let prior = empty_project();
    let detail = custom_detail(
        verb.compute_patch(&prior, &args_value_default())
            .expect_err("v1 floor should fail"),
    );
    assert!(detail.contains("E_ANALYSIS_FAILED"));
}

#[cfg(feature = "native")]
#[test]
fn native_mutate_via_verb_returns_analysis_failed_for_clip_and_track() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        empty_project(),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry");

    for target in [format!("clip:{CLIP_ID}"), format!("track:{TRACK_ID}")] {
        let raw = json!({
            "project_id": FIXTURE_PROJECT_ID,
            "target": target,
        });
        let outcome = store.mutate_via_verb("audio.detect_beats", raw, None);
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
fn native_mutate_via_verb_returns_unknown_algorithm_for_bad_algorithm() {
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
        "audio.detect_beats",
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "target": format!("clip:{CLIP_ID}"),
            "algorithm": "bad_algo",
        }),
        None,
    );

    let Err(LifecycleError::VerbExecutionFailed { source, .. }) = outcome else {
        panic!("expected VerbExecutionFailed, got {outcome:?}");
    };
    let VerbError::Custom(detail) = source else {
        panic!("expected Custom source, got {source:?}");
    };
    assert!(detail.contains("E_ANALYSIS_UNKNOWN_ALGORITHM"));
}

#[test]
fn compute_patch_helper_returns_analysis_failed_for_accepted_target() {
    let prior = empty_project();
    let args = args_default();
    let err = compute_patch(&prior, &args).expect_err("v1 floor should error");
    assert!(matches!(err, AudioDetectBeatsError::AnalysisFailed { .. }));
}
