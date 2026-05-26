//! Tests for `tracker.create` (§18.1) — seventy-first production verb.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::tracker_create::{
    TrackerAlgorithm, TrackerCreateArgs, TrackerCreateData, TrackerCreateError, TrackerCreateVerb,
    W_TRACKER_CREATE_ENVELOPE_CODE, compute_patch, data_envelope_from_args_warnings,
};
use verbreel_state::{
    Project, VerbRegistry, default_fixtures, default_registry, validate_reconstructors,
};

#[cfg(feature = "native")]
use verbreel_state::{MutateOutcome, ProjectStore};

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";

const VIDEO_TRACK_ID: &str = "01900000-0000-7000-8000-000000aa7001";
const AUDIO_TRACK_ID: &str = "01900000-0000-7000-8000-000000aa7002";
const TEXT_TRACK_ID: &str = "01900000-0000-7000-8000-000000aa7003";
const VIDEO_CLIP_ID: &str = "01900000-0000-7000-8000-000000bb7001";
const AUDIO_CLIP_ID: &str = "01900000-0000-7000-8000-000000bb7002";
const TEXT_CLIP_ID: &str = "01900000-0000-7000-8000-000000bb7003";
const VIDEO_ASSET_ID: &str = "01900000-0000-7000-8000-000000cc7001";
const AUDIO_ASSET_ID: &str = "01900000-0000-7000-8000-000000cc7002";
const UNKNOWN_CLIP_ID: &str = "01900000-0000-7000-8000-000000bb9999";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture -> Project")
}

fn fixture_project_id() -> verbreel_types::ProjectId {
    FIXTURE_PROJECT_ID
        .parse()
        .expect("fixture project id parses")
}

fn project_with_video_clip() -> Project {
    let mut project = empty_project();
    project.assets.push(
        serde_json::from_value(json!({
            "id": VIDEO_ASSET_ID,
            "kind": "video",
            "hash": "36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da",
            "path": "assets/36/36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da.mp4",
            "original_filename": "tracker-test.mp4",
            "imported_at": "2026-05-24T00:00:00Z",
            "metadata": {
                "duration_tk": 240_000,
                "width": 1920,
                "height": 1080,
                "fps_num": 30,
                "fps_den": 1,
                "video_codec": "h264",
                "container": "mp4",
                "fingerprint": {
                    "mtime_ms": 1_700_000_000_000_i64,
                    "size_bytes": 1024,
                }
            }
        }))
        .expect("video asset parses"),
    );
    // Insert before the empty_fixture's seed audio track so the
    // video-block invariant (all video tracks before audio tracks)
    // holds. The seed project's track[0] is the auto-created `Video 1`,
    // track[1] is `Audio 1`; we slot our analyzable video clip at
    // index 1.
    let new_track = serde_json::from_value(json!({
        "id": VIDEO_TRACK_ID,
        "kind": "video",
        "name": "Video Source",
        "locked": false,
        "clips": [{
            "id": VIDEO_CLIP_ID,
            "name": "Source Clip",
            "asset_id": VIDEO_ASSET_ID,
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 240_000,
            "locked": false,
        }],
    }))
    .expect("video track parses");
    project.tracks.insert(1, new_track);
    project.duration_tk = verbreel_types::Tick::new(240_000);
    project
}

fn project_with_audio_clip() -> Project {
    let mut project = empty_project();
    project.assets.push(
        serde_json::from_value(json!({
            "id": AUDIO_ASSET_ID,
            "kind": "audio",
            "hash": "53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658",
            "path": "assets/53/53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.m4a",
            "original_filename": "audio-tracker-test.m4a",
            "imported_at": "2026-05-24T00:00:00Z",
            "metadata": {
                "duration_tk": 240_000,
                "audio_codec": "aac",
                "audio_channels": 2,
                "audio_sample_rate_hz": 48000,
                "container": "m4a",
                "fingerprint": {
                    "mtime_ms": 1_700_000_000_000_i64,
                    "size_bytes": 1024,
                }
            }
        }))
        .expect("audio asset parses"),
    );
    project.tracks.push(
        serde_json::from_value(json!({
            "id": AUDIO_TRACK_ID,
            "kind": "audio",
            "name": "Audio Source",
            "locked": false,
            "clips": [{
                "id": AUDIO_CLIP_ID,
                "name": "Audio Clip",
                "asset_id": AUDIO_ASSET_ID,
                "track_position_tk": 0,
                "source_in_tk": 0,
                "source_out_tk": 240_000,
                "locked": false,
            }],
        }))
        .expect("audio track parses"),
    );
    project
}

fn project_with_text_clip() -> Project {
    let mut project = empty_project();
    project.tracks.push(
        serde_json::from_value(json!({
            "id": TEXT_TRACK_ID,
            "kind": "text",
            "name": "Text Source",
            "locked": false,
            "clips": [{
                "id": TEXT_CLIP_ID,
                "name": "Text Clip",
                "asset_id": "00000000-0000-0000-0000-000000000000",
                "track_position_tk": 0,
                "source_in_tk": 0,
                "source_out_tk": 240_000,
                "locked": false,
                "text": {
                    "content": "Hello",
                    "font_family": "Arial",
                    "font_size_px": 24,
                },
            }],
        }))
        .expect("text track parses"),
    );
    project
}

fn object_args(clip: &str) -> TrackerCreateArgs {
    TrackerCreateArgs {
        project_id: fixture_project_id(),
        clip: clip.to_string(),
        algorithm: TrackerAlgorithm::Object,
        params: Some(json!({
            "object_bbox_at_tk": {
                "x": 640.0,
                "y": 360.0,
                "w": 120.0,
                "h": 160.0,
                "at_tk": 0,
            }
        })),
    }
}

fn face_args(clip: &str, params: Option<Value>) -> TrackerCreateArgs {
    TrackerCreateArgs {
        project_id: fixture_project_id(),
        clip: clip.to_string(),
        algorithm: TrackerAlgorithm::Face,
        params,
    }
}

fn optical_flow_args(clip: &str) -> TrackerCreateArgs {
    TrackerCreateArgs {
        project_id: fixture_project_id(),
        clip: clip.to_string(),
        algorithm: TrackerAlgorithm::OpticalFlow,
        params: Some(json!({
            "point_at_tk": {
                "x": 200.0,
                "y": 300.0,
                "at_tk": 100_000,
            }
        })),
    }
}

fn apply_patch(prior: &Project, patch: Value) -> Project {
    let patch: json_patch::Patch = serde_json::from_value(patch).expect("patch parses");
    prior.apply(&patch).expect("patch applies")
}

// ---------------------------------------------------------------------
// Args / algorithm serde
// ---------------------------------------------------------------------

#[test]
fn args_deserialize_object_algorithm_snake_case() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "clip": VIDEO_CLIP_ID,
        "algorithm": "object",
        "params": { "object_bbox_at_tk": { "x": 1.0, "y": 1.0, "w": 1.0, "h": 1.0, "at_tk": 0 } }
    });
    let parsed: TrackerCreateArgs = serde_json::from_value(raw).expect("args parse");
    assert_eq!(parsed.algorithm, TrackerAlgorithm::Object);
}

#[test]
fn args_deserialize_optical_flow_uses_snake_case() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "clip": VIDEO_CLIP_ID,
        "algorithm": "optical_flow",
        "params": { "point_at_tk": { "x": 1.0, "y": 1.0, "at_tk": 0 } }
    });
    let parsed: TrackerCreateArgs = serde_json::from_value(raw).expect("args parse");
    assert_eq!(parsed.algorithm, TrackerAlgorithm::OpticalFlow);
}

#[test]
fn args_deserialize_face_without_params() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "clip": VIDEO_CLIP_ID,
        "algorithm": "face",
    });
    let parsed: TrackerCreateArgs = serde_json::from_value(raw).expect("args parse");
    assert_eq!(parsed.algorithm, TrackerAlgorithm::Face);
    assert!(parsed.params.is_none());
}

#[test]
fn args_camelcase_algorithm_rejected_by_serde() {
    // Snake-case discriminant is the spec contract; PascalCase variants
    // must fail.
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "clip": VIDEO_CLIP_ID,
        "algorithm": "OpticalFlow",
    });
    let result: Result<TrackerCreateArgs, _> = serde_json::from_value(raw);
    assert!(result.is_err(), "PascalCase algorithm must fail to parse");
}

// ---------------------------------------------------------------------
// Algorithm rejection
// ---------------------------------------------------------------------

#[test]
fn hands_algorithm_rejected_with_unknown_algorithm() {
    let prior = project_with_video_clip();
    let args = TrackerCreateArgs {
        algorithm: TrackerAlgorithm::Hands,
        ..object_args(VIDEO_CLIP_ID)
    };
    let err = compute_patch(&prior, &args).expect_err("hands rejected");
    assert!(matches!(
        err,
        TrackerCreateError::UnknownAlgorithm { requested } if requested == "hands"
    ));
}

// ---------------------------------------------------------------------
// Clip resolution + selectors
// ---------------------------------------------------------------------

#[test]
fn bare_uuid_not_found_returns_not_found() {
    let prior = project_with_video_clip();
    let args = object_args(UNKNOWN_CLIP_ID);
    let err = compute_patch(&prior, &args).expect_err("missing clip");
    assert!(matches!(
        err,
        TrackerCreateError::NotFound { ref clip_id } if clip_id == UNKNOWN_CLIP_ID
    ));
}

#[test]
fn qualified_selector_not_found_returns_no_match() {
    let prior = project_with_video_clip();
    let selector = format!("clip:{UNKNOWN_CLIP_ID}");
    let args = object_args(&selector);
    let err = compute_patch(&prior, &args).expect_err("missing qualified clip");
    let TrackerCreateError::NoMatch { selector: got } = err else {
        panic!("expected NoMatch");
    };
    assert_eq!(got, format!("clip:{UNKNOWN_CLIP_ID}"));
}

#[test]
fn qualified_selector_with_non_clip_prefix_returns_selector_kind_mismatch() {
    let prior = project_with_video_clip();
    let args = object_args(&format!("track:{VIDEO_TRACK_ID}"));
    let err = compute_patch(&prior, &args).expect_err("wrong prefix");
    assert!(matches!(
        err,
        TrackerCreateError::SelectorKindMismatch { ref actual_kind } if actual_kind == "track"
    ));
}

#[test]
fn malformed_uuid_returns_bad_selector() {
    let prior = project_with_video_clip();
    let args = object_args("not-a-uuid");
    let err = compute_patch(&prior, &args).expect_err("bad uuid");
    assert!(matches!(err, TrackerCreateError::BadSelector { .. }));
}

#[test]
fn qualified_selector_resolves_to_same_clip_as_bare() {
    let prior = project_with_video_clip();
    let bare = compute_patch(&prior, &object_args(VIDEO_CLIP_ID)).expect("bare ok");
    let qual = compute_patch(&prior, &object_args(&format!("clip:{VIDEO_CLIP_ID}")))
        .expect("qualified ok");
    assert_eq!(bare.2.source_clip_id, qual.2.source_clip_id);
}

// ---------------------------------------------------------------------
// Source-clip kind enforcement
// ---------------------------------------------------------------------

#[test]
fn audio_clip_rejected_with_clip_kind_mismatch() {
    let prior = project_with_audio_clip();
    let args = object_args(AUDIO_CLIP_ID);
    let err = compute_patch(&prior, &args).expect_err("audio rejected");
    assert!(matches!(
        err,
        TrackerCreateError::ClipKindMismatch { ref actual_kind } if actual_kind == "audio"
    ));
}

#[test]
fn text_clip_rejected_with_clip_kind_mismatch() {
    let prior = project_with_text_clip();
    let args = object_args(TEXT_CLIP_ID);
    let err = compute_patch(&prior, &args).expect_err("text rejected");
    assert!(matches!(
        err,
        TrackerCreateError::ClipKindMismatch { ref actual_kind } if actual_kind == "text"
    ));
}

// ---------------------------------------------------------------------
// Object-algorithm params validation
// ---------------------------------------------------------------------

#[test]
fn object_missing_params_returns_bad_params() {
    let prior = project_with_video_clip();
    let mut args = object_args(VIDEO_CLIP_ID);
    args.params = None;
    let err = compute_patch(&prior, &args).expect_err("missing params");
    assert!(matches!(
        err,
        TrackerCreateError::BadParams { ref field, .. } if field == "params"
    ));
}

#[test]
fn object_missing_bbox_field_returns_bad_params() {
    let prior = project_with_video_clip();
    let args = TrackerCreateArgs {
        params: Some(json!({})),
        ..object_args(VIDEO_CLIP_ID)
    };
    let err = compute_patch(&prior, &args).expect_err("missing bbox");
    assert!(matches!(
        err,
        TrackerCreateError::BadParams { ref field, .. } if field == "object_bbox_at_tk"
    ));
}

#[test]
fn object_bbox_missing_x_returns_bad_params() {
    let prior = project_with_video_clip();
    let args = TrackerCreateArgs {
        params: Some(json!({
            "object_bbox_at_tk": { "y": 0.0, "w": 1.0, "h": 1.0, "at_tk": 0 }
        })),
        ..object_args(VIDEO_CLIP_ID)
    };
    let err = compute_patch(&prior, &args).expect_err("missing x");
    assert!(matches!(
        err,
        TrackerCreateError::BadParams { ref field, .. } if field == "object_bbox_at_tk.x"
    ));
}

#[test]
fn object_at_tk_before_window_returns_bad_params() {
    let prior = project_with_video_clip();
    let args = TrackerCreateArgs {
        params: Some(json!({
            "object_bbox_at_tk": { "x": 1.0, "y": 1.0, "w": 1.0, "h": 1.0, "at_tk": -1_i64 }
        })),
        ..object_args(VIDEO_CLIP_ID)
    };
    let err = compute_patch(&prior, &args).expect_err("at_tk < window");
    let TrackerCreateError::BadParams { field, error } = err else {
        panic!("expected BadParams");
    };
    assert_eq!(field, "object_bbox_at_tk.at_tk");
    assert!(error.contains("outside source clip window"));
}

#[test]
fn object_at_tk_at_window_end_excluded_returns_bad_params() {
    // The window is [0, 240_000) — the upper bound is exclusive.
    let prior = project_with_video_clip();
    let args = TrackerCreateArgs {
        params: Some(json!({
            "object_bbox_at_tk": { "x": 1.0, "y": 1.0, "w": 1.0, "h": 1.0, "at_tk": 240_000 }
        })),
        ..object_args(VIDEO_CLIP_ID)
    };
    let err = compute_patch(&prior, &args).expect_err("at_tk == end excluded");
    assert!(matches!(
        err,
        TrackerCreateError::BadParams { ref field, .. } if field == "object_bbox_at_tk.at_tk"
    ));
}

#[test]
fn object_at_tk_one_before_window_end_accepted() {
    let prior = project_with_video_clip();
    let args = TrackerCreateArgs {
        params: Some(json!({
            "object_bbox_at_tk": { "x": 1.0, "y": 1.0, "w": 1.0, "h": 1.0, "at_tk": 239_999 }
        })),
        ..object_args(VIDEO_CLIP_ID)
    };
    let result = compute_patch(&prior, &args);
    assert!(result.is_ok(), "at_tk one before end is in window");
}

// ---------------------------------------------------------------------
// Face-algorithm params validation
// ---------------------------------------------------------------------

#[test]
fn face_without_params_succeeds() {
    let prior = project_with_video_clip();
    let args = face_args(VIDEO_CLIP_ID, None);
    let (_patch, _warnings, data) = compute_patch(&prior, &args).expect("face ok");
    assert_eq!(data.algorithm, "face");
}

#[test]
fn face_with_empty_params_succeeds() {
    let prior = project_with_video_clip();
    let args = face_args(VIDEO_CLIP_ID, Some(json!({})));
    let result = compute_patch(&prior, &args);
    assert!(result.is_ok());
}

#[test]
fn face_min_face_size_below_one_returns_bad_params() {
    let prior = project_with_video_clip();
    let args = face_args(VIDEO_CLIP_ID, Some(json!({ "min_face_size_px": 0 })));
    let err = compute_patch(&prior, &args).expect_err("min_face_size_px < 1");
    assert!(matches!(
        err,
        TrackerCreateError::BadParams { ref field, .. } if field == "min_face_size_px"
    ));
}

#[test]
fn face_confidence_above_one_returns_bad_params() {
    let prior = project_with_video_clip();
    let args = face_args(VIDEO_CLIP_ID, Some(json!({ "confidence_threshold": 1.5 })));
    let err = compute_patch(&prior, &args).expect_err("confidence > 1");
    assert!(matches!(
        err,
        TrackerCreateError::BadParams { ref field, .. } if field == "confidence_threshold"
    ));
}

#[test]
fn face_confidence_negative_returns_bad_params() {
    let prior = project_with_video_clip();
    let args = face_args(VIDEO_CLIP_ID, Some(json!({ "confidence_threshold": -0.1 })));
    let err = compute_patch(&prior, &args).expect_err("confidence < 0");
    assert!(matches!(
        err,
        TrackerCreateError::BadParams { ref field, .. } if field == "confidence_threshold"
    ));
}

// ---------------------------------------------------------------------
// Optical-flow-algorithm params validation
// ---------------------------------------------------------------------

#[test]
fn optical_flow_missing_params_returns_bad_params() {
    let prior = project_with_video_clip();
    let mut args = optical_flow_args(VIDEO_CLIP_ID);
    args.params = None;
    let err = compute_patch(&prior, &args).expect_err("optical_flow missing params");
    assert!(matches!(
        err,
        TrackerCreateError::BadParams { ref field, .. } if field == "params"
    ));
}

#[test]
fn optical_flow_at_tk_outside_window_returns_bad_params() {
    let prior = project_with_video_clip();
    let args = TrackerCreateArgs {
        params: Some(json!({
            "point_at_tk": { "x": 1.0, "y": 1.0, "at_tk": 1_000_000 }
        })),
        ..optical_flow_args(VIDEO_CLIP_ID)
    };
    let err = compute_patch(&prior, &args).expect_err("at_tk out of window");
    assert!(matches!(
        err,
        TrackerCreateError::BadParams { ref field, .. } if field == "point_at_tk.at_tk"
    ));
}

#[test]
fn optical_flow_happy_path_produces_tracker() {
    let prior = project_with_video_clip();
    let args = optical_flow_args(VIDEO_CLIP_ID);
    let (_patch, _warnings, data) = compute_patch(&prior, &args).expect("optical_flow ok");
    assert_eq!(data.algorithm, "optical_flow");
}

// ---------------------------------------------------------------------
// Happy path + patch shape + post-state
// ---------------------------------------------------------------------

#[test]
fn happy_path_emits_single_add_op_at_trackers_minus() {
    let prior = project_with_video_clip();
    let args = object_args(VIDEO_CLIP_ID);
    let (patch, _warnings, _data) = compute_patch(&prior, &args).expect("happy");
    let ops = patch.as_array().expect("patch is array");
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0]["op"], "add");
    assert_eq!(ops[0]["path"], "/trackers/-");
}

#[test]
fn happy_path_patch_applies_and_appends_one_tracker_record() {
    let prior = project_with_video_clip();
    let args = object_args(VIDEO_CLIP_ID);
    let (patch, _warnings, data) = compute_patch(&prior, &args).expect("happy");
    let post = apply_patch(&prior, patch);
    assert_eq!(post.trackers.len(), 1);
    let map = &post.trackers[0].0;
    assert_eq!(
        map.get("tracker_id").and_then(Value::as_str),
        Some(data.tracker_id.as_str())
    );
    assert_eq!(
        map.get("source_clip_id").and_then(Value::as_str),
        Some(VIDEO_CLIP_ID)
    );
    assert_eq!(map.get("algorithm").and_then(Value::as_str), Some("object"));
    assert_eq!(map.get("sample_count").and_then(Value::as_i64), Some(-1));
    assert_eq!(map.get("cache_hash").and_then(Value::as_str), Some(""));
    assert_eq!(map.get("cache_path").and_then(Value::as_str), Some(""));
    let applied = map.get("applied_to_clip_ids").and_then(Value::as_array);
    assert!(applied.is_some_and(|arr| arr.is_empty()));
}

#[test]
fn data_envelope_echoes_resolved_clip_id_and_algorithm() {
    let prior = project_with_video_clip();
    let args = object_args(VIDEO_CLIP_ID);
    let (_patch, _warnings, data) = compute_patch(&prior, &args).expect("happy");
    assert_eq!(data.source_clip_id, VIDEO_CLIP_ID);
    assert_eq!(data.algorithm, "object");
}

#[test]
fn two_creates_in_sequence_mint_distinct_tracker_ids() {
    // Clock-derived `tracker_id` uniqueness invariant: two consecutive
    // creates on the same source clip must produce different ids.
    let prior = project_with_video_clip();
    let args = object_args(VIDEO_CLIP_ID);

    let (patch_a, _, data_a) = compute_patch(&prior, &args).expect("first");
    let after_a = apply_patch(&prior, patch_a);
    let (_patch_b, _, data_b) = compute_patch(&after_a, &args).expect("second");

    assert_ne!(
        data_a.tracker_id, data_b.tracker_id,
        "tracker_ids must be unique across sibling creates"
    );
}

// ---------------------------------------------------------------------
// Envelope warning + reconstructor
// ---------------------------------------------------------------------

#[test]
fn envelope_warning_emitted_with_correct_code() {
    let prior = project_with_video_clip();
    let args = object_args(VIDEO_CLIP_ID);
    let (_patch, warnings, _data) = compute_patch(&prior, &args).expect("happy");
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_TRACKER_CREATE_ENVELOPE_CODE);
}

#[test]
fn envelope_warning_carries_tracker_id_source_clip_id_algorithm() {
    let prior = project_with_video_clip();
    let args = object_args(VIDEO_CLIP_ID);
    let (_patch, warnings, data) = compute_patch(&prior, &args).expect("happy");
    let details = &warnings[0]["details"];
    assert_eq!(details["tracker_id"], data.tracker_id);
    assert_eq!(details["source_clip_id"], VIDEO_CLIP_ID);
    assert_eq!(details["algorithm"], "object");
}

#[test]
fn reconstruct_round_trip_matches_forward_data() {
    let prior = project_with_video_clip();
    let args = object_args(VIDEO_CLIP_ID);
    let (_patch, warnings, data) = compute_patch(&prior, &args).expect("happy");

    let reconstructed = data_envelope_from_args_warnings(&args, &warnings).expect("round-trip");
    assert_eq!(data, reconstructed);
    let lhs = serde_json::to_vec(&data).expect("forward serializes");
    let rhs = serde_json::to_vec(&reconstructed).expect("reconstructed serializes");
    assert_eq!(lhs, rhs);
}

#[test]
fn reconstruct_rejects_warning_set_without_envelope() {
    let args = object_args(VIDEO_CLIP_ID);
    let warnings: Vec<Value> = vec![json!({
        "code": "W_OTHER",
        "message": "not the envelope",
        "details": {},
    })];
    let err = data_envelope_from_args_warnings(&args, &warnings)
        .expect_err("missing envelope warning must surface");
    assert!(matches!(
        err,
        verbreel_state::ReconstructError::MissingField { .. }
    ));
}

// ---------------------------------------------------------------------
// Default-registry + Verb-trait integration
// ---------------------------------------------------------------------

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "tracker.create")
        .expect("default_fixtures includes tracker.create");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(TrackerCreateVerb))
        .expect("register tracker.create verb");

    let report = validate_reconstructors(&registry, &[fixture])
        .expect("tracker.create reconstructor passes");
    assert_eq!(report.verbs_checked, vec!["tracker.create"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn verb_trait_surface_lookup_via_default_registry() {
    let registry = default_registry();
    let verb = registry
        .get("tracker.create")
        .expect("tracker.create registered in default_registry");
    assert_eq!(verb.verb(), "tracker.create");
}

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let prior = project_with_video_clip();
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        prior,
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry succeeds");

    let outcome = store
        .mutate_via_verb(
            "tracker.create",
            serde_json::to_value(object_args(VIDEO_CLIP_ID)).expect("args serialize"),
            None,
        )
        .expect("tracker.create should route");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("happy path must apply");
    };
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_TRACKER_CREATE_ENVELOPE_CODE);
    let envelope: TrackerCreateData = serde_json::from_value(data).expect("data parses");
    assert_eq!(envelope.algorithm, "object");
    assert_eq!(envelope.source_clip_id, VIDEO_CLIP_ID);
    assert!(!envelope.tracker_id.is_empty());
}
