//! Tests for `describe` (§1.3) — sixty-second production verb.

use serde_json::{Value, json};
use std::sync::Arc;

use verbreel_state::verbs::describe::{compute_patch, data_envelope_from_post_state};
use verbreel_state::{
    DescribeArgs, DescribeData, DescribeError, DescribeKind, DescribeVerb, Project, Track,
    VerbRegistry, default_fixtures, default_registry, validate_reconstructors,
};
use verbreel_types::{ProjectId, Tick};

#[cfg(feature = "native")]
use verbreel_state::{MutateOutcome, ProjectStore};

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";

const TRACK_VIDEO_ID: &str = "0190b8d3-15e3-7000-bd00-0000000aad01";
const TRACK_EFFECT_ID: &str = "0190b8d3-15e3-7000-bd00-0000000aad02";
const CLIP_ID: &str = "0190b8d3-15e3-7000-bd00-0000000bbd01";
const ASSET_ID: &str = "0190b8d3-15e3-7000-bd00-0000000ccd01";
const MARKER_ID: &str = "0190b8d3-15e3-7000-bd00-0000000ddd01";
const CLIP_EFFECT_ID: &str = "0190b8d3-15e3-7000-bd00-0000000eed01";
const TRACK_EFFECT_INST_ID: &str = "0190b8d3-15e3-7000-bd00-0000000eed02";
const KEYFRAME_ID: &str = "0190b8d3-15e3-7000-bd00-0000000ffd01";

const MISSING_UUID: &str = "0190b8d3-15e3-7000-bd00-000000ffffff";
const OTHER_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-0000000abcde";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture -> Project")
}

fn fixture_project_id() -> ProjectId {
    empty_project().id
}

fn args(target: impl Into<String>) -> DescribeArgs {
    DescribeArgs {
        project_id: fixture_project_id(),
        target: target.into(),
    }
}

fn video_asset_json(id: &str) -> Value {
    json!({
        "id": id,
        "kind": "video",
        "hash": "53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658",
        "path": "assets/53/53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.mp4",
        "original_filename": "describe.mp4",
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
    })
}

fn marker_json(id: &str) -> Value {
    json!({
        "id": id,
        "time_tk": 500,
        "label": "Marker",
        "color": "#ffaa00ff",
    })
}

/// A project with a video track holding one clip + one clip-effect +
/// one keyframe, plus a separate effect track carrying one
/// (untyped-`Value`) track-level effect, plus one asset and one marker.
/// The full entity menu in a single graph so each prefix has a real id
/// to resolve.
fn project_with_all_entities() -> Project {
    let mut project = empty_project();
    // Empty fixture pre-seeds `[Video, Audio]` tracks; clear them so the
    // bespoke `[Video, Effect]` ordering below satisfies the §0.13
    // grouped-by-kind invariant when ProjectStore::create writes it.
    project.tracks.clear();

    let video_track: Track = serde_json::from_value(json!({
        "id": TRACK_VIDEO_ID,
        "kind": "video",
        "name": "Describe Video",
        "locked": false,
        "clips": [{
            "id": CLIP_ID,
            "name": "Describe Clip",
            "asset_id": ASSET_ID,
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 240_000,
            "locked": false,
            "effects": [{
                "id": CLIP_EFFECT_ID,
                "kind": "blur",
                "enabled": true,
                "params": {},
            }],
            "keyframes": [{
                "id": KEYFRAME_ID,
                "property": "opacity",
                "time_tk": 0,
                "value": 1.0,
            }],
        }],
    }))
    .expect("video track for describe fixture parses");

    let effect_track: Track = serde_json::from_value(json!({
        "id": TRACK_EFFECT_ID,
        "kind": "effect",
        "name": "Adjustment",
        "locked": false,
        "clips": [],
        "effects": [{
            "id": TRACK_EFFECT_INST_ID,
            "kind": "vignette",
            "enabled": true,
            "params": {},
        }],
    }))
    .expect("effect track for describe fixture parses");

    project.tracks.push(video_track);
    project.tracks.push(effect_track);
    project.duration_tk = Tick::new(240_000);

    project
        .assets
        .push(serde_json::from_value(video_asset_json(ASSET_ID)).expect("asset parses"));
    project
        .markers
        .push(serde_json::from_value(marker_json(MARKER_ID)).expect("marker parses"));

    project
}

// ---------------------------------------------------------------------
// Args deserialization
// ---------------------------------------------------------------------

#[test]
fn args_deserialize_round_trip() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "target": format!("asset:{ASSET_ID}"),
    });
    let parsed: DescribeArgs = serde_json::from_value(raw).expect("args deserialize");
    assert_eq!(parsed.target, format!("asset:{ASSET_ID}"));
}

#[test]
fn args_missing_target_errors() {
    let raw = json!({ "project_id": FIXTURE_PROJECT_ID });
    let err =
        serde_json::from_value::<DescribeArgs>(raw).expect_err("missing `target` must reject");
    assert!(err.to_string().contains("target"));
}

#[test]
fn args_missing_project_id_errors() {
    let raw = json!({ "target": format!("asset:{ASSET_ID}") });
    let err =
        serde_json::from_value::<DescribeArgs>(raw).expect_err("missing `project_id` must reject");
    assert!(err.to_string().contains("project_id"));
}

#[test]
fn args_wrong_target_type_errors() {
    let raw = json!({ "project_id": FIXTURE_PROJECT_ID, "target": 42 });
    let err =
        serde_json::from_value::<DescribeArgs>(raw).expect_err("non-string `target` must reject");
    let msg = err.to_string();
    assert!(
        msg.contains("string") || msg.contains("target"),
        "error mentions string-shape rejection: {msg}"
    );
}

// ---------------------------------------------------------------------
// E_BAD_SELECTOR
// ---------------------------------------------------------------------

#[test]
fn empty_target_errors_bad_selector() {
    let prior = empty_project();
    let err = compute_patch(&prior, &args("")).expect_err("empty target");
    assert!(matches!(err, DescribeError::BadSelector { .. }));
}

#[test]
fn malformed_no_colon_errors_bad_selector() {
    let prior = empty_project();
    let err = compute_patch(&prior, &args("not-a-selector")).expect_err("no colon");
    let DescribeError::BadSelector { detail } = err else {
        panic!("expected BadSelector");
    };
    assert!(detail.contains("unqualified") || detail.contains("missing"));
}

#[test]
fn bare_uuid_errors_bad_selector() {
    let prior = empty_project();
    let err = compute_patch(&prior, &args(ASSET_ID)).expect_err("bare uuid rejected");
    assert!(matches!(err, DescribeError::BadSelector { .. }));
}

#[test]
fn unknown_prefix_errors_bad_selector() {
    let prior = empty_project();
    let err =
        compute_patch(&prior, &args(format!("unknown:{ASSET_ID}"))).expect_err("unknown prefix");
    let DescribeError::BadSelector { detail } = err else {
        panic!("expected BadSelector");
    };
    assert!(
        detail.contains("unknown"),
        "detail mentions prefix: {detail}"
    );
}

#[test]
fn empty_prefix_errors_bad_selector() {
    let prior = empty_project();
    let err = compute_patch(&prior, &args(format!(":{ASSET_ID}"))).expect_err("empty prefix");
    assert!(matches!(err, DescribeError::BadSelector { .. }));
}

#[test]
fn empty_body_errors_bad_selector() {
    let prior = empty_project();
    let err = compute_patch(&prior, &args("clip:")).expect_err("empty body");
    assert!(matches!(err, DescribeError::BadSelector { .. }));
}

#[test]
fn unparseable_uuid_body_errors_bad_selector() {
    let prior = empty_project();
    let err = compute_patch(&prior, &args("clip:not-a-uuid")).expect_err("unparseable uuid body");
    assert!(matches!(err, DescribeError::BadSelector { .. }));
}

// ---------------------------------------------------------------------
// Happy paths — one per kind (effect counts twice)
// ---------------------------------------------------------------------

#[test]
fn happy_path_project_self() {
    let prior = project_with_all_entities();
    let target = format!("project:{}", prior.id);
    let (patch, warnings, data) =
        compute_patch(&prior, &args(target)).expect("project self happy path");
    assert_eq!(patch.as_array().map(Vec::len), Some(0));
    assert!(warnings.is_empty());
    assert_eq!(data.kind, DescribeKind::Project);
    assert_eq!(
        data.entity["id"].as_str(),
        Some(prior.id.to_string().as_str())
    );
}

#[test]
fn happy_path_track() {
    let prior = project_with_all_entities();
    let (_, _, data) =
        compute_patch(&prior, &args(format!("track:{TRACK_VIDEO_ID}"))).expect("track happy path");
    assert_eq!(data.kind, DescribeKind::Track);
    assert_eq!(data.entity["id"].as_str(), Some(TRACK_VIDEO_ID));
}

#[test]
fn happy_path_clip() {
    let prior = project_with_all_entities();
    let (_, _, data) =
        compute_patch(&prior, &args(format!("clip:{CLIP_ID}"))).expect("clip happy path");
    assert_eq!(data.kind, DescribeKind::Clip);
    assert_eq!(data.entity["id"].as_str(), Some(CLIP_ID));
}

#[test]
fn happy_path_effect_clip_attached() {
    let prior = project_with_all_entities();
    let (_, _, data) = compute_patch(&prior, &args(format!("effect:{CLIP_EFFECT_ID}")))
        .expect("clip-attached effect happy path");
    assert_eq!(data.kind, DescribeKind::Effect);
    assert_eq!(data.entity["id"].as_str(), Some(CLIP_EFFECT_ID));
    assert_eq!(data.entity["kind"].as_str(), Some("blur"));
}

#[test]
fn happy_path_effect_track_level() {
    let prior = project_with_all_entities();
    let (_, _, data) = compute_patch(&prior, &args(format!("effect:{TRACK_EFFECT_INST_ID}")))
        .expect("track-level effect happy path");
    assert_eq!(data.kind, DescribeKind::Effect);
    assert_eq!(data.entity["id"].as_str(), Some(TRACK_EFFECT_INST_ID));
    assert_eq!(data.entity["kind"].as_str(), Some("vignette"));
}

#[test]
fn happy_path_keyframe() {
    let prior = project_with_all_entities();
    let (_, _, data) = compute_patch(&prior, &args(format!("keyframe:{KEYFRAME_ID}")))
        .expect("keyframe happy path");
    assert_eq!(data.kind, DescribeKind::Keyframe);
    assert_eq!(data.entity["id"].as_str(), Some(KEYFRAME_ID));
}

#[test]
fn happy_path_asset() {
    let prior = project_with_all_entities();
    let (_, _, data) =
        compute_patch(&prior, &args(format!("asset:{ASSET_ID}"))).expect("asset happy path");
    assert_eq!(data.kind, DescribeKind::Asset);
    assert_eq!(data.entity["id"].as_str(), Some(ASSET_ID));
    assert_eq!(data.entity["kind"].as_str(), Some("video"));
}

#[test]
fn happy_path_marker() {
    let prior = project_with_all_entities();
    let (_, _, data) =
        compute_patch(&prior, &args(format!("marker:{MARKER_ID}"))).expect("marker happy path");
    assert_eq!(data.kind, DescribeKind::Marker);
    assert_eq!(data.entity["id"].as_str(), Some(MARKER_ID));
}

// ---------------------------------------------------------------------
// E_NOT_FOUND — one per non-project kind (project mismatch is
// E_ARGS_INCOMPATIBLE, not E_NOT_FOUND).
// ---------------------------------------------------------------------

#[test]
fn missing_track_errors_not_found() {
    let prior = project_with_all_entities();
    let err =
        compute_patch(&prior, &args(format!("track:{MISSING_UUID}"))).expect_err("track not found");
    assert!(matches!(err, DescribeError::NotFound { kind: "track", .. }));
}

#[test]
fn missing_clip_errors_not_found() {
    let prior = project_with_all_entities();
    let err =
        compute_patch(&prior, &args(format!("clip:{MISSING_UUID}"))).expect_err("clip not found");
    assert!(matches!(err, DescribeError::NotFound { kind: "clip", .. }));
}

#[test]
fn missing_effect_errors_not_found() {
    let prior = project_with_all_entities();
    let err = compute_patch(&prior, &args(format!("effect:{MISSING_UUID}")))
        .expect_err("effect not found");
    assert!(matches!(
        err,
        DescribeError::NotFound { kind: "effect", .. }
    ));
}

#[test]
fn missing_keyframe_errors_not_found() {
    let prior = project_with_all_entities();
    let err = compute_patch(&prior, &args(format!("keyframe:{MISSING_UUID}")))
        .expect_err("keyframe not found");
    assert!(matches!(
        err,
        DescribeError::NotFound {
            kind: "keyframe",
            ..
        }
    ));
}

#[test]
fn missing_asset_errors_not_found() {
    let prior = project_with_all_entities();
    let err =
        compute_patch(&prior, &args(format!("asset:{MISSING_UUID}"))).expect_err("asset not found");
    assert!(matches!(err, DescribeError::NotFound { kind: "asset", .. }));
}

#[test]
fn missing_marker_errors_not_found() {
    let prior = project_with_all_entities();
    let err = compute_patch(&prior, &args(format!("marker:{MISSING_UUID}")))
        .expect_err("marker not found");
    assert!(matches!(
        err,
        DescribeError::NotFound { kind: "marker", .. }
    ));
}

#[test]
fn unknown_project_uuid_errors_args_incompatible() {
    // `project:<other>` with project_id != other → ArgsIncompatible per
    // spec §1.3 decision table. This stands in for the per-prefix
    // "missing project" failure: the project branch never returns
    // NotFound because the engine resolves prior from project_id.
    let prior = project_with_all_entities();
    let err = compute_patch(&prior, &args(format!("project:{OTHER_PROJECT_ID}")))
        .expect_err("project mismatch");
    let DescribeError::ArgsIncompatible {
        target_project_id,
        supplied_project_id,
    } = err
    else {
        panic!("expected ArgsIncompatible");
    };
    assert_eq!(target_project_id, OTHER_PROJECT_ID);
    assert_eq!(supplied_project_id, prior.id.to_string());
}

// ---------------------------------------------------------------------
// Empty-patch / empty-warnings invariants
// ---------------------------------------------------------------------

#[test]
fn success_emits_empty_patch() {
    let prior = project_with_all_entities();
    let (patch, _, _) =
        compute_patch(&prior, &args(format!("asset:{ASSET_ID}"))).expect("happy path");
    assert!(
        patch.as_array().map(|a| a.is_empty()).unwrap_or(false),
        "describe is read-only: patch MUST be []"
    );
}

#[test]
fn success_emits_empty_warnings() {
    let prior = project_with_all_entities();
    let (_, warnings, _) =
        compute_patch(&prior, &args(format!("asset:{ASSET_ID}"))).expect("happy path");
    assert!(warnings.is_empty(), "describe emits no warnings");
}

// ---------------------------------------------------------------------
// Reconstructor round-trip
// ---------------------------------------------------------------------

#[test]
fn reconstructor_round_trip_byte_identical() {
    let prior = project_with_all_entities();
    let args = args(format!("clip:{CLIP_ID}"));
    let (_, _, data) = compute_patch(&prior, &args).expect("happy path");
    // Read-only verb: post-state == pre-state, so reconstruct against
    // the same graph and the envelope must equal byte-for-byte.
    let reconstructed =
        data_envelope_from_post_state(&args, &prior).expect("reconstruct from post-state");
    let lhs = serde_json::to_value(&data).expect("data -> value");
    let rhs = serde_json::to_value(&reconstructed).expect("reconstructed -> value");
    assert_eq!(lhs, rhs);
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "describe")
        .expect("default_fixtures includes describe");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(DescribeVerb))
        .expect("register describe verb");

    let report =
        validate_reconstructors(&registry, &[fixture]).expect("describe reconstruct from fixture");
    assert_eq!(report.verbs_checked, vec!["describe"]);
    assert_eq!(report.fixtures_run, 1);
}

// ---------------------------------------------------------------------
// Verb-trait surface + data-shape lock
// ---------------------------------------------------------------------

#[test]
fn verb_trait_routes_through_registry() {
    let registry = default_registry();
    let verb = registry
        .get("describe")
        .expect("describe is in default_registry");
    let prior = project_with_all_entities();
    let args_value = json!({
        "project_id": prior.id.to_string(),
        "target": format!("clip:{CLIP_ID}"),
    });
    let (patch, data, warnings) = verb
        .compute_patch(&prior, &args_value)
        .expect("verb compute_patch happy path");
    assert!(patch.0.is_empty(), "describe patch is empty");
    assert!(warnings.is_empty());
    assert_eq!(data["kind"].as_str(), Some("clip"));
    assert_eq!(data["entity"]["id"].as_str(), Some(CLIP_ID));
}

#[test]
fn data_shape_has_exactly_kind_and_entity() {
    let prior = project_with_all_entities();
    let (_, _, data) =
        compute_patch(&prior, &args(format!("marker:{MARKER_ID}"))).expect("happy path");
    let value = serde_json::to_value(&data).expect("data -> value");
    let obj = value.as_object().expect("data is object");
    let keys: Vec<&String> = obj.keys().collect();
    assert_eq!(
        keys.len(),
        2,
        "data has exactly two top-level keys: {keys:?}"
    );
    assert!(obj.contains_key("kind"));
    assert!(obj.contains_key("entity"));
}

// ---------------------------------------------------------------------
// Native-feature route via ProjectStore::mutate_via_verb
// ---------------------------------------------------------------------

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        project_with_all_entities(),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry clears the gate and writes project.json");

    let outcome = store
        .mutate_via_verb(
            "describe",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "target": format!("clip:{CLIP_ID}"),
            }),
            None,
        )
        .expect("mutate_via_verb happy path");

    let MutateOutcome::NoOp { data, warnings, .. } = outcome else {
        panic!("happy path must return NoOp, got {outcome:?}");
    };

    let typed: DescribeData = serde_json::from_value(data).expect("describe data is DescribeData");
    assert_eq!(typed.kind, DescribeKind::Clip);
    assert_eq!(typed.entity["id"].as_str(), Some(CLIP_ID));
    assert!(warnings.is_empty());
}
