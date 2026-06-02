//! Tests for `keyframe.add` (§8.1) — thirty-third production verb.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::keyframe_add::{
    KeyframeAddArgs, KeyframeAddData, KeyframeAddError, KeyframeAddVerb, compute_patch,
    data_envelope_from_args_and_patch,
};
use verbreel_state::{
    Easing, Keyframe, MutateOutcome, Project, RecordedEvent, Verb, VerbError, VerbRegistry,
    default_fixtures, default_registry, validate_reconstructors,
};
use verbreel_types::{ClipId, KeyframeId, Tick};

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const TRACK_ID: &str = "01900000-0000-7000-8000-0000000aa901";
const CLIP_ID: &str = "01900000-0000-7000-8000-0000000bb901";
const ASSET_ID: &str = "01900000-0000-7000-8000-0000000cc901";
const EFFECT_ID: &str = "01900000-0000-7000-8000-0000000dd901";
const MISSING_EFFECT_ID: &str = "01900000-0000-7000-8000-0000000dd902";
const MISSING_CLIP_ID: &str = "01900000-0000-7000-8000-0000000ee901";
const EXISTING_KEYFRAME_ID: &str = "01900000-0000-7000-8000-0000000ff901";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture -> Project")
}

fn fixture_project_id() -> verbreel_types::ProjectId {
    FIXTURE_PROJECT_ID
        .parse()
        .expect("fixture project id parses")
}

fn base_args(property: &str, value: Value) -> KeyframeAddArgs {
    KeyframeAddArgs {
        project_id: fixture_project_id(),
        clip: CLIP_ID.to_string(),
        property: property.to_string(),
        time_tk: 0,
        value,
        easing: Some("linear".to_string()),
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
        "original_filename": "keyframe-add.mp4",
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
    project_with_clip(serde_json::Map::new())
}

fn project_with_existing_keyframe() -> Project {
    let mut extra = serde_json::Map::new();
    extra.insert(
        "keyframes".to_string(),
        json!([{
            "id": EXISTING_KEYFRAME_ID,
            "property": "opacity",
            "time_tk": 0,
            "value": 0.75,
        }]),
    );
    project_with_clip(extra)
}

fn project_with_effect() -> Project {
    let mut extra = serde_json::Map::new();
    extra.insert(
        "effects".to_string(),
        json!([{
            "id": EFFECT_ID,
            "kind": "blur",
            "enabled": true,
            "params": { "radius": 5.0 },
        }]),
    );
    project_with_clip(extra)
}

fn project_with_mask(mask: Value) -> Project {
    let mut extra = serde_json::Map::new();
    extra.insert("mask".to_string(), mask);
    project_with_clip(extra)
}

fn patch_value(patch: &Value) -> &Value {
    let ops = patch.as_array().expect("patch is array");
    assert_eq!(ops.len(), 1);
    let op = ops[0].as_object().expect("op is object");
    assert_eq!(op.get("op").and_then(Value::as_str), Some("add"));
    assert_eq!(
        op.get("path").and_then(Value::as_str),
        Some("/tracks/0/clips/0/keyframes/-")
    );
    op.get("value").expect("op has value")
}

fn patch_keyframe(patch: &Value) -> Keyframe {
    serde_json::from_value(patch_value(patch).clone()).expect("patch value parses as keyframe")
}

#[test]
fn compute_patch_happy_opacity_keyframe_appends() {
    let prior = project();
    let args = base_args("opacity", json!(0.5));
    let (patch, warnings, data) = compute_patch(&prior, &args).expect("happy path");

    assert!(warnings.is_empty());
    assert_eq!(data.clip_id.to_string(), CLIP_ID);
    let value = patch_value(&patch);
    assert_eq!(value["property"], "opacity");
    assert_eq!(value["time_tk"], 0);
    assert_eq!(value["value"], 0.5);
    assert_eq!(value["easing"], "linear");
}

#[test]
fn compute_patch_happy_transform_x_keyframe() {
    let prior = project();
    let args = base_args("transform.x", json!(42.0));
    let (patch, warnings, _data) = compute_patch(&prior, &args).expect("happy path");

    assert!(warnings.is_empty());
    assert_eq!(patch_value(&patch)["property"], "transform.x");
    assert_eq!(patch_value(&patch)["value"], 42.0);
}

#[test]
fn compute_patch_cubic_bezier_with_bezier_array_succeeds() {
    let prior = project();
    let mut args = base_args("opacity", json!(0.5));
    args.easing = Some("cubic-bezier".to_string());
    args.bezier = Some([0.1, 0.2, 0.3, 0.4]);

    let (patch, warnings, _data) = compute_patch(&prior, &args).expect("happy path");

    assert!(warnings.is_empty());
    assert_eq!(patch_value(&patch)["easing"], "cubic-bezier");
    assert_eq!(patch_value(&patch)["bezier"], json!([0.1, 0.2, 0.3, 0.4]));
}

#[test]
fn compute_patch_cubic_bezier_without_bezier_is_schema_violation() {
    let prior = project();
    let mut args = base_args("opacity", json!(0.5));
    args.easing = Some("cubic-bezier".to_string());

    let err = compute_patch(&prior, &args).expect_err("missing bezier rejects");
    assert!(matches!(err, KeyframeAddError::SchemaViolation { .. }));
}

#[test]
fn compute_patch_unknown_easing_is_schema_violation() {
    let prior = project();
    let mut args = base_args("opacity", json!(0.5));
    args.easing = Some("snappy".to_string());

    let err = compute_patch(&prior, &args).expect_err("unknown easing rejects");
    assert!(matches!(err, KeyframeAddError::SchemaViolation { .. }));
}

#[test]
fn compute_patch_property_regex_invalid_is_bad_property() {
    let prior = project();
    let args = base_args("not.a.real.path", json!(1.0));

    let err = compute_patch(&prior, &args).expect_err("bad property rejects");
    assert!(matches!(err, KeyframeAddError::BadProperty { .. }));
}

#[test]
fn compute_patch_opacity_above_one_is_bad_value() {
    let err = compute_patch(&project(), &base_args("opacity", json!(1.01)))
        .expect_err("opacity above 1 rejects");
    assert!(matches!(err, KeyframeAddError::BadValue { .. }));
}

#[test]
fn compute_patch_opacity_below_zero_is_bad_value() {
    let err = compute_patch(&project(), &base_args("opacity", json!(-0.01)))
        .expect_err("opacity below 0 rejects");
    assert!(matches!(err, KeyframeAddError::BadValue { .. }));
}

#[test]
fn compute_patch_volume_above_four_is_bad_value() {
    let err = compute_patch(&project(), &base_args("volume", json!(4.01)))
        .expect_err("volume above 4 rejects");
    assert!(matches!(err, KeyframeAddError::BadValue { .. }));
}

#[test]
fn compute_patch_nan_value_is_bad_value() {
    let err = compute_patch(&project(), &base_args("opacity", non_finite_json(f64::NAN)))
        .expect_err("nan rejects");
    assert!(matches!(err, KeyframeAddError::BadValue { .. }));
}

#[test]
fn compute_patch_infinity_value_is_bad_value() {
    let err = compute_patch(
        &project(),
        &base_args("opacity", non_finite_json(f64::INFINITY)),
    )
    .expect_err("infinity rejects");
    assert!(matches!(err, KeyframeAddError::BadValue { .. }));
}

#[test]
fn compute_patch_negative_time_is_bad_time() {
    let prior = project();
    let mut args = base_args("opacity", json!(0.5));
    args.time_tk = -1;

    let err = compute_patch(&prior, &args).expect_err("negative time rejects");
    assert!(matches!(err, KeyframeAddError::BadTime { time_tk: -1, .. }));
}

#[test]
fn compute_patch_time_beyond_clip_duration_is_bad_time() {
    let prior = project();
    let mut args = base_args("opacity", json!(0.5));
    args.time_tk = 480_001;

    let err = compute_patch(&prior, &args).expect_err("time beyond duration rejects");
    assert!(matches!(
        err,
        KeyframeAddError::BadTime {
            time_tk: 480_001,
            clip_duration_tk: 480_000
        }
    ));
}

#[test]
fn compute_patch_duplicate_property_time_triple_is_duplicate() {
    let prior = project_with_existing_keyframe();
    let err = compute_patch(&prior, &base_args("opacity", json!(0.5)))
        .expect_err("duplicate property/time rejects");

    match err {
        KeyframeAddError::Duplicate {
            existing_keyframe_id,
        } => assert_eq!(existing_keyframe_id.to_string(), EXISTING_KEYFRAME_ID),
        other => panic!("expected Duplicate, got {other:?}"),
    }
}

#[test]
fn compute_patch_effects_uuid_not_on_clip_is_bad_property() {
    let prior = project_with_effect();
    let args = base_args(
        &format!("effects[{MISSING_EFFECT_ID}].params.radius"),
        json!(5.0),
    );

    let err = compute_patch(&prior, &args).expect_err("missing effect rejects");
    assert!(matches!(err, KeyframeAddError::BadProperty { .. }));
}

#[test]
fn compute_patch_relight_param_keyframe_appends() {
    // relight params are keyframable the moment the kind is registered:
    // keyframe.add accepts effects[<uuid>].params.<leaf> generically with
    // per-kind leaf validation deferred (keyframe_add.rs §0.13).
    let mut extra = serde_json::Map::new();
    extra.insert(
        "effects".to_string(),
        json!([{
            "id": EFFECT_ID,
            "kind": "relight",
            "enabled": true,
            "params": { "intensity": 0.0 },
        }]),
    );
    let prior = project_with_clip(extra);
    let args = base_args(
        &format!("effects[{EFFECT_ID}].params.intensity"),
        json!(0.75),
    );

    let (patch, warnings, _data) =
        compute_patch(&prior, &args).expect("relight param keyframe accepted");

    assert!(warnings.is_empty());
    let value = patch_value(&patch);
    assert_eq!(
        value["property"],
        format!("effects[{EFFECT_ID}].params.intensity")
    );
    assert_eq!(value["value"], 0.75);
}

#[test]
fn compute_patch_matte_refine_param_keyframe_appends() {
    // matte_refine params are keyframable the moment the kind is registered:
    // keyframe.add accepts effects[<uuid>].params.<leaf> generically with
    // per-kind leaf validation deferred (keyframe_add.rs §0.13).
    let mut extra = serde_json::Map::new();
    extra.insert(
        "effects".to_string(),
        json!([{
            "id": EFFECT_ID,
            "kind": "matte_refine",
            "enabled": true,
            "params": { "feather_px": 0.0 },
        }]),
    );
    let prior = project_with_clip(extra);
    let args = base_args(
        &format!("effects[{EFFECT_ID}].params.feather_px"),
        json!(2.0),
    );

    let (patch, warnings, _data) =
        compute_patch(&prior, &args).expect("matte_refine param keyframe accepted");

    assert!(warnings.is_empty());
    let value = patch_value(&patch);
    assert_eq!(
        value["property"],
        format!("effects[{EFFECT_ID}].params.feather_px")
    );
    assert_eq!(value["value"], 2.0);
}

#[test]
fn compute_patch_mask_property_when_clip_has_no_mask_is_bad_property() {
    let prior = project();
    let args = base_args("mask.feather_px", json!(1.0));

    let err = compute_patch(&prior, &args).expect_err("missing mask rejects");
    assert!(matches!(err, KeyframeAddError::BadProperty { .. }));
}

#[test]
fn compute_patch_clip_not_found() {
    let prior = project();
    let mut args = base_args("opacity", json!(0.5));
    args.clip = MISSING_CLIP_ID.to_string();

    let err = compute_patch(&prior, &args).expect_err("missing clip rejects");
    assert!(matches!(err, KeyframeAddError::ClipNotFound { .. }));
}

#[test]
fn compute_patch_bad_selector() {
    let prior = project();
    let mut args = base_args("opacity", json!(0.5));
    args.clip = "not-a-uuid".to_string();

    let err = compute_patch(&prior, &args).expect_err("bad selector rejects");
    assert!(matches!(err, KeyframeAddError::BadSelector { .. }));
}

#[test]
fn compute_patch_locked_clip_rejects_with_e_locked() {
    let mut extra = serde_json::Map::new();
    extra.insert("locked".to_string(), json!(true));
    let prior = project_with_clip(extra);

    let err =
        compute_patch(&prior, &base_args("opacity", json!(0.5))).expect_err("locked clip rejects");
    assert!(matches!(err, KeyframeAddError::Locked { kind: "clip", .. }));
}

#[test]
fn compute_patch_locked_track_rejects_with_e_locked() {
    let mut prior = project();
    prior.tracks[0].locked = true;

    let err =
        compute_patch(&prior, &base_args("opacity", json!(0.5))).expect_err("locked track rejects");
    assert!(matches!(
        err,
        KeyframeAddError::Locked { kind: "track", .. }
    ));
}

#[test]
fn compute_patch_locked_precedes_bad_value() {
    let mut extra = serde_json::Map::new();
    extra.insert("locked".to_string(), json!(true));
    let prior = project_with_clip(extra);

    let err = compute_patch(&prior, &base_args("opacity", json!(2.0)))
        .expect_err("locked precedes value validation");
    assert!(matches!(err, KeyframeAddError::Locked { kind: "clip", .. }));
}

#[test]
fn compute_patch_mask_rect_leaf_succeeds() {
    let prior = project_with_mask(json!({
        "kind": "rect",
        "params": { "x": 0.0, "y": 0.0, "w": 100.0, "h": 100.0 },
        "feather_px": 0.0,
    }));
    let args = base_args("mask.params.x", json!(12.0));

    let (patch, _warnings, _data) = compute_patch(&prior, &args).expect("rect mask path");
    assert_eq!(patch_value(&patch)["property"], "mask.params.x");
}

#[test]
fn compute_patch_verb_error_mapping() {
    let prior = project();
    let verb = KeyframeAddVerb;
    let args = serde_json::to_value(base_args("opacity", json!(2.0))).expect("args serialize");

    let err = verb
        .compute_patch(&prior, &args)
        .expect_err("bad value maps");
    assert!(matches!(err, VerbError::BadArgs { .. }));
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

    let outcome = store
        .mutate_via_verb(
            "keyframe.add",
            serde_json::to_value(base_args("opacity", json!(0.5))).expect("args serialize"),
            None,
        )
        .expect("keyframe.add should route");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("happy path must apply");
    };
    assert!(warnings.is_empty());
    let envelope: KeyframeAddData = serde_json::from_value(data).expect("data parses");
    assert_eq!(envelope.clip_id.to_string(), CLIP_ID);
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "keyframe.add")
        .expect("default_fixtures includes keyframe.add");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(KeyframeAddVerb))
        .expect("register keyframe.add verb");

    let report = validate_reconstructors(&registry, &[fixture])
        .expect("keyframe.add reconstructor should pass");
    assert_eq!(report.verbs_checked, vec!["keyframe.add"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn round_trip_keyframe_add() {
    let prior = project();
    let args = base_args("opacity", json!(0.5));
    let (patch_value, warnings, data) = compute_patch(&prior, &args).expect("happy path");
    let patch: json_patch::Patch =
        serde_json::from_value(patch_value.clone()).expect("patch parses");
    let post = prior.apply(&patch).expect("patch applies");
    let envelope =
        data_envelope_from_args_and_patch(&args, &patch_value).expect("envelope reconstructs");

    assert!(warnings.is_empty());
    assert_eq!(data, envelope);
    let post_keyframe = post.tracks[0].clips[0]
        .keyframes
        .first()
        .expect("post-state has keyframe");
    assert_eq!(post_keyframe.id, data.keyframe_id);
    assert_eq!(post_keyframe.property.as_str(), "opacity");
    assert_eq!(post_keyframe.time_tk, Tick::new(0));
    assert_eq!(post_keyframe.value, json!(0.5));
    assert_eq!(post_keyframe.easing, Easing::Linear);
}

#[test]
fn data_envelope_returns_minted_keyframe_id_and_clip_id() {
    let prior = project();
    let args = base_args("opacity", json!(0.5));
    let (patch, _warnings, data) = compute_patch(&prior, &args).expect("happy path");
    let keyframe_id_from_patch: KeyframeId = patch_keyframe(&patch).id;
    let clip_id: ClipId = CLIP_ID.parse().expect("clip id parses");

    assert_eq!(data.keyframe_id, keyframe_id_from_patch);
    assert_eq!(data.clip_id, clip_id);
}

#[test]
fn recorded_event_round_trip_keyframe_add() {
    let prior = project();
    let args = base_args("transform.x", json!(10.0));
    let (patch_value, _warnings, data) = compute_patch(&prior, &args).expect("happy path");
    let patch: json_patch::Patch =
        serde_json::from_value(patch_value.clone()).expect("patch parses");
    let post_state = prior.apply(&patch).expect("patch applies");
    let event = RecordedEvent {
        verb: "keyframe.add".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings: vec![],
        post_state,
        expected_data: serde_json::to_value(data).expect("data serializes"),
    };

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(KeyframeAddVerb))
        .expect("register keyframe.add");
    validate_reconstructors(&registry, &[event]).expect("recorded event validates");
}
