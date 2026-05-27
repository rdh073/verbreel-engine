//! Tests for `preview.thumbnail` (§14.3) — v1 thumbnail/cache floor.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::preview_thumbnail::compute_patch;
use verbreel_state::{
    PreviewThumbnailArgs, PreviewThumbnailData, PreviewThumbnailError, PreviewThumbnailVerb,
    Project, Verb, VerbError, VerbRegistry, default_fixtures, default_registry,
    validate_reconstructors,
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

fn args_default() -> PreviewThumbnailArgs {
    PreviewThumbnailArgs {
        project_id: fixture_project_id(),
        target: "asset:0190b8d3-15e3-7000-bd00-00000000aaaa".to_string(),
        count: 4,
        out_dir: None,
        width_px: None,
    }
}

fn args_value() -> Value {
    json!({
        "project_id": FIXTURE_PROJECT_ID,
        "target": "asset:0190b8d3-15e3-7000-bd00-00000000aaaa",
        "count": 4,
    })
}

#[test]
fn args_deserialize_ok_with_minimal_fields() {
    let typed: PreviewThumbnailArgs = serde_json::from_value(args_value()).expect("args parse");
    assert_eq!(typed.project_id.to_string(), FIXTURE_PROJECT_ID);
    assert_eq!(typed.target, "asset:0190b8d3-15e3-7000-bd00-00000000aaaa");
    assert_eq!(typed.count, 4);
    assert_eq!(typed.out_dir, None);
    assert_eq!(typed.width_px, None);
}

#[test]
fn args_deserialize_ok_with_all_optionals() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "target": "clip:0190b8d3-15e3-7000-bd00-00000000bbbb",
        "count": 8,
        "out_dir": "tmp/thumbs",
        "width_px": 640,
    });
    let typed: PreviewThumbnailArgs = serde_json::from_value(raw).expect("args parse");
    assert_eq!(typed.project_id.to_string(), FIXTURE_PROJECT_ID);
    assert_eq!(typed.target, "clip:0190b8d3-15e3-7000-bd00-00000000bbbb");
    assert_eq!(typed.count, 8);
    assert_eq!(typed.out_dir.as_deref(), Some("tmp/thumbs"));
    assert_eq!(typed.width_px, Some(640));
}

#[test]
fn unknown_field_rejected_through_verb() {
    let prior = empty_project();
    let verb = PreviewThumbnailVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "target": "asset:0190b8d3-15e3-7000-bd00-00000000aaaa",
                "count": 4,
                "extra": true
            }),
        )
        .expect_err("deny_unknown_fields rejects extras");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn missing_project_id_fails_through_verb() {
    let prior = empty_project();
    let verb = PreviewThumbnailVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "target": "asset:0190b8d3-15e3-7000-bd00-00000000aaaa",
                "count": 4
            }),
        )
        .expect_err("missing project_id should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn missing_target_fails_through_verb() {
    let prior = empty_project();
    let verb = PreviewThumbnailVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "count": 4
            }),
        )
        .expect_err("missing target should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn missing_count_fails_through_verb() {
    let prior = empty_project();
    let verb = PreviewThumbnailVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "target": "asset:0190b8d3-15e3-7000-bd00-00000000aaaa"
            }),
        )
        .expect_err("missing count should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn non_integer_count_fails_through_verb() {
    let prior = empty_project();
    let verb = PreviewThumbnailVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "target": "asset:0190b8d3-15e3-7000-bd00-00000000aaaa",
                "count": "many"
            }),
        )
        .expect_err("non-integer count should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn non_integer_width_px_fails_through_verb() {
    let prior = empty_project();
    let verb = PreviewThumbnailVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "target": "asset:0190b8d3-15e3-7000-bd00-00000000aaaa",
                "count": 4,
                "width_px": "wide"
            }),
        )
        .expect_err("non-integer width_px should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn empty_target_returns_bad_selector() {
    let prior = empty_project();
    let err = compute_patch(
        &prior,
        &PreviewThumbnailArgs {
            target: String::new(),
            ..args_default()
        },
    )
    .expect_err("empty target should fail");

    let PreviewThumbnailError::BadSelector { detail, .. } = err else {
        panic!("expected BadSelector, got {err:?}");
    };
    assert!(detail.contains("empty"));
}

#[test]
fn bare_uuid_target_returns_bad_selector() {
    let prior = empty_project();
    let err = compute_patch(
        &prior,
        &PreviewThumbnailArgs {
            target: "0190b8d3-15e3-7000-bd00-00000000aaaa".to_string(),
            ..args_default()
        },
    )
    .expect_err("bare uuid target should fail");
    let PreviewThumbnailError::BadSelector { .. } = err else {
        panic!("expected BadSelector, got {err:?}");
    };
}

#[test]
fn unknown_prefix_target_returns_bad_selector() {
    let prior = empty_project();
    let err = compute_patch(
        &prior,
        &PreviewThumbnailArgs {
            target: "video:0190b8d3-15e3-7000-bd00-00000000aaaa".to_string(),
            ..args_default()
        },
    )
    .expect_err("unknown target prefix should fail");
    let PreviewThumbnailError::BadSelector { detail, .. } = err else {
        panic!("expected BadSelector, got {err:?}");
    };
    assert!(detail.contains("unknown"));
}

#[test]
fn selector_shape_errors_map_to_bad_args_through_verb() {
    let prior = empty_project();
    let verb = PreviewThumbnailVerb;
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
                    "count": 4
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
fn track_target_returns_selector_kind_mismatch() {
    let prior = empty_project();
    let err = compute_patch(
        &prior,
        &PreviewThumbnailArgs {
            target: "track:audio[0]".to_string(),
            ..args_default()
        },
    )
    .expect_err("track target should fail with selector kind mismatch");
    let PreviewThumbnailError::SelectorKindMismatch { actual_prefix } = err else {
        panic!("expected SelectorKindMismatch, got {err:?}");
    };
    assert_eq!(actual_prefix, "track");
}

#[test]
fn known_non_thumbnail_prefixes_return_selector_kind_mismatch() {
    let prior = empty_project();
    let prefixes = ["effect", "keyframe", "marker"];

    for prefix in prefixes {
        let err = compute_patch(
            &prior,
            &PreviewThumbnailArgs {
                target: format!("{prefix}:deadbeef"),
                ..args_default()
            },
        )
        .expect_err("known unsupported prefix should fail");
        let PreviewThumbnailError::SelectorKindMismatch { actual_prefix } = err else {
            panic!("expected SelectorKindMismatch, got {err:?}");
        };
        assert_eq!(actual_prefix, prefix);
    }
}

#[test]
fn selector_kind_mismatch_maps_to_bad_args_through_verb() {
    let prior = empty_project();
    let verb = PreviewThumbnailVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "target": "track:audio[0]",
                "count": 4
            }),
        )
        .expect_err("selector kind mismatch should map to BadArgs");
    let VerbError::BadArgs { detail } = err else {
        panic!("expected BadArgs, got {err:?}");
    };
    assert!(detail.contains("E_SELECTOR_KIND_MISMATCH"));
}

#[test]
fn qualified_asset_target_reaches_io_floor() {
    let prior = empty_project();
    let err = compute_patch(&prior, &args_default()).expect_err("v1 floor should return Io");
    assert!(matches!(err, PreviewThumbnailError::Io { .. }));
}

#[test]
fn qualified_clip_target_reaches_io_floor() {
    let prior = empty_project();
    let err = compute_patch(
        &prior,
        &PreviewThumbnailArgs {
            target: "clip:0190b8d3-15e3-7000-bd00-00000000bbbb".to_string(),
            ..args_default()
        },
    )
    .expect_err("v1 floor should return Io");
    assert!(matches!(err, PreviewThumbnailError::Io { .. }));
}

#[test]
fn count_out_of_range_returns_bad_range() {
    let prior = empty_project();
    let cases = [0_i64, -1, 1001];

    for count in cases {
        let err = compute_patch(
            &prior,
            &PreviewThumbnailArgs {
                count,
                ..args_default()
            },
        )
        .expect_err("out-of-range count should fail");
        let PreviewThumbnailError::BadRange {
            field,
            value,
            allowed,
        } = err
        else {
            panic!("expected BadRange, got {err:?}");
        };
        assert_eq!(field, "count");
        assert_eq!(value, count);
        assert_eq!(allowed, "[1, 1000]");
    }
}

#[test]
fn count_out_of_range_maps_to_bad_args_through_verb() {
    let prior = empty_project();
    let verb = PreviewThumbnailVerb;
    let cases = [0_i64, -1, 1001];

    for count in cases {
        let err = verb
            .compute_patch(
                &prior,
                &json!({
                    "project_id": FIXTURE_PROJECT_ID,
                    "target": "asset:0190b8d3-15e3-7000-bd00-00000000aaaa",
                    "count": count
                }),
            )
            .expect_err("out-of-range count should map to BadArgs");
        let VerbError::BadArgs { detail } = err else {
            panic!("expected BadArgs, got {err:?}");
        };
        assert!(detail.contains("E_BAD_RANGE"));
    }
}

#[test]
fn width_px_out_of_range_returns_bad_range() {
    let prior = empty_project();
    let cases = [0_i64, -1, 8193];

    for width_px in cases {
        let err = compute_patch(
            &prior,
            &PreviewThumbnailArgs {
                width_px: Some(width_px),
                ..args_default()
            },
        )
        .expect_err("out-of-range width_px should fail");
        let PreviewThumbnailError::BadRange {
            field,
            value,
            allowed,
        } = err
        else {
            panic!("expected BadRange, got {err:?}");
        };
        assert_eq!(field, "width_px");
        assert_eq!(value, width_px);
        assert_eq!(allowed, "[1, 8192]");
    }
}

#[test]
fn width_px_out_of_range_maps_to_bad_args_through_verb() {
    let prior = empty_project();
    let verb = PreviewThumbnailVerb;
    let cases = [0_i64, -1, 8193];

    for width_px in cases {
        let err = verb
            .compute_patch(
                &prior,
                &json!({
                    "project_id": FIXTURE_PROJECT_ID,
                    "target": "asset:0190b8d3-15e3-7000-bd00-00000000aaaa",
                    "count": 4,
                    "width_px": width_px
                }),
            )
            .expect_err("out-of-range width_px should map to BadArgs");
        let VerbError::BadArgs { detail } = err else {
            panic!("expected BadArgs, got {err:?}");
        };
        assert!(detail.contains("E_BAD_RANGE"));
    }
}

#[test]
fn count_bounds_reach_io_floor() {
    let prior = empty_project();
    for count in [1_i64, 1000] {
        let err = compute_patch(
            &prior,
            &PreviewThumbnailArgs {
                count,
                ..args_default()
            },
        )
        .expect_err("v1 floor should still return Io");
        assert!(matches!(err, PreviewThumbnailError::Io { .. }));
    }
}

#[test]
fn width_bounds_reach_io_floor() {
    let prior = empty_project();
    for width_px in [1_i64, 8192] {
        let err = compute_patch(
            &prior,
            &PreviewThumbnailArgs {
                width_px: Some(width_px),
                ..args_default()
            },
        )
        .expect_err("v1 floor should still return Io");
        assert!(matches!(err, PreviewThumbnailError::Io { .. }));
    }
}

#[test]
fn runtime_io_maps_to_custom_and_includes_context() {
    let prior = empty_project();
    let verb = PreviewThumbnailVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "target": "clip:0190b8d3-15e3-7000-bd00-00000000bbbb",
                "count": 12,
                "out_dir": "tmp/thumbs",
                "width_px": 480
            }),
        )
        .expect_err("well-formed args should hit v1 Io floor");
    let VerbError::Custom(detail) = err else {
        panic!("expected Custom, got {err:?}");
    };
    assert!(detail.contains("E_IO"));
    assert!(detail.contains("clip:0190b8d3-15e3-7000-bd00-00000000bbbb"));
    assert!(detail.contains("count 12"));
    assert!(detail.contains("tmp/thumbs"));
}

#[test]
fn future_success_data_serializes_parallel_arrays() {
    let data = PreviewThumbnailData {
        paths: vec![
            "cache/thumbnails/a/thumb_0001.png".to_string(),
            "cache/thumbnails/a/thumb_0002.png".to_string(),
        ],
        sha256s: vec!["abc".to_string(), "def".to_string()],
    };
    let value = serde_json::to_value(&data).expect("data serializes");
    let obj = value.as_object().expect("data object");
    assert_eq!(
        obj.get("paths"),
        Some(&json!([
            "cache/thumbnails/a/thumb_0001.png",
            "cache/thumbnails/a/thumb_0002.png"
        ]))
    );
    assert_eq!(obj.get("sha256s"), Some(&json!(["abc", "def"])));
}

#[test]
fn reserved_error_variants_display_e_literals() {
    let messages = [
        PreviewThumbnailError::NotFound {
            target: "asset:dead".to_string(),
        }
        .to_string(),
        PreviewThumbnailError::NoMatch {
            selector: "clip:dead".to_string(),
        }
        .to_string(),
        PreviewThumbnailError::ClipKindMismatch {
            target: "clip:dead".to_string(),
            actual_kind: "audio".to_string(),
        }
        .to_string(),
        PreviewThumbnailError::AssetUnsupportedKind {
            target: "asset:dead".to_string(),
            actual_kind: "subtitle".to_string(),
        }
        .to_string(),
        PreviewThumbnailError::PathEscape {
            path: "../escape".to_string(),
        }
        .to_string(),
    ];

    let expected_codes = [
        "E_NOT_FOUND",
        "E_NO_MATCH",
        "E_CLIP_KIND_MISMATCH",
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
    let verb = PreviewThumbnailVerb;
    let prior = empty_project();
    let data = verb
        .reconstruct(&args_value(), &json!([]), &[], &prior)
        .expect("reconstruct succeeds for well-formed args");
    assert_eq!(data, Value::Null);
}

#[test]
fn reconstruct_rejects_malformed_args() {
    let verb = PreviewThumbnailVerb;
    let prior = empty_project();
    let err = verb
        .reconstruct(&json!({}), &json!([]), &[], &prior)
        .expect_err("malformed args should fail reconstruction");
    let msg = err.to_string();
    assert!(
        msg.contains("PreviewThumbnailArgs") || msg.contains("wrong type"),
        "unexpected reconstruct error: {msg}",
    );
}

#[test]
fn default_fixture_validates_with_only_preview_thumbnail_verb() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "preview.thumbnail")
        .expect("default_fixtures includes preview.thumbnail");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(PreviewThumbnailVerb))
        .expect("register preview.thumbnail verb");

    let report = validate_reconstructors(&registry, &[fixture]).expect("fixture validates");
    assert_eq!(report.verbs_checked, vec!["preview.thumbnail"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn default_fixture_has_empty_patch_warnings_and_null_expected_data() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "preview.thumbnail")
        .expect("default_fixtures includes preview.thumbnail");
    assert_eq!(fixture.patch, json!([]));
    assert!(fixture.warnings.is_empty());
    assert_eq!(fixture.expected_data, Value::Null);
}

#[test]
fn default_registry_contains_preview_thumbnail() {
    let registry = default_registry();
    let verb = registry
        .get("preview.thumbnail")
        .expect("preview.thumbnail is in default_registry");
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

    let outcome = store.mutate_via_verb("preview.thumbnail", args_value(), None);
    let Err(LifecycleError::VerbExecutionFailed { source, .. }) = outcome else {
        panic!("expected VerbExecutionFailed, got {outcome:?}");
    };
    let VerbError::Custom(detail) = source else {
        panic!("expected Custom source, got {source:?}");
    };
    assert!(detail.contains("E_IO"));
}
