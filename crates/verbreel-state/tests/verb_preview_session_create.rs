//! Tests for `preview.session.create` (§15.1) — v1 session-manager floor.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::preview_session_create::{
    compute_patch, resolved_audio_enabled, resolved_channel_kind, resolved_playback_rate,
    resolved_start_at_tk,
};
use verbreel_state::{
    PreviewSessionChannelKind, PreviewSessionCreateArgs, PreviewSessionCreateData,
    PreviewSessionCreateError, PreviewSessionCreateVerb, Project, Verb, VerbError, VerbRegistry,
    default_fixtures, default_registry, validate_reconstructors,
};

#[cfg(feature = "native")]
use verbreel_state::{LifecycleError, ProjectStore};

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture -> Project")
}

fn fixture_project_id() -> verbreel_types::ProjectId {
    empty_project().id
}

fn args_default() -> PreviewSessionCreateArgs {
    PreviewSessionCreateArgs {
        project_id: fixture_project_id(),
        playback_rate: None,
        width_px: None,
        audio_enabled: true,
        format: PreviewSessionChannelKind::Ndjson,
        start_at_tk: None,
    }
}

#[test]
fn args_deserialize_ok_with_minimal_fields() {
    let typed: PreviewSessionCreateArgs =
        serde_json::from_value(json!({ "project_id": FIXTURE_PROJECT_ID })).expect("args parse");
    assert_eq!(typed.project_id.to_string(), FIXTURE_PROJECT_ID);
    assert_eq!(typed.playback_rate, None);
    assert_eq!(typed.width_px, None);
    assert!(typed.audio_enabled);
    assert_eq!(typed.format, PreviewSessionChannelKind::Ndjson);
    assert_eq!(typed.start_at_tk, None);
}

#[test]
fn args_deserialize_ok_with_all_optionals() {
    let typed: PreviewSessionCreateArgs = serde_json::from_value(json!({
        "project_id": FIXTURE_PROJECT_ID,
        "playback_rate": 2.5,
        "width_px": 1920,
        "audio_enabled": false,
        "format": "ndjson",
        "start_at_tk": 240000,
    }))
    .expect("args parse");
    assert_eq!(typed.playback_rate, Some(2.5));
    assert_eq!(typed.width_px, Some(1920));
    assert!(!typed.audio_enabled);
    assert_eq!(typed.format, PreviewSessionChannelKind::Ndjson);
    assert_eq!(typed.start_at_tk, Some(240000));
}

#[test]
fn default_helpers_resolve_omitted_values() {
    let args: PreviewSessionCreateArgs =
        serde_json::from_value(json!({ "project_id": FIXTURE_PROJECT_ID })).expect("args parse");
    assert_eq!(resolved_playback_rate(&args), 1.0);
    assert!(resolved_audio_enabled(&args));
    assert_eq!(
        resolved_channel_kind(&args),
        PreviewSessionChannelKind::Ndjson
    );
    assert_eq!(resolved_start_at_tk(&args), 0);
}

#[test]
fn unknown_field_rejected_through_verb() {
    let prior = empty_project();
    let verb = PreviewSessionCreateVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "extra": true
            }),
        )
        .expect_err("deny_unknown_fields rejects extras");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn missing_project_id_fails_through_verb() {
    let prior = empty_project();
    let verb = PreviewSessionCreateVerb;
    let err = verb
        .compute_patch(&prior, &json!({}))
        .expect_err("missing project_id should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn non_numeric_playback_rate_fails_through_verb() {
    let prior = empty_project();
    let verb = PreviewSessionCreateVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "playback_rate": "fast"
            }),
        )
        .expect_err("non-number playback_rate should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn non_integer_width_px_fails_through_verb() {
    let prior = empty_project();
    let verb = PreviewSessionCreateVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "width_px": "wide"
            }),
        )
        .expect_err("non-integer width_px should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn non_boolean_audio_enabled_fails_through_verb() {
    let prior = empty_project();
    let verb = PreviewSessionCreateVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "audio_enabled": "yes"
            }),
        )
        .expect_err("non-boolean audio_enabled should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn playback_rate_out_of_range_returns_bad_range() {
    let prior = empty_project();
    for rate in [0.09_f64, 8.1_f64] {
        let err = compute_patch(
            &prior,
            &PreviewSessionCreateArgs {
                playback_rate: Some(rate),
                ..args_default()
            },
        )
        .expect_err("out-of-range playback_rate should fail");
        let PreviewSessionCreateError::BadRange {
            field,
            value,
            allowed,
        } = err
        else {
            panic!("expected BadRange, got {err:?}");
        };
        assert_eq!(field, "playback_rate");
        assert_eq!(value, rate.to_string());
        assert_eq!(allowed, "[0.1, 8.0]");
    }
}

#[test]
fn playback_rate_out_of_range_maps_to_bad_args_through_verb() {
    let prior = empty_project();
    let verb = PreviewSessionCreateVerb;
    for rate in [0.09_f64, 8.1_f64] {
        let err = verb
            .compute_patch(
                &prior,
                &json!({
                    "project_id": FIXTURE_PROJECT_ID,
                    "playback_rate": rate
                }),
            )
            .expect_err("out-of-range playback_rate should map to BadArgs");
        let VerbError::BadArgs { detail } = err else {
            panic!("expected BadArgs, got {err:?}");
        };
        assert!(detail.contains("E_BAD_RANGE"));
    }
}

#[test]
fn width_px_zero_or_negative_returns_bad_range() {
    let prior = empty_project();
    for width in [0_i64, -1_i64] {
        let err = compute_patch(
            &prior,
            &PreviewSessionCreateArgs {
                width_px: Some(width),
                ..args_default()
            },
        )
        .expect_err("width_px <= 0 should fail");
        let PreviewSessionCreateError::BadRange {
            field,
            value,
            allowed,
        } = err
        else {
            panic!("expected BadRange, got {err:?}");
        };
        assert_eq!(field, "width_px");
        assert_eq!(value, width.to_string());
        assert_eq!(allowed, ">= 1");
    }
}

#[test]
fn width_px_8193_reaches_runtime_floor_not_bad_range() {
    let prior = empty_project();
    let err = compute_patch(
        &prior,
        &PreviewSessionCreateArgs {
            width_px: Some(8193),
            ..args_default()
        },
    )
    .expect_err("v1 floor should still return SessionLimit");
    assert!(matches!(
        err,
        PreviewSessionCreateError::SessionLimit { .. }
    ));
}

#[test]
fn negative_start_at_tk_returns_bad_time() {
    let prior = empty_project();
    let err = compute_patch(
        &prior,
        &PreviewSessionCreateArgs {
            start_at_tk: Some(-1),
            ..args_default()
        },
    )
    .expect_err("negative start_at_tk should fail");
    let PreviewSessionCreateError::BadTime { start_at_tk } = err else {
        panic!("expected BadTime, got {err:?}");
    };
    assert_eq!(start_at_tk, -1);
}

#[test]
fn start_at_tk_past_duration_reaches_runtime_floor_in_v1() {
    let prior = empty_project();
    let err = compute_patch(
        &prior,
        &PreviewSessionCreateArgs {
            start_at_tk: Some(i64::MAX / 2),
            ..args_default()
        },
    )
    .expect_err("v1 floor should return SessionLimit for large non-negative start");
    assert!(matches!(
        err,
        PreviewSessionCreateError::SessionLimit { .. }
    ));
}

#[test]
fn omitted_format_reaches_preview_session_limit() {
    let prior = empty_project();
    let err = compute_patch(&prior, &args_default()).expect_err("v1 floor always errors");
    assert!(matches!(
        err,
        PreviewSessionCreateError::SessionLimit { .. }
    ));
}

#[test]
fn explicit_ndjson_reaches_preview_session_limit() {
    let prior = empty_project();
    let err = compute_patch(
        &prior,
        &PreviewSessionCreateArgs {
            format: PreviewSessionChannelKind::Ndjson,
            ..args_default()
        },
    )
    .expect_err("v1 floor always errors");
    assert!(matches!(
        err,
        PreviewSessionCreateError::SessionLimit { .. }
    ));
}

#[test]
fn webrtc_and_mse_return_unknown_kind_with_ndjson_allowed() {
    let prior = empty_project();
    for kind in [
        PreviewSessionChannelKind::Webrtc,
        PreviewSessionChannelKind::Mse,
    ] {
        let err = compute_patch(
            &prior,
            &PreviewSessionCreateArgs {
                format: kind,
                ..args_default()
            },
        )
        .expect_err("reserved kinds should fail");
        let PreviewSessionCreateError::UnknownKind {
            requested,
            allowed,
            hint,
        } = err
        else {
            panic!("expected UnknownKind, got {err:?}");
        };
        let expected = match kind {
            PreviewSessionChannelKind::Webrtc => "webrtc",
            PreviewSessionChannelKind::Mse => "mse",
            PreviewSessionChannelKind::Ndjson => unreachable!("not in loop"),
        };
        assert_eq!(requested, expected);
        assert_eq!(allowed, vec!["ndjson".to_string()]);
        assert!(hint.contains("use ndjson"));
    }
}

#[test]
fn unknown_kind_maps_to_bad_args_through_verb() {
    let prior = empty_project();
    let verb = PreviewSessionCreateVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "format": "webrtc"
            }),
        )
        .expect_err("reserved kind should map to BadArgs");
    let VerbError::BadArgs { detail } = err else {
        panic!("expected BadArgs, got {err:?}");
    };
    assert!(detail.contains("E_UNKNOWN_KIND"));
}

#[test]
fn runtime_session_limit_maps_to_custom_and_includes_context() {
    let prior = empty_project();
    let verb = PreviewSessionCreateVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "playback_rate": 1.25,
                "width_px": 8193,
                "audio_enabled": true,
                "format": "ndjson",
                "start_at_tk": 0
            }),
        )
        .expect_err("well-formed args should hit SessionLimit floor");
    let VerbError::Custom(detail) = err else {
        panic!("expected Custom, got {err:?}");
    };
    assert!(detail.contains("E_PREVIEW_SESSION_LIMIT"));
    assert!(detail.contains(FIXTURE_PROJECT_ID));
    assert!(detail.contains("cap 4"));
    assert!(detail.contains("active_session_ids=[]"));
}

#[test]
fn future_success_data_shape_serializes_expected_fields() {
    let data = PreviewSessionCreateData {
        session_id: "0190b8d3-15e3-7000-bd00-0000feedbeef".to_string(),
        playback_rate: 1.25,
        width_px: 1920,
        height_px: 1080,
        audio_enabled: true,
        frame_count_estimate: 42,
        channel_kind: PreviewSessionChannelKind::Ndjson,
    };
    let value = serde_json::to_value(&data).expect("data serializes");
    let obj = value.as_object().expect("object");
    assert_eq!(obj.len(), 7);
    assert_eq!(
        obj.get("session_id"),
        Some(&json!("0190b8d3-15e3-7000-bd00-0000feedbeef"))
    );
    assert_eq!(obj.get("playback_rate"), Some(&json!(1.25)));
    assert_eq!(obj.get("width_px"), Some(&json!(1920)));
    assert_eq!(obj.get("height_px"), Some(&json!(1080)));
    assert_eq!(obj.get("audio_enabled"), Some(&json!(true)));
    assert_eq!(obj.get("frame_count_estimate"), Some(&json!(42)));
    assert_eq!(obj.get("channel_kind"), Some(&json!("ndjson")));
}

#[test]
fn reserved_error_variants_display_e_literals() {
    let not_found = PreviewSessionCreateError::ProjectNotFound {
        project_id: FIXTURE_PROJECT_ID.to_string(),
    }
    .to_string();
    assert!(not_found.contains("E_PROJECT_NOT_FOUND"));

    let session_limit = PreviewSessionCreateError::SessionLimit {
        project_id: FIXTURE_PROJECT_ID.to_string(),
        active_session_ids: vec![],
        cap: 4,
        detail: "v1 floor".to_string(),
    }
    .to_string();
    assert!(session_limit.contains("E_PREVIEW_SESSION_LIMIT"));
}

#[test]
fn reconstruct_returns_null_for_well_formed_args() {
    let verb = PreviewSessionCreateVerb;
    let prior = empty_project();
    let args = serde_json::to_value(args_default()).expect("args -> Value");
    let data = verb
        .reconstruct(&args, &json!([]), &[], &prior)
        .expect("reconstruct succeeds for well-formed args");
    assert_eq!(data, Value::Null);
}

#[test]
fn reconstruct_rejects_malformed_args() {
    let verb = PreviewSessionCreateVerb;
    let prior = empty_project();
    let err = verb
        .reconstruct(&json!({}), &json!([]), &[], &prior)
        .expect_err("malformed args should fail reconstruction");
    let msg = err.to_string();
    assert!(
        msg.contains("PreviewSessionCreateArgs") || msg.contains("wrong type"),
        "unexpected reconstruct error: {msg}",
    );
}

#[test]
fn default_fixture_validates_with_only_preview_session_create_verb() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "preview.session.create")
        .expect("default_fixtures includes preview.session.create");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(PreviewSessionCreateVerb))
        .expect("register preview.session.create verb");

    let report = validate_reconstructors(&registry, &[fixture]).expect("fixture validates");
    assert_eq!(report.verbs_checked, vec!["preview.session.create"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn default_fixture_has_empty_patch_warnings_and_null_expected_data() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "preview.session.create")
        .expect("default_fixtures includes preview.session.create");
    assert_eq!(fixture.patch, json!([]));
    assert!(fixture.warnings.is_empty());
    assert_eq!(fixture.expected_data, Value::Null);
}

#[test]
fn default_registry_contains_preview_session_create() {
    let registry = default_registry();
    let verb = registry
        .get("preview.session.create")
        .expect("preview.session.create is in default_registry");
    let prior = empty_project();
    let err = verb
        .compute_patch(&prior, &json!({ "project_id": FIXTURE_PROJECT_ID }))
        .expect_err("v1 floor always errors");
    let VerbError::Custom(detail) = err else {
        panic!("expected Custom, got {err:?}");
    };
    assert!(detail.contains("E_PREVIEW_SESSION_LIMIT"));
}

#[cfg(feature = "native")]
#[test]
fn mutate_via_verb_route_returns_runtime_session_limit_floor() {
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
        "preview.session.create",
        json!({ "project_id": FIXTURE_PROJECT_ID }),
        None,
    );
    let Err(LifecycleError::VerbExecutionFailed { source, .. }) = outcome else {
        panic!("expected VerbExecutionFailed, got {outcome:?}");
    };
    let VerbError::Custom(detail) = source else {
        panic!("expected Custom source, got {source:?}");
    };
    assert!(detail.contains("E_PREVIEW_SESSION_LIMIT"));
}
