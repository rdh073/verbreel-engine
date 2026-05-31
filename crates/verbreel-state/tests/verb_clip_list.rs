//! Tests for `clip.list` (§5.14) — twenty-fourth production verb.

use std::sync::Arc;

use serde_json::json;
use verbreel_state::verbs::clip_list::{
    ClipListArgs, ClipListData, ClipListError, ClipListVerb, compute_patch,
    data_envelope_from_post_state,
};
use verbreel_state::{
    MutateOutcome, Project, Track, TrackKind, VerbRegistry, default_fixtures, default_registry,
    validate_reconstructors,
};
use verbreel_types::{ProjectId, Tick};

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const TRACK_AUDIO: &str = "0190b8d3-15e3-7000-bd00-0000000aa101";
const TRACK_VIDEO: &str = "0190b8d3-15e3-7000-bd00-0000000aa201";
const CLIP_AUDIO_A: &str = "0190b8d3-15e3-7000-bd00-0000000bb101";
const CLIP_AUDIO_B: &str = "0190b8d3-15e3-7000-bd00-0000000bb201";
const CLIP_VIDEO_A: &str = "0190b8d3-15e3-7000-bd00-0000000bb301";
const ASSET_AUDIO_ID: &str = "01900000-0000-7000-8000-0000000ee001";
const ASSET_VIDEO_ID: &str = "01900000-0000-7000-8000-0000000ee002";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture → Project")
}

fn track(id: &str, kind: TrackKind, clips: Vec<(&str, i64)>, asset_id: &str) -> Track {
    let mut raw = json!({
        "id": id,
        "kind": kind,
        "name": "track",
        "locked": false,
        "clips": [],
    });

    let mut clip_values = Vec::new();
    for (clip_id, position) in clips {
        clip_values.push(json!({
            "id": clip_id,
            "name": "clip",
            "asset_id": asset_id,
            "track_position_tk": position,
            "source_in_tk": 0,
            "source_out_tk": 1_000,
        }));
    }
    raw["clips"] = serde_json::Value::Array(clip_values);
    serde_json::from_value(raw).expect("track fixture parses")
}

fn project_with_tracks(tracks: Vec<Track>) -> Project {
    let mut project = empty_project();
    project.tracks = tracks;
    project.duration_tk = Tick::new(4_000);
    project
}

fn default_args() -> ClipListArgs {
    ClipListArgs {
        project_id: FIXTURE_PROJECT_ID
            .parse::<ProjectId>()
            .expect("fixture project id parses"),
        track: None,
        at_tk: None,
    }
}

fn base_project() -> Project {
    let mut project = project_with_tracks(vec![
        track(
            TRACK_AUDIO,
            TrackKind::Audio,
            vec![(CLIP_AUDIO_B, 3_000), (CLIP_AUDIO_A, 500)],
            ASSET_AUDIO_ID,
        ),
        track(
            TRACK_VIDEO,
            TrackKind::Video,
            vec![(CLIP_VIDEO_A, 2_500)],
            ASSET_VIDEO_ID,
        ),
    ]);

    project.assets.push(
        serde_json::from_value(json!({
            "kind": "audio",
            "id": ASSET_AUDIO_ID,
            "hash": "36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da",
            "path": "assets/36/36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da.m4a",
            "original_filename": "clip-list-audio.m4a",
            "imported_at": "2026-05-24T00:00:00Z",
            "metadata": {
                "duration_tk": 720_000,
                "audio_codec": "aac",
                "audio_channels": 2,
                "audio_sample_rate_hz": 48000,
                "container": "m4a",
                "fingerprint": {
                    "mtime_ms": 1_700_000_000_000_u64,
                    "size_bytes": 524_288_u64,
                },
            },
        }))
        .expect("audio fixture asset parses"),
    );
    project.assets.push(
        serde_json::from_value(json!({
            "kind": "video",
            "id": ASSET_VIDEO_ID,
            "hash": "53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658",
            "path": "assets/53/53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.mp4",
            "original_filename": "clip-list-video.mp4",
            "imported_at": "2026-05-24T00:00:00Z",
            "metadata": {
                "duration_tk": 2_400_000,
                "width": 1920,
                "height": 1080,
                "fps_num": 30,
                "fps_den": 1,
                "video_codec": "h264",
                "container": "mp4",
                "fingerprint": {
                    "mtime_ms": 1_700_000_000_000_u64,
                    "size_bytes": 1_048_576_u64,
                },
            },
        }))
        .expect("video fixture asset parses"),
    );

    project
}

#[test]
fn happy_empty_project_returns_empty_list() {
    let prior = project_with_tracks(Vec::new());
    let args = default_args();

    let (patch, warnings, data) =
        compute_patch(&prior, &args).expect("empty project should succeed");

    assert_eq!(patch, json!([]));
    assert!(warnings.is_empty());
    assert!(data.clips.is_empty());
}

#[test]
fn happy_one_clip_returns_single_clip() {
    let prior = project_with_tracks(vec![track(
        TRACK_AUDIO,
        TrackKind::Audio,
        vec![(CLIP_AUDIO_A, 500)],
        ASSET_AUDIO_ID,
    )]);
    let args = default_args();

    let (_patch, warnings, data) =
        compute_patch(&prior, &args).expect("single clip should succeed");

    assert!(warnings.is_empty());
    assert_eq!(data.clips.len(), 1);
    assert_eq!(data.clips[0].id.to_string(), CLIP_AUDIO_A);
}

#[test]
fn happy_three_clips_sorted_by_track_and_position() {
    let prior = base_project();
    let args = default_args();

    let (_patch, warnings, data) = compute_patch(&prior, &args).expect("three clips should sort");

    assert!(warnings.is_empty());
    assert_eq!(data.clips.len(), 3);
    assert_eq!(data.clips[0].id.to_string(), CLIP_AUDIO_A);
    assert_eq!(data.clips[1].id.to_string(), CLIP_AUDIO_B);
    assert_eq!(data.clips[2].id.to_string(), CLIP_VIDEO_A);
}

#[test]
fn filter_by_track_returns_only_that_track() {
    let prior = base_project();
    let args = ClipListArgs {
        track: Some(TRACK_AUDIO.to_string()),
        ..default_args()
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("track filter should apply");
    assert_eq!(patch, json!([]));
    assert!(warnings.is_empty());
    assert_eq!(data.clips.len(), 2);
    assert_eq!(data.clips[0].id.to_string(), CLIP_AUDIO_A);
    assert_eq!(data.clips[1].id.to_string(), CLIP_AUDIO_B);
}

#[test]
fn filter_by_at_tk_in_clip_range() {
    let prior = base_project();
    let args = ClipListArgs {
        at_tk: Some(750),
        ..default_args()
    };

    let (_patch, warnings, data) =
        compute_patch(&prior, &args).expect("at_tk clip-range filter should apply");
    assert!(warnings.is_empty());
    assert_eq!(data.clips.len(), 1);
    assert_eq!(data.clips[0].id.to_string(), CLIP_AUDIO_A);
}

#[test]
fn at_tk_inclusive_start() {
    let prior = project_with_tracks(vec![track(
        TRACK_AUDIO,
        TrackKind::Audio,
        vec![(CLIP_AUDIO_A, 500)],
        ASSET_AUDIO_ID,
    )]);
    let args = ClipListArgs {
        at_tk: Some(500),
        ..default_args()
    };

    let (_patch, warnings, data) = compute_patch(&prior, &args).expect("inclusive start");
    assert!(warnings.is_empty());
    assert_eq!(data.clips.len(), 1);
    assert_eq!(data.clips[0].id.to_string(), CLIP_AUDIO_A);
}

#[test]
fn at_tk_exclusive_end() {
    let prior = project_with_tracks(vec![track(
        TRACK_AUDIO,
        TrackKind::Audio,
        vec![(CLIP_AUDIO_A, 500)],
        ASSET_AUDIO_ID,
    )]);
    let args = ClipListArgs {
        at_tk: Some(1_500),
        ..default_args()
    };

    let (_patch, warnings, data) = compute_patch(&prior, &args).expect("exclusive end");
    assert!(warnings.is_empty());
    assert!(data.clips.is_empty());
}

#[test]
fn filter_by_track_and_at_tk() {
    let prior = base_project();
    let args = ClipListArgs {
        track: Some(TRACK_AUDIO.to_string()),
        at_tk: Some(3_250),
        ..default_args()
    };

    let (_patch, warnings, data) =
        compute_patch(&prior, &args).expect("track+at_tk filter should apply");
    assert!(warnings.is_empty());
    assert_eq!(data.clips.len(), 1);
    assert_eq!(data.clips[0].id.to_string(), CLIP_AUDIO_B);
}

#[test]
fn at_tk_in_gap_with_track_filter() {
    let prior = base_project();
    let args = ClipListArgs {
        track: Some(TRACK_AUDIO.to_string()),
        at_tk: Some(1_500),
        ..default_args()
    };

    let (_patch, warnings, data) = compute_patch(&prior, &args).expect("gap should yield no clip");
    assert!(warnings.is_empty());
    assert!(data.clips.is_empty());
}

#[test]
fn at_tk_without_track_filter_finds_other_track() {
    let prior = base_project();
    let args = ClipListArgs {
        at_tk: Some(2_600),
        ..default_args()
    };

    let (_patch, warnings, data) = compute_patch(&prior, &args).expect("other track should match");
    assert!(warnings.is_empty());
    assert_eq!(data.clips.len(), 1);
    assert_eq!(data.clips[0].id.to_string(), CLIP_VIDEO_A);
}

#[test]
fn sort_stable_when_input_unsorted() {
    let prior = project_with_tracks(vec![track(
        TRACK_AUDIO,
        TrackKind::Audio,
        vec![(CLIP_AUDIO_B, 3_000), (CLIP_AUDIO_A, 500)],
        ASSET_AUDIO_ID,
    )]);
    let args = default_args();

    let (_patch, warnings, data) =
        compute_patch(&prior, &args).expect("sort should stabilize to insertion-agnostic output");
    assert!(warnings.is_empty());
    assert_eq!(data.clips[0].id.to_string(), CLIP_AUDIO_A);
    assert_eq!(data.clips[1].id.to_string(), CLIP_AUDIO_B);
}

#[test]
fn sort_tracks_by_track_id_first() {
    let prior = base_project();
    let args = ClipListArgs {
        at_tk: None,
        ..default_args()
    };

    let (_patch, warnings, data) = compute_patch(&prior, &args).expect("track id sort order");
    assert!(warnings.is_empty());
    assert_eq!(data.clips[0].id.to_string(), CLIP_AUDIO_A);
    assert_eq!(data.clips[1].id.to_string(), CLIP_AUDIO_B);
    assert_eq!(data.clips[2].id.to_string(), CLIP_VIDEO_A);
}

#[test]
fn clip_list_bad_selector() {
    let prior = base_project();
    let err = compute_patch(
        &prior,
        &ClipListArgs {
            track: Some("not-a-uuid".to_string()),
            ..default_args()
        },
    )
    .expect_err("bad selector should reject");

    assert!(matches!(err, ClipListError::BadSelector { .. }));
}

#[test]
fn clip_list_track_not_found() {
    let prior = base_project();
    let err = compute_patch(
        &prior,
        &ClipListArgs {
            track: Some("0190b8d3-15e3-7000-bd00-0000000dd404".to_string()),
            ..default_args()
        },
    )
    .expect_err("missing track should reject");

    match err {
        ClipListError::TrackNotFound { track_id } => {
            assert_eq!(track_id, "0190b8d3-15e3-7000-bd00-0000000dd404")
        }
        other => panic!("expected TrackNotFound, got {other:?}"),
    }
}

#[test]
fn patch_is_always_empty() {
    let prior = base_project();
    let args = default_args();

    let (patch, _warnings, _data) = compute_patch(&prior, &args).expect("valid call");
    assert_eq!(patch, json!([]));

    let args_track = ClipListArgs {
        track: Some(TRACK_AUDIO.to_string()),
        ..default_args()
    };
    let (filtered_patch, _warnings, _data) =
        compute_patch(&prior, &args_track).expect("valid call");
    assert_eq!(filtered_patch, json!([]));
}

#[test]
fn warnings_are_always_empty() {
    let prior = base_project();
    let args = default_args();

    let (_patch, warnings, _data) = compute_patch(&prior, &args).expect("valid call");
    assert!(warnings.is_empty());
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "clip.list")
        .expect("default_fixtures includes clip.list");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(ClipListVerb))
        .expect("register clip.list verb");

    let report =
        validate_reconstructors(&registry, &[fixture]).expect("default fixture should pass");
    assert_eq!(report.verbs_checked, vec!["clip.list"]);
}

#[test]
fn data_envelope_from_post_state_matches_compute_patch_data() {
    let prior = base_project();
    let args = ClipListArgs {
        at_tk: Some(3_250),
        ..default_args()
    };

    let (_, _, compute_data) = compute_patch(&prior, &args).expect("compute should succeed");
    let replay_data = data_envelope_from_post_state(&args, &prior)
        .expect("replay envelope should match post-state");
    assert_eq!(compute_data, replay_data);
}

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        base_project(),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry clears the gate and writes project.json");

    let outcome = store
        .mutate_via_verb(
            "clip.list",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
            }),
            None,
        )
        .expect("clip.list should route");

    let MutateOutcome::NoOp { data, warnings, .. } = outcome else {
        panic!("clip.list should be NoOp");
    };
    let data: ClipListData = serde_json::from_value(data).expect("clip.list data deserializes");
    assert_eq!(data.clips.len(), 3);
    assert!(warnings.is_empty());
}

#[test]
fn off_frame_at_tk_is_never_special() {
    let prior = base_project();
    let args = ClipListArgs {
        at_tk: Some(12_345),
        ..default_args()
    };

    let (patch, warnings, data) =
        compute_patch(&prior, &args).expect("off-frame at_tk should work");
    assert_eq!(patch, json!([]));
    assert!(warnings.is_empty());
    assert!(data.clips.is_empty());
}
