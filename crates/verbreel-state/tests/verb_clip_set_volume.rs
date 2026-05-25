//! Tests for `clip.set_volume` (§5.11) — twenty-first production verb.

use serde_json::{Value, json};
use std::sync::Arc;

use verbreel_state::verbs::clip_set_volume::{
    W_NOOP_CODE, compute_patch, data_envelope_from_post_state,
};
use verbreel_state::{
    ClipSetVolumeArgs, ClipSetVolumeData, ClipSetVolumeError, ClipSetVolumeVerb, MutateOutcome,
    Project, RecordedEvent, Track, TrackKind, Verb, VerbError, VerbRegistry, default_fixtures,
    default_registry, validate_reconstructors,
};
use verbreel_types::{ProjectId, Tick};

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const TRACK_TEXT_A: &str = "0190b8d3-15e3-7000-bd00-0000000aa101";
const TRACK_VIDEO_A: &str = "0190b8d3-15e3-7000-bd00-0000000aa201";
const TRACK_AUDIO_A: &str = "0190b8d3-15e3-7000-bd00-0000000aa301";
const CLIP_TEXT_A: &str = "0190b8d3-15e3-7000-bd00-0000000bb101";
const CLIP_VIDEO_A: &str = "0190b8d3-15e3-7000-bd00-0000000bb201";
const CLIP_AUDIO_A: &str = "0190b8d3-15e3-7000-bd00-0000000bb301";
const MISSING_CLIP: &str = "0190b8d3-15e3-7000-bd00-0000000cc101";
const ASSET_AUDIO_ID: &str = "0190b8d3-15e3-7000-bd00-0000000dd101";
const ASSET_VIDEO_ID: &str = "0190b8d3-15e3-7000-bd00-0000000dd102";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture → Project")
}

fn fixture_project_id() -> ProjectId {
    empty_project().id
}

fn audio_track(
    id: &str,
    name: &str,
    track_locked: bool,
    clip_id: &str,
    clip_locked: bool,
    volume: f64,
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
            "volume": volume,
            "locked": clip_locked,
        }],
    }))
    .expect("audio track fixture parses")
}

fn video_track(
    id: &str,
    name: &str,
    track_locked: bool,
    clip_id: &str,
    clip_locked: bool,
    volume: f64,
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
            "source_out_tk": 240_000,
            "volume": volume,
            "locked": clip_locked,
        }],
    }))
    .expect("video track fixture parses")
}

fn text_track(
    id: &str,
    name: &str,
    track_locked: bool,
    clip_id: &str,
    clip_locked: bool,
    volume: f64,
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
            "source_out_tk": 240_000,
            "volume": volume,
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

fn project_with_tracks(tracks: Vec<Track>) -> Project {
    let mut project = empty_project();
    project.tracks = tracks;
    project.duration_tk = Tick::new(240_000);
    project
}

fn audio_project_with_assets() -> Project {
    let mut project = project_with_tracks(vec![audio_track(
        TRACK_AUDIO_A,
        "Audio 1",
        false,
        CLIP_AUDIO_A,
        false,
        1.0,
    )]);

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

    project.assets.push(
        serde_json::from_value(json!({
            "id": ASSET_VIDEO_ID,
            "kind": "video",
            "hash": "53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658",
            "path": "assets/53/53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.mp4",
            "original_filename": "video1.mp4",
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
        .expect("video asset fixture parses"),
    );

    project
}

fn patch_volume_value(patch: &Value) -> f64 {
    let arr = patch.as_array().expect("patch is an array");
    assert_eq!(arr.len(), 1, "clip.set_volume emits one replace op");
    let op = arr[0].as_object().expect("patch op is object");
    assert_eq!(op.get("op").and_then(Value::as_str), Some("replace"));
    op.get("value")
        .and_then(Value::as_f64)
        .expect("replace op value is f64")
}

#[test]
fn compute_patch_audio_clip_set_volume_1_5() {
    let prior = project_with_tracks(vec![audio_track(
        TRACK_AUDIO_A,
        "Audio 1",
        false,
        CLIP_AUDIO_A,
        false,
        1.0,
    )]);

    let args = ClipSetVolumeArgs {
        project_id: fixture_project_id(),
        clip: CLIP_AUDIO_A.to_string(),
        volume: 1.5,
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("happy-path volume");
    assert_eq!(patch_volume_value(&patch), 1.5);
    assert!(warnings.is_empty());
    assert_eq!(data.clip_id.to_string(), CLIP_AUDIO_A);
    assert_eq!(data.volume, 1.5);
    assert_eq!(
        patch.as_array().expect("patch is array")[0]
            .get("path")
            .and_then(Value::as_str),
        Some("/tracks/0/clips/0/volume")
    );
}

#[test]
fn compute_patch_set_volume_boundary_low() {
    let prior = project_with_tracks(vec![audio_track(
        TRACK_AUDIO_A,
        "Audio 1",
        false,
        CLIP_AUDIO_A,
        false,
        1.0,
    )]);

    let args = ClipSetVolumeArgs {
        project_id: fixture_project_id(),
        clip: CLIP_AUDIO_A.to_string(),
        volume: 0.0,
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("boundary low");
    assert_eq!(patch_volume_value(&patch), 0.0);
    assert!(warnings.is_empty());
    assert_eq!(data.clip_id.to_string(), CLIP_AUDIO_A);
    assert_eq!(data.volume, 0.0);
}

#[test]
fn compute_patch_set_volume_boundary_high() {
    let prior = project_with_tracks(vec![audio_track(
        TRACK_AUDIO_A,
        "Audio 1",
        false,
        CLIP_AUDIO_A,
        false,
        1.0,
    )]);

    let args = ClipSetVolumeArgs {
        project_id: fixture_project_id(),
        clip: CLIP_AUDIO_A.to_string(),
        volume: 4.0,
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("boundary high");
    assert_eq!(patch_volume_value(&patch), 4.0);
    assert!(warnings.is_empty());
    assert_eq!(data.clip_id.to_string(), CLIP_AUDIO_A);
    assert_eq!(data.volume, 4.0);
}

#[test]
fn compute_patch_set_volume_noop() {
    let prior = project_with_tracks(vec![audio_track(
        TRACK_AUDIO_A,
        "Audio 1",
        false,
        CLIP_AUDIO_A,
        false,
        1.5,
    )]);

    let args = ClipSetVolumeArgs {
        project_id: fixture_project_id(),
        clip: CLIP_AUDIO_A.to_string(),
        volume: 1.5,
    };

    let (patch, warnings, data) =
        compute_patch(&prior, &args).expect("same-volume set should be a no-op");
    assert!(patch.as_array().expect("patch is array").is_empty());
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_NOOP_CODE);
    assert_eq!(warnings[0]["message"], "clip volume unchanged");
    assert_eq!(warnings[0]["details"]["clip_id"], CLIP_AUDIO_A);
    assert_eq!(warnings[0]["details"]["volume"], 1.5);
    assert_eq!(data.clip_id.to_string(), CLIP_AUDIO_A);
    assert_eq!(data.volume, 1.5);
}

#[test]
fn compute_patch_set_volume_bad_range_below_zero() {
    let prior = project_with_tracks(vec![audio_track(
        TRACK_AUDIO_A,
        "Audio 1",
        false,
        CLIP_AUDIO_A,
        false,
        1.0,
    )]);

    let err = compute_patch(
        &prior,
        &ClipSetVolumeArgs {
            project_id: fixture_project_id(),
            clip: CLIP_AUDIO_A.to_string(),
            volume: -0.1,
        },
    )
    .expect_err("below-range must reject");

    match err {
        ClipSetVolumeError::BadRange { value } => assert_eq!(value, -0.1),
        other => panic!("expected BadRange, got {other:?}"),
    }
}

#[test]
fn compute_patch_set_volume_bad_range_above_four() {
    let prior = project_with_tracks(vec![audio_track(
        TRACK_AUDIO_A,
        "Audio 1",
        false,
        CLIP_AUDIO_A,
        false,
        1.0,
    )]);

    let err = compute_patch(
        &prior,
        &ClipSetVolumeArgs {
            project_id: fixture_project_id(),
            clip: CLIP_AUDIO_A.to_string(),
            volume: 4.1,
        },
    )
    .expect_err("above-range must reject");

    match err {
        ClipSetVolumeError::BadRange { value } => assert_eq!(value, 4.1),
        other => panic!("expected BadRange, got {other:?}"),
    }
}

#[test]
fn compute_patch_set_volume_bad_range_nan() {
    let prior = project_with_tracks(vec![audio_track(
        TRACK_AUDIO_A,
        "Audio 1",
        false,
        CLIP_AUDIO_A,
        false,
        1.0,
    )]);

    let err = compute_patch(
        &prior,
        &ClipSetVolumeArgs {
            project_id: fixture_project_id(),
            clip: CLIP_AUDIO_A.to_string(),
            volume: f64::NAN,
        },
    )
    .expect_err("NaN must reject");

    match err {
        ClipSetVolumeError::BadRange { value } => assert!(value.is_nan()),
        other => panic!("expected BadRange, got {other:?}"),
    }
}

#[test]
fn compute_patch_set_volume_bad_range_infinity() {
    let prior = project_with_tracks(vec![audio_track(
        TRACK_AUDIO_A,
        "Audio 1",
        false,
        CLIP_AUDIO_A,
        false,
        1.0,
    )]);

    let err = compute_patch(
        &prior,
        &ClipSetVolumeArgs {
            project_id: fixture_project_id(),
            clip: CLIP_AUDIO_A.to_string(),
            volume: f64::INFINITY,
        },
    )
    .expect_err("infinity must reject");

    match err {
        ClipSetVolumeError::BadRange { value } => {
            assert!(value.is_infinite() && value.is_sign_positive())
        }
        other => panic!("expected BadRange, got {other:?}"),
    }
}

#[test]
fn compute_patch_set_volume_bad_range_neg_infinity() {
    let prior = project_with_tracks(vec![audio_track(
        TRACK_AUDIO_A,
        "Audio 1",
        false,
        CLIP_AUDIO_A,
        false,
        1.0,
    )]);

    let err = compute_patch(
        &prior,
        &ClipSetVolumeArgs {
            project_id: fixture_project_id(),
            clip: CLIP_AUDIO_A.to_string(),
            volume: f64::NEG_INFINITY,
        },
    )
    .expect_err("-infinity must reject");

    match err {
        ClipSetVolumeError::BadRange { value } => {
            assert!(value.is_infinite() && value.is_sign_negative())
        }
        other => panic!("expected BadRange, got {other:?}"),
    }
}

#[test]
fn compute_patch_set_volume_rejects_video_clips() {
    let prior = project_with_tracks(vec![video_track(
        TRACK_VIDEO_A,
        "Video 1",
        false,
        CLIP_VIDEO_A,
        false,
        1.0,
    )]);

    let err = compute_patch(
        &prior,
        &ClipSetVolumeArgs {
            project_id: fixture_project_id(),
            clip: CLIP_VIDEO_A.to_string(),
            volume: 1.5,
        },
    )
    .expect_err("video clip should reject");

    match err {
        ClipSetVolumeError::KindMismatch {
            clip_id,
            found_kind,
        } => {
            assert_eq!(clip_id, CLIP_VIDEO_A);
            assert_eq!(found_kind, TrackKind::Video);
        }
        other => panic!("expected KindMismatch, got {other:?}"),
    }
}

#[test]
fn compute_patch_set_volume_rejects_text_clips() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
        1.0,
    )]);

    let err = compute_patch(
        &prior,
        &ClipSetVolumeArgs {
            project_id: fixture_project_id(),
            clip: CLIP_TEXT_A.to_string(),
            volume: 1.5,
        },
    )
    .expect_err("text clip should reject");

    match err {
        ClipSetVolumeError::KindMismatch {
            clip_id,
            found_kind,
        } => {
            assert_eq!(clip_id, CLIP_TEXT_A);
            assert_eq!(found_kind, TrackKind::Text);
        }
        other => panic!("expected KindMismatch, got {other:?}"),
    }
}

#[test]
fn compute_patch_set_volume_rejects_image_clips_if_supported() {
    // Image tracks are not represented in this model slice, so this
    // assertion is intentionally skipped until TrackKind::Image exists.
    // See §5.11 test matrix requirement.
}

#[test]
fn compute_patch_set_volume_kind_mismatch_beats_lock_and_range_checks() {
    let prior = project_with_tracks(vec![video_track(
        TRACK_VIDEO_A,
        "Video 1",
        false,
        CLIP_VIDEO_A,
        true,
        1.0,
    )]);

    let err = compute_patch(
        &prior,
        &ClipSetVolumeArgs {
            project_id: fixture_project_id(),
            clip: CLIP_VIDEO_A.to_string(),
            volume: 9.0,
        },
    )
    .expect_err("kind mismatch should beat lock and range checks");

    match err {
        ClipSetVolumeError::KindMismatch {
            clip_id,
            found_kind,
        } => {
            assert_eq!(clip_id, CLIP_VIDEO_A);
            assert_eq!(found_kind, TrackKind::Video);
        }
        other => panic!("expected KindMismatch, got {other:?}"),
    }
}

#[test]
fn compute_patch_set_volume_locked_rejects() {
    let prior = project_with_tracks(vec![audio_track(
        TRACK_AUDIO_A,
        "Audio 1",
        false,
        CLIP_AUDIO_A,
        true,
        1.0,
    )]);

    let err = compute_patch(
        &prior,
        &ClipSetVolumeArgs {
            project_id: fixture_project_id(),
            clip: CLIP_AUDIO_A.to_string(),
            volume: 1.5,
        },
    )
    .expect_err("locked clip should reject");

    match err {
        ClipSetVolumeError::Locked { clip_id } => assert_eq!(clip_id, CLIP_AUDIO_A),
        other => panic!("expected Locked, got {other:?}"),
    }
}

#[test]
fn compute_patch_set_volume_locked_beats_range_check() {
    let prior = project_with_tracks(vec![audio_track(
        TRACK_AUDIO_A,
        "Audio 1",
        false,
        CLIP_AUDIO_A,
        true,
        1.0,
    )]);

    let err = compute_patch(
        &prior,
        &ClipSetVolumeArgs {
            project_id: fixture_project_id(),
            clip: CLIP_AUDIO_A.to_string(),
            volume: 9.0,
        },
    )
    .expect_err("locked should beat bad range check");

    assert!(matches!(err, ClipSetVolumeError::Locked { .. }));
}

#[test]
fn compute_patch_set_volume_track_lock_does_not_block() {
    let prior = project_with_tracks(vec![audio_track(
        TRACK_AUDIO_A,
        "Audio 1",
        true,
        CLIP_AUDIO_A,
        false,
        1.0,
    )]);

    let args = ClipSetVolumeArgs {
        project_id: fixture_project_id(),
        clip: CLIP_AUDIO_A.to_string(),
        volume: 1.5,
    };

    let (patch, warnings, data) =
        compute_patch(&prior, &args).expect("track lock should not block clip.set_volume");
    assert_eq!(patch_volume_value(&patch), 1.5);
    assert!(warnings.is_empty());
    assert_eq!(data.clip_id.to_string(), CLIP_AUDIO_A);
    assert_eq!(data.volume, 1.5);
}

#[test]
fn compute_patch_set_volume_bad_selector() {
    let prior = project_with_tracks(vec![audio_track(
        TRACK_AUDIO_A,
        "Audio 1",
        false,
        CLIP_AUDIO_A,
        false,
        1.0,
    )]);

    let err = compute_patch(
        &prior,
        &ClipSetVolumeArgs {
            project_id: fixture_project_id(),
            clip: "not-a-uuid".to_string(),
            volume: 1.5,
        },
    )
    .expect_err("bad clip selector must reject");

    match err {
        ClipSetVolumeError::BadSelector { detail } => {
            assert!(detail.contains("UUID"), "{detail}");
        }
        other => panic!("expected BadSelector, got {other:?}"),
    }
}

#[test]
fn compute_patch_set_volume_not_found() {
    let prior = project_with_tracks(vec![audio_track(
        TRACK_AUDIO_A,
        "Audio 1",
        false,
        CLIP_AUDIO_A,
        false,
        1.0,
    )]);

    let err = compute_patch(
        &prior,
        &ClipSetVolumeArgs {
            project_id: fixture_project_id(),
            clip: MISSING_CLIP.to_string(),
            volume: 1.5,
        },
    )
    .expect_err("missing clip must reject");

    match err {
        ClipSetVolumeError::ClipNotFound { clip_id } => assert_eq!(clip_id, MISSING_CLIP),
        other => panic!("expected ClipNotFound, got {other:?}"),
    }
}

#[test]
fn reconstructor_round_trip() {
    let prior = audio_project_with_assets();
    let args = ClipSetVolumeArgs {
        project_id: fixture_project_id(),
        clip: CLIP_AUDIO_A.to_string(),
        volume: 1.5,
    };

    let (patch, _warnings, _data) = compute_patch(&prior, &args).expect("compute_patch ok");
    let typed_patch: json_patch::Patch =
        serde_json::from_value(patch.clone()).expect("patch parses to typed JSON patch");
    let post_state = prior
        .apply(&typed_patch)
        .expect("clip.set_volume patch should apply cleanly");

    let expected_data = serde_json::to_value(
        data_envelope_from_post_state(&args, &post_state).expect("envelope from post-state"),
    )
    .expect("envelope serializes");

    let recorded = RecordedEvent {
        verb: "clip.set_volume".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch,
        warnings: vec![],
        post_state,
        expected_data,
    };

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(ClipSetVolumeVerb))
        .expect("register clip.set_volume verb");

    let report = validate_reconstructors(&registry, std::slice::from_ref(&recorded))
        .expect("reconstructor validation should pass");
    assert_eq!(report.verbs_checked, vec!["clip.set_volume"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn compute_patch_error_variants_map_to_bad_args() {
    let prior = project_with_tracks(vec![
        text_track(TRACK_TEXT_A, "Text 1", false, CLIP_TEXT_A, false, 1.0),
        audio_track(TRACK_AUDIO_A, "Audio 1", false, CLIP_AUDIO_A, false, 1.0),
    ]);
    let verb = ClipSetVolumeVerb;

    let bad_selector = serde_json::to_value(ClipSetVolumeArgs {
        project_id: fixture_project_id(),
        clip: "not-a-uuid".to_string(),
        volume: 1.5,
    })
    .expect("bad selector args serialize");
    let err = verb
        .compute_patch(&prior, &bad_selector)
        .expect_err("bad selector maps");
    assert!(matches!(err, VerbError::BadArgs { .. }));

    let clip_not_found = serde_json::to_value(ClipSetVolumeArgs {
        project_id: fixture_project_id(),
        clip: MISSING_CLIP.to_string(),
        volume: 1.5,
    })
    .expect("missing clip args serialize");
    let err = verb
        .compute_patch(&prior, &clip_not_found)
        .expect_err("clip not found maps");
    assert!(matches!(err, VerbError::BadArgs { .. }));

    let kind_mismatch = serde_json::to_value(ClipSetVolumeArgs {
        project_id: fixture_project_id(),
        clip: CLIP_VIDEO_A.to_string(),
        volume: 1.5,
    })
    .expect("kind mismatch args serialize");
    let prior_video = project_with_tracks(vec![video_track(
        TRACK_VIDEO_A,
        "Video 1",
        false,
        CLIP_VIDEO_A,
        false,
        1.0,
    )]);
    let err = verb
        .compute_patch(&prior_video, &kind_mismatch)
        .expect_err("kind mismatch maps");
    assert!(matches!(err, VerbError::BadArgs { .. }));

    let locked = serde_json::to_value(ClipSetVolumeArgs {
        project_id: fixture_project_id(),
        clip: CLIP_AUDIO_A.to_string(),
        volume: 1.5,
    })
    .expect("locked args serialize");
    let prior_locked = project_with_tracks(vec![audio_track(
        TRACK_AUDIO_A,
        "Audio 1",
        false,
        CLIP_AUDIO_A,
        true,
        1.0,
    )]);
    let err = verb
        .compute_patch(&prior_locked, &locked)
        .expect_err("locked maps");
    assert!(matches!(err, VerbError::BadArgs { .. }));

    let bad_range = serde_json::to_value(ClipSetVolumeArgs {
        project_id: fixture_project_id(),
        clip: CLIP_AUDIO_A.to_string(),
        volume: 4.1,
    })
    .expect("bad range args serialize");
    let err = verb
        .compute_patch(&prior, &bad_range)
        .expect_err("bad range maps");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "clip.set_volume")
        .expect("default_fixtures includes clip.set_volume");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(ClipSetVolumeVerb))
        .expect("register clip.set_volume verb");

    let report = validate_reconstructors(&registry, &[fixture])
        .expect("clip.set_volume reconstruct from fixture");
    assert_eq!(report.verbs_checked, vec!["clip.set_volume"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn data_envelope_from_post_state_returns_post_volume() {
    let post_state = project_with_tracks(vec![audio_track(
        TRACK_AUDIO_A,
        "Audio 1",
        false,
        CLIP_AUDIO_A,
        false,
        0.75,
    )]);
    let args = ClipSetVolumeArgs {
        project_id: fixture_project_id(),
        clip: CLIP_AUDIO_A.to_string(),
        volume: 0.5,
    };

    let data = data_envelope_from_post_state(&args, &post_state)
        .expect("post-state envelope should be readable");
    assert_eq!(data.clip_id.to_string(), CLIP_AUDIO_A);
    assert_eq!(data.volume, 0.75);
}

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        audio_project_with_assets(),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry clears the gate and writes project.json");

    let outcome = store
        .mutate_via_verb(
            "clip.set_volume",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "clip": CLIP_AUDIO_A,
                "volume": 0.5,
            }),
            None,
        )
        .expect("mutate_via_verb happy path");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("happy path must return Applied, got {outcome:?}");
    };

    let data: ClipSetVolumeData =
        serde_json::from_value(data).expect("clip.set_volume data is ClipSetVolumeData");
    assert_eq!(data.clip_id.to_string(), CLIP_AUDIO_A);
    assert_eq!(store.project().tracks[0].clips[0].volume, 0.5);
    assert_eq!(data.volume, 0.5);
    assert_eq!(warnings, Vec::<Value>::new());
}
