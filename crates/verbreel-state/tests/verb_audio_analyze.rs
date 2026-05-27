//! Tests for `audio.analyze` (§19.2) — v1 analysis-runtime floor.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::audio_analyze::{compute_patch, registered_features, resolved_features};
use verbreel_state::{
    AudioAnalysisTargetKind, AudioAnalyzeArgs, AudioAnalyzeData, AudioAnalyzeError,
    AudioAnalyzeFeature, AudioAnalyzeSection, AudioAnalyzeStage, AudioAnalyzeVerb, Project, Verb,
    VerbError, VerbRegistry, default_fixtures, default_registry, validate_reconstructors,
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

fn args_default() -> AudioAnalyzeArgs {
    AudioAnalyzeArgs {
        project_id: fixture_project_id(),
        target: format!("clip:{CLIP_ID}"),
        features: None,
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
    let parsed: AudioAnalyzeArgs = serde_json::from_value(raw).expect("minimal args parse");
    assert_eq!(parsed.project_id.to_string(), FIXTURE_PROJECT_ID);
    assert_eq!(parsed.target, format!("clip:{CLIP_ID}"));
    assert_eq!(parsed.features, None);
    assert_eq!(parsed.from_tk, None);
    assert_eq!(parsed.to_tk, None);
}

#[test]
fn args_deserialize_all_optional_fields() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "target": format!("track:{TRACK_ID}"),
        "features": ["sections", "tempo"],
        "from_tk": 100,
        "to_tk": 200,
    });
    let parsed: AudioAnalyzeArgs = serde_json::from_value(raw).expect("args parse");
    assert_eq!(parsed.target, format!("track:{TRACK_ID}"));
    assert_eq!(
        parsed.features,
        Some(vec!["sections".to_string(), "tempo".to_string()])
    );
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
    let err = serde_json::from_value::<AudioAnalyzeArgs>(raw).expect_err("unknown field rejects");
    assert!(err.to_string().contains("unknown field"));
}

#[test]
fn args_missing_project_id_fails_through_verb() {
    let prior = empty_project();
    let verb = AudioAnalyzeVerb;
    let err = verb
        .compute_patch(&prior, &json!({ "target": format!("clip:{CLIP_ID}") }))
        .expect_err("missing project_id should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_missing_target_fails_through_verb() {
    let prior = empty_project();
    let verb = AudioAnalyzeVerb;
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
    let parsed = serde_json::from_value::<AudioAnalyzeArgs>(raw);
    assert!(parsed.is_err(), "non-string target should reject");
}

#[test]
fn args_non_array_features_rejected() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "target": format!("clip:{CLIP_ID}"),
        "features": "tempo",
    });
    let parsed = serde_json::from_value::<AudioAnalyzeArgs>(raw);
    assert!(parsed.is_err(), "non-array features should reject");
}

#[test]
fn args_non_string_feature_entries_rejected() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "target": format!("clip:{CLIP_ID}"),
        "features": ["tempo", 123],
    });
    let parsed = serde_json::from_value::<AudioAnalyzeArgs>(raw);
    assert!(parsed.is_err(), "non-string feature entry should reject");
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
        let parsed = serde_json::from_value::<AudioAnalyzeArgs>(raw);
        assert!(parsed.is_err(), "non-integer tick field should reject");
    }
}

#[test]
fn registered_features_helper_returns_spec_literals() {
    assert_eq!(
        registered_features(),
        ["tempo", "key", "energy", "sections", "spectral_centroid"]
    );
}

#[test]
fn omitted_features_default_to_all_in_spec_order() {
    let resolved = resolved_features(&args_default()).expect("features resolve");
    assert_eq!(
        resolved,
        vec![
            AudioAnalyzeFeature::Tempo,
            AudioAnalyzeFeature::Key,
            AudioAnalyzeFeature::Energy,
            AudioAnalyzeFeature::Sections,
            AudioAnalyzeFeature::SpectralCentroid,
        ]
    );
}

#[test]
fn each_registered_feature_literal_resolves_and_serializes() {
    let cases = vec![
        ("tempo", AudioAnalyzeFeature::Tempo),
        ("key", AudioAnalyzeFeature::Key),
        ("energy", AudioAnalyzeFeature::Energy),
        ("sections", AudioAnalyzeFeature::Sections),
        ("spectral_centroid", AudioAnalyzeFeature::SpectralCentroid),
    ];

    for (literal, expected) in cases {
        let mut args = args_default();
        args.features = Some(vec![literal.to_string()]);
        assert_eq!(
            resolved_features(&args).expect("feature resolves"),
            vec![expected]
        );
        assert_eq!(
            serde_json::to_value(expected).expect("feature serializes"),
            json!(literal)
        );
    }
}

#[test]
fn feature_order_is_normalized_and_duplicates_are_deduplicated() {
    let mut args = args_default();
    args.features = Some(vec![
        "sections".to_string(),
        "tempo".to_string(),
        "sections".to_string(),
        "key".to_string(),
        "tempo".to_string(),
    ]);
    let resolved = resolved_features(&args).expect("features resolve");
    assert_eq!(
        resolved,
        vec![
            AudioAnalyzeFeature::Tempo,
            AudioAnalyzeFeature::Key,
            AudioAnalyzeFeature::Sections,
        ]
    );
}

#[test]
fn unknown_feature_maps_to_custom_with_registered_list() {
    let prior = empty_project();
    let verb = AudioAnalyzeVerb;
    let mut raw = args_value_default();
    raw["features"] = json!(["tempo", "loudness"]);

    let detail = custom_detail(
        verb.compute_patch(&prior, &raw)
            .expect_err("unknown feature should fail"),
    );

    assert!(detail.contains("E_ANALYSIS_UNKNOWN_FEATURE"));
    assert!(detail.contains("loudness"));
    assert!(detail.contains("tempo"));
    assert!(detail.contains("key"));
    assert!(detail.contains("energy"));
    assert!(detail.contains("sections"));
    assert!(detail.contains("spectral_centroid"));
}

#[test]
fn qualified_clip_target_reaches_analysis_failed() {
    let prior = empty_project();
    let verb = AudioAnalyzeVerb;
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
    let verb = AudioAnalyzeVerb;
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
    let verb = AudioAnalyzeVerb;
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
    let verb = AudioAnalyzeVerb;
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
    let verb = AudioAnalyzeVerb;
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
    let verb = AudioAnalyzeVerb;
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
    let verb = AudioAnalyzeVerb;
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
    let verb = AudioAnalyzeVerb;
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
    let verb = AudioAnalyzeVerb;
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
fn accepted_failures_include_resolved_failed_features() {
    let prior = empty_project();
    let verb = AudioAnalyzeVerb;
    let mut raw = args_value_default();
    raw["features"] = json!(["sections", "tempo", "sections", "energy"]);

    let detail = custom_detail(
        verb.compute_patch(&prior, &raw)
            .expect_err("v1 floor should fail"),
    );

    assert!(detail.contains("E_ANALYSIS_FAILED"));
    assert!(detail.contains("failed_features"));
    assert!(detail.contains("tempo"));
    assert!(detail.contains("energy"));
    assert!(detail.contains("sections"));
    assert!(!detail.contains("key"));
    assert!(!detail.contains("spectral_centroid"));
}

#[test]
fn future_data_serializes_exact_fields_and_omits_optional_features_when_none() {
    let data = AudioAnalyzeData {
        target_id: CLIP_ID.to_string(),
        target_kind: AudioAnalysisTargetKind::Clip,
        features_returned: vec!["tempo".to_string(), "key".to_string()],
        tempo_bpm: None,
        key: None,
        energy_envelope_tk_value_pairs: None,
        sections: None,
        spectral_centroid_hz_avg: None,
        cache_path: "/tmp/cache/audio_analysis/clip.json".to_string(),
        cache_hit: false,
    };

    let value = serde_json::to_value(&data).expect("data serializes");
    let obj = value.as_object().expect("data object");
    assert!(!obj.contains_key("tempo_bpm"));
    assert!(!obj.contains_key("key"));
    assert!(!obj.contains_key("energy_envelope_tk_value_pairs"));
    assert!(!obj.contains_key("sections"));
    assert!(!obj.contains_key("spectral_centroid_hz_avg"));
    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "cache_hit",
            "cache_path",
            "features_returned",
            "target_id",
            "target_kind",
        ]
    );
}

#[test]
fn future_data_includes_optional_feature_fields_when_present() {
    let data = AudioAnalyzeData {
        target_id: ASSET_ID.to_string(),
        target_kind: AudioAnalysisTargetKind::Asset,
        features_returned: vec![
            "tempo".to_string(),
            "key".to_string(),
            "energy".to_string(),
            "sections".to_string(),
            "spectral_centroid".to_string(),
        ],
        tempo_bpm: Some(123.4),
        key: Some("C major".to_string()),
        energy_envelope_tk_value_pairs: Some(vec![(0, 0.1), (24_000, 0.2)]),
        sections: Some(vec![AudioAnalyzeSection {
            start_tk: 0,
            end_tk: 240_000,
            label: "intro".to_string(),
        }]),
        spectral_centroid_hz_avg: Some(2345.6),
        cache_path: "/tmp/cache/audio_analysis/asset.json".to_string(),
        cache_hit: true,
    };
    let value = serde_json::to_value(&data).expect("data serializes");
    assert!(value.get("tempo_bpm").is_some());
    assert!(value.get("key").is_some());
    assert!(value.get("energy_envelope_tk_value_pairs").is_some());
    assert!(value.get("sections").is_some());
    assert!(value.get("spectral_centroid_hz_avg").is_some());
}

#[test]
fn reserved_error_variants_display_spec_codes_and_map_to_custom() {
    let cases = vec![
        AudioAnalyzeError::AssetNoAudio {
            target_id: CLIP_ID.to_string(),
            target_kind: AudioAnalysisTargetKind::Clip,
        },
        AudioAnalyzeError::AssetUnsupportedKind {
            asset_kind: "image".to_string(),
        },
        AudioAnalyzeError::NoMatch {
            selector: "clip:audio[name=\"missing\"][0]".to_string(),
        },
        AudioAnalyzeError::NotFound {
            target_kind: "clip".to_string(),
            target_id: CLIP_ID.to_string(),
        },
        AudioAnalyzeError::TrackKindMismatch {
            track_id: TRACK_ID.to_string(),
            actual_kind: "video".to_string(),
        },
        AudioAnalyzeError::ClipKindMismatch {
            clip_id: CLIP_ID.to_string(),
            actual_kind: "video".to_string(),
        },
        AudioAnalyzeError::BadRange {
            field: "features".to_string(),
            detail: "unsupported shape".to_string(),
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
        ("decoder_init", AudioAnalyzeStage::DecoderInit),
        ("audio_decode", AudioAnalyzeStage::AudioDecode),
        ("algorithm_step", AudioAnalyzeStage::AlgorithmStep),
        ("cache_write", AudioAnalyzeStage::CacheWrite),
    ];
    for (literal, expected) in cases {
        let parsed: AudioAnalyzeStage =
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
    let verb = AudioAnalyzeVerb;
    let prior = empty_project();
    let value = verb
        .reconstruct(&args_value_default(), &json!([]), &[], &prior)
        .expect("reconstruct should succeed");
    assert_eq!(value, Value::Null);
}

#[test]
fn reconstruct_rejects_malformed_args() {
    let verb = AudioAnalyzeVerb;
    let prior = empty_project();
    let err = verb
        .reconstruct(&json!({}), &json!([]), &[], &prior)
        .expect_err("malformed args should reject");
    assert!(err.to_string().contains("AudioAnalyzeArgs"));
}

#[test]
fn reconstruct_from_default_fixture_with_single_verb_registry() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "audio.analyze")
        .expect("fixture present");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(AudioAnalyzeVerb))
        .expect("register verb");

    let report = validate_reconstructors(&registry, &[fixture]).expect("validate fixture");
    assert_eq!(report.verbs_checked, vec!["audio.analyze"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn default_fixture_has_empty_patch_warnings_and_null_expected_data() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "audio.analyze")
        .expect("fixture present");
    assert_eq!(fixture.patch, json!([]));
    assert_eq!(fixture.warnings, Vec::<Value>::new());
    assert_eq!(fixture.expected_data, Value::Null);
}

#[test]
fn default_registry_contains_audio_analyze() {
    let registry = default_registry();
    assert!(
        registry.get("audio.analyze").is_some(),
        "audio.analyze should be registered"
    );
}

#[test]
fn verb_trait_lookup_via_default_registry_returns_analysis_failed() {
    let registry = default_registry();
    let verb = registry
        .get("audio.analyze")
        .expect("audio.analyze in registry");
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
        let outcome = store.mutate_via_verb("audio.analyze", raw, None);
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
fn native_mutate_via_verb_returns_unknown_feature_for_bad_feature() {
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
        "audio.analyze",
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "target": format!("clip:{CLIP_ID}"),
            "features": ["tempo", "bad_feature"],
        }),
        None,
    );

    let Err(LifecycleError::VerbExecutionFailed { source, .. }) = outcome else {
        panic!("expected VerbExecutionFailed, got {outcome:?}");
    };
    let VerbError::Custom(detail) = source else {
        panic!("expected Custom source, got {source:?}");
    };
    assert!(detail.contains("E_ANALYSIS_UNKNOWN_FEATURE"));
}

#[test]
fn compute_patch_helper_returns_analysis_failed_for_accepted_target() {
    let prior = empty_project();
    let args = args_default();
    let err = compute_patch(&prior, &args).expect_err("v1 floor should error");
    let AudioAnalyzeError::AnalysisFailed {
        stage,
        failed_features,
        ..
    } = err
    else {
        panic!("expected AnalysisFailed");
    };
    assert_eq!(stage, AudioAnalyzeStage::AlgorithmStep);
    assert_eq!(
        failed_features,
        vec![
            "tempo".to_string(),
            "key".to_string(),
            "energy".to_string(),
            "sections".to_string(),
            "spectral_centroid".to_string(),
        ]
    );
}
