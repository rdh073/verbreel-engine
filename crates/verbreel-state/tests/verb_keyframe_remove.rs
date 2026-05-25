//! Tests for `keyframe.remove` (§8.2) — thirty-fifth production verb.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::keyframe_remove::{
    KEYFRAMES_MAX_BATCH, KeyframeRemoveArgs, KeyframeRemoveData, KeyframeRemoveError,
    KeyframeRemoveVerb, W_NOOP_CODE, compute_patch, data_envelope_from_args_warnings,
};
use verbreel_state::{
    MutateOutcome, Project, VerbRegistry, default_fixtures, default_registry,
    validate_reconstructors,
};
use verbreel_types::Tick;

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const TRACK_ID: &str = "01900000-0000-7000-8000-0000000aa901";
const CLIP_ID: &str = "01900000-0000-7000-8000-0000000bb901";
const SECOND_CLIP_ID: &str = "01900000-0000-7000-8000-0000000bb902";
const ASSET_ID: &str = "01900000-0000-7000-8000-0000000cc901";
const K1: &str = "01900000-0000-7000-8000-0000000ff901";
const K2: &str = "01900000-0000-7000-8000-0000000ff902";
const K3: &str = "01900000-0000-7000-8000-0000000ff903";
const K4: &str = "01900000-0000-7000-8000-0000000ff904";
const MISSING_LOW: &str = "01900000-0000-7000-8000-0000000ff001";
const MISSING_HIGH: &str = "01900000-0000-7000-8000-0000000ff999";

#[derive(Debug, Clone)]
struct ClipFixture {
    id: &'static str,
    position_tk: i64,
    locked: bool,
    keyframes: Vec<Value>,
}

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture -> Project")
}

fn fixture_project_id() -> verbreel_types::ProjectId {
    FIXTURE_PROJECT_ID
        .parse()
        .expect("fixture project id parses")
}

fn keyframe_value(id: &str, time_tk: i64, value: f64) -> Value {
    json!({
        "id": id,
        "property": "opacity",
        "time_tk": time_tk,
        "value": value,
        "easing": "linear",
    })
}

fn video_asset() -> Value {
    json!({
        "id": ASSET_ID,
        "kind": "video",
        "hash": "53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658",
        "path": "assets/53/53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.mp4",
        "original_filename": "keyframe-remove.mp4",
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

fn project_with_clips(track_locked: bool, clips: Vec<ClipFixture>) -> Project {
    let mut prior = empty_project();
    prior.tracks.clear();
    prior.assets.clear();
    prior
        .assets
        .push(serde_json::from_value(video_asset()).expect("asset fixture parses"));

    let duration_tk = clips
        .iter()
        .map(|clip| clip.position_tk + 480_000)
        .max()
        .unwrap_or(0);
    let clip_values: Vec<Value> = clips
        .iter()
        .map(|clip| {
            json!({
                "id": clip.id,
                "name": "Clip",
                "asset_id": ASSET_ID,
                "track_position_tk": clip.position_tk,
                "source_in_tk": 0,
                "source_out_tk": 480_000,
                "locked": clip.locked,
                "keyframes": clip.keyframes,
            })
        })
        .collect();

    let track = serde_json::from_value(json!({
        "id": TRACK_ID,
        "kind": "video",
        "name": "Video 1",
        "locked": track_locked,
        "clips": clip_values,
    }))
    .expect("track fixture parses");

    prior.tracks.push(track);
    prior.duration_tk = Tick::new(duration_tk);
    prior
}

fn project_with_keyframes(keyframes: Vec<Value>) -> Project {
    project_with_clips(
        false,
        vec![ClipFixture {
            id: CLIP_ID,
            position_tk: 0,
            locked: false,
            keyframes,
        }],
    )
}

fn project_with_three_keyframes() -> Project {
    project_with_keyframes(vec![
        keyframe_value(K1, 0, 1.0),
        keyframe_value(K2, 100, 0.5),
        keyframe_value(K3, 200, 0.0),
    ])
}

fn base_args(keyframes: Vec<&str>) -> KeyframeRemoveArgs {
    KeyframeRemoveArgs {
        project_id: fixture_project_id(),
        keyframes: keyframes.into_iter().map(ToString::to_string).collect(),
        soft: None,
    }
}

fn soft_args(keyframes: Vec<&str>) -> KeyframeRemoveArgs {
    KeyframeRemoveArgs {
        soft: Some(true),
        ..base_args(keyframes)
    }
}

fn patch_ops(patch: &Value) -> &[Value] {
    patch.as_array().expect("patch is array")
}

fn apply_patch(prior: &Project, patch: Value) -> Project {
    let patch: json_patch::Patch = serde_json::from_value(patch).expect("patch parses");
    prior.apply(&patch).expect("patch applies")
}

fn keyframe_ids(project: &Project, clip_idx: usize) -> Vec<String> {
    project.tracks[0].clips[clip_idx]
        .keyframes
        .iter()
        .map(|keyframe| keyframe.id.to_string())
        .collect()
}

#[test]
fn compute_patch_remove_single_keyframe() {
    let prior = project_with_three_keyframes();
    let args = base_args(vec![K2]);

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("remove one");

    assert!(warnings.is_empty());
    let ops = patch_ops(&patch);
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0]["op"], "remove");
    assert_eq!(ops[0]["path"], "/tracks/0/clips/0/keyframes/1");
    assert_eq!(data.removed_keyframe_ids[0].to_string(), K2);
    assert!(data.missing_keyframe_ids.is_empty());
}

#[test]
fn compute_patch_remove_multiple_keyframes_atomic() {
    let prior = project_with_three_keyframes();
    let args = base_args(vec![K1, K3]);

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("remove multiple");

    assert!(warnings.is_empty());
    let ops = patch_ops(&patch);
    assert_eq!(ops.len(), 2);
    assert_eq!(ops[0]["path"], "/tracks/0/clips/0/keyframes/2");
    assert_eq!(ops[1]["path"], "/tracks/0/clips/0/keyframes/0");
    assert_eq!(
        data.removed_keyframe_ids
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        vec![K1.to_string(), K3.to_string()]
    );
}

#[test]
fn compute_patch_remove_keyframes_from_different_clips_atomic() {
    let prior = project_with_clips(
        false,
        vec![
            ClipFixture {
                id: CLIP_ID,
                position_tk: 0,
                locked: false,
                keyframes: vec![keyframe_value(K1, 0, 1.0)],
            },
            ClipFixture {
                id: SECOND_CLIP_ID,
                position_tk: 480_000,
                locked: false,
                keyframes: vec![keyframe_value(K4, 0, 0.5)],
            },
        ],
    );
    let args = base_args(vec![K1, K4]);

    let (patch, warnings, _data) = compute_patch(&prior, &args).expect("remove across clips");

    assert!(warnings.is_empty());
    let ops = patch_ops(&patch);
    assert_eq!(ops.len(), 2);
    assert_eq!(ops[0]["path"], "/tracks/0/clips/1/keyframes/0");
    assert_eq!(ops[1]["path"], "/tracks/0/clips/0/keyframes/0");
    let post = apply_patch(&prior, patch);
    assert!(post.tracks[0].clips[0].keyframes.is_empty());
    assert!(post.tracks[0].clips[1].keyframes.is_empty());
}

#[test]
fn compute_patch_empty_keyframes_is_noop() {
    let prior = project_with_three_keyframes();
    let args = base_args(Vec::new());

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("empty no-op");

    assert_eq!(patch, json!([]));
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_NOOP_CODE);
    assert!(data.removed_keyframe_ids.is_empty());
    assert!(data.missing_keyframe_ids.is_empty());
}

#[test]
fn compute_patch_dedup_same_id_twice_removes_once() {
    let prior = project_with_three_keyframes();
    let args = base_args(vec![K1, K1]);

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("dedup remove");

    assert!(warnings.is_empty());
    let ops = patch_ops(&patch);
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0]["path"], "/tracks/0/clips/0/keyframes/0");
    assert_eq!(data.removed_keyframe_ids.len(), 1);
    assert_eq!(data.removed_keyframe_ids[0].to_string(), K1);
}

#[test]
fn compute_patch_soft_true_missing_id_reported_in_data() {
    let prior = project_with_three_keyframes();
    let args = soft_args(vec![K1, MISSING_HIGH]);

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("soft mixed");

    assert_eq!(patch_ops(&patch).len(), 1);
    assert_eq!(warnings[0]["details"]["keyframe_id"], MISSING_HIGH);
    assert_eq!(data.removed_keyframe_ids[0].to_string(), K1);
    assert_eq!(data.missing_keyframe_ids[0].to_string(), MISSING_HIGH);
}

#[test]
fn compute_patch_soft_true_all_missing_is_noop_with_data() {
    let prior = project_with_three_keyframes();
    let args = soft_args(vec![MISSING_HIGH]);

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("soft all missing");

    assert_eq!(patch, json!([]));
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_NOOP_CODE);
    assert!(data.removed_keyframe_ids.is_empty());
    assert_eq!(data.missing_keyframe_ids[0].to_string(), MISSING_HIGH);
}

#[test]
fn compute_patch_soft_false_missing_id_returns_not_found_with_failed_index() {
    let prior = project_with_three_keyframes();
    let args = base_args(vec![K1, MISSING_HIGH]);

    let err = compute_patch(&prior, &args).expect_err("strict missing");

    assert!(matches!(
        err,
        KeyframeRemoveError::NotFound {
            failed_index: 1,
            ref failed_target,
        } if failed_target == MISSING_HIGH
    ));
}

#[test]
fn compute_patch_soft_false_first_missing_wins_over_later_locked() {
    let prior = project_with_clips(
        false,
        vec![ClipFixture {
            id: CLIP_ID,
            position_tk: 0,
            locked: true,
            keyframes: vec![keyframe_value(K1, 0, 1.0)],
        }],
    );
    let args = base_args(vec![MISSING_HIGH, K1]);

    let err = compute_patch(&prior, &args).expect_err("missing wins");

    assert!(matches!(
        err,
        KeyframeRemoveError::NotFound {
            failed_index: 0,
            ref failed_target,
        } if failed_target == MISSING_HIGH
    ));
}

#[test]
fn compute_patch_locked_clip_returns_e_locked_with_failed_index() {
    let prior = project_with_clips(
        false,
        vec![ClipFixture {
            id: CLIP_ID,
            position_tk: 0,
            locked: true,
            keyframes: vec![keyframe_value(K1, 0, 1.0)],
        }],
    );
    let args = base_args(vec![K1]);

    let err = compute_patch(&prior, &args).expect_err("locked clip");

    assert!(matches!(
        err,
        KeyframeRemoveError::Locked {
            failed_index: 0,
            kind: "clip",
            ref failed_target,
            ..
        } if failed_target == K1
    ));
}

#[test]
fn compute_patch_locked_track_returns_e_locked_with_failed_index() {
    let prior = project_with_clips(
        true,
        vec![ClipFixture {
            id: CLIP_ID,
            position_tk: 0,
            locked: false,
            keyframes: vec![keyframe_value(K1, 0, 1.0)],
        }],
    );
    let args = base_args(vec![K1]);

    let err = compute_patch(&prior, &args).expect_err("locked track");

    assert!(matches!(
        err,
        KeyframeRemoveError::Locked {
            failed_index: 0,
            kind: "track",
            ref failed_target,
            ..
        } if failed_target == K1
    ));
}

#[test]
fn compute_patch_locked_first_within_batch_wins() {
    let prior = project_with_clips(
        false,
        vec![
            ClipFixture {
                id: CLIP_ID,
                position_tk: 0,
                locked: true,
                keyframes: vec![keyframe_value(K1, 0, 1.0)],
            },
            ClipFixture {
                id: SECOND_CLIP_ID,
                position_tk: 480_000,
                locked: true,
                keyframes: vec![keyframe_value(K4, 0, 0.5)],
            },
        ],
    );
    let args = base_args(vec![K4, K1]);

    let err = compute_patch(&prior, &args).expect_err("first locked wins");

    assert!(matches!(
        err,
        KeyframeRemoveError::Locked {
            failed_index: 0,
            ref failed_target,
            ..
        } if failed_target == K4
    ));
}

#[test]
fn compute_patch_keyframes_above_max_items_is_schema_violation() {
    let prior = project_with_three_keyframes();
    let args = KeyframeRemoveArgs {
        project_id: fixture_project_id(),
        keyframes: vec!["not-a-uuid".to_string(); KEYFRAMES_MAX_BATCH + 1],
        soft: None,
    };

    let err = compute_patch(&prior, &args).expect_err("batch too large");

    assert!(matches!(
        err,
        KeyframeRemoveError::SchemaViolation {
            field: "keyframes",
            hint: "split the batch into smaller calls",
            actual,
            max,
        } if actual == KEYFRAMES_MAX_BATCH + 1 && max == KEYFRAMES_MAX_BATCH
    ));
}

#[test]
fn compute_patch_bad_selector_malformed_id_with_failed_index() {
    let prior = project_with_three_keyframes();
    let args = base_args(vec![K1, "not-a-uuid"]);

    let err = compute_patch(&prior, &args).expect_err("bad selector");

    assert!(matches!(
        err,
        KeyframeRemoveError::BadSelector {
            failed_index: 1,
            ref failed_target,
            ..
        } if failed_target == "not-a-uuid"
    ));
}

#[test]
fn compute_patch_reverse_sorted_indices_apply_without_shift() {
    let prior = project_with_three_keyframes();
    let args = base_args(vec![K1, K3]);

    let (patch, warnings, _data) = compute_patch(&prior, &args).expect("remove endpoints");

    assert!(warnings.is_empty());
    let ops = patch_ops(&patch);
    assert_eq!(ops[0]["path"], "/tracks/0/clips/0/keyframes/2");
    assert_eq!(ops[1]["path"], "/tracks/0/clips/0/keyframes/0");
    let post = apply_patch(&prior, patch);
    assert_eq!(keyframe_ids(&post, 0), vec![K2.to_string()]);
}

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let prior = project_with_three_keyframes();
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        prior,
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry succeeds");

    let outcome = store
        .mutate_via_verb(
            "keyframe.remove",
            serde_json::to_value(base_args(vec![K2])).expect("args serialize"),
            None,
        )
        .expect("keyframe.remove should route");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("happy path must apply");
    };
    assert!(warnings.is_empty());
    let envelope: KeyframeRemoveData = serde_json::from_value(data).expect("data parses");
    assert_eq!(envelope.removed_keyframe_ids[0].to_string(), K2);
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "keyframe.remove")
        .expect("default_fixtures includes keyframe.remove");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(KeyframeRemoveVerb))
        .expect("register keyframe.remove verb");

    let report = validate_reconstructors(&registry, &[fixture])
        .expect("keyframe.remove reconstructor should pass");
    assert_eq!(report.verbs_checked, vec!["keyframe.remove"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn round_trip_keyframe_remove() {
    let prior = project_with_three_keyframes();
    let args = base_args(vec![K2]);
    let (patch_value, warnings, data) = compute_patch(&prior, &args).expect("happy path");
    let post = apply_patch(&prior, patch_value);
    let envelope =
        data_envelope_from_args_warnings(&args, &warnings).expect("envelope reconstructs");

    assert!(warnings.is_empty());
    assert_eq!(data, envelope);
    assert_eq!(keyframe_ids(&post, 0), vec![K1.to_string(), K3.to_string()]);
}

#[test]
fn data_envelope_returns_sorted_removed_and_missing_ids() {
    let prior = project_with_three_keyframes();
    let args = soft_args(vec![K3, MISSING_HIGH, K1, MISSING_LOW]);

    let (_patch, warnings, data) = compute_patch(&prior, &args).expect("mixed soft");
    let envelope =
        data_envelope_from_args_warnings(&args, &warnings).expect("envelope reconstructs");

    assert_eq!(data, envelope);
    assert_eq!(
        data.removed_keyframe_ids
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        vec![K1.to_string(), K3.to_string()]
    );
    assert_eq!(
        data.missing_keyframe_ids
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        vec![MISSING_LOW.to_string(), MISSING_HIGH.to_string()]
    );
}
