//! Tests for `preview.waveform` (§14.2) — v1 waveform/cache floor.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::preview_waveform::{compute_patch, resolved_samples};
use verbreel_state::{
    PreviewWaveformArgs, PreviewWaveformData, PreviewWaveformError, PreviewWaveformVerb, Project,
    Verb, VerbError, VerbRegistry, default_fixtures, default_registry, validate_reconstructors,
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

fn args_default() -> PreviewWaveformArgs {
    PreviewWaveformArgs {
        project_id: fixture_project_id(),
        target: "asset:0190b8d3-15e3-7000-bd00-00000000aaaa".to_string(),
        samples: None,
        out_path: None,
    }
}

fn args_value() -> Value {
    json!({
        "project_id": FIXTURE_PROJECT_ID,
        "target": "asset:0190b8d3-15e3-7000-bd00-00000000aaaa",
    })
}

#[test]
fn args_deserialize_ok_with_minimal_fields() {
    let typed: PreviewWaveformArgs = serde_json::from_value(args_value()).expect("args parse");
    assert_eq!(typed.project_id.to_string(), FIXTURE_PROJECT_ID);
    assert_eq!(typed.target, "asset:0190b8d3-15e3-7000-bd00-00000000aaaa");
    assert_eq!(typed.samples, None);
    assert_eq!(typed.out_path, None);
}

#[test]
fn args_deserialize_ok_with_all_optionals() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "target": "clip:0190b8d3-15e3-7000-bd00-00000000bbbb",
        "samples": 4096,
        "out_path": "tmp/waveform.json",
    });
    let typed: PreviewWaveformArgs = serde_json::from_value(raw).expect("args parse");
    assert_eq!(typed.project_id.to_string(), FIXTURE_PROJECT_ID);
    assert_eq!(typed.target, "clip:0190b8d3-15e3-7000-bd00-00000000bbbb");
    assert_eq!(typed.samples, Some(4096));
    assert_eq!(typed.out_path.as_deref(), Some("tmp/waveform.json"));
}

#[test]
fn omitted_samples_resolves_to_1024() {
    let typed: PreviewWaveformArgs = serde_json::from_value(args_value()).expect("args parse");
    assert_eq!(resolved_samples(&typed), 1024);
}

#[test]
fn explicit_samples_preserved_and_resolved() {
    let typed: PreviewWaveformArgs = serde_json::from_value(json!({
        "project_id": FIXTURE_PROJECT_ID,
        "target": "track:audio[0]",
        "samples": 16384
    }))
    .expect("args parse");
    assert_eq!(typed.samples, Some(16384));
    assert_eq!(resolved_samples(&typed), 16384);
}

#[test]
fn unknown_field_rejected_through_verb() {
    let prior = empty_project();
    let verb = PreviewWaveformVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "target": "asset:0190b8d3-15e3-7000-bd00-00000000aaaa",
                "extra": true
            }),
        )
        .expect_err("deny_unknown_fields rejects extras");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn missing_project_id_fails_through_verb() {
    let prior = empty_project();
    let verb = PreviewWaveformVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({ "target": "asset:0190b8d3-15e3-7000-bd00-00000000aaaa" }),
        )
        .expect_err("missing project_id should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn missing_target_fails_through_verb() {
    let prior = empty_project();
    let verb = PreviewWaveformVerb;
    let err = verb
        .compute_patch(&prior, &json!({ "project_id": FIXTURE_PROJECT_ID }))
        .expect_err("missing target should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn non_integer_samples_fails_through_verb() {
    let prior = empty_project();
    let verb = PreviewWaveformVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "target": "asset:0190b8d3-15e3-7000-bd00-00000000aaaa",
                "samples": "many"
            }),
        )
        .expect_err("non-integer samples should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn empty_target_returns_bad_selector() {
    let prior = empty_project();
    let err = compute_patch(
        &prior,
        &PreviewWaveformArgs {
            target: String::new(),
            ..args_default()
        },
    )
    .expect_err("empty target should fail");

    let PreviewWaveformError::BadSelector { detail, .. } = err else {
        panic!("expected BadSelector, got {err:?}");
    };
    assert!(detail.contains("empty"));
}

#[test]
fn bare_uuid_target_returns_bad_selector() {
    let prior = empty_project();
    let err = compute_patch(
        &prior,
        &PreviewWaveformArgs {
            target: "0190b8d3-15e3-7000-bd00-00000000aaaa".to_string(),
            ..args_default()
        },
    )
    .expect_err("bare uuid target should fail");
    let PreviewWaveformError::BadSelector { .. } = err else {
        panic!("expected BadSelector, got {err:?}");
    };
}

#[test]
fn unknown_prefix_target_returns_bad_selector() {
    let prior = empty_project();
    let err = compute_patch(
        &prior,
        &PreviewWaveformArgs {
            target: "video:0190b8d3-15e3-7000-bd00-00000000aaaa".to_string(),
            ..args_default()
        },
    )
    .expect_err("unknown target prefix should fail");
    let PreviewWaveformError::BadSelector { detail, .. } = err else {
        panic!("expected BadSelector, got {err:?}");
    };
    assert!(detail.contains("unknown"));
}

#[test]
fn selector_shape_errors_map_to_bad_args_through_verb() {
    let prior = empty_project();
    let verb = PreviewWaveformVerb;
    let cases = [
        "",
        "0190b8d3-15e3-7000-bd00-00000000aaaa",
        "video:0190b8d3-15e3-7000-bd00-00000000aaaa",
    ];

    for target in cases {
        let err = verb
            .compute_patch(
                &prior,
                &json!({
                    "project_id": FIXTURE_PROJECT_ID,
                    "target": target,
                }),
            )
            .expect_err("invalid selector should map to BadArgs");
        let VerbError::BadArgs { detail } = err else {
            panic!("expected BadArgs, got {err:?}");
        };
        assert!(detail.contains("E_BAD_SELECTOR"));
    }
}

#[test]
fn qualified_asset_target_reaches_io_floor() {
    let prior = empty_project();
    let err = compute_patch(&prior, &args_default()).expect_err("v1 floor should return Io");
    assert!(matches!(err, PreviewWaveformError::Io { .. }));
}

#[test]
fn qualified_clip_target_reaches_io_floor() {
    let prior = empty_project();
    let err = compute_patch(
        &prior,
        &PreviewWaveformArgs {
            target: "clip:0190b8d3-15e3-7000-bd00-00000000bbbb".to_string(),
            ..args_default()
        },
    )
    .expect_err("v1 floor should return Io");
    assert!(matches!(err, PreviewWaveformError::Io { .. }));
}

#[test]
fn qualified_track_target_reaches_io_floor() {
    let prior = empty_project();
    let err = compute_patch(
        &prior,
        &PreviewWaveformArgs {
            target: "track:audio[0]".to_string(),
            ..args_default()
        },
    )
    .expect_err("v1 floor should return Io");
    assert!(matches!(err, PreviewWaveformError::Io { .. }));
}

#[test]
fn samples_out_of_range_returns_bad_range() {
    let prior = empty_project();
    let cases = [0_i64, -1, 100_001];

    for samples in cases {
        let err = compute_patch(
            &prior,
            &PreviewWaveformArgs {
                samples: Some(samples),
                ..args_default()
            },
        )
        .expect_err("out-of-range samples should fail");
        let PreviewWaveformError::BadRange {
            field,
            value,
            allowed,
        } = err
        else {
            panic!("expected BadRange, got {err:?}");
        };
        assert_eq!(field, "samples");
        assert_eq!(value, samples);
        assert_eq!(allowed, "[1, 100000]");
    }
}

#[test]
fn samples_out_of_range_maps_to_bad_args_through_verb() {
    let prior = empty_project();
    let verb = PreviewWaveformVerb;
    let cases = [0_i64, -1, 100_001];

    for samples in cases {
        let err = verb
            .compute_patch(
                &prior,
                &json!({
                    "project_id": FIXTURE_PROJECT_ID,
                    "target": "asset:0190b8d3-15e3-7000-bd00-00000000aaaa",
                    "samples": samples,
                }),
            )
            .expect_err("out-of-range samples should map to BadArgs");
        let VerbError::BadArgs { detail } = err else {
            panic!("expected BadArgs, got {err:?}");
        };
        assert!(detail.contains("E_BAD_RANGE"));
    }
}

#[test]
fn samples_lower_bound_reaches_io_floor() {
    let prior = empty_project();
    let err = compute_patch(
        &prior,
        &PreviewWaveformArgs {
            samples: Some(1),
            ..args_default()
        },
    )
    .expect_err("v1 floor should still return Io");
    assert!(matches!(err, PreviewWaveformError::Io { .. }));
}

#[test]
fn samples_upper_bound_reaches_io_floor() {
    let prior = empty_project();
    let err = compute_patch(
        &prior,
        &PreviewWaveformArgs {
            samples: Some(100_000),
            ..args_default()
        },
    )
    .expect_err("v1 floor should still return Io");
    assert!(matches!(err, PreviewWaveformError::Io { .. }));
}

#[test]
fn runtime_io_maps_to_custom_and_includes_context() {
    let prior = empty_project();
    let verb = PreviewWaveformVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "target": "clip:0190b8d3-15e3-7000-bd00-00000000bbbb",
                "samples": 2048,
                "out_path": "tmp/wave.json",
            }),
        )
        .expect_err("well-formed args should hit v1 Io floor");
    let VerbError::Custom(detail) = err else {
        panic!("expected Custom, got {err:?}");
    };
    assert!(detail.contains("E_IO"));
    assert!(detail.contains("clip:0190b8d3-15e3-7000-bd00-00000000bbbb"));
    assert!(detail.contains("samples 2048"));
    assert!(detail.contains("tmp/wave.json"));
}

#[test]
fn future_success_data_serializes_without_out_path_when_none() {
    let data = PreviewWaveformData {
        peaks: vec![],
        rms: vec![],
        samples: 1024,
        channels: "mono".to_string(),
        cache_path: "cache/waveforms/a.json".to_string(),
        out_path: None,
    };
    let value = serde_json::to_value(&data).expect("data serializes");
    let obj = value.as_object().expect("data object");
    assert_eq!(obj.get("peaks"), Some(&json!([])));
    assert_eq!(obj.get("rms"), Some(&json!([])));
    assert_eq!(obj.get("samples"), Some(&json!(1024)));
    assert_eq!(obj.get("channels"), Some(&json!("mono")));
    assert_eq!(
        obj.get("cache_path"),
        Some(&json!("cache/waveforms/a.json"))
    );
    assert!(!obj.contains_key("out_path"));
}

#[test]
fn future_success_data_with_float_arrays_serializes() {
    let data = PreviewWaveformData {
        peaks: vec![0.12, 0.34],
        rms: vec![0.08, 0.21],
        samples: 2,
        channels: "mono".to_string(),
        cache_path: "cache/waveforms/b.json".to_string(),
        out_path: Some("tmp/out.json".to_string()),
    };
    let value = serde_json::to_value(&data).expect("data serializes");
    let obj = value.as_object().expect("data object");
    assert_eq!(obj.get("peaks"), Some(&json!([0.12, 0.34])));
    assert_eq!(obj.get("rms"), Some(&json!([0.08, 0.21])));
    assert_eq!(obj.get("out_path"), Some(&json!("tmp/out.json")));
}

#[test]
fn reserved_error_variants_display_e_literals() {
    let messages = [
        PreviewWaveformError::NotFound {
            target: "asset:dead".to_string(),
        }
        .to_string(),
        PreviewWaveformError::NoMatch {
            selector: "clip:dead".to_string(),
        }
        .to_string(),
        PreviewWaveformError::TrackKindMismatch {
            target: "track:audio[0]".to_string(),
            actual_kind: "video".to_string(),
        }
        .to_string(),
        PreviewWaveformError::ClipKindMismatch {
            target: "clip:dead".to_string(),
            actual_kind: "text".to_string(),
        }
        .to_string(),
        PreviewWaveformError::AssetNoAudio {
            target: "asset:dead".to_string(),
        }
        .to_string(),
        PreviewWaveformError::AssetUnsupportedKind {
            target: "asset:dead".to_string(),
            actual_kind: "image".to_string(),
        }
        .to_string(),
        PreviewWaveformError::PathEscape {
            path: "../escape.json".to_string(),
        }
        .to_string(),
    ];

    let expected_codes = [
        "E_NOT_FOUND",
        "E_NO_MATCH",
        "E_TRACK_KIND_MISMATCH",
        "E_CLIP_KIND_MISMATCH",
        "E_ASSET_NO_AUDIO",
        "E_ASSET_UNSUPPORTED_KIND",
        "E_PATH_ESCAPE",
    ];

    for (message, code) in messages.iter().zip(expected_codes) {
        assert!(
            message.contains(code),
            "message `{message}` missing `{code}`"
        );
    }
}

#[test]
fn reconstruct_returns_null_for_well_formed_args() {
    let verb = PreviewWaveformVerb;
    let prior = empty_project();
    let data = verb
        .reconstruct(&args_value(), &json!([]), &[], &prior)
        .expect("reconstruct succeeds for well-formed args");
    assert_eq!(data, Value::Null);
}

#[test]
fn reconstruct_rejects_malformed_args() {
    let verb = PreviewWaveformVerb;
    let prior = empty_project();
    let err = verb
        .reconstruct(&json!({}), &json!([]), &[], &prior)
        .expect_err("malformed args should fail reconstruction");
    let msg = err.to_string();
    assert!(
        msg.contains("PreviewWaveformArgs") || msg.contains("wrong type"),
        "unexpected reconstruct error: {msg}",
    );
}

#[test]
fn default_fixture_validates_with_only_preview_waveform_verb() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "preview.waveform")
        .expect("default_fixtures includes preview.waveform");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(PreviewWaveformVerb))
        .expect("register preview.waveform verb");

    let report = validate_reconstructors(&registry, &[fixture]).expect("fixture validates");
    assert_eq!(report.verbs_checked, vec!["preview.waveform"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn default_fixture_has_empty_patch_warnings_and_null_expected_data() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "preview.waveform")
        .expect("default_fixtures includes preview.waveform");
    assert_eq!(fixture.patch, json!([]));
    assert!(fixture.warnings.is_empty());
    assert_eq!(fixture.expected_data, Value::Null);
}

#[test]
fn default_registry_contains_preview_waveform() {
    let registry = default_registry();
    let verb = registry
        .get("preview.waveform")
        .expect("preview.waveform is in default_registry");
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
fn mutate_via_verb_route_returns_custom_io() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        empty_project(),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry succeeds");

    let outcome = store.mutate_via_verb("preview.waveform", args_value(), None);
    let Err(LifecycleError::VerbExecutionFailed { source, .. }) = outcome else {
        panic!("expected VerbExecutionFailed, got {outcome:?}");
    };
    let VerbError::Custom(detail) = source else {
        panic!("expected Custom source, got {source:?}");
    };
    assert!(detail.contains("E_IO"));
}
