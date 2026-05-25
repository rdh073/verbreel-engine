//! Tests for `clip.delete` (§5.5) — thirty-ninth production verb.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::clip_delete::{
    CLIPS_MAX_BATCH, ClipDeleteArgs, ClipDeleteData, ClipDeleteError, ClipDeleteVerb,
    W_CLIP_DELETE_ENVELOPE_CODE, W_LINK_GROUP_CLEARED_ON_LOCKED_CODE, compute_patch,
    data_envelope_from_args_warnings,
};
use verbreel_state::{
    MutateOutcome, Project, TrackKind, VerbRegistry, default_fixtures, default_registry,
    validate_reconstructors,
};
use verbreel_types::Tick;

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";

const VIDEO_TRACK: &str = "01900000-0000-7000-8000-0000000aa501";
const VIDEO_TRACK_2: &str = "01900000-0000-7000-8000-0000000aa502";
const AUDIO_TRACK: &str = "01900000-0000-7000-8000-0000000aa503";

const CLIP_1: &str = "01900000-0000-7000-8000-0000000bb501";
const CLIP_2: &str = "01900000-0000-7000-8000-0000000bb502";
const CLIP_3: &str = "01900000-0000-7000-8000-0000000bb503";
const CLIP_4: &str = "01900000-0000-7000-8000-0000000bb504";

const VIDEO_ASSET: &str = "01900000-0000-7000-8000-0000000cc501";
const AUDIO_ASSET: &str = "01900000-0000-7000-8000-0000000cc502";

const LINK_GROUP: &str = "01900000-0000-7000-8000-0000000dd501";
const OTHER_LINK_GROUP: &str = "01900000-0000-7000-8000-0000000dd502";
const MISSING_LOW: &str = "01900000-0000-7000-8000-0000000bb001";
const MISSING_HIGH: &str = "01900000-0000-7000-8000-0000000bb999";

#[derive(Debug, Clone)]
struct ClipFixture {
    id: &'static str,
    position_tk: i64,
    locked: bool,
    link_group: Option<&'static str>,
}

#[derive(Debug, Clone)]
struct TrackFixture {
    id: &'static str,
    kind: TrackKind,
    locked: bool,
    clips: Vec<ClipFixture>,
}

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture -> Project")
}

fn fixture_project_id() -> verbreel_types::ProjectId {
    FIXTURE_PROJECT_ID
        .parse()
        .expect("fixture project id parses")
}

fn video_asset() -> Value {
    json!({
        "id": VIDEO_ASSET,
        "kind": "video",
        "hash": "53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658",
        "path": "assets/53/53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.mp4",
        "original_filename": "clip-delete.mp4",
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

fn audio_asset() -> Value {
    json!({
        "id": AUDIO_ASSET,
        "kind": "audio",
        "hash": "36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da",
        "path": "assets/36/36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da.m4a",
        "original_filename": "clip-delete.m4a",
        "imported_at": "2026-05-24T00:00:00Z",
        "metadata": {
            "duration_tk": 480_000,
            "audio_codec": "aac",
            "audio_channels": 2,
            "audio_sample_rate_hz": 48000,
            "container": "m4a",
            "fingerprint": {
                "mtime_ms": 1_700_000_000_000_i64,
                "size_bytes": 1024,
            }
        }
    })
}

fn clip_value(kind: TrackKind, fixture: &ClipFixture) -> Value {
    let asset_id = if kind == TrackKind::Audio {
        AUDIO_ASSET
    } else {
        VIDEO_ASSET
    };
    let mut clip = json!({
        "id": fixture.id,
        "name": "Clip",
        "asset_id": asset_id,
        "track_position_tk": fixture.position_tk,
        "source_in_tk": 0,
        "source_out_tk": 480_000,
        "locked": fixture.locked,
    });
    if kind == TrackKind::Audio {
        clip["volume"] = json!(1.0);
    }
    if let Some(link_group) = fixture.link_group {
        clip["link_group"] = json!(link_group);
    }
    clip
}

fn project_with_tracks(tracks: Vec<TrackFixture>) -> Project {
    let mut project = empty_project();
    let duration_tk = tracks
        .iter()
        .flat_map(|track| track.clips.iter())
        .map(|clip| clip.position_tk + 480_000)
        .max()
        .unwrap_or(0);
    project.tracks = tracks
        .into_iter()
        .map(|track| {
            let clip_values = track
                .clips
                .iter()
                .map(|clip| clip_value(track.kind, clip))
                .collect::<Vec<_>>();
            serde_json::from_value(json!({
                "id": track.id,
                "kind": track.kind,
                "name": "Track",
                "locked": track.locked,
                "clips": clip_values,
            }))
            .expect("track fixture parses")
        })
        .collect();
    project.assets.clear();
    project
        .assets
        .push(serde_json::from_value(video_asset()).expect("video asset parses"));
    project
        .assets
        .push(serde_json::from_value(audio_asset()).expect("audio asset parses"));
    project.duration_tk = Tick::new(duration_tk);
    project
}

fn single_track_project(clips: Vec<ClipFixture>) -> Project {
    project_with_tracks(vec![TrackFixture {
        id: VIDEO_TRACK,
        kind: TrackKind::Video,
        locked: false,
        clips,
    }])
}

fn unlocked_clip(id: &'static str, position_tk: i64) -> ClipFixture {
    ClipFixture {
        id,
        position_tk,
        locked: false,
        link_group: None,
    }
}

fn locked_clip(id: &'static str, position_tk: i64) -> ClipFixture {
    ClipFixture {
        id,
        position_tk,
        locked: true,
        link_group: None,
    }
}

fn linked_clip(id: &'static str, position_tk: i64, link_group: &'static str) -> ClipFixture {
    ClipFixture {
        id,
        position_tk,
        locked: false,
        link_group: Some(link_group),
    }
}

fn linked_locked_clip(id: &'static str, position_tk: i64, link_group: &'static str) -> ClipFixture {
    ClipFixture {
        id,
        position_tk,
        locked: true,
        link_group: Some(link_group),
    }
}

fn base_args(clips: Vec<&str>) -> ClipDeleteArgs {
    ClipDeleteArgs {
        project_id: fixture_project_id(),
        clips: clips.into_iter().map(ToString::to_string).collect(),
        soft: None,
        ripple: None,
        ripple_scope: None,
    }
}

fn soft_args(clips: Vec<&str>) -> ClipDeleteArgs {
    ClipDeleteArgs {
        soft: Some(true),
        ..base_args(clips)
    }
}

fn patch_ops(patch: &Value) -> &[Value] {
    patch.as_array().expect("patch is array")
}

fn apply_patch(prior: &Project, patch: Value) -> Project {
    let patch: json_patch::Patch = serde_json::from_value(patch).expect("patch parses");
    prior.apply(&patch).expect("patch applies")
}

fn clip_ids(project: &Project, track_idx: usize) -> Vec<String> {
    project.tracks[track_idx]
        .clips
        .iter()
        .map(|clip| clip.id.to_string())
        .collect()
}

fn two_track_linked_project(survivor_locked: bool) -> Project {
    let survivor = if survivor_locked {
        linked_locked_clip(CLIP_2, 0, LINK_GROUP)
    } else {
        linked_clip(CLIP_2, 0, LINK_GROUP)
    };
    project_with_tracks(vec![
        TrackFixture {
            id: VIDEO_TRACK,
            kind: TrackKind::Video,
            locked: false,
            clips: vec![linked_clip(CLIP_1, 0, LINK_GROUP)],
        },
        TrackFixture {
            id: AUDIO_TRACK,
            kind: TrackKind::Audio,
            locked: false,
            clips: vec![survivor],
        },
    ])
}

#[test]
fn compute_patch_remove_single_clip() {
    let prior = single_track_project(vec![unlocked_clip(CLIP_1, 0)]);
    let args = base_args(vec![CLIP_1]);

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("remove one");

    assert_eq!(warnings[0]["code"], W_CLIP_DELETE_ENVELOPE_CODE);
    let ops = patch_ops(&patch);
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0]["op"], "remove");
    assert_eq!(ops[0]["path"], "/tracks/0/clips/0");
    assert_eq!(data.removed_clip_ids[0].to_string(), CLIP_1);
    assert!(data.missing_clip_ids.is_empty());
    assert!(data.cleared_link_group_clip_ids.is_empty());
}

#[test]
fn compute_patch_remove_multiple_clips_atomic() {
    let prior = single_track_project(vec![
        unlocked_clip(CLIP_1, 0),
        unlocked_clip(CLIP_2, 480_000),
        unlocked_clip(CLIP_3, 960_000),
    ]);
    let args = base_args(vec![CLIP_1, CLIP_2]);

    let (patch, _warnings, data) = compute_patch(&prior, &args).expect("remove multiple");

    let ops = patch_ops(&patch);
    assert_eq!(ops[0]["path"], "/tracks/0/clips/1");
    assert_eq!(ops[1]["path"], "/tracks/0/clips/0");
    let post = apply_patch(&prior, patch);
    assert_eq!(clip_ids(&post, 0), vec![CLIP_3.to_string()]);
    assert_eq!(
        data.removed_clip_ids
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        vec![CLIP_1.to_string(), CLIP_2.to_string()]
    );
}

#[test]
fn compute_patch_remove_clips_from_different_tracks_atomic() {
    let prior = project_with_tracks(vec![
        TrackFixture {
            id: VIDEO_TRACK,
            kind: TrackKind::Video,
            locked: false,
            clips: vec![unlocked_clip(CLIP_1, 0), unlocked_clip(CLIP_3, 480_000)],
        },
        TrackFixture {
            id: VIDEO_TRACK_2,
            kind: TrackKind::Video,
            locked: false,
            clips: vec![unlocked_clip(CLIP_2, 0)],
        },
    ]);
    let args = base_args(vec![CLIP_1, CLIP_2]);

    let (patch, _warnings, _data) = compute_patch(&prior, &args).expect("remove across tracks");

    let ops = patch_ops(&patch);
    assert_eq!(ops.len(), 2);
    assert_eq!(ops[0]["path"], "/tracks/1/clips/0");
    assert_eq!(ops[1]["path"], "/tracks/0/clips/0");
    let post = apply_patch(&prior, patch);
    assert_eq!(clip_ids(&post, 0), vec![CLIP_3.to_string()]);
    assert!(post.tracks[1].clips.is_empty());
}

#[test]
fn compute_patch_empty_clips_is_noop_with_envelope_warning() {
    let prior = single_track_project(vec![unlocked_clip(CLIP_1, 0)]);
    let args = base_args(Vec::new());

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("empty no-op");

    assert_eq!(patch, json!([]));
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_CLIP_DELETE_ENVELOPE_CODE);
    assert!(data.removed_clip_ids.is_empty());
    assert!(data.missing_clip_ids.is_empty());
    assert!(data.cleared_link_group_clip_ids.is_empty());
}

#[test]
fn compute_patch_dedup_same_id_twice_removes_once() {
    let prior = single_track_project(vec![unlocked_clip(CLIP_1, 0)]);
    let args = base_args(vec![CLIP_1, CLIP_1]);

    let (patch, _warnings, data) = compute_patch(&prior, &args).expect("dedup remove");

    assert_eq!(patch_ops(&patch).len(), 1);
    assert_eq!(data.removed_clip_ids.len(), 1);
    assert_eq!(data.removed_clip_ids[0].to_string(), CLIP_1);
}

#[test]
fn compute_patch_soft_true_missing_id_reported_in_envelope() {
    let prior = single_track_project(vec![unlocked_clip(CLIP_1, 0)]);
    let args = soft_args(vec![CLIP_1, MISSING_HIGH]);

    let (_patch, warnings, data) = compute_patch(&prior, &args).expect("soft mixed");

    assert_eq!(warnings.len(), 1);
    assert_eq!(
        warnings[0]["details"]["missing_clip_ids"],
        json!([MISSING_HIGH])
    );
    assert_eq!(data.removed_clip_ids[0].to_string(), CLIP_1);
    assert_eq!(data.missing_clip_ids[0].to_string(), MISSING_HIGH);
}

#[test]
fn compute_patch_soft_true_all_missing_is_noop_with_envelope() {
    let prior = single_track_project(vec![unlocked_clip(CLIP_1, 0)]);
    let args = soft_args(vec![MISSING_HIGH]);

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("soft all missing");

    assert_eq!(patch, json!([]));
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_CLIP_DELETE_ENVELOPE_CODE);
    assert!(data.removed_clip_ids.is_empty());
    assert_eq!(data.missing_clip_ids[0].to_string(), MISSING_HIGH);
}

#[test]
fn compute_patch_soft_false_missing_id_returns_not_found_with_failed_index() {
    let prior = single_track_project(vec![unlocked_clip(CLIP_1, 0)]);
    let args = base_args(vec![CLIP_1, MISSING_HIGH]);

    let err = compute_patch(&prior, &args).expect_err("strict missing");

    assert!(matches!(
        err,
        ClipDeleteError::NotFound {
            failed_index: 1,
            ref failed_target,
        } if failed_target == MISSING_HIGH
    ));
}

#[test]
fn compute_patch_soft_false_first_missing_wins_over_later_locked() {
    let prior = single_track_project(vec![locked_clip(CLIP_1, 0)]);
    let args = base_args(vec![MISSING_HIGH, CLIP_1]);

    let err = compute_patch(&prior, &args).expect_err("missing wins");

    assert!(matches!(
        err,
        ClipDeleteError::NotFound {
            failed_index: 0,
            ref failed_target,
        } if failed_target == MISSING_HIGH
    ));
}

#[test]
fn compute_patch_locked_clip_returns_e_locked_with_failed_index() {
    let prior = single_track_project(vec![locked_clip(CLIP_1, 0)]);
    let args = base_args(vec![CLIP_1]);

    let err = compute_patch(&prior, &args).expect_err("locked clip");

    assert!(matches!(
        err,
        ClipDeleteError::Locked {
            failed_index: 0,
            kind: "clip",
            ref failed_target,
            ..
        } if failed_target == CLIP_1
    ));
}

#[test]
fn compute_patch_locked_track_returns_e_locked_with_failed_index() {
    let prior = project_with_tracks(vec![TrackFixture {
        id: VIDEO_TRACK,
        kind: TrackKind::Video,
        locked: true,
        clips: vec![unlocked_clip(CLIP_1, 0)],
    }]);
    let args = base_args(vec![CLIP_1]);

    let err = compute_patch(&prior, &args).expect_err("locked track");

    assert!(matches!(
        err,
        ClipDeleteError::Locked {
            failed_index: 0,
            kind: "track",
            ref failed_target,
            ..
        } if failed_target == CLIP_1
    ));
}

#[test]
fn compute_patch_existence_precedes_lock_per_spec() {
    let prior = single_track_project(vec![locked_clip(CLIP_1, 0)]);
    let args = base_args(vec![MISSING_HIGH, CLIP_1]);

    let err = compute_patch(&prior, &args).expect_err("existence before lock");

    assert!(matches!(
        err,
        ClipDeleteError::NotFound {
            failed_index: 0,
            ..
        }
    ));
}

#[test]
fn compute_patch_clips_above_max_items_is_schema_violation() {
    let prior = single_track_project(vec![unlocked_clip(CLIP_1, 0)]);
    let args = ClipDeleteArgs {
        project_id: fixture_project_id(),
        clips: vec!["not-a-uuid".to_string(); CLIPS_MAX_BATCH + 1],
        soft: None,
        ripple: None,
        ripple_scope: None,
    };

    let err = compute_patch(&prior, &args).expect_err("batch too large");

    assert!(matches!(
        err,
        ClipDeleteError::SchemaViolation {
            field: "clips",
            hint: "split the batch into smaller calls",
            actual: Some(actual),
            max: Some(max),
        } if actual == CLIPS_MAX_BATCH + 1 && max == CLIPS_MAX_BATCH
    ));
}

#[test]
fn compute_patch_bad_selector_malformed_id_with_failed_index() {
    let prior = single_track_project(vec![unlocked_clip(CLIP_1, 0)]);
    let args = base_args(vec![CLIP_1, "not-a-uuid"]);

    let err = compute_patch(&prior, &args).expect_err("bad selector");

    assert!(matches!(
        err,
        ClipDeleteError::BadSelector {
            failed_index: 1,
            ref failed_target,
            ..
        } if failed_target == "not-a-uuid"
    ));
}

#[test]
fn compute_patch_ripple_true_is_schema_violation_deferred() {
    let prior = single_track_project(vec![unlocked_clip(CLIP_1, 0)]);
    let args = ClipDeleteArgs {
        ripple: Some(true),
        ..base_args(vec![CLIP_1])
    };

    let err = compute_patch(&prior, &args).expect_err("ripple deferred");

    assert!(matches!(
        err,
        ClipDeleteError::SchemaViolation {
            field: "ripple",
            hint: "ripple semantics deferred to follow-up",
            ..
        }
    ));
}

#[test]
fn compute_patch_ripple_scope_without_ripple_is_schema_violation() {
    let prior = single_track_project(vec![unlocked_clip(CLIP_1, 0)]);
    let args = ClipDeleteArgs {
        ripple_scope: Some("track".to_string()),
        ..base_args(vec![CLIP_1])
    };

    let err = compute_patch(&prior, &args).expect_err("ripple_scope deferred");

    assert!(matches!(
        err,
        ClipDeleteError::SchemaViolation {
            field: "ripple_scope",
            hint: "ripple semantics deferred to follow-up",
            ..
        }
    ));
}

#[test]
fn compute_patch_link_group_drops_to_one_clears_link_group() {
    let prior = two_track_linked_project(false);
    let args = base_args(vec![CLIP_1]);

    let (patch, _warnings, data) = compute_patch(&prior, &args).expect("clear survivor");

    let ops = patch_ops(&patch);
    assert_eq!(ops[0]["op"], "replace");
    assert_eq!(ops[0]["path"], "/tracks/1/clips/0/link_group");
    assert_eq!(ops[0]["value"], Value::Null);
    assert_eq!(ops[1]["path"], "/tracks/0/clips/0");
    assert_eq!(data.cleared_link_group_clip_ids[0].to_string(), CLIP_2);
    let post = apply_patch(&prior, patch);
    assert!(post.tracks[1].clips[0].link_group.is_none());
}

#[test]
fn compute_patch_link_group_drops_to_zero_no_clear_op() {
    let prior = two_track_linked_project(false);
    let args = base_args(vec![CLIP_1, CLIP_2]);

    let (patch, _warnings, data) = compute_patch(&prior, &args).expect("drop to zero");

    assert!(data.cleared_link_group_clip_ids.is_empty());
    assert!(patch_ops(&patch).iter().all(|op| op["op"] == "remove"));
}

#[test]
fn compute_patch_link_group_stays_at_two_no_clear_op() {
    let prior = project_with_tracks(vec![
        TrackFixture {
            id: VIDEO_TRACK,
            kind: TrackKind::Video,
            locked: false,
            clips: vec![linked_clip(CLIP_1, 0, LINK_GROUP)],
        },
        TrackFixture {
            id: VIDEO_TRACK_2,
            kind: TrackKind::Video,
            locked: false,
            clips: vec![linked_clip(CLIP_2, 0, LINK_GROUP)],
        },
        TrackFixture {
            id: AUDIO_TRACK,
            kind: TrackKind::Audio,
            locked: false,
            clips: vec![linked_clip(CLIP_3, 0, LINK_GROUP)],
        },
    ]);
    let args = base_args(vec![CLIP_1]);

    let (patch, _warnings, data) = compute_patch(&prior, &args).expect("stays at two");

    assert!(data.cleared_link_group_clip_ids.is_empty());
    assert_eq!(patch_ops(&patch).len(), 1);
    assert_eq!(patch_ops(&patch)[0]["op"], "remove");
}

#[test]
fn compute_patch_locked_survivor_emits_w_link_group_cleared_on_locked_warning() {
    let prior = two_track_linked_project(true);
    let args = base_args(vec![CLIP_1]);

    let (_patch, warnings, data) = compute_patch(&prior, &args).expect("locked survivor clear");

    assert_eq!(warnings.len(), 2);
    assert_eq!(warnings[0]["code"], W_CLIP_DELETE_ENVELOPE_CODE);
    assert_eq!(warnings[1]["code"], W_LINK_GROUP_CLEARED_ON_LOCKED_CODE);
    assert_eq!(warnings[1]["details"]["clip_id"], CLIP_2);
    assert_eq!(data.cleared_link_group_clip_ids[0].to_string(), CLIP_2);
}

#[test]
fn compute_patch_envelope_warning_always_present() {
    let prior = single_track_project(vec![unlocked_clip(CLIP_1, 0)]);
    let args = base_args(vec![CLIP_1]);

    let (_patch, warnings, _data) = compute_patch(&prior, &args).expect("happy path");

    assert_eq!(warnings[0]["code"], W_CLIP_DELETE_ENVELOPE_CODE);
}

#[test]
fn reconstruct_from_warning_round_trip() {
    let prior = two_track_linked_project(false);
    let args = base_args(vec![CLIP_1]);
    let (_patch, warnings, data) = compute_patch(&prior, &args).expect("happy path");

    let envelope =
        data_envelope_from_args_warnings(&args, &warnings).expect("envelope reconstructs");

    assert_eq!(data, envelope);
}

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let prior = single_track_project(vec![
        unlocked_clip(CLIP_1, 0),
        unlocked_clip(CLIP_2, 480_000),
    ]);
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        prior,
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry succeeds");

    let outcome = store
        .mutate_via_verb(
            "clip.delete",
            serde_json::to_value(base_args(vec![CLIP_1])).expect("args serialize"),
            None,
        )
        .expect("clip.delete should route");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("happy path must apply");
    };
    assert_eq!(warnings[0]["code"], W_CLIP_DELETE_ENVELOPE_CODE);
    let envelope: ClipDeleteData = serde_json::from_value(data).expect("data parses");
    assert_eq!(envelope.removed_clip_ids[0].to_string(), CLIP_1);
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "clip.delete")
        .expect("default_fixtures includes clip.delete");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(ClipDeleteVerb))
        .expect("register clip.delete verb");

    let report = validate_reconstructors(&registry, &[fixture])
        .expect("clip.delete reconstructor should pass");
    assert_eq!(report.verbs_checked, vec!["clip.delete"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn round_trip_clip_delete() {
    let prior = single_track_project(vec![
        unlocked_clip(CLIP_1, 0),
        unlocked_clip(CLIP_2, 480_000),
    ]);
    let args = base_args(vec![CLIP_1]);

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("happy path");
    let post = apply_patch(&prior, patch);
    let envelope =
        data_envelope_from_args_warnings(&args, &warnings).expect("envelope reconstructs");

    assert_eq!(data, envelope);
    assert_eq!(clip_ids(&post, 0), vec![CLIP_2.to_string()]);
    let round_trip = serde_json::to_value(&post).expect("post-state serializes");
    let from_json: Project = serde_json::from_value(round_trip).expect("post-state deserializes");
    assert_eq!(from_json, post);
}

#[test]
fn data_envelope_returns_sorted_ids() {
    let prior = project_with_tracks(vec![
        TrackFixture {
            id: VIDEO_TRACK,
            kind: TrackKind::Video,
            locked: false,
            clips: vec![
                linked_clip(CLIP_4, 0, OTHER_LINK_GROUP),
                linked_clip(CLIP_3, 480_000, LINK_GROUP),
            ],
        },
        TrackFixture {
            id: AUDIO_TRACK,
            kind: TrackKind::Audio,
            locked: false,
            clips: vec![linked_clip(CLIP_2, 0, LINK_GROUP)],
        },
    ]);
    let args = ClipDeleteArgs {
        soft: Some(true),
        ..base_args(vec![CLIP_3, MISSING_HIGH, CLIP_4, MISSING_LOW])
    };

    let (_patch, warnings, data) = compute_patch(&prior, &args).expect("mixed soft");
    let envelope =
        data_envelope_from_args_warnings(&args, &warnings).expect("envelope reconstructs");

    assert_eq!(data, envelope);
    assert_eq!(
        data.removed_clip_ids
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        vec![CLIP_3.to_string(), CLIP_4.to_string()]
    );
    assert_eq!(
        data.missing_clip_ids
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        vec![MISSING_LOW.to_string(), MISSING_HIGH.to_string()]
    );
    assert_eq!(
        data.cleared_link_group_clip_ids
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        vec![CLIP_2.to_string()]
    );
}
