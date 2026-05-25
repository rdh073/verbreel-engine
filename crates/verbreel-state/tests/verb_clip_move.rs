//! Tests for `clip.move` (§5.4) — forty-sixth production verb.

use serde_json::{Value, json};
use std::sync::Arc;

use verbreel_state::verbs::clip_move::{
    ARGS_INCOMPATIBLE_HINT, LINK_GROUP_SEMANTICS_MIX_HINT, W_TIME_SNAPPED_CODE, compute_patch,
    data_envelope_from_post_state,
};
use verbreel_state::{
    ClipMoveArgs, ClipMoveData, ClipMoveError, ClipMoveVerb, MutateOutcome, Project, RecordedEvent,
    TrackKind, Verb, VerbError, VerbRegistry, default_fixtures, default_registry,
    validate_reconstructors,
};
use verbreel_types::{ClipId, ProjectId, Tick};

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const TRACK_VIDEO_A: &str = "01900000-0000-7000-8000-0000000aa601";
const TRACK_VIDEO_B: &str = "01900000-0000-7000-8000-0000000aa602";
const TRACK_AUDIO_A: &str = "01900000-0000-7000-8000-0000000aa603";
const CLIP_VIDEO_A: &str = "01900000-0000-7000-8000-0000000bb601";
const CLIP_VIDEO_B: &str = "01900000-0000-7000-8000-0000000bb602";
const CLIP_AUDIO_A: &str = "01900000-0000-7000-8000-0000000bb603";
const CLIP_AUDIO_B: &str = "01900000-0000-7000-8000-0000000bb604";
const CLIP_IMAGE_A: &str = "01900000-0000-7000-8000-0000000bb605";
const MISSING_CLIP: &str = "01900000-0000-7000-8000-0000000cc601";
const MISSING_TRACK: &str = "01900000-0000-7000-8000-0000000aa999";
const LINK_GROUP_ID: &str = "01900000-0000-7000-8000-0000000dd601";
const ASSET_VIDEO_ID: &str = "01900000-0000-7000-8000-0000000ee601";
const ASSET_AUDIO_ID: &str = "01900000-0000-7000-8000-0000000ee602";
const ASSET_IMAGE_ID: &str = "01900000-0000-7000-8000-0000000ee603";

#[derive(Debug, Clone)]
struct ClipFixture {
    id: &'static str,
    position_tk: i64,
    locked: bool,
    link_group: Option<&'static str>,
    asset_id: &'static str,
    is_text: bool,
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

fn fixture_project_id() -> ProjectId {
    FIXTURE_PROJECT_ID
        .parse()
        .expect("fixture project id parses")
}

fn video_asset() -> Value {
    json!({
        "id": ASSET_VIDEO_ID,
        "kind": "video",
        "hash": "36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da",
        "path": "assets/36/36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da.mp4",
        "original_filename": "clip-move.mp4",
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
        "id": ASSET_AUDIO_ID,
        "kind": "audio",
        "hash": "53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658",
        "path": "assets/53/53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.m4a",
        "original_filename": "clip-move.m4a",
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

fn image_asset() -> Value {
    json!({
        "id": ASSET_IMAGE_ID,
        "kind": "image",
        "hash": "d78685cbed99000e92b1e62dae6cc40404f68c3f5069135de242ad7201a3d552",
        "path": "assets/d7/d78685cbed99000e92b1e62dae6cc40404f68c3f5069135de242ad7201a3d552.png",
        "original_filename": "still.png",
        "imported_at": "2026-05-24T00:00:00Z",
        "metadata": {
            "width": 1920,
            "height": 1080,
            "container": "png",
            "fingerprint": {
                "mtime_ms": 1_700_000_000_000_i64,
                "size_bytes": 1024,
            }
        }
    })
}

fn clip_value(kind: TrackKind, fixture: &ClipFixture) -> Value {
    let mut clip = json!({
        "id": fixture.id,
        "name": "Clip",
        "asset_id": fixture.asset_id,
        "track_position_tk": fixture.position_tk,
        "source_in_tk": 0,
        "source_out_tk": 240_000,
        "locked": fixture.locked,
    });
    if kind == TrackKind::Audio {
        clip["volume"] = json!(1.0);
    }
    if fixture.is_text {
        clip["text"] = json!({
            "content": "Move",
            "font_family": "Arial",
            "font_size_px": 24
        });
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
        .map(|clip| clip.position_tk + 240_000)
        .max()
        .unwrap_or(0);
    project.tracks = tracks
        .into_iter()
        .map(|track| {
            let clips = track
                .clips
                .iter()
                .map(|clip| clip_value(track.kind, clip))
                .collect::<Vec<_>>();
            serde_json::from_value(json!({
                "id": track.id,
                "kind": track.kind,
                "name": "Track",
                "locked": track.locked,
                "clips": clips,
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
    project
        .assets
        .push(serde_json::from_value(image_asset()).expect("image asset parses"));
    project.duration_tk = Tick::new(duration_tk);
    project
}

fn clip(
    id: &'static str,
    position_tk: i64,
    locked: bool,
    link_group: Option<&'static str>,
    asset_id: &'static str,
) -> ClipFixture {
    ClipFixture {
        id,
        position_tk,
        locked,
        link_group,
        asset_id,
        is_text: asset_id == "00000000-0000-0000-0000-000000000000",
    }
}

fn video_clip(id: &'static str, position_tk: i64) -> ClipFixture {
    clip(id, position_tk, false, None, ASSET_VIDEO_ID)
}

fn audio_clip(id: &'static str, position_tk: i64) -> ClipFixture {
    clip(id, position_tk, false, None, ASSET_AUDIO_ID)
}

fn linked_video_clip(id: &'static str, position_tk: i64) -> ClipFixture {
    clip(id, position_tk, false, Some(LINK_GROUP_ID), ASSET_VIDEO_ID)
}

fn linked_audio_clip(id: &'static str, position_tk: i64) -> ClipFixture {
    clip(id, position_tk, false, Some(LINK_GROUP_ID), ASSET_AUDIO_ID)
}

fn move_args(clip: &str, track_position_tk: Option<i64>, to_track: Option<&str>) -> ClipMoveArgs {
    ClipMoveArgs {
        project_id: fixture_project_id(),
        clip: clip.to_string(),
        track_position_tk,
        to_track: to_track.map(ToString::to_string),
    }
}

fn apply_patch(prior: &Project, patch: &Value) -> Project {
    let patch: json_patch::Patch =
        serde_json::from_value(patch.clone()).expect("patch parses to typed JSON patch");
    prior
        .apply(&patch)
        .expect("clip.move patch applies cleanly")
}

fn patch_paths(patch: &Value) -> Vec<String> {
    patch
        .as_array()
        .expect("patch is array")
        .iter()
        .map(|op| {
            op.get("path")
                .and_then(Value::as_str)
                .expect("op has path")
                .to_string()
        })
        .collect()
}

fn singleton_project() -> Project {
    project_with_tracks(vec![TrackFixture {
        id: TRACK_VIDEO_A,
        kind: TrackKind::Video,
        locked: false,
        clips: vec![video_clip(CLIP_VIDEO_A, 0)],
    }])
}

fn linked_video_audio_project() -> Project {
    project_with_tracks(vec![
        TrackFixture {
            id: TRACK_VIDEO_A,
            kind: TrackKind::Video,
            locked: false,
            clips: vec![linked_video_clip(CLIP_VIDEO_A, 0)],
        },
        TrackFixture {
            id: TRACK_AUDIO_A,
            kind: TrackKind::Audio,
            locked: false,
            clips: vec![linked_audio_clip(CLIP_AUDIO_A, 0)],
        },
    ])
}

#[test]
fn both_setter_args_omitted_returns_args_incompatible() {
    let err = compute_patch(&singleton_project(), &move_args(CLIP_VIDEO_A, None, None))
        .expect_err("empty setter args reject");
    match err {
        ClipMoveError::ArgsIncompatible { hint } => assert_eq!(hint, ARGS_INCOMPATIBLE_HINT),
        other => panic!("expected ArgsIncompatible, got {other:?}"),
    }
}

#[test]
fn position_only_singleton_moves_clip() {
    let prior = singleton_project();
    let (patch, warnings, data) =
        compute_patch(&prior, &move_args(CLIP_VIDEO_A, Some(240_000), None)).expect("move");
    let post = apply_patch(&prior, &patch);

    assert!(warnings.is_empty());
    assert_eq!(
        patch_paths(&patch),
        ["/tracks/0/clips/0/track_position_tk", "/duration_tk"]
    );
    assert_eq!(post.tracks[0].clips[0].track_position_tk.get(), 240_000);
    assert_eq!(post.duration_tk.get(), 480_000);
    assert_eq!(data.track_id.to_string(), TRACK_VIDEO_A);
    assert_eq!(data.track_position_tk, 240_000);
    assert!(data.linked_clip_ids.is_empty());
}

#[test]
fn to_track_only_relocates_target_and_preserves_position() {
    let prior = project_with_tracks(vec![
        TrackFixture {
            id: TRACK_VIDEO_A,
            kind: TrackKind::Video,
            locked: false,
            clips: vec![video_clip(CLIP_VIDEO_A, 0)],
        },
        TrackFixture {
            id: TRACK_VIDEO_B,
            kind: TrackKind::Video,
            locked: false,
            clips: vec![],
        },
    ]);

    let (patch, warnings, data) =
        compute_patch(&prior, &move_args(CLIP_VIDEO_A, None, Some(TRACK_VIDEO_B)))
            .expect("relocate");
    let post = apply_patch(&prior, &patch);

    assert!(warnings.is_empty());
    assert_eq!(
        patch_paths(&patch),
        ["/tracks/1/clips/-", "/tracks/0/clips/0"]
    );
    assert!(post.tracks[0].clips.is_empty());
    assert_eq!(post.tracks[1].clips[0].id.to_string(), CLIP_VIDEO_A);
    assert_eq!(post.tracks[1].clips[0].track_position_tk.get(), 0);
    assert_eq!(data.track_id.to_string(), TRACK_VIDEO_B);
    assert_eq!(data.track_position_tk, 0);
}

#[test]
fn position_and_to_track_relocates_and_moves_target() {
    let prior = project_with_tracks(vec![
        TrackFixture {
            id: TRACK_VIDEO_A,
            kind: TrackKind::Video,
            locked: false,
            clips: vec![video_clip(CLIP_VIDEO_A, 0)],
        },
        TrackFixture {
            id: TRACK_VIDEO_B,
            kind: TrackKind::Video,
            locked: false,
            clips: vec![],
        },
    ]);

    let (patch, warnings, data) = compute_patch(
        &prior,
        &move_args(CLIP_VIDEO_A, Some(240_000), Some(TRACK_VIDEO_B)),
    )
    .expect("relocate and move");
    let post = apply_patch(&prior, &patch);

    assert!(warnings.is_empty());
    assert_eq!(
        patch_paths(&patch),
        ["/tracks/1/clips/-", "/tracks/0/clips/0", "/duration_tk"]
    );
    assert_eq!(post.tracks[1].clips[0].track_position_tk.get(), 240_000);
    assert_eq!(post.duration_tk.get(), 480_000);
    assert_eq!(data.track_id.to_string(), TRACK_VIDEO_B);
    assert_eq!(data.track_position_tk, 240_000);
}

#[test]
fn linked_video_audio_position_only_moves_both_by_delta() {
    let prior = linked_video_audio_project();
    let (patch, warnings, data) =
        compute_patch(&prior, &move_args(CLIP_VIDEO_A, Some(240_000), None)).expect("move group");
    let post = apply_patch(&prior, &patch);

    assert!(warnings.is_empty());
    assert_eq!(
        patch_paths(&patch),
        [
            "/tracks/0/clips/0/track_position_tk",
            "/tracks/1/clips/0/track_position_tk",
            "/duration_tk"
        ]
    );
    assert_eq!(post.tracks[0].clips[0].track_position_tk.get(), 240_000);
    assert_eq!(post.tracks[1].clips[0].track_position_tk.get(), 240_000);
    assert_eq!(data.linked_clip_ids, vec![CLIP_AUDIO_A.parse().unwrap()]);
}

#[test]
fn linked_video_to_track_moves_only_target_track_but_shifts_audio_position() {
    let prior = project_with_tracks(vec![
        TrackFixture {
            id: TRACK_VIDEO_A,
            kind: TrackKind::Video,
            locked: false,
            clips: vec![linked_video_clip(CLIP_VIDEO_A, 0)],
        },
        TrackFixture {
            id: TRACK_VIDEO_B,
            kind: TrackKind::Video,
            locked: false,
            clips: vec![],
        },
        TrackFixture {
            id: TRACK_AUDIO_A,
            kind: TrackKind::Audio,
            locked: false,
            clips: vec![linked_audio_clip(CLIP_AUDIO_A, 0)],
        },
    ]);

    let (patch, warnings, data) = compute_patch(
        &prior,
        &move_args(CLIP_VIDEO_A, Some(240_000), Some(TRACK_VIDEO_B)),
    )
    .expect("linked relocate");
    let post = apply_patch(&prior, &patch);

    assert!(warnings.is_empty());
    assert_eq!(
        patch_paths(&patch),
        [
            "/tracks/2/clips/0/track_position_tk",
            "/tracks/1/clips/-",
            "/tracks/0/clips/0",
            "/duration_tk"
        ]
    );
    assert!(post.tracks[0].clips.is_empty());
    assert_eq!(post.tracks[1].clips[0].id.to_string(), CLIP_VIDEO_A);
    assert_eq!(post.tracks[1].clips[0].track_position_tk.get(), 240_000);
    assert_eq!(post.tracks[2].clips[0].id.to_string(), CLIP_AUDIO_A);
    assert_eq!(post.tracks[2].clips[0].track_position_tk.get(), 240_000);
    assert_eq!(data.track_id.to_string(), TRACK_VIDEO_B);
}

#[test]
fn missing_clip_returns_not_found() {
    let err = compute_patch(
        &singleton_project(),
        &move_args(MISSING_CLIP, Some(0), None),
    )
    .expect_err("missing clip");
    match err {
        ClipMoveError::ClipNotFound { clip_id } => assert_eq!(clip_id, MISSING_CLIP),
        other => panic!("expected ClipNotFound, got {other:?}"),
    }
}

#[test]
fn malformed_clip_id_returns_bad_selector() {
    let err = compute_patch(
        &singleton_project(),
        &move_args("not-a-uuid", Some(0), None),
    )
    .expect_err("bad clip selector");
    match err {
        ClipMoveError::BadSelector { field, detail } => {
            assert_eq!(field, "clip");
            assert!(detail.contains("UUID"), "{detail}");
        }
        other => panic!("expected BadSelector, got {other:?}"),
    }
}

#[test]
fn malformed_to_track_id_returns_bad_selector() {
    let err = compute_patch(
        &singleton_project(),
        &move_args(CLIP_VIDEO_A, None, Some("not-a-uuid")),
    )
    .expect_err("bad track selector");
    match err {
        ClipMoveError::BadSelector { field, detail } => {
            assert_eq!(field, "to_track");
            assert!(detail.contains("UUID"), "{detail}");
        }
        other => panic!("expected BadSelector, got {other:?}"),
    }
}

#[test]
fn unknown_to_track_returns_track_not_found() {
    let err = compute_patch(
        &singleton_project(),
        &move_args(CLIP_VIDEO_A, None, Some(MISSING_TRACK)),
    )
    .expect_err("missing track");
    match err {
        ClipMoveError::TrackNotFound { track_id } => assert_eq!(track_id, MISSING_TRACK),
        other => panic!("expected TrackNotFound, got {other:?}"),
    }
}

#[test]
fn cross_kind_to_track_returns_track_kind_mismatch() {
    let prior = project_with_tracks(vec![
        TrackFixture {
            id: TRACK_VIDEO_A,
            kind: TrackKind::Video,
            locked: false,
            clips: vec![video_clip(CLIP_VIDEO_A, 0)],
        },
        TrackFixture {
            id: TRACK_AUDIO_A,
            kind: TrackKind::Audio,
            locked: false,
            clips: vec![],
        },
    ]);
    let err = compute_patch(&prior, &move_args(CLIP_VIDEO_A, None, Some(TRACK_AUDIO_A)))
        .expect_err("kind mismatch");
    match err {
        ClipMoveError::TrackKindMismatch {
            expected_kind,
            actual_kind,
        } => {
            assert_eq!(expected_kind, "video");
            assert_eq!(actual_kind, "audio");
        }
        other => panic!("expected TrackKindMismatch, got {other:?}"),
    }
}

#[test]
fn negative_track_position_returns_bad_time() {
    let err = compute_patch(
        &singleton_project(),
        &move_args(CLIP_VIDEO_A, Some(-1), None),
    )
    .expect_err("bad time");
    match err {
        ClipMoveError::BadTime { field, value } => {
            assert_eq!(field, "track_position_tk");
            assert_eq!(value, -1);
        }
        other => panic!("expected BadTime, got {other:?}"),
    }
}

#[test]
fn locked_target_clip_returns_locked() {
    let prior = project_with_tracks(vec![TrackFixture {
        id: TRACK_VIDEO_A,
        kind: TrackKind::Video,
        locked: false,
        clips: vec![clip(CLIP_VIDEO_A, 0, true, None, ASSET_VIDEO_ID)],
    }]);
    let err =
        compute_patch(&prior, &move_args(CLIP_VIDEO_A, Some(240_000), None)).expect_err("locked");
    match err {
        ClipMoveError::Locked { failed_clip } => assert_eq!(failed_clip, CLIP_VIDEO_A),
        other => panic!("expected Locked, got {other:?}"),
    }
}

#[test]
fn locked_target_new_track_returns_locked_when_relocating() {
    let prior = project_with_tracks(vec![
        TrackFixture {
            id: TRACK_VIDEO_A,
            kind: TrackKind::Video,
            locked: false,
            clips: vec![video_clip(CLIP_VIDEO_A, 0)],
        },
        TrackFixture {
            id: TRACK_VIDEO_B,
            kind: TrackKind::Video,
            locked: true,
            clips: vec![],
        },
    ]);
    let err = compute_patch(&prior, &move_args(CLIP_VIDEO_A, None, Some(TRACK_VIDEO_B)))
        .expect_err("locked target track");
    match err {
        ClipMoveError::Locked { failed_clip } => assert_eq!(failed_clip, CLIP_VIDEO_A),
        other => panic!("expected Locked, got {other:?}"),
    }
}

#[test]
fn locked_linked_sibling_returns_locked() {
    let prior = project_with_tracks(vec![
        TrackFixture {
            id: TRACK_VIDEO_A,
            kind: TrackKind::Video,
            locked: false,
            clips: vec![linked_video_clip(CLIP_VIDEO_A, 0)],
        },
        TrackFixture {
            id: TRACK_AUDIO_A,
            kind: TrackKind::Audio,
            locked: false,
            clips: vec![clip(
                CLIP_AUDIO_A,
                0,
                true,
                Some(LINK_GROUP_ID),
                ASSET_AUDIO_ID,
            )],
        },
    ]);
    let err = compute_patch(&prior, &move_args(CLIP_VIDEO_A, Some(240_000), None))
        .expect_err("locked sibling");
    match err {
        ClipMoveError::Locked { failed_clip } => assert_eq!(failed_clip, CLIP_AUDIO_A),
        other => panic!("expected Locked, got {other:?}"),
    }
}

#[test]
fn target_overlap_returns_clip_overlap() {
    let prior = project_with_tracks(vec![TrackFixture {
        id: TRACK_VIDEO_A,
        kind: TrackKind::Video,
        locked: false,
        clips: vec![
            video_clip(CLIP_VIDEO_A, 0),
            video_clip(CLIP_VIDEO_B, 240_000),
        ],
    }]);
    let err = compute_patch(&prior, &move_args(CLIP_VIDEO_A, Some(120_000), None))
        .expect_err("target overlap");
    match err {
        ClipMoveError::ClipOverlap { failed_clip } => assert_eq!(failed_clip, CLIP_VIDEO_A),
        other => panic!("expected ClipOverlap, got {other:?}"),
    }
}

#[test]
fn sibling_delta_overlap_returns_clip_overlap() {
    let prior = project_with_tracks(vec![
        TrackFixture {
            id: TRACK_VIDEO_A,
            kind: TrackKind::Video,
            locked: false,
            clips: vec![linked_video_clip(CLIP_VIDEO_A, 0)],
        },
        TrackFixture {
            id: TRACK_AUDIO_A,
            kind: TrackKind::Audio,
            locked: false,
            clips: vec![
                linked_audio_clip(CLIP_AUDIO_A, 0),
                audio_clip(CLIP_AUDIO_B, 240_000),
            ],
        },
    ]);
    let err = compute_patch(&prior, &move_args(CLIP_VIDEO_A, Some(240_000), None))
        .expect_err("sibling overlap");
    match err {
        ClipMoveError::ClipOverlap { failed_clip } => assert_eq!(failed_clip, CLIP_AUDIO_A),
        other => panic!("expected ClipOverlap, got {other:?}"),
    }
}

#[test]
fn video_image_link_group_returns_semantics_mix() {
    let prior = project_with_tracks(vec![
        TrackFixture {
            id: TRACK_VIDEO_A,
            kind: TrackKind::Video,
            locked: false,
            clips: vec![linked_video_clip(CLIP_VIDEO_A, 0)],
        },
        TrackFixture {
            id: TRACK_VIDEO_B,
            kind: TrackKind::Video,
            locked: false,
            clips: vec![clip(
                CLIP_IMAGE_A,
                0,
                false,
                Some(LINK_GROUP_ID),
                ASSET_IMAGE_ID,
            )],
        },
    ]);
    let err = compute_patch(&prior, &move_args(CLIP_VIDEO_A, Some(240_000), None))
        .expect_err("semantics mix");
    match err {
        ClipMoveError::LinkGroupSemanticsMix {
            link_group,
            member_kinds,
            semantics_classes,
            hint,
        } => {
            assert_eq!(link_group.to_string(), LINK_GROUP_ID);
            assert_eq!(member_kinds.video, 1);
            assert_eq!(member_kinds.image, 1);
            assert_eq!(semantics_classes.source_slice, 1);
            assert_eq!(semantics_classes.display_duration, 1);
            assert_eq!(hint, LINK_GROUP_SEMANTICS_MIX_HINT);
        }
        other => panic!("expected LinkGroupSemanticsMix, got {other:?}"),
    }
}

#[test]
fn video_off_frame_position_snaps_and_warns() {
    let prior = singleton_project();
    let (patch, warnings, data) =
        compute_patch(&prior, &move_args(CLIP_VIDEO_A, Some(8_001), None)).expect("snap");
    let post = apply_patch(&prior, &patch);

    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_TIME_SNAPPED_CODE);
    assert_eq!(warnings[0]["details"]["clip_id"], CLIP_VIDEO_A);
    assert_eq!(warnings[0]["details"]["from_tk"], 8_001);
    assert_eq!(warnings[0]["details"]["to_tk"], 8_000);
    assert_eq!(post.tracks[0].clips[0].track_position_tk.get(), 8_000);
    assert_eq!(data.track_position_tk, 8_000);
}

#[test]
fn audio_off_frame_position_does_not_snap_or_warn() {
    let prior = project_with_tracks(vec![TrackFixture {
        id: TRACK_AUDIO_A,
        kind: TrackKind::Audio,
        locked: false,
        clips: vec![audio_clip(CLIP_AUDIO_A, 0)],
    }]);
    let (patch, warnings, data) =
        compute_patch(&prior, &move_args(CLIP_AUDIO_A, Some(8_001), None)).expect("audio move");
    let post = apply_patch(&prior, &patch);

    assert!(warnings.is_empty());
    assert_eq!(post.tracks[0].clips[0].track_position_tk.get(), 8_001);
    assert_eq!(data.track_position_tk, 8_001);
}

#[test]
fn linked_video_member_snaps_when_audio_target_delta_is_off_frame() {
    let prior = linked_video_audio_project();
    let (patch, warnings, data) =
        compute_patch(&prior, &move_args(CLIP_AUDIO_A, Some(8_001), None))
            .expect("linked audio target move");
    let post = apply_patch(&prior, &patch);

    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_TIME_SNAPPED_CODE);
    assert_eq!(warnings[0]["details"]["clip_id"], CLIP_VIDEO_A);
    assert_eq!(post.tracks[0].clips[0].track_position_tk.get(), 8_000);
    assert_eq!(post.tracks[1].clips[0].track_position_tk.get(), 8_001);
    assert_eq!(data.clip_id.to_string(), CLIP_AUDIO_A);
    assert_eq!(data.track_position_tk, 8_001);
}

#[test]
fn reconstructor_round_trip() {
    let prior = linked_video_audio_project();
    let args = move_args(CLIP_VIDEO_A, Some(240_000), None);
    let (patch, warnings, _data) = compute_patch(&prior, &args).expect("compute_patch ok");
    let post_state = apply_patch(&prior, &patch);
    let expected_data =
        serde_json::to_value(data_envelope_from_post_state(&args, &post_state).expect("envelope"))
            .expect("envelope serializes");
    let recorded = RecordedEvent {
        verb: "clip.move".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch,
        warnings,
        post_state,
        expected_data,
    };

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(ClipMoveVerb))
        .expect("register clip.move");
    let report = validate_reconstructors(&registry, &[recorded]).expect("round trip");
    assert_eq!(report.verbs_checked, vec!["clip.move"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "clip.move")
        .expect("default_fixtures includes clip.move");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(ClipMoveVerb))
        .expect("register clip.move");
    let report = validate_reconstructors(&registry, &[fixture]).expect("default fixture");
    assert_eq!(report.verbs_checked, vec!["clip.move"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn mutate_via_verb_routing_error_maps_to_bad_args() {
    let prior = singleton_project();
    let verb = ClipMoveVerb;
    let args = serde_json::to_value(move_args("not-a-uuid", Some(0), None)).unwrap();
    let err = verb
        .compute_patch(&prior, &args)
        .expect_err("bad selector maps");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let base_project = singleton_project();
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        base_project,
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry clears the gate");

    let outcome = store
        .mutate_via_verb(
            "clip.move",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "clip": CLIP_VIDEO_A,
                "track_position_tk": 240_000,
            }),
            None,
        )
        .expect("mutate_via_verb happy path");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("happy path must return Applied, got {outcome:?}");
    };

    let data: ClipMoveData = serde_json::from_value(data).expect("clip.move data is ClipMoveData");
    assert_eq!(
        store.project().tracks[0].clips[0].track_position_tk.get(),
        240_000
    );
    assert_eq!(data.clip_id.to_string(), CLIP_VIDEO_A);
    assert_eq!(data.track_position_tk, 240_000);
    assert_eq!(warnings, Vec::<Value>::new());
}

#[test]
fn linked_clip_ids_are_sorted_and_exclude_target() {
    let prior = project_with_tracks(vec![
        TrackFixture {
            id: TRACK_VIDEO_A,
            kind: TrackKind::Video,
            locked: false,
            clips: vec![linked_video_clip(CLIP_VIDEO_A, 0)],
        },
        TrackFixture {
            id: TRACK_VIDEO_B,
            kind: TrackKind::Video,
            locked: false,
            clips: vec![clip(
                CLIP_VIDEO_B,
                0,
                false,
                Some(LINK_GROUP_ID),
                ASSET_VIDEO_ID,
            )],
        },
        TrackFixture {
            id: TRACK_AUDIO_A,
            kind: TrackKind::Audio,
            locked: false,
            clips: vec![linked_audio_clip(CLIP_AUDIO_A, 0)],
        },
    ]);

    let (_patch, _warnings, data) =
        compute_patch(&prior, &move_args(CLIP_VIDEO_B, Some(240_000), None)).expect("linked move");
    let actual = data
        .linked_clip_ids
        .iter()
        .map(ClipId::to_string)
        .collect::<Vec<_>>();
    assert_eq!(actual, vec![CLIP_VIDEO_A, CLIP_AUDIO_A]);
}
