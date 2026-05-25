//! Tests for `clip.set_blend_mode` (§5.18) — twenty-second production verb.

use serde_json::{Value, json};
use std::sync::Arc;

use verbreel_state::verbs::clip_set_blend_mode::{
    W_BLEND_MODE_INERT_ON_AUDIO_CODE, W_NOOP_CODE, compute_patch, data_envelope_from_post_state,
};
use verbreel_state::{
    BlendMode, ClipSetBlendModeArgs, ClipSetBlendModeData, ClipSetBlendModeError,
    ClipSetBlendModeVerb, MutateOutcome, Project, RecordedEvent, Track, TrackKind, Verb, VerbError,
    VerbRegistry, default_fixtures, default_registry, validate_reconstructors,
};
use verbreel_types::{ProjectId, Tick};

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const TRACK_TEXT_A: &str = "0190b8d3-15e3-7000-bd00-0000000aa101";
const TRACK_VIDEO_A: &str = "0190b8d3-15e3-7000-bd00-0000000aa102";
const TRACK_AUDIO_A: &str = "0190b8d3-15e3-7000-bd00-0000000aa103";
const CLIP_TEXT_A: &str = "0190b8d3-15e3-7000-bd00-0000000bb101";
const CLIP_VIDEO_A: &str = "0190b8d3-15e3-7000-bd00-0000000bb102";
const CLIP_AUDIO_A: &str = "0190b8d3-15e3-7000-bd00-0000000bb103";
const MISSING_CLIP: &str = "0190b8d3-15e3-7000-bd00-0000000cc101";
const ASSET_AUDIO_ID: &str = "0190b8d3-15e3-7000-bd00-0000000dd102";
const ASSET_VIDEO_ID: &str = "0190b8d3-15e3-7000-bd00-0000000dd201";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture → Project")
}

fn fixture_project_id() -> ProjectId {
    empty_project().id
}

fn text_track(
    id: &str,
    name: &str,
    track_locked: bool,
    clip_id: &str,
    clip_locked: bool,
    blend_mode: BlendMode,
) -> Track {
    serde_json::from_value(json!({
        "id": id,
        "kind": TrackKind::Text,
        "name": name,
        "locked": track_locked,
        "clips": [{
            "id": clip_id,
            "name": "Text Clip",
            "asset_id": "00000000-0000-0000-0000-000000000000",
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 480_000,
            "blend_mode": blend_mode,
            "locked": clip_locked,
            "text": {
                "content": "Hello",
                "font_family": "Arial",
                "font_size_px": 24,
            },
        }],
    }))
    .expect("text track fixture parses")
}

fn video_track(
    id: &str,
    name: &str,
    track_locked: bool,
    clip_id: &str,
    clip_locked: bool,
    blend_mode: BlendMode,
) -> Track {
    serde_json::from_value(json!({
        "id": id,
        "kind": TrackKind::Video,
        "name": name,
        "locked": track_locked,
        "clips": [{
            "id": clip_id,
            "name": "Video Clip",
            "asset_id": ASSET_VIDEO_ID,
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 480_000,
            "blend_mode": blend_mode,
            "locked": clip_locked,
        }],
    }))
    .expect("video track fixture parses")
}

fn audio_track(
    id: &str,
    name: &str,
    track_locked: bool,
    clip_id: &str,
    clip_locked: bool,
    blend_mode: BlendMode,
) -> Track {
    serde_json::from_value(json!({
        "id": id,
        "kind": TrackKind::Audio,
        "name": name,
        "locked": track_locked,
        "clips": [{
            "id": clip_id,
            "name": "Audio Clip",
            "asset_id": ASSET_AUDIO_ID,
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 240_000,
            "blend_mode": blend_mode,
            "locked": clip_locked,
        }],
    }))
    .expect("audio track fixture parses")
}

fn project_with_tracks(tracks: Vec<Track>) -> Project {
    let mut project = empty_project();
    project.tracks = tracks;
    project.duration_tk = Tick::new(480_000);
    project
}

fn project_with_tracks_and_assets(tracks: Vec<Track>) -> Project {
    let mut project = project_with_tracks(tracks);

    if project
        .tracks
        .iter()
        .any(|track| track.kind == TrackKind::Video)
    {
        project.assets.push(
            serde_json::from_value(json!({
                "id": ASSET_VIDEO_ID,
                "kind": "video",
                "hash": "53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658",
                "path": "assets/53/53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.mp4",
                "original_filename": "video1.mp4",
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
            }))
            .expect("video asset fixture parses"),
        );
    }

    if project
        .tracks
        .iter()
        .any(|track| track.kind == TrackKind::Audio)
    {
        project.assets.push(
            serde_json::from_value(json!({
                "id": ASSET_AUDIO_ID,
                "kind": "audio",
                "hash": "36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da",
                "path": "assets/36/36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da.m4a",
                "original_filename": "audio1.m4a",
                "imported_at": "2026-05-24T00:00:00Z",
                "metadata": {
                    "duration_tk": 240_000,
                    "audio_codec": "aac",
                    "audio_channels": 2,
                    "audio_sample_rate_hz": 48_000,
                    "container": "m4a",
                    "fingerprint": {
                        "mtime_ms": 1_700_000_000_000_i64,
                        "size_bytes": 512,
                    }
                }
            }))
            .expect("audio asset fixture parses"),
        );
    }

    project
}

fn patch_blend_mode_value(patch: &Value) -> BlendMode {
    let arr = patch.as_array().expect("patch is an array");
    assert_eq!(arr.len(), 1, "clip.set_blend_mode emits one replace op");
    let op = arr[0].as_object().expect("patch op is object");
    assert_eq!(op.get("op").and_then(Value::as_str), Some("replace"));
    let value = op.get("value").expect("replace op has value");
    serde_json::from_value(value.clone()).expect("blend mode is enum string")
}

#[test]
fn compute_patch_video_clip_set_blend_mode_multiply() {
    let prior = project_with_tracks_and_assets(vec![video_track(
        TRACK_VIDEO_A,
        "Video 1",
        false,
        CLIP_VIDEO_A,
        false,
        BlendMode::Normal,
    )]);

    let args = ClipSetBlendModeArgs {
        project_id: fixture_project_id(),
        clip: CLIP_VIDEO_A.to_string(),
        blend_mode: BlendMode::Multiply,
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("happy-path video");
    assert_eq!(patch_blend_mode_value(&patch), BlendMode::Multiply);
    assert!(warnings.is_empty());
    assert_eq!(data.blend_mode, BlendMode::Multiply);
    assert_eq!(data.clip_id.to_string(), CLIP_VIDEO_A);
    assert_eq!(
        patch.as_array().expect("patch is array")[0]
            .get("path")
            .and_then(Value::as_str),
        Some("/tracks/0/clips/0/blend_mode")
    );
}

#[test]
fn compute_patch_text_clip_set_blend_mode_screen() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
        BlendMode::Normal,
    )]);

    let args = ClipSetBlendModeArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_A.to_string(),
        blend_mode: BlendMode::Screen,
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("happy-path text");
    assert_eq!(patch_blend_mode_value(&patch), BlendMode::Screen);
    assert!(warnings.is_empty());
    assert_eq!(data.blend_mode, BlendMode::Screen);
}

#[test]
fn compute_patch_audio_clip_set_blend_mode_multiply_emits_inert_warning() {
    let prior = project_with_tracks_and_assets(vec![audio_track(
        TRACK_AUDIO_A,
        "Audio 1",
        false,
        CLIP_AUDIO_A,
        false,
        BlendMode::Normal,
    )]);

    let args = ClipSetBlendModeArgs {
        project_id: fixture_project_id(),
        clip: CLIP_AUDIO_A.to_string(),
        blend_mode: BlendMode::Multiply,
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("happy-path audio");
    assert_eq!(patch_blend_mode_value(&patch), BlendMode::Multiply);
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_BLEND_MODE_INERT_ON_AUDIO_CODE);
    assert_eq!(
        warnings[0]["message"],
        "blend_mode is inert on audio tracks (stored but ignored at render time)"
    );
    assert_eq!(warnings[0]["details"]["clip_id"], CLIP_AUDIO_A);
    assert_eq!(warnings[0]["details"]["blend_mode"], "multiply");
    assert_eq!(data.blend_mode, BlendMode::Multiply);
}

#[test]
fn compute_patch_audio_clip_set_blend_mode_noop_does_not_emit_inert_warning() {
    let prior = project_with_tracks_and_assets(vec![audio_track(
        TRACK_AUDIO_A,
        "Audio 1",
        false,
        CLIP_AUDIO_A,
        false,
        BlendMode::Normal,
    )]);

    let args = ClipSetBlendModeArgs {
        project_id: fixture_project_id(),
        clip: CLIP_AUDIO_A.to_string(),
        blend_mode: BlendMode::Normal,
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("no-op audio");
    assert!(patch.as_array().expect("patch is array").is_empty());
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_NOOP_CODE);
    assert_eq!(warnings[0]["message"], "clip blend_mode unchanged");
    assert_eq!(warnings[0]["details"]["clip_id"], CLIP_AUDIO_A);
    assert_eq!(warnings[0]["details"]["blend_mode"], "normal");
    assert_eq!(warnings.len(), 1);
    assert_eq!(data.blend_mode, BlendMode::Normal);
    assert_eq!(data.clip_id.to_string(), CLIP_AUDIO_A);
}

#[test]
fn compute_patch_set_blend_mode_noop() {
    let prior = project_with_tracks(vec![video_track(
        TRACK_VIDEO_A,
        "Video 1",
        false,
        CLIP_VIDEO_A,
        false,
        BlendMode::Overlay,
    )]);

    let args = ClipSetBlendModeArgs {
        project_id: fixture_project_id(),
        clip: CLIP_VIDEO_A.to_string(),
        blend_mode: BlendMode::Overlay,
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("no-op video");
    assert!(patch.as_array().expect("patch is array").is_empty());
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_NOOP_CODE);
    assert_eq!(warnings[0]["message"], "clip blend_mode unchanged");
    assert_eq!(warnings[0]["details"]["clip_id"], CLIP_VIDEO_A);
    assert_eq!(warnings[0]["details"]["blend_mode"], "overlay");
    assert_eq!(data.blend_mode, BlendMode::Overlay);
}

#[test]
fn compute_patch_set_blend_mode_locked_rejects() {
    let prior = project_with_tracks(vec![video_track(
        TRACK_VIDEO_A,
        "Video 1",
        false,
        CLIP_VIDEO_A,
        true,
        BlendMode::Normal,
    )]);

    let err = compute_patch(
        &prior,
        &ClipSetBlendModeArgs {
            project_id: fixture_project_id(),
            clip: CLIP_VIDEO_A.to_string(),
            blend_mode: BlendMode::Multiply,
        },
    )
    .expect_err("locked clip should reject");

    match err {
        ClipSetBlendModeError::Locked { clip_id } => assert_eq!(clip_id, CLIP_VIDEO_A),
        other => panic!("expected Locked, got {other:?}"),
    }
}

#[test]
fn compute_patch_set_blend_mode_locked_beats_inert_warning() {
    let prior = project_with_tracks_and_assets(vec![audio_track(
        TRACK_AUDIO_A,
        "Audio 1",
        false,
        CLIP_AUDIO_A,
        true,
        BlendMode::Normal,
    )]);

    let err = compute_patch(
        &prior,
        &ClipSetBlendModeArgs {
            project_id: fixture_project_id(),
            clip: CLIP_AUDIO_A.to_string(),
            blend_mode: BlendMode::Screen,
        },
    )
    .expect_err("locked should beat inert warning");

    assert!(matches!(err, ClipSetBlendModeError::Locked { .. }));
}

#[test]
fn compute_patch_set_blend_mode_track_lock_does_not_block() {
    let prior = project_with_tracks(vec![video_track(
        TRACK_VIDEO_A,
        "Video 1",
        true,
        CLIP_VIDEO_A,
        false,
        BlendMode::Normal,
    )]);

    let args = ClipSetBlendModeArgs {
        project_id: fixture_project_id(),
        clip: CLIP_VIDEO_A.to_string(),
        blend_mode: BlendMode::Difference,
    };

    let (patch, warnings, data) =
        compute_patch(&prior, &args).expect("track lock should not block clip.set_blend_mode");
    assert_eq!(patch_blend_mode_value(&patch), BlendMode::Difference);
    assert!(warnings.is_empty());
    assert_eq!(data.blend_mode, BlendMode::Difference);
}

#[test]
fn compute_patch_set_blend_mode_bad_selector() {
    let prior = project_with_tracks(vec![video_track(
        TRACK_VIDEO_A,
        "Video 1",
        false,
        CLIP_VIDEO_A,
        false,
        BlendMode::Normal,
    )]);

    let err = compute_patch(
        &prior,
        &ClipSetBlendModeArgs {
            project_id: fixture_project_id(),
            clip: "not-a-uuid".to_string(),
            blend_mode: BlendMode::Normal,
        },
    )
    .expect_err("bad clip selector must reject");

    match err {
        ClipSetBlendModeError::BadSelector { detail } => {
            assert!(detail.contains("UUID"), "{detail}");
        }
        other => panic!("expected BadSelector, got {other:?}"),
    }
}

#[test]
fn compute_patch_set_blend_mode_not_found() {
    let prior = project_with_tracks(vec![video_track(
        TRACK_VIDEO_A,
        "Video 1",
        false,
        CLIP_VIDEO_A,
        false,
        BlendMode::Normal,
    )]);

    let err = compute_patch(
        &prior,
        &ClipSetBlendModeArgs {
            project_id: fixture_project_id(),
            clip: MISSING_CLIP.to_string(),
            blend_mode: BlendMode::Normal,
        },
    )
    .expect_err("missing clip must reject");

    match err {
        ClipSetBlendModeError::ClipNotFound { clip_id } => assert_eq!(clip_id, MISSING_CLIP),
        other => panic!("expected ClipNotFound, got {other:?}"),
    }
}

#[test]
fn compute_patch_set_blend_mode_invalid_enum_is_schema_violation() {
    let prior = project_with_tracks(vec![video_track(
        TRACK_VIDEO_A,
        "Video 1",
        false,
        CLIP_VIDEO_A,
        false,
        BlendMode::Normal,
    )]);

    let verb = ClipSetBlendModeVerb;
    let bad_args = json!({
        "project_id": fixture_project_id().to_string(),
        "clip": CLIP_VIDEO_A,
        "blend_mode": "not-a-mode",
    });

    let err = verb
        .compute_patch(&prior, &bad_args)
        .expect_err("invalid blend mode should be BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn compute_patch_set_blend_mode_round_trip() {
    let prior = project_with_tracks_and_assets(vec![video_track(
        TRACK_VIDEO_A,
        "Video 1",
        false,
        CLIP_VIDEO_A,
        false,
        BlendMode::Normal,
    )]);
    let args = ClipSetBlendModeArgs {
        project_id: fixture_project_id(),
        clip: CLIP_VIDEO_A.to_string(),
        blend_mode: BlendMode::Multiply,
    };

    let (patch, _warnings, _data) = compute_patch(&prior, &args).expect("compute_patch ok");
    let typed_patch: json_patch::Patch =
        serde_json::from_value(patch.clone()).expect("patch parses to RFC 6902");
    let post_state = prior
        .apply(&typed_patch)
        .expect("clip.set_blend_mode patch should apply cleanly");

    let expected_data = serde_json::to_value(
        data_envelope_from_post_state(&args, &post_state)
            .expect("envelope from post-state should be readable"),
    )
    .expect("envelope serializes");

    let recorded = RecordedEvent {
        verb: "clip.set_blend_mode".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch,
        warnings: vec![],
        post_state,
        expected_data,
    };

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(ClipSetBlendModeVerb))
        .expect("register clip.set_blend_mode verb");

    let report = validate_reconstructors(&registry, std::slice::from_ref(&recorded))
        .expect("reconstructor validation should pass");
    assert_eq!(report.verbs_checked, vec!["clip.set_blend_mode"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn compute_patch_error_variants_map_to_bad_args() {
    let prior = project_with_tracks(vec![
        text_track(
            TRACK_TEXT_A,
            "Text 1",
            false,
            CLIP_TEXT_A,
            false,
            BlendMode::Normal,
        ),
        video_track(
            TRACK_VIDEO_A,
            "Video 1",
            false,
            CLIP_VIDEO_A,
            false,
            BlendMode::Normal,
        ),
        audio_track(
            TRACK_AUDIO_A,
            "Audio 1",
            false,
            CLIP_AUDIO_A,
            false,
            BlendMode::Normal,
        ),
    ]);
    let verb = ClipSetBlendModeVerb;

    let bad_selector = serde_json::to_value(ClipSetBlendModeArgs {
        project_id: fixture_project_id(),
        clip: "not-a-uuid".to_string(),
        blend_mode: BlendMode::Normal,
    })
    .expect("bad selector args serialize");
    let err = verb
        .compute_patch(&prior, &bad_selector)
        .expect_err("bad selector maps");
    assert!(matches!(err, VerbError::BadArgs { .. }));

    let clip_not_found = serde_json::to_value(ClipSetBlendModeArgs {
        project_id: fixture_project_id(),
        clip: MISSING_CLIP.to_string(),
        blend_mode: BlendMode::Normal,
    })
    .expect("missing clip args serialize");
    let err = verb
        .compute_patch(&prior, &clip_not_found)
        .expect_err("missing clip maps");
    assert!(matches!(err, VerbError::BadArgs { .. }));

    let locked = serde_json::to_value(ClipSetBlendModeArgs {
        project_id: fixture_project_id(),
        clip: CLIP_VIDEO_A.to_string(),
        blend_mode: BlendMode::Multiply,
    })
    .expect("locked args serialize");
    let locked_state = project_with_tracks(vec![video_track(
        TRACK_VIDEO_A,
        "Video 1",
        false,
        CLIP_VIDEO_A,
        true,
        BlendMode::Normal,
    )]);
    let err = verb
        .compute_patch(&locked_state, &locked)
        .expect_err("locked maps");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "clip.set_blend_mode")
        .expect("default_fixtures includes clip.set_blend_mode");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(ClipSetBlendModeVerb))
        .expect("register clip.set_blend_mode verb");

    let report = validate_reconstructors(&registry, &[fixture])
        .expect("clip.set_blend_mode reconstruct from fixture");
    assert_eq!(report.verbs_checked, vec!["clip.set_blend_mode"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn data_envelope_from_post_state_returns_post_blend_mode() {
    let post_state = project_with_tracks(vec![video_track(
        TRACK_VIDEO_A,
        "Video 1",
        false,
        CLIP_VIDEO_A,
        false,
        BlendMode::ColorBurn,
    )]);
    let args = ClipSetBlendModeArgs {
        project_id: fixture_project_id(),
        clip: CLIP_VIDEO_A.to_string(),
        blend_mode: BlendMode::Normal,
    };

    let data = data_envelope_from_post_state(&args, &post_state)
        .expect("post-state envelope should be readable");
    assert_eq!(data.blend_mode, BlendMode::ColorBurn);
    assert_eq!(data.clip_id.to_string(), CLIP_VIDEO_A);
}

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let base_project = project_with_tracks_and_assets(vec![video_track(
        TRACK_VIDEO_A,
        "Video 1",
        false,
        CLIP_VIDEO_A,
        false,
        BlendMode::Normal,
    )]);
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        base_project,
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry clears the gate and writes project.json");

    let outcome = store
        .mutate_via_verb(
            "clip.set_blend_mode",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "clip": CLIP_VIDEO_A,
                "blend_mode": "screen",
            }),
            None,
        )
        .expect("mutate_via_verb happy path");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("happy path must return Applied, got {outcome:?}");
    };

    let data: ClipSetBlendModeData =
        serde_json::from_value(data).expect("clip.set_blend_mode data is ClipSetBlendModeData");
    assert_eq!(data.clip_id.to_string(), CLIP_VIDEO_A);
    assert_eq!(
        store.project().tracks[0].clips[0].blend_mode,
        BlendMode::Screen
    );
    assert_eq!(data.blend_mode, BlendMode::Screen);
    assert_eq!(warnings, Vec::<Value>::new());
}

#[test]
fn multi_track_clip_resolution_uses_track_index() {
    let prior = project_with_tracks(vec![
        text_track(
            TRACK_TEXT_A,
            "Text 1",
            false,
            CLIP_TEXT_A,
            false,
            BlendMode::Normal,
        ),
        video_track(
            TRACK_VIDEO_A,
            "Video 1",
            false,
            CLIP_VIDEO_A,
            false,
            BlendMode::Normal,
        ),
    ]);

    let args = ClipSetBlendModeArgs {
        project_id: fixture_project_id(),
        clip: CLIP_VIDEO_A.to_string(),
        blend_mode: BlendMode::Overlay,
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("search in second track");
    assert_eq!(patch_blend_mode_value(&patch), BlendMode::Overlay);
    assert!(warnings.is_empty());
    assert_eq!(data.clip_id.to_string(), CLIP_VIDEO_A);
    assert_eq!(data.blend_mode, BlendMode::Overlay);
    assert_eq!(
        patch.as_array().expect("patch is array")[0]
            .get("path")
            .and_then(Value::as_str),
        Some("/tracks/1/clips/0/blend_mode")
    );
}

#[test]
fn compute_patch_all_blend_modes_accepted() {
    let prior = project_with_tracks(vec![video_track(
        TRACK_VIDEO_A,
        "Video 1",
        false,
        CLIP_VIDEO_A,
        false,
        BlendMode::Normal,
    )]);

    let variants = [
        BlendMode::Normal,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::Overlay,
        BlendMode::SoftLight,
        BlendMode::HardLight,
        BlendMode::Darken,
        BlendMode::Lighten,
        BlendMode::Difference,
        BlendMode::ColorDodge,
        BlendMode::ColorBurn,
    ];

    for (idx, blend_mode) in variants.iter().enumerate() {
        let args = ClipSetBlendModeArgs {
            project_id: fixture_project_id(),
            clip: CLIP_VIDEO_A.to_string(),
            blend_mode: *blend_mode,
        };

        let (patch, warnings, data) =
            compute_patch(&prior, &args).expect("all valid blend modes are accepted");
        assert_eq!(data.blend_mode, *blend_mode);

        if idx == 0 {
            assert!(patch.as_array().expect("patch is array").is_empty());
            assert_eq!(warnings.len(), 1);
            assert_eq!(warnings[0]["code"], W_NOOP_CODE);
            continue;
        }

        assert_eq!(patch_blend_mode_value(&patch), *blend_mode);
        assert!(warnings.is_empty());
    }
}

#[test]
fn data_envelope_blend_mode_serializes_as_kebab_case() {
    let prior = project_with_tracks_and_assets(vec![video_track(
        TRACK_VIDEO_A,
        "Video 1",
        false,
        CLIP_VIDEO_A,
        false,
        BlendMode::Normal,
    )]);
    let args = ClipSetBlendModeArgs {
        project_id: fixture_project_id(),
        clip: CLIP_VIDEO_A.to_string(),
        blend_mode: BlendMode::ColorDodge,
    };

    let (patch, _warnings, _data) = compute_patch(&prior, &args).expect("color-dodge");
    let typed_patch: json_patch::Patch =
        serde_json::from_value(patch).expect("patch parses to RFC 6902");
    let post_state = prior.apply(&typed_patch).expect("patch should apply");
    let data = data_envelope_from_post_state(&args, &post_state)
        .expect("post-state envelope should be readable");
    let value = serde_json::to_value(data).expect("envelope serializes");
    assert_eq!(value["blend_mode"], "color-dodge");

    let args = ClipSetBlendModeArgs {
        project_id: fixture_project_id(),
        clip: CLIP_VIDEO_A.to_string(),
        blend_mode: BlendMode::SoftLight,
    };
    let (patch, _warnings, _data) = compute_patch(&prior, &args).expect("soft-light");
    let typed_patch: json_patch::Patch =
        serde_json::from_value(patch).expect("patch parses to RFC 6902");
    let post_state = prior.apply(&typed_patch).expect("patch should apply");
    let data = data_envelope_from_post_state(&args, &post_state)
        .expect("post-state envelope should be readable");
    let value = serde_json::to_value(data).expect("envelope serializes");
    assert_eq!(value["blend_mode"], "soft-light");
}
