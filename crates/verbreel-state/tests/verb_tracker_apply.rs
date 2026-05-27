//! Tests for `tracker.apply` (§18.3) — v1 not-run floor.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::tracker::Tracker;
use verbreel_state::verbs::tracker_apply::compute_patch;
use verbreel_state::{
    DEFAULT_TRACKER_APPLY_PROPERTIES, Project, ReconstructError, TrackerApplyArgs,
    TrackerApplyData, TrackerApplyError, TrackerApplyOffset, TrackerApplyVerb, Verb, VerbError,
    VerbRegistry, default_fixtures, default_registry, validate_reconstructors,
};

#[cfg(feature = "native")]
use verbreel_state::{LifecycleError, ProjectStore};

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";

const TRACKER_OBJECT: &str = "01900000-0000-7000-8000-0000000ea101";
const TRACKER_FACE: &str = "01900000-0000-7000-8000-0000000ea102";
const TRACKER_FLOW: &str = "01900000-0000-7000-8000-0000000ea103";
const TRACKER_MISSING: &str = "01900000-0000-7000-8000-0000000ea199";
const CLIP_ID: &str = "01900000-0000-7000-8000-0000000cb101";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture -> Project")
}

fn fixture_project_id() -> verbreel_types::ProjectId {
    FIXTURE_PROJECT_ID
        .parse()
        .expect("fixture project id parses")
}

fn tracker_from_json(value: Value) -> Tracker {
    serde_json::from_value(value).expect("tracker fixture parses")
}

fn project_with_tracker(
    tracker_id: &str,
    algorithm: &str,
    cache_hash: &str,
    cache_path: Option<&str>,
) -> Project {
    let mut project = empty_project();
    let cache_path_value = cache_path.unwrap_or("");
    project.trackers.push(tracker_from_json(json!({
        "tracker_id": tracker_id,
        "source_clip_id": "",
        "algorithm": algorithm,
        "applied_to_clip_ids": [],
        "sample_count": -1,
        "cache_hash": cache_hash,
        "cache_path": cache_path_value,
    })));
    project
}

fn args(tracker_id: &str) -> TrackerApplyArgs {
    TrackerApplyArgs {
        project_id: fixture_project_id(),
        tracker_id: tracker_id.to_string(),
        to_clip: CLIP_ID.to_string(),
        properties: None,
        offset: None,
        decimate_to_every: None,
    }
}

fn args_value(tracker_id: &str) -> Value {
    serde_json::to_value(args(tracker_id)).expect("args serialize")
}

fn verb_custom_error(prior: &Project, args: Value) -> String {
    let verb = TrackerApplyVerb;
    let err = verb
        .compute_patch(prior, &args)
        .expect_err("expected tracker.apply to fail in v1 floor");
    let VerbError::Custom(detail) = err else {
        panic!("expected VerbError::Custom, got {err:?}");
    };
    detail
}

// ---------------------------------------------------------------------
// Args shape / serde
// ---------------------------------------------------------------------

#[test]
fn args_deserialize_minimal() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "tracker_id": TRACKER_OBJECT,
        "to_clip": CLIP_ID,
    });
    let parsed: TrackerApplyArgs = serde_json::from_value(raw).expect("minimal args parse");
    assert_eq!(parsed.project_id.to_string(), FIXTURE_PROJECT_ID);
    assert_eq!(parsed.tracker_id, TRACKER_OBJECT);
    assert_eq!(parsed.to_clip, CLIP_ID);
    assert!(parsed.properties.is_none());
    assert!(parsed.offset.is_none());
    assert!(parsed.decimate_to_every.is_none());
}

#[test]
fn args_deserialize_with_optional_fields() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "tracker_id": TRACKER_OBJECT,
        "to_clip": format!("clip:{CLIP_ID}"),
        "properties": ["transform.x", "transform.y", "transform.scale_x", "transform.scale_y"],
        "offset": {"x": 10.5, "y": -12.0, "scale": 0.75},
        "decimate_to_every": 8000,
    });
    let parsed: TrackerApplyArgs = serde_json::from_value(raw).expect("args parse");
    assert_eq!(
        parsed.properties,
        Some(vec![
            "transform.x".to_string(),
            "transform.y".to_string(),
            "transform.scale_x".to_string(),
            "transform.scale_y".to_string(),
        ])
    );
    assert_eq!(
        parsed.offset,
        Some(TrackerApplyOffset {
            x: 10.5,
            y: -12.0,
            scale: 0.75,
        })
    );
    assert_eq!(parsed.decimate_to_every, Some(8000));
}

#[test]
fn offset_defaults_when_fields_omitted() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "tracker_id": TRACKER_OBJECT,
        "to_clip": CLIP_ID,
        "offset": {},
    });
    let parsed: TrackerApplyArgs = serde_json::from_value(raw).expect("args parse");
    assert_eq!(
        parsed.offset,
        Some(TrackerApplyOffset {
            x: 0.0,
            y: 0.0,
            scale: 1.0,
        })
    );
}

#[test]
fn offset_unknown_fields_rejected() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "tracker_id": TRACKER_OBJECT,
        "to_clip": CLIP_ID,
        "offset": {"x": 1.0, "z": 2.0},
    });
    let parsed: Result<TrackerApplyArgs, _> = serde_json::from_value(raw);
    assert!(
        parsed.is_err(),
        "offset deny_unknown_fields must reject `z`"
    );
}

#[test]
fn top_level_unknown_fields_rejected() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "tracker_id": TRACKER_OBJECT,
        "to_clip": CLIP_ID,
        "extra": true,
    });
    let parsed: Result<TrackerApplyArgs, _> = serde_json::from_value(raw);
    assert!(
        parsed.is_err(),
        "top-level deny_unknown_fields must reject `extra`"
    );
}

#[test]
fn args_missing_required_fields_fail_through_verb() {
    let prior = empty_project();
    let verb = TrackerApplyVerb;
    let cases = vec![
        json!({ "tracker_id": TRACKER_OBJECT, "to_clip": CLIP_ID }),
        json!({ "project_id": FIXTURE_PROJECT_ID, "to_clip": CLIP_ID }),
        json!({ "project_id": FIXTURE_PROJECT_ID, "tracker_id": TRACKER_OBJECT }),
    ];
    for raw in cases {
        let err = verb
            .compute_patch(&prior, &raw)
            .expect_err("missing required field maps to BadArgs");
        assert!(matches!(err, VerbError::BadArgs { .. }));
    }
}

#[test]
fn args_non_string_required_fields_rejected() {
    let cases = vec![
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "tracker_id": 42,
            "to_clip": CLIP_ID
        }),
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "tracker_id": TRACKER_OBJECT,
            "to_clip": 99
        }),
    ];
    for raw in cases {
        let parsed: Result<TrackerApplyArgs, _> = serde_json::from_value(raw);
        assert!(parsed.is_err(), "non-string required field must fail parse");
    }
}

#[test]
fn args_non_integer_decimate_to_every_rejected() {
    let cases = vec![
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "tracker_id": TRACKER_OBJECT,
            "to_clip": CLIP_ID,
            "decimate_to_every": 1.5
        }),
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "tracker_id": TRACKER_OBJECT,
            "to_clip": CLIP_ID,
            "decimate_to_every": "1000"
        }),
    ];
    for raw in cases {
        let parsed: Result<TrackerApplyArgs, _> = serde_json::from_value(raw);
        assert!(parsed.is_err(), "non-integer decimate_to_every must fail");
    }
}

// ---------------------------------------------------------------------
// v1 floor behavior and error mapping
// ---------------------------------------------------------------------

#[test]
fn omitted_properties_resolve_to_default_shape_and_reach_not_run() {
    let prior = project_with_tracker(TRACKER_OBJECT, "object", "", None);
    let detail = verb_custom_error(&prior, args_value(TRACKER_OBJECT));
    assert!(detail.contains("E_TRACKER_NOT_RUN"));
}

#[test]
fn accepted_properties_shapes_both_reach_not_run() {
    let prior = project_with_tracker(TRACKER_FACE, "face", "", None);
    let cases = vec![
        json!(["transform.x", "transform.y"]),
        json!([
            "transform.x",
            "transform.y",
            "transform.scale_x",
            "transform.scale_y"
        ]),
    ];
    for properties in cases {
        let mut raw = args_value(TRACKER_FACE);
        raw["properties"] = properties;
        let detail = verb_custom_error(&prior, raw);
        assert!(detail.contains("E_TRACKER_NOT_RUN"));
    }
}

#[test]
fn malformed_properties_shapes_map_to_custom_e_tracker_bad_params() {
    let prior = project_with_tracker(TRACKER_OBJECT, "object", "", None);
    let bad_shapes = vec![
        json!(["transform.y", "transform.x"]),
        json!(["transform.x"]),
        json!(["transform.x", "transform.y", "transform.scale_x"]),
        json!(["transform.x", "transform.y", "transform.y"]),
        json!(["transform.x", "transform.y", "transform.rotate"]),
    ];

    for properties in bad_shapes {
        let mut raw = args_value(TRACKER_OBJECT);
        raw["properties"] = properties;
        let detail = verb_custom_error(&prior, raw);
        assert!(detail.contains("E_TRACKER_BAD_PARAMS"));
        assert!(detail.contains("properties"));
    }
}

#[test]
fn non_positive_decimate_maps_to_custom_e_tracker_bad_params() {
    let prior = project_with_tracker(TRACKER_OBJECT, "object", "", None);
    for decimate in [0_i64, -1_i64] {
        let mut raw = args_value(TRACKER_OBJECT);
        raw["decimate_to_every"] = json!(decimate);
        let detail = verb_custom_error(&prior, raw);
        assert!(detail.contains("E_TRACKER_BAD_PARAMS"));
        assert!(detail.contains("decimate_to_every"));
    }
}

#[test]
fn bad_to_clip_uuid_maps_to_custom_e_bad_selector() {
    let prior = project_with_tracker(TRACKER_OBJECT, "object", "", None);
    let mut raw = args_value(TRACKER_OBJECT);
    raw["to_clip"] = json!("not-a-uuid");
    let detail = verb_custom_error(&prior, raw);
    assert!(detail.contains("E_BAD_SELECTOR"));
}

#[test]
fn non_clip_selector_prefix_maps_to_custom_e_selector_kind_mismatch() {
    let prior = project_with_tracker(TRACKER_OBJECT, "object", "", None);
    let mut raw = args_value(TRACKER_OBJECT);
    raw["to_clip"] = json!(format!("track:{CLIP_ID}"));
    let detail = verb_custom_error(&prior, raw);
    assert!(detail.contains("E_SELECTOR_KIND_MISMATCH"));
    assert!(detail.contains("track"));
}

#[test]
fn unknown_tracker_maps_to_custom_e_tracker_not_found() {
    let prior = empty_project();
    let detail = verb_custom_error(&prior, args_value(TRACKER_MISSING));
    assert!(detail.contains("E_TRACKER_NOT_FOUND"));
    assert!(detail.contains(TRACKER_MISSING));
}

#[test]
fn existing_unrun_trackers_map_to_custom_e_tracker_not_run() {
    let cases = vec![
        (TRACKER_OBJECT, "object"),
        (TRACKER_FACE, "face"),
        (TRACKER_FLOW, "optical_flow"),
    ];
    for (tracker_id, algorithm) in cases {
        let prior = project_with_tracker(tracker_id, algorithm, "", None);
        let detail = verb_custom_error(&prior, args_value(tracker_id));
        assert!(detail.contains("E_TRACKER_NOT_RUN"));
        assert!(detail.contains("cache_missing=false"));
    }
}

#[test]
fn scale_properties_with_optical_flow_map_to_custom_e_tracker_bad_params() {
    let prior = project_with_tracker(TRACKER_FLOW, "optical_flow", "", None);
    let mut raw = args_value(TRACKER_FLOW);
    raw["properties"] = json!([
        "transform.x",
        "transform.y",
        "transform.scale_x",
        "transform.scale_y"
    ]);
    let detail = verb_custom_error(&prior, raw);
    assert!(detail.contains("E_TRACKER_BAD_PARAMS"));
    assert!(detail.contains("optical_flow"));
}

#[test]
fn non_empty_cache_hash_with_cache_path_returns_dangling_not_run() {
    let cache_hash = "53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658";
    let cache_path = "cache/trackers/custom-from-tracker.json";
    let prior = project_with_tracker(TRACKER_OBJECT, "object", cache_hash, Some(cache_path));
    let err = compute_patch(&prior, &args(TRACKER_OBJECT)).expect_err("v1 floor must error");
    let TrackerApplyError::TrackerNotRun {
        cache_missing,
        expected_cache_path,
        ..
    } = err
    else {
        panic!("expected TrackerNotRun");
    };
    assert!(cache_missing);
    assert_eq!(expected_cache_path, Some(cache_path.to_string()));
}

#[test]
fn non_empty_cache_hash_without_cache_path_uses_first16_fallback() {
    let cache_hash = "abcdef1234567890fedcba0987654321";
    let prior = project_with_tracker(TRACKER_OBJECT, "object", cache_hash, None);
    let err = compute_patch(&prior, &args(TRACKER_OBJECT)).expect_err("v1 floor must error");
    let TrackerApplyError::TrackerNotRun {
        cache_missing,
        expected_cache_path,
        ..
    } = err
    else {
        panic!("expected TrackerNotRun");
    };
    assert!(cache_missing);
    assert_eq!(
        expected_cache_path.as_deref(),
        Some("cache/trackers/abcdef1234567890.json")
    );
}

#[test]
fn future_success_data_serializes_exact_section_18_3_fields() {
    let data = TrackerApplyData {
        to_clip_id: CLIP_ID.to_string(),
        added_keyframe_ids: vec!["01900000-0000-7000-8000-0000000dd001".to_string()],
        removed_keyframe_ids: vec!["01900000-0000-7000-8000-0000000dd002".to_string()],
        properties: DEFAULT_TRACKER_APPLY_PROPERTIES
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
        applied_sample_count: 24,
    };
    let value = serde_json::to_value(&data).expect("data serializes");
    let object = value.as_object().expect("data is object");

    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "added_keyframe_ids",
            "applied_sample_count",
            "properties",
            "removed_keyframe_ids",
            "to_clip_id",
        ]
    );
}

#[test]
fn reserved_error_variants_display_codes_and_map_to_custom() {
    let reserved = vec![
        TrackerApplyError::NotFound {
            clip_id: CLIP_ID.to_string(),
        },
        TrackerApplyError::NoMatch {
            selector: "clip:video[0]".to_string(),
        },
        TrackerApplyError::Locked {
            clip_id: CLIP_ID.to_string(),
        },
        TrackerApplyError::Io {
            cache_path: "cache/trackers/a.json".to_string(),
            errno: "ENOENT".to_string(),
        },
    ];

    for err in reserved {
        let text = err.to_string();
        assert!(
            text.contains("E_NOT_FOUND")
                || text.contains("E_NO_MATCH")
                || text.contains("E_LOCKED")
                || text.contains("E_IO"),
            "reserved variant must include spec code, got `{text}`",
        );

        let mapped: VerbError = err.into();
        let VerbError::Custom(detail) = mapped else {
            panic!("reserved variant must map to Custom");
        };
        assert!(
            detail.contains("E_NOT_FOUND")
                || detail.contains("E_NO_MATCH")
                || detail.contains("E_LOCKED")
                || detail.contains("E_IO"),
            "mapped detail should carry spec code, got `{detail}`",
        );
    }
}

// ---------------------------------------------------------------------
// Reconstruct / fixtures / registry
// ---------------------------------------------------------------------

#[test]
fn reconstruct_returns_null_for_well_formed_args() {
    let prior = empty_project();
    let verb = TrackerApplyVerb;
    let data = verb
        .reconstruct(&args_value(TRACKER_OBJECT), &json!([]), &[], &prior)
        .expect("well-formed args reconstruct");
    assert_eq!(data, Value::Null);
}

#[test]
fn reconstruct_rejects_malformed_args() {
    let prior = empty_project();
    let verb = TrackerApplyVerb;
    let err = verb
        .reconstruct(&json!({}), &json!([]), &[], &prior)
        .expect_err("malformed args should fail");
    assert!(matches!(
        err,
        ReconstructError::TypeMismatch {
            expected: "TrackerApplyArgs",
            ..
        }
    ));
}

#[test]
fn default_fixture_validates_with_tracker_apply_verb_only() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "tracker.apply")
        .expect("default fixtures include tracker.apply");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(TrackerApplyVerb))
        .expect("register tracker.apply");

    let report = validate_reconstructors(&registry, &[fixture]).expect("reconstructor validates");
    assert_eq!(report.verbs_checked, vec!["tracker.apply"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn default_fixture_has_empty_patch_warnings_and_null_expected_data() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "tracker.apply")
        .expect("default fixtures include tracker.apply");
    assert_eq!(fixture.patch, json!([]));
    assert!(fixture.warnings.is_empty());
    assert_eq!(fixture.expected_data, Value::Null);
}

#[test]
fn default_registry_contains_tracker_apply() {
    let registry = default_registry();
    let verb = registry
        .get("tracker.apply")
        .expect("tracker.apply registered in default registry");
    assert_eq!(verb.verb(), "tracker.apply");
}

#[cfg(feature = "native")]
#[test]
fn mutate_via_verb_existing_tracker_returns_not_run() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        project_with_tracker(TRACKER_OBJECT, "object", "", None),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry succeeds");

    let outcome = store.mutate_via_verb("tracker.apply", args_value(TRACKER_OBJECT), None);
    let Err(LifecycleError::VerbExecutionFailed { source, .. }) = outcome else {
        panic!("expected VerbExecutionFailed, got {outcome:?}");
    };
    let VerbError::Custom(detail) = source else {
        panic!("expected Custom source, got {source:?}");
    };
    assert!(detail.contains("E_TRACKER_NOT_RUN"));
}

#[cfg(feature = "native")]
#[test]
fn mutate_via_verb_missing_tracker_returns_not_found() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        empty_project(),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry succeeds");

    let outcome = store.mutate_via_verb("tracker.apply", args_value(TRACKER_MISSING), None);
    let Err(LifecycleError::VerbExecutionFailed { source, .. }) = outcome else {
        panic!("expected VerbExecutionFailed, got {outcome:?}");
    };
    let VerbError::Custom(detail) = source else {
        panic!("expected Custom source, got {source:?}");
    };
    assert!(detail.contains("E_TRACKER_NOT_FOUND"));
}
