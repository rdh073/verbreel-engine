//! Tests for `keyframe.set` (§8.3) — thirty-fourth production verb.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::keyframe_set::{
    KeyframeSetArgs, KeyframeSetData, KeyframeSetError, KeyframeSetVerb, W_NOOP_CODE,
    compute_patch, data_envelope_from_post_state,
};
use verbreel_state::{
    Easing, MutateOutcome, Project, Verb, VerbRegistry, default_fixtures, default_registry,
    validate_reconstructors,
};
use verbreel_types::{KeyframeId, Tick};

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const TRACK_ID: &str = "01900000-0000-7000-8000-0000000aa901";
const CLIP_ID: &str = "01900000-0000-7000-8000-0000000bb901";
const ASSET_ID: &str = "01900000-0000-7000-8000-0000000cc901";
const KEYFRAME_ID: &str = "01900000-0000-7000-8000-0000000ff901";
const SECOND_KEYFRAME_ID: &str = "01900000-0000-7000-8000-0000000ff902";
const MISSING_KEYFRAME_ID: &str = "01900000-0000-7000-8000-0000000ff903";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture -> Project")
}

fn fixture_project_id() -> verbreel_types::ProjectId {
    FIXTURE_PROJECT_ID
        .parse()
        .expect("fixture project id parses")
}

fn base_args() -> KeyframeSetArgs {
    KeyframeSetArgs {
        project_id: fixture_project_id(),
        keyframe: KEYFRAME_ID.to_string(),
        time_tk: None,
        value: None,
        easing: None,
        bezier: None,
    }
}

fn non_finite_json(value: f64) -> Value {
    serde_json::Number::from_f64(value)
        .map(Value::Number)
        .unwrap_or(Value::Null)
}

fn video_asset() -> Value {
    json!({
        "id": ASSET_ID,
        "kind": "video",
        "hash": "53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658",
        "path": "assets/53/53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.mp4",
        "original_filename": "keyframe-set.mp4",
        "imported_at": "2026-05-24T00:00:00Z",
        "metadata": {
            "duration_tk": 480_000,
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

fn project_with_clip(extra_clip_fields: serde_json::Map<String, Value>) -> Project {
    let mut prior = empty_project();
    prior.tracks.clear();
    prior.assets.clear();
    prior
        .assets
        .push(serde_json::from_value(video_asset()).expect("asset fixture parses"));

    let mut clip = serde_json::Map::new();
    clip.insert("id".to_string(), json!(CLIP_ID));
    clip.insert("name".to_string(), json!("Clip 1"));
    clip.insert("asset_id".to_string(), json!(ASSET_ID));
    clip.insert("track_position_tk".to_string(), json!(0));
    clip.insert("source_in_tk".to_string(), json!(0));
    clip.insert("source_out_tk".to_string(), json!(480_000));
    clip.insert("locked".to_string(), json!(false));
    for (key, value) in extra_clip_fields {
        clip.insert(key, value);
    }

    let track = serde_json::from_value(json!({
        "id": TRACK_ID,
        "kind": "video",
        "name": "Video 1",
        "locked": false,
        "clips": [Value::Object(clip)],
    }))
    .expect("track fixture parses");

    prior.tracks.push(track);
    prior.duration_tk = Tick::new(480_000);
    prior
}

fn project() -> Project {
    let mut extra = serde_json::Map::new();
    extra.insert(
        "keyframes".to_string(),
        json!([{
            "id": KEYFRAME_ID,
            "property": "opacity",
            "time_tk": 0,
            "value": 0.5,
            "easing": "linear",
        }]),
    );
    project_with_clip(extra)
}

fn project_with_duplicate_candidate() -> Project {
    let mut extra = serde_json::Map::new();
    extra.insert(
        "keyframes".to_string(),
        json!([
            {
                "id": KEYFRAME_ID,
                "property": "opacity",
                "time_tk": 0,
                "value": 0.5,
            },
            {
                "id": SECOND_KEYFRAME_ID,
                "property": "opacity",
                "time_tk": 100,
                "value": 0.75,
            }
        ]),
    );
    project_with_clip(extra)
}

fn project_with_cubic_bezier_keyframe() -> Project {
    let mut extra = serde_json::Map::new();
    extra.insert(
        "keyframes".to_string(),
        json!([{
            "id": KEYFRAME_ID,
            "property": "opacity",
            "time_tk": 0,
            "value": 0.5,
            "easing": "cubic-bezier",
            "bezier": [0.1, 0.2, 0.3, 0.4],
        }]),
    );
    project_with_clip(extra)
}

fn patch_ops(patch: &Value) -> &[Value] {
    patch.as_array().expect("patch is array")
}

fn sole_op(patch: &Value) -> &serde_json::Map<String, Value> {
    let ops = patch_ops(patch);
    assert_eq!(ops.len(), 1);
    ops[0].as_object().expect("op is object")
}

#[test]
fn compute_patch_change_value_only() {
    let prior = project();
    let mut args = base_args();
    args.value = Some(json!(0.8));

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("happy path");

    assert!(warnings.is_empty());
    let op = sole_op(&patch);
    assert_eq!(op["op"], "replace");
    assert_eq!(op["path"], "/tracks/0/clips/0/keyframes/0/value");
    assert_eq!(op["value"], 0.8);
    assert_eq!(data.keyframe.value, json!(0.8));
}

#[test]
fn compute_patch_change_time_only() {
    let prior = project();
    let mut args = base_args();
    args.time_tk = Some(100);

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("happy path");

    assert!(warnings.is_empty());
    let op = sole_op(&patch);
    assert_eq!(op["op"], "replace");
    assert_eq!(op["path"], "/tracks/0/clips/0/keyframes/0/time_tk");
    assert_eq!(op["value"], 100);
    assert_eq!(data.keyframe.time_tk, Tick::new(100));
}

#[test]
fn compute_patch_change_easing_to_ease_in() {
    let prior = project();
    let mut args = base_args();
    args.easing = Some("ease-in".to_string());

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("happy path");

    assert!(warnings.is_empty());
    let op = sole_op(&patch);
    assert_eq!(op["op"], "replace");
    assert_eq!(op["path"], "/tracks/0/clips/0/keyframes/0/easing");
    assert_eq!(op["value"], "ease-in");
    assert_eq!(data.keyframe.easing, Easing::EaseIn);
}

#[test]
fn compute_patch_change_easing_to_cubic_bezier_with_bezier_succeeds() {
    let prior = project();
    let mut args = base_args();
    args.easing = Some("cubic-bezier".to_string());
    args.bezier = Some([0.1, 0.2, 0.3, 0.4]);

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("happy path");

    assert!(warnings.is_empty());
    let ops = patch_ops(&patch);
    assert_eq!(ops.len(), 2);
    assert_eq!(ops[0]["path"], "/tracks/0/clips/0/keyframes/0/easing");
    assert_eq!(ops[0]["value"], "cubic-bezier");
    assert_eq!(ops[1]["op"], "add");
    assert_eq!(ops[1]["path"], "/tracks/0/clips/0/keyframes/0/bezier");
    assert_eq!(ops[1]["value"], json!([0.1, 0.2, 0.3, 0.4]));
    assert_eq!(
        data.keyframe.easing,
        Easing::CubicBezier {
            bezier: [0.1, 0.2, 0.3, 0.4]
        }
    );
}

#[test]
fn compute_patch_change_easing_to_cubic_bezier_without_bezier_is_schema_violation() {
    let prior = project();
    let mut args = base_args();
    args.easing = Some("cubic-bezier".to_string());

    let err = compute_patch(&prior, &args).expect_err("missing bezier rejects");
    assert!(matches!(err, KeyframeSetError::SchemaViolation { .. }));
}

#[test]
fn compute_patch_change_bezier_only_while_easing_already_cubic_bezier_succeeds() {
    let prior = project_with_cubic_bezier_keyframe();
    let mut args = base_args();
    args.bezier = Some([0.4, 0.3, 0.2, 0.1]);

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("happy path");

    assert!(warnings.is_empty());
    let op = sole_op(&patch);
    assert_eq!(op["op"], "replace");
    assert_eq!(op["path"], "/tracks/0/clips/0/keyframes/0/bezier");
    assert_eq!(op["value"], json!([0.4, 0.3, 0.2, 0.1]));
    assert_eq!(
        data.keyframe.easing,
        Easing::CubicBezier {
            bezier: [0.4, 0.3, 0.2, 0.1]
        }
    );
}

#[test]
fn compute_patch_supply_bezier_with_non_bezier_easing_is_schema_violation() {
    let prior = project();
    let mut args = base_args();
    args.easing = Some("ease-in".to_string());
    args.bezier = Some([0.1, 0.2, 0.3, 0.4]);

    let err = compute_patch(&prior, &args).expect_err("bezier with non-bezier rejects");
    assert!(matches!(err, KeyframeSetError::SchemaViolation { .. }));
}

#[test]
fn compute_patch_change_value_invalid_opacity_is_bad_value() {
    let prior = project();
    let mut args = base_args();
    args.value = Some(json!(1.01));

    let err = compute_patch(&prior, &args).expect_err("bad opacity rejects");
    assert!(matches!(err, KeyframeSetError::BadValue { .. }));
}

#[test]
fn compute_patch_nan_value_is_bad_value() {
    let prior = project();
    let mut args = base_args();
    args.value = Some(non_finite_json(f64::NAN));

    let err = compute_patch(&prior, &args).expect_err("nan rejects");
    assert!(matches!(err, KeyframeSetError::BadValue { .. }));
}

#[test]
fn compute_patch_negative_time_is_bad_time() {
    let prior = project();
    let mut args = base_args();
    args.time_tk = Some(-1);

    let err = compute_patch(&prior, &args).expect_err("negative time rejects");
    assert!(matches!(err, KeyframeSetError::BadTime { time_tk: -1, .. }));
}

#[test]
fn compute_patch_time_beyond_clip_duration_is_bad_time() {
    let prior = project();
    let mut args = base_args();
    args.time_tk = Some(480_001);

    let err = compute_patch(&prior, &args).expect_err("time beyond duration rejects");
    assert!(matches!(
        err,
        KeyframeSetError::BadTime {
            time_tk: 480_001,
            clip_duration_tk: 480_000
        }
    ));
}

#[test]
fn compute_patch_duplicate_time_on_same_property_is_duplicate() {
    let prior = project_with_duplicate_candidate();
    let mut args = base_args();
    args.time_tk = Some(100);

    let err = compute_patch(&prior, &args).expect_err("duplicate time rejects");
    match err {
        KeyframeSetError::Duplicate {
            existing_keyframe_id,
        } => assert_eq!(existing_keyframe_id.to_string(), SECOND_KEYFRAME_ID),
        other => panic!("expected Duplicate, got {other:?}"),
    }
}

#[test]
fn compute_patch_keyframe_not_found() {
    let prior = project();
    let mut args = base_args();
    args.keyframe = MISSING_KEYFRAME_ID.to_string();
    args.value = Some(json!(0.8));

    let err = compute_patch(&prior, &args).expect_err("missing keyframe rejects");
    assert!(matches!(err, KeyframeSetError::KeyframeNotFound { .. }));
}

#[test]
fn compute_patch_bad_selector_structural_form_rejected() {
    let prior = project();
    let mut args = base_args();
    args.keyframe = format!("clip({CLIP_ID}).keyframes[0]");
    args.value = Some(json!(0.8));

    let err = compute_patch(&prior, &args).expect_err("structural selector rejects");
    assert!(matches!(err, KeyframeSetError::BadSelector { .. }));
}

#[test]
fn compute_patch_locked_clip_rejects_with_e_locked() {
    let mut prior = project();
    prior.tracks[0].clips[0].locked = true;
    let mut args = base_args();
    args.value = Some(json!(0.8));

    let err = compute_patch(&prior, &args).expect_err("locked clip rejects");
    assert!(matches!(err, KeyframeSetError::Locked { kind: "clip", .. }));
}

#[test]
fn compute_patch_locked_precedes_bad_value() {
    let mut prior = project();
    prior.tracks[0].clips[0].locked = true;
    let mut args = base_args();
    args.value = Some(json!(2.0));

    let err = compute_patch(&prior, &args).expect_err("locked precedes bad value");
    assert!(matches!(err, KeyframeSetError::Locked { kind: "clip", .. }));
}

#[test]
fn compute_patch_locked_precedes_bad_time() {
    let mut prior = project();
    prior.tracks[0].locked = true;
    let mut args = base_args();
    args.time_tk = Some(-1);

    let err = compute_patch(&prior, &args).expect_err("locked precedes bad time");
    assert!(matches!(
        err,
        KeyframeSetError::Locked { kind: "track", .. }
    ));
}

#[test]
fn compute_patch_no_changes_is_noop() {
    let prior = project();
    let mut args = base_args();
    args.time_tk = Some(0);
    args.value = Some(json!(0.5));
    args.easing = Some("linear".to_string());

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("same fields is no-op");

    assert_eq!(patch, json!([]));
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_NOOP_CODE);
    assert_eq!(warnings[0]["details"]["keyframe_id"], KEYFRAME_ID);
    assert_eq!(data.keyframe.value, json!(0.5));
}

#[test]
fn compute_patch_empty_args_is_noop() {
    let prior = project();
    let args = base_args();

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("empty update is no-op");

    assert_eq!(patch, json!([]));
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_NOOP_CODE);
    assert_eq!(data.keyframe.id.to_string(), KEYFRAME_ID);
}

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let prior = project();
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        prior,
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry succeeds");

    let mut args = base_args();
    args.value = Some(json!(0.8));
    let outcome = store
        .mutate_via_verb(
            "keyframe.set",
            serde_json::to_value(args).expect("args serialize"),
            None,
        )
        .expect("keyframe.set should route");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("happy path must apply");
    };
    assert!(warnings.is_empty());
    let envelope: KeyframeSetData = serde_json::from_value(data).expect("data parses");
    assert_eq!(envelope.keyframe_id.to_string(), KEYFRAME_ID);
    assert_eq!(envelope.keyframe.value, json!(0.8));
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "keyframe.set")
        .expect("default_fixtures includes keyframe.set");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(KeyframeSetVerb))
        .expect("register keyframe.set verb");

    let report = validate_reconstructors(&registry, &[fixture])
        .expect("keyframe.set reconstructor should pass");
    assert_eq!(report.verbs_checked, vec!["keyframe.set"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn round_trip_keyframe_set() {
    let prior = project();
    let mut args = base_args();
    args.value = Some(json!(0.8));
    let (patch_value, warnings, data) = compute_patch(&prior, &args).expect("happy path");
    let patch: json_patch::Patch =
        serde_json::from_value(patch_value.clone()).expect("patch parses");
    let post = prior.apply(&patch).expect("patch applies");
    let envelope = data_envelope_from_post_state(&args, &post).expect("envelope reconstructs");

    assert!(warnings.is_empty());
    assert_eq!(data, envelope);
    let post_keyframe = &post.tracks[0].clips[0].keyframes[0];
    assert_eq!(post_keyframe.id.to_string(), KEYFRAME_ID);
    assert_eq!(post_keyframe.value, json!(0.8));
}

#[test]
fn data_envelope_returns_post_state_keyframe() {
    let prior = project();
    let mut args = base_args();
    args.time_tk = Some(100);
    args.value = Some(json!(0.8));
    let (patch_value, _warnings, _data) = compute_patch(&prior, &args).expect("happy path");
    let patch: json_patch::Patch =
        serde_json::from_value(patch_value.clone()).expect("patch parses");
    let post = prior.apply(&patch).expect("patch applies");

    let envelope = data_envelope_from_post_state(&args, &post).expect("envelope");
    let expected_id: KeyframeId = KEYFRAME_ID.parse().expect("keyframe id parses");
    assert_eq!(envelope.keyframe_id, expected_id);
    assert_eq!(envelope.keyframe, post.tracks[0].clips[0].keyframes[0]);
}

#[test]
fn verb_compute_patch_maps_errors_to_bad_args() {
    let prior = project();
    let verb = KeyframeSetVerb;
    let mut args = base_args();
    args.value = Some(json!(2.0));
    let args = serde_json::to_value(args).expect("args serialize");

    let err = verb
        .compute_patch(&prior, &args)
        .expect_err("bad value maps");
    assert!(matches!(err, verbreel_state::VerbError::BadArgs { .. }));
}
