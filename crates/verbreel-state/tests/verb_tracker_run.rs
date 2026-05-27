//! Tests for `tracker.run` (§18.2) — ninety-fourth production verb.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::tracker::Tracker;
use verbreel_state::verbs::tracker_run::compute_patch;
use verbreel_state::{
    Project, ReconstructError, TrackerBBoxTraceSummary, TrackerRunArgs, TrackerRunData,
    TrackerRunError, TrackerRunStage, TrackerRunVerb, Verb, VerbError, VerbRegistry,
    default_fixtures, default_registry, validate_reconstructors,
};

#[cfg(feature = "native")]
use verbreel_state::{LifecycleError, ProjectStore};

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const TRACKER_ID_A: &str = "01900000-0000-7000-8000-0000000ee201";
const TRACKER_ID_B: &str = "01900000-0000-7000-8000-0000000ee202";
const TRACKER_ID_C: &str = "01900000-0000-7000-8000-0000000ee203";
const MISSING_TRACKER_ID: &str = "01900000-0000-7000-8000-0000000ee999";

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

fn project_with_tracker(tracker_id: &str, algorithm: Option<&str>) -> Project {
    let mut project = empty_project();
    let mut tracker = json!({
        "tracker_id": tracker_id,
        "source_clip_id": "",
        "algorithm": "object",
        "applied_to_clip_ids": [],
        "sample_count": -1,
        "cache_hash": "",
        "cache_path": "",
    });
    if let Some(algorithm) = algorithm {
        tracker["algorithm"] = json!(algorithm);
    } else {
        tracker
            .as_object_mut()
            .expect("tracker object")
            .remove("algorithm");
    }
    project.trackers.push(tracker_from_json(tracker));
    project
}

fn args(tracker_id: &str) -> TrackerRunArgs {
    TrackerRunArgs {
        project_id: fixture_project_id(),
        tracker_id: tracker_id.to_string(),
        from_tk: None,
        to_tk: None,
        sample_every_ticks: None,
    }
}

fn args_value(tracker_id: &str) -> Value {
    serde_json::to_value(args(tracker_id)).expect("args serialize")
}

// ---------------------------------------------------------------------
// Args shape / serde
// ---------------------------------------------------------------------

#[test]
fn args_deserialize_minimal() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "tracker_id": TRACKER_ID_A,
    });
    let parsed: TrackerRunArgs = serde_json::from_value(raw).expect("minimal args parse");
    assert_eq!(parsed.project_id.to_string(), FIXTURE_PROJECT_ID);
    assert_eq!(parsed.tracker_id, TRACKER_ID_A);
    assert_eq!(parsed.from_tk, None);
    assert_eq!(parsed.to_tk, None);
    assert_eq!(parsed.sample_every_ticks, None);
}

#[test]
fn args_deserialize_with_all_optional_ticks() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "tracker_id": TRACKER_ID_A,
        "from_tk": 100,
        "to_tk": 200,
        "sample_every_ticks": 10,
    });
    let parsed: TrackerRunArgs = serde_json::from_value(raw).expect("args parse");
    assert_eq!(parsed.from_tk, Some(100));
    assert_eq!(parsed.to_tk, Some(200));
    assert_eq!(parsed.sample_every_ticks, Some(10));
}

#[test]
fn args_unknown_field_rejected() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "tracker_id": TRACKER_ID_A,
        "extra": true,
    });
    let result: Result<TrackerRunArgs, _> = serde_json::from_value(raw);
    assert!(
        result.is_err(),
        "deny_unknown_fields should reject unexpected keys"
    );
}

#[test]
fn args_missing_project_id_fails_through_verb() {
    let prior = empty_project();
    let verb = TrackerRunVerb;
    let err = verb
        .compute_patch(&prior, &json!({ "tracker_id": TRACKER_ID_A }))
        .expect_err("missing project_id should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_missing_tracker_id_fails_through_verb() {
    let prior = empty_project();
    let verb = TrackerRunVerb;
    let err = verb
        .compute_patch(&prior, &json!({ "project_id": FIXTURE_PROJECT_ID }))
        .expect_err("missing tracker_id should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_non_string_tracker_id_rejected() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "tracker_id": 42,
    });
    let result: Result<TrackerRunArgs, _> = serde_json::from_value(raw);
    assert!(result.is_err(), "non-string tracker_id must fail");
}

#[test]
fn args_non_integer_tick_fields_rejected() {
    let bad_shapes = vec![
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "tracker_id": TRACKER_ID_A,
            "from_tk": 1.25,
        }),
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "tracker_id": TRACKER_ID_A,
            "to_tk": "100",
        }),
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "tracker_id": TRACKER_ID_A,
            "sample_every_ticks": false,
        }),
    ];

    for raw in bad_shapes {
        let parsed: Result<TrackerRunArgs, _> = serde_json::from_value(raw);
        assert!(parsed.is_err(), "non-integer tick field must fail parse");
    }
}

// ---------------------------------------------------------------------
// Stage + data wire surface
// ---------------------------------------------------------------------

#[test]
fn tracker_run_stage_serde_accepts_only_spec_literals() {
    let ok = vec![
        ("decoder_init", TrackerRunStage::DecoderInit),
        ("frame_decode", TrackerRunStage::FrameDecode),
        ("algorithm_step", TrackerRunStage::AlgorithmStep),
        ("cache_write", TrackerRunStage::CacheWrite),
    ];
    for (literal, variant) in ok {
        let parsed: TrackerRunStage =
            serde_json::from_value(json!(literal)).expect("stage literal parses");
        assert_eq!(parsed, variant);
        assert_eq!(
            serde_json::to_value(variant).expect("stage serializes"),
            json!(literal)
        );
    }

    let bad: Result<TrackerRunStage, _> = serde_json::from_value(json!("decode"));
    assert!(bad.is_err(), "non-spec stage literal must fail");
}

#[test]
fn tracker_run_data_serializes_exact_section_18_2_fields() {
    let data = TrackerRunData {
        tracker_id: TRACKER_ID_A.to_string(),
        sample_count: 120,
        cache_path: "/tmp/cache/trackers/abc.json".to_string(),
        cache_hit: true,
        mean_confidence: 0.85,
        bbox_trace_summary: TrackerBBoxTraceSummary {
            min_x: 1.0,
            max_x: 2.0,
            min_y: 3.0,
            max_y: 4.0,
        },
    };
    let value = serde_json::to_value(&data).expect("data serializes");
    let obj = value.as_object().expect("data is object");

    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "bbox_trace_summary",
            "cache_hit",
            "cache_path",
            "mean_confidence",
            "sample_count",
            "tracker_id",
        ]
    );
}

#[test]
fn bbox_trace_summary_serializes_all_four_f64_fields() {
    let summary = TrackerBBoxTraceSummary {
        min_x: -10.5,
        max_x: 640.25,
        min_y: 0.0,
        max_y: 360.75,
    };
    let value = serde_json::to_value(summary).expect("summary serializes");
    assert_eq!(value["min_x"], json!(-10.5));
    assert_eq!(value["max_x"], json!(640.25));
    assert_eq!(value["min_y"], json!(0.0));
    assert_eq!(value["max_y"], json!(360.75));
}

// ---------------------------------------------------------------------
// Error mapping / v1 floor behavior
// ---------------------------------------------------------------------

#[test]
fn negative_from_tk_maps_to_custom_e_bad_time() {
    let prior = project_with_tracker(TRACKER_ID_A, Some("object"));
    let verb = TrackerRunVerb;
    let mut raw = args_value(TRACKER_ID_A);
    raw["from_tk"] = json!(-1);

    let err = verb
        .compute_patch(&prior, &raw)
        .expect_err("negative from_tk should error");
    let VerbError::Custom(detail) = err else {
        panic!("expected Custom, got {err:?}");
    };
    assert!(detail.contains("E_BAD_TIME"));
    assert!(detail.contains("from_tk"));
}

#[test]
fn negative_to_tk_maps_to_custom_e_bad_time() {
    let prior = project_with_tracker(TRACKER_ID_A, Some("object"));
    let verb = TrackerRunVerb;
    let mut raw = args_value(TRACKER_ID_A);
    raw["to_tk"] = json!(-1);

    let err = verb
        .compute_patch(&prior, &raw)
        .expect_err("negative to_tk should error");
    let VerbError::Custom(detail) = err else {
        panic!("expected Custom, got {err:?}");
    };
    assert!(detail.contains("E_BAD_TIME"));
    assert!(detail.contains("to_tk"));
}

#[test]
fn sample_every_ticks_zero_or_negative_maps_to_custom_e_bad_time() {
    let prior = project_with_tracker(TRACKER_ID_A, Some("object"));
    let verb = TrackerRunVerb;
    for sample in [0_i64, -1_i64] {
        let mut raw = args_value(TRACKER_ID_A);
        raw["sample_every_ticks"] = json!(sample);
        let err = verb
            .compute_patch(&prior, &raw)
            .expect_err("non-positive sample_every_ticks should error");
        let VerbError::Custom(detail) = err else {
            panic!("expected Custom, got {err:?}");
        };
        assert!(detail.contains("E_BAD_TIME"));
        assert!(detail.contains("sample_every_ticks"));
    }
}

#[test]
fn unknown_tracker_maps_to_custom_e_tracker_not_found() {
    let prior = empty_project();
    let verb = TrackerRunVerb;
    let err = verb
        .compute_patch(&prior, &args_value(MISSING_TRACKER_ID))
        .expect_err("missing tracker should error");
    let VerbError::Custom(detail) = err else {
        panic!("expected Custom, got {err:?}");
    };
    assert!(detail.contains("E_TRACKER_NOT_FOUND"));
    assert!(detail.contains(MISSING_TRACKER_ID));
}

#[test]
fn existing_tracker_algorithms_all_map_to_custom_e_tracker_run_failed() {
    let cases = vec![
        (TRACKER_ID_A, Some("object"), "object"),
        (TRACKER_ID_B, Some("face"), "face"),
        (TRACKER_ID_C, Some("optical_flow"), "optical_flow"),
        ("01900000-0000-7000-8000-0000000ee204", None, ""),
        (
            "01900000-0000-7000-8000-0000000ee205",
            Some("unknown_algo"),
            "unknown_algo",
        ),
    ];

    let verb = TrackerRunVerb;
    for (tracker_id, algorithm, expected_algorithm) in cases {
        let prior = project_with_tracker(tracker_id, algorithm);
        let err = verb
            .compute_patch(&prior, &args_value(tracker_id))
            .expect_err("v1 floor should fail for existing tracker");
        let VerbError::Custom(detail) = err else {
            panic!("expected Custom, got {err:?}");
        };
        assert!(detail.contains("E_TRACKER_RUN_FAILED"));
        assert!(detail.contains("algorithm_step"));
        assert!(detail.contains(expected_algorithm));
    }
}

#[test]
fn reserved_error_variants_display_spec_codes_and_map_to_custom() {
    let reserved = vec![
        TrackerRunError::Busy {
            active_verb: "render.start".to_string(),
            job_id: "job-1".to_string(),
            running_since: "2026-05-28T00:00:00Z".to_string(),
        },
        TrackerRunError::BadRange {
            bound: "from_tk > to_tk".to_string(),
        },
    ];

    for err in reserved {
        let text = err.to_string();
        assert!(
            text.contains("E_BUSY") || text.contains("E_BAD_RANGE"),
            "reserved variant should include spec code, got `{text}`",
        );
        let mapped: VerbError = err.into();
        let VerbError::Custom(detail) = mapped else {
            panic!("reserved variant must map to Custom");
        };
        assert!(
            detail.contains("E_BUSY") || detail.contains("E_BAD_RANGE"),
            "mapped detail should carry code, got `{detail}`",
        );
    }
}

// ---------------------------------------------------------------------
// Verb helper / reconstruct path
// ---------------------------------------------------------------------

#[test]
fn compute_patch_existing_tracker_returns_run_failed_variant() {
    let prior = project_with_tracker(TRACKER_ID_A, Some("object"));
    let err = compute_patch(&prior, &args(TRACKER_ID_A)).expect_err("v1 floor run-failed");
    assert!(matches!(
        err,
        TrackerRunError::RunFailed {
            ref tracker_id,
            ref algorithm,
            stage: TrackerRunStage::AlgorithmStep,
            ..
        } if tracker_id == TRACKER_ID_A && algorithm == "object"
    ));
}

#[test]
fn reconstruct_returns_null_for_well_formed_args() {
    let prior = empty_project();
    let verb = TrackerRunVerb;
    let data = verb
        .reconstruct(&args_value(TRACKER_ID_A), &json!([]), &[], &prior)
        .expect("well-formed args reconstruct");
    assert_eq!(data, Value::Null);
}

#[test]
fn reconstruct_rejects_malformed_args() {
    let prior = empty_project();
    let verb = TrackerRunVerb;
    let err = verb
        .reconstruct(&json!({}), &json!([]), &[], &prior)
        .expect_err("malformed args should fail reconstruction");
    assert!(matches!(
        err,
        ReconstructError::TypeMismatch {
            expected: "TrackerRunArgs",
            ..
        }
    ));
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "tracker.run")
        .expect("default fixtures include tracker.run");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(TrackerRunVerb))
        .expect("register tracker.run verb");

    let report = validate_reconstructors(&registry, &[fixture]).expect("reconstructor validates");
    assert_eq!(report.verbs_checked, vec!["tracker.run"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn default_fixture_has_empty_patch_warnings_and_null_expected_data() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "tracker.run")
        .expect("default fixtures include tracker.run");
    assert_eq!(fixture.patch, json!([]));
    assert!(fixture.warnings.is_empty());
    assert_eq!(fixture.expected_data, Value::Null);
}

#[test]
fn default_registry_contains_tracker_run() {
    let registry = default_registry();
    let verb = registry
        .get("tracker.run")
        .expect("tracker.run registered in default registry");
    assert_eq!(verb.verb(), "tracker.run");
}

#[cfg(feature = "native")]
#[test]
fn mutate_via_verb_existing_tracker_returns_verb_execution_failed_run_failed() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        project_with_tracker(TRACKER_ID_A, Some("object")),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry succeeds");

    let outcome = store.mutate_via_verb("tracker.run", args_value(TRACKER_ID_A), None);
    let Err(LifecycleError::VerbExecutionFailed { source, .. }) = outcome else {
        panic!("expected VerbExecutionFailed, got {outcome:?}");
    };
    let VerbError::Custom(detail) = source else {
        panic!("expected Custom source, got {source:?}");
    };
    assert!(detail.contains("E_TRACKER_RUN_FAILED"));
}

#[cfg(feature = "native")]
#[test]
fn mutate_via_verb_missing_tracker_returns_verb_execution_failed_not_found() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        empty_project(),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry succeeds");

    let outcome = store.mutate_via_verb("tracker.run", args_value(MISSING_TRACKER_ID), None);
    let Err(LifecycleError::VerbExecutionFailed { source, .. }) = outcome else {
        panic!("expected VerbExecutionFailed, got {outcome:?}");
    };
    let VerbError::Custom(detail) = source else {
        panic!("expected Custom source, got {source:?}");
    };
    assert!(detail.contains("E_TRACKER_NOT_FOUND"));
}
