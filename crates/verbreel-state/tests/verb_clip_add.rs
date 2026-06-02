//! Tests for `clip.add` (§5.1) — fifty-sixth production verb.
//!
//! This slice ships the basic verb only: no auto-pair audio (§5.15),
//! no edge snap (§0.17), no structural selectors. See the verb's module
//! docs for the deferred-scope list.

use serde_json::{Value, json};
use std::sync::Arc;

use verbreel_state::verbs::clip_add::{
    ClipAddArgs, ClipAddData, ClipAddError, ClipAddVerb, W_CLIP_ADD_ENVELOPE_CODE,
    W_TIME_SNAPPED_CODE, compute_patch, data_envelope_from_warnings,
};
use verbreel_state::{
    Project, RecordedEvent, Track, TrackKind, Verb, VerbRegistry, default_fixtures,
    default_registry, validate_reconstructors,
};
use verbreel_types::{ProjectId, Tick};

#[cfg(feature = "native")]
use verbreel_state::{MutateOutcome, ProjectStore};

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";

const TRACK_VIDEO_A: &str = "0190b8d3-15e3-7000-bd00-0000000aa901";
const TRACK_AUDIO_A: &str = "0190b8d3-15e3-7000-bd00-0000000aa902";
const TRACK_TEXT_A: &str = "0190b8d3-15e3-7000-bd00-0000000aa903";
const MISSING_TRACK: &str = "0190b8d3-15e3-7000-bd00-0000000aa9ff";

const ASSET_VIDEO_NO_AUDIO: &str = "0190b8d3-15e3-7000-bd00-0000000dd901";
const ASSET_VIDEO_WITH_AUDIO: &str = "0190b8d3-15e3-7000-bd00-0000000dd902";
const ASSET_AUDIO: &str = "0190b8d3-15e3-7000-bd00-0000000dd903";
const ASSET_IMAGE: &str = "0190b8d3-15e3-7000-bd00-0000000dd904";
const ASSET_SUBTITLE: &str = "0190b8d3-15e3-7000-bd00-0000000dd905";
const ASSET_MISSING: &str = "0190b8d3-15e3-7000-bd00-0000000dd9ff";

const CLIP_EXISTING: &str = "0190b8d3-15e3-7000-bd00-0000000bb901";

const VIDEO_NO_AUDIO_PATH: &str =
    "assets/36/36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da.mp4";
const VIDEO_WITH_AUDIO_PATH: &str =
    "assets/53/53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.mov";
const AUDIO_PATH: &str =
    "assets/64/64ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.m4a";
const IMAGE_PATH: &str =
    "assets/75/75ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.png";
const SUBTITLE_PATH: &str =
    "assets/86/86ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.srt";

const ASSET_DURATION_TK: i64 = 240_000;

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture -> Project")
}

fn fixture_project_id() -> ProjectId {
    FIXTURE_PROJECT_ID
        .parse()
        .expect("fixture project id parses")
}

fn video_track(id: &str, name: &str, locked: bool, clips: Vec<Value>) -> Track {
    serde_json::from_value(json!({
        "id": id,
        "kind": TrackKind::Video,
        "name": name,
        "locked": locked,
        "clips": clips,
    }))
    .expect("video track parses")
}

fn audio_track(id: &str, name: &str, locked: bool, clips: Vec<Value>) -> Track {
    serde_json::from_value(json!({
        "id": id,
        "kind": TrackKind::Audio,
        "name": name,
        "locked": locked,
        "clips": clips,
    }))
    .expect("audio track parses")
}

fn text_track(id: &str, name: &str) -> Track {
    serde_json::from_value(json!({
        "id": id,
        "kind": TrackKind::Text,
        "name": name,
        "locked": false,
        "clips": [],
    }))
    .expect("text track parses")
}

fn video_asset(id: &str, path: &str, with_audio: bool) -> Value {
    let mut metadata = json!({
        "duration_tk": ASSET_DURATION_TK,
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
    });
    if with_audio {
        metadata["audio_codec"] = json!("aac");
        metadata["audio_channels"] = json!(2);
        metadata["audio_sample_rate_hz"] = json!(48_000);
    }
    json!({
        "id": id,
        "kind": "video",
        "hash": path.split('/').nth(2).expect("hash segment present")
            .split('.').next().expect("ext stripped"),
        "path": path,
        "original_filename": path.rsplit('/').next().unwrap(),
        "imported_at": "2026-05-24T00:00:00Z",
        "metadata": metadata,
    })
}

fn audio_asset(id: &str, path: &str) -> Value {
    json!({
        "id": id,
        "kind": "audio",
        "hash": path.split('/').nth(2).expect("hash segment present")
            .split('.').next().expect("ext stripped"),
        "path": path,
        "original_filename": path.rsplit('/').next().unwrap(),
        "imported_at": "2026-05-24T00:00:00Z",
        "metadata": {
            "duration_tk": ASSET_DURATION_TK,
            "audio_codec": "aac",
            "audio_channels": 2,
            "audio_sample_rate_hz": 48_000,
            "container": "m4a",
            "fingerprint": {
                "mtime_ms": 1_700_000_000_000_i64,
                "size_bytes": 1024,
            }
        }
    })
}

fn image_asset(id: &str, path: &str) -> Value {
    json!({
        "id": id,
        "kind": "image",
        "hash": path.split('/').nth(2).expect("hash segment present")
            .split('.').next().expect("ext stripped"),
        "path": path,
        "original_filename": path.rsplit('/').next().unwrap(),
        "imported_at": "2026-05-24T00:00:00Z",
        "metadata": {
            "width": 3840,
            "height": 2160,
            "container": "png",
            "fingerprint": {
                "mtime_ms": 1_700_000_000_000_i64,
                "size_bytes": 1024,
            }
        }
    })
}

fn subtitle_asset(id: &str, path: &str) -> Value {
    json!({
        "id": id,
        "kind": "subtitle",
        "hash": path.split('/').nth(2).expect("hash segment present")
            .split('.').next().expect("ext stripped"),
        "path": path,
        "original_filename": path.rsplit('/').next().unwrap(),
        "imported_at": "2026-05-24T00:00:00Z",
        "metadata": {
            "container": "srt",
            "language": "en",
            "fingerprint": {
                "mtime_ms": 1_700_000_000_000_i64,
                "size_bytes": 1024,
            }
        }
    })
}

fn project_with(tracks: Vec<Track>, assets: Vec<Value>, duration_tk: i64) -> Project {
    let mut project = empty_project();
    project.tracks = tracks;
    project.assets = assets
        .into_iter()
        .map(|v| serde_json::from_value(v).expect("asset parses"))
        .collect();
    project.duration_tk = Tick::new(duration_tk);
    project
}

fn happy_args() -> ClipAddArgs {
    ClipAddArgs {
        project_id: fixture_project_id(),
        asset_id: ASSET_VIDEO_NO_AUDIO.to_string(),
        track: TRACK_VIDEO_A.to_string(),
        track_position_tk: 0,
        source_in_tk: None,
        source_out_tk: None,
        name: None,
    }
}

fn apply_patch(prior: &Project, patch: Value) -> Project {
    let patch: json_patch::Patch = serde_json::from_value(patch).expect("patch parses");
    prior.apply(&patch).expect("patch applies")
}

fn last_clip(project: &Project, track_idx: usize) -> &verbreel_state::Clip {
    project.tracks[track_idx]
        .clips
        .last()
        .expect("track has at least one clip")
}

// ---------------------------------------------------------------------
// Happy paths
// ---------------------------------------------------------------------

#[test]
fn video_no_audio_on_video_track_with_defaults() {
    let prior = project_with(
        vec![video_track(TRACK_VIDEO_A, "Video 1", false, vec![])],
        vec![video_asset(
            ASSET_VIDEO_NO_AUDIO,
            VIDEO_NO_AUDIO_PATH,
            false,
        )],
        0,
    );

    let (patch, warnings, data) = compute_patch(&prior, &happy_args()).expect("clip.add");
    let post = apply_patch(&prior, patch);
    let clip = last_clip(&post, 0);

    assert_eq!(clip.id, data.clip_id);
    assert_eq!(data.track_id.to_string(), TRACK_VIDEO_A);
    assert_eq!(clip.source_in_tk.get(), 0);
    assert_eq!(clip.source_out_tk.get(), ASSET_DURATION_TK);
    assert_eq!(clip.track_position_tk.get(), 0);
    assert!(clip.text.is_none());
    assert!(clip.link_group.is_none());
    assert_eq!(post.duration_tk.get(), ASSET_DURATION_TK);
    assert_eq!(
        warnings.last().expect("envelope")["code"],
        W_CLIP_ADD_ENVELOPE_CODE
    );
}

#[test]
fn audio_asset_on_audio_track_with_defaults() {
    let prior = project_with(
        vec![audio_track(TRACK_AUDIO_A, "Audio 1", false, vec![])],
        vec![audio_asset(ASSET_AUDIO, AUDIO_PATH)],
        0,
    );
    let mut args = happy_args();
    args.asset_id = ASSET_AUDIO.to_string();
    args.track = TRACK_AUDIO_A.to_string();

    let (patch, _warnings, data) = compute_patch(&prior, &args).expect("clip.add audio");
    let post = apply_patch(&prior, patch);
    let clip = last_clip(&post, 0);

    assert_eq!(clip.id, data.clip_id);
    assert_eq!(clip.source_out_tk.get(), ASSET_DURATION_TK);
}

#[test]
fn image_asset_on_video_track_defaults_to_five_seconds() {
    let prior = project_with(
        vec![video_track(TRACK_VIDEO_A, "Video 1", false, vec![])],
        vec![image_asset(ASSET_IMAGE, IMAGE_PATH)],
        0,
    );
    let mut args = happy_args();
    args.asset_id = ASSET_IMAGE.to_string();

    let (patch, _warnings, _data) = compute_patch(&prior, &args).expect("clip.add image");
    let post = apply_patch(&prior, patch);
    let clip = last_clip(&post, 0);

    assert_eq!(clip.source_in_tk.get(), 0);
    assert_eq!(clip.source_out_tk.get(), 5 * 240_000);
}

#[test]
fn video_with_audio_on_video_track_places_video_only_no_auto_pair() {
    let prior = project_with(
        vec![
            video_track(TRACK_VIDEO_A, "Video 1", false, vec![]),
            audio_track(TRACK_AUDIO_A, "Audio 1", false, vec![]),
        ],
        vec![video_asset(
            ASSET_VIDEO_WITH_AUDIO,
            VIDEO_WITH_AUDIO_PATH,
            true,
        )],
        0,
    );
    let mut args = happy_args();
    args.asset_id = ASSET_VIDEO_WITH_AUDIO.to_string();

    let (patch, _warnings, _data) = compute_patch(&prior, &args).expect("clip.add");
    let post = apply_patch(&prior, patch);

    assert_eq!(post.tracks[0].clips.len(), 1);
    // Auto-pair is deferred — audio track must remain empty.
    assert_eq!(post.tracks[1].clips.len(), 0);
}

#[test]
fn video_with_audio_on_audio_track_places_audio_clip() {
    let prior = project_with(
        vec![audio_track(TRACK_AUDIO_A, "Audio 1", false, vec![])],
        vec![video_asset(
            ASSET_VIDEO_WITH_AUDIO,
            VIDEO_WITH_AUDIO_PATH,
            true,
        )],
        0,
    );
    let mut args = happy_args();
    args.asset_id = ASSET_VIDEO_WITH_AUDIO.to_string();
    args.track = TRACK_AUDIO_A.to_string();

    let (patch, _warnings, _data) = compute_patch(&prior, &args).expect("clip.add");
    let post = apply_patch(&prior, patch);
    let clip = last_clip(&post, 0);

    // Audio clip references the source video asset's id (no extraction).
    assert_eq!(
        clip.asset_id.id().expect("non-nil").to_string(),
        ASSET_VIDEO_WITH_AUDIO
    );
}

#[test]
fn track_supplied_as_bare_uuid() {
    let prior = project_with(
        vec![video_track(TRACK_VIDEO_A, "Video 1", false, vec![])],
        vec![video_asset(
            ASSET_VIDEO_NO_AUDIO,
            VIDEO_NO_AUDIO_PATH,
            false,
        )],
        0,
    );

    let (_patch, _warnings, data) = compute_patch(&prior, &happy_args()).expect("clip.add");
    assert_eq!(data.track_id.to_string(), TRACK_VIDEO_A);
}

#[test]
fn track_supplied_as_qualified_track_uuid() {
    let prior = project_with(
        vec![video_track(TRACK_VIDEO_A, "Video 1", false, vec![])],
        vec![video_asset(
            ASSET_VIDEO_NO_AUDIO,
            VIDEO_NO_AUDIO_PATH,
            false,
        )],
        0,
    );
    let mut args = happy_args();
    args.track = format!("track:{TRACK_VIDEO_A}");

    let (_patch, _warnings, data) = compute_patch(&prior, &args).expect("clip.add");
    assert_eq!(data.track_id.to_string(), TRACK_VIDEO_A);
}

#[test]
fn supplied_name_is_used_verbatim() {
    let prior = project_with(
        vec![video_track(TRACK_VIDEO_A, "Video 1", false, vec![])],
        vec![video_asset(
            ASSET_VIDEO_NO_AUDIO,
            VIDEO_NO_AUDIO_PATH,
            false,
        )],
        0,
    );
    let mut args = happy_args();
    args.name = Some("My Hero Shot".to_string());

    let (patch, _warnings, _data) = compute_patch(&prior, &args).expect("clip.add");
    let post = apply_patch(&prior, patch);
    assert_eq!(last_clip(&post, 0).name, "My Hero Shot");
}

#[test]
fn name_derived_from_asset_path_basename_without_extension() {
    let prior = project_with(
        vec![video_track(TRACK_VIDEO_A, "Video 1", false, vec![])],
        vec![video_asset(
            ASSET_VIDEO_NO_AUDIO,
            VIDEO_NO_AUDIO_PATH,
            false,
        )],
        0,
    );

    let (patch, _warnings, _data) = compute_patch(&prior, &happy_args()).expect("clip.add");
    let post = apply_patch(&prior, patch);
    let basename = VIDEO_NO_AUDIO_PATH
        .rsplit('/')
        .next()
        .unwrap()
        .rsplit_once('.')
        .unwrap()
        .0;
    assert_eq!(last_clip(&post, 0).name, basename);
}

// ---------------------------------------------------------------------
// Error paths
// ---------------------------------------------------------------------

#[test]
fn unknown_asset_errors_asset_not_found() {
    let prior = project_with(
        vec![video_track(TRACK_VIDEO_A, "Video 1", false, vec![])],
        vec![],
        0,
    );
    let mut args = happy_args();
    args.asset_id = ASSET_MISSING.to_string();

    let err = compute_patch(&prior, &args).expect_err("missing asset");

    assert!(
        matches!(&err, ClipAddError::AssetNotFound { asset_id } if asset_id == ASSET_MISSING),
        "{err:?}"
    );
}

#[test]
fn subtitle_asset_errors_asset_kind_unroutable() {
    let prior = project_with(
        vec![video_track(TRACK_VIDEO_A, "Video 1", false, vec![])],
        vec![subtitle_asset(ASSET_SUBTITLE, SUBTITLE_PATH)],
        0,
    );
    let mut args = happy_args();
    args.asset_id = ASSET_SUBTITLE.to_string();

    let err = compute_patch(&prior, &args).expect_err("subtitle unroutable");

    assert!(
        matches!(
            err,
            ClipAddError::AssetKindUnroutable {
                asset_kind: "subtitle",
                ..
            }
        ),
        "{err:?}"
    );
}

#[test]
fn video_no_audio_on_audio_track_errors_kind_mismatch() {
    let prior = project_with(
        vec![audio_track(TRACK_AUDIO_A, "Audio 1", false, vec![])],
        vec![video_asset(
            ASSET_VIDEO_NO_AUDIO,
            VIDEO_NO_AUDIO_PATH,
            false,
        )],
        0,
    );
    let mut args = happy_args();
    args.track = TRACK_AUDIO_A.to_string();

    let err = compute_patch(&prior, &args).expect_err("kind mismatch");

    assert!(
        matches!(err, ClipAddError::TrackKindMismatch { .. }),
        "{err:?}"
    );
}

#[test]
fn audio_on_video_track_errors_kind_mismatch() {
    let prior = project_with(
        vec![video_track(TRACK_VIDEO_A, "Video 1", false, vec![])],
        vec![audio_asset(ASSET_AUDIO, AUDIO_PATH)],
        0,
    );
    let mut args = happy_args();
    args.asset_id = ASSET_AUDIO.to_string();

    let err = compute_patch(&prior, &args).expect_err("audio on video track");

    assert!(
        matches!(err, ClipAddError::TrackKindMismatch { .. }),
        "{err:?}"
    );
}

#[test]
fn image_on_audio_track_errors_kind_mismatch() {
    let prior = project_with(
        vec![audio_track(TRACK_AUDIO_A, "Audio 1", false, vec![])],
        vec![image_asset(ASSET_IMAGE, IMAGE_PATH)],
        0,
    );
    let mut args = happy_args();
    args.asset_id = ASSET_IMAGE.to_string();
    args.track = TRACK_AUDIO_A.to_string();

    let err = compute_patch(&prior, &args).expect_err("image on audio");

    assert!(
        matches!(err, ClipAddError::TrackKindMismatch { .. }),
        "{err:?}"
    );
}

#[test]
fn any_asset_on_text_track_errors_kind_mismatch() {
    let prior = project_with(
        vec![text_track(TRACK_TEXT_A, "Captions")],
        vec![video_asset(
            ASSET_VIDEO_NO_AUDIO,
            VIDEO_NO_AUDIO_PATH,
            false,
        )],
        0,
    );
    let mut args = happy_args();
    args.track = TRACK_TEXT_A.to_string();

    let err = compute_patch(&prior, &args).expect_err("video on text");

    assert!(
        matches!(err, ClipAddError::TrackKindMismatch { .. }),
        "{err:?}"
    );
}

#[test]
fn clip_prefix_track_selector_errors_bad_selector() {
    let prior = project_with(
        vec![video_track(TRACK_VIDEO_A, "Video 1", false, vec![])],
        vec![video_asset(
            ASSET_VIDEO_NO_AUDIO,
            VIDEO_NO_AUDIO_PATH,
            false,
        )],
        0,
    );
    let mut args = happy_args();
    args.track = format!("clip:{TRACK_VIDEO_A}");

    let err = compute_patch(&prior, &args).expect_err("clip: prefix");

    assert!(
        matches!(err, ClipAddError::BadSelector { field: "track", .. }),
        "{err:?}"
    );
}

#[test]
fn malformed_track_uuid_errors_bad_selector() {
    let prior = project_with(
        vec![],
        vec![video_asset(
            ASSET_VIDEO_NO_AUDIO,
            VIDEO_NO_AUDIO_PATH,
            false,
        )],
        0,
    );
    let mut args = happy_args();
    args.track = "not-a-uuid".to_string();

    let err = compute_patch(&prior, &args).expect_err("bad uuid");

    assert!(
        matches!(err, ClipAddError::BadSelector { field: "track", .. }),
        "{err:?}"
    );
}

#[test]
fn unknown_track_uuid_errors_track_not_found() {
    let prior = project_with(
        vec![],
        vec![video_asset(
            ASSET_VIDEO_NO_AUDIO,
            VIDEO_NO_AUDIO_PATH,
            false,
        )],
        0,
    );
    let mut args = happy_args();
    args.track = MISSING_TRACK.to_string();

    let err = compute_patch(&prior, &args).expect_err("missing track");

    assert!(matches!(err, ClipAddError::TrackNotFound { .. }), "{err:?}");
}

#[test]
fn negative_source_in_errors_bad_time() {
    let prior = project_with(
        vec![video_track(TRACK_VIDEO_A, "Video 1", false, vec![])],
        vec![video_asset(
            ASSET_VIDEO_NO_AUDIO,
            VIDEO_NO_AUDIO_PATH,
            false,
        )],
        0,
    );
    let mut args = happy_args();
    args.source_in_tk = Some(-1);

    let err = compute_patch(&prior, &args).expect_err("negative source_in");

    assert!(
        matches!(
            err,
            ClipAddError::BadTime {
                field: "source_in_tk",
                value: -1,
                ..
            }
        ),
        "{err:?}"
    );
}

#[test]
fn source_in_geq_source_out_errors_bad_time() {
    let prior = project_with(
        vec![video_track(TRACK_VIDEO_A, "Video 1", false, vec![])],
        vec![video_asset(
            ASSET_VIDEO_NO_AUDIO,
            VIDEO_NO_AUDIO_PATH,
            false,
        )],
        0,
    );
    let mut args = happy_args();
    args.source_in_tk = Some(120_000);
    args.source_out_tk = Some(120_000);

    let err = compute_patch(&prior, &args).expect_err("degenerate slice");

    assert!(
        matches!(
            err,
            ClipAddError::BadTime {
                field: "source_out_tk",
                ..
            }
        ),
        "{err:?}"
    );
}

#[test]
fn source_out_past_asset_duration_errors_bad_time() {
    let prior = project_with(
        vec![video_track(TRACK_VIDEO_A, "Video 1", false, vec![])],
        vec![video_asset(
            ASSET_VIDEO_NO_AUDIO,
            VIDEO_NO_AUDIO_PATH,
            false,
        )],
        0,
    );
    let mut args = happy_args();
    args.source_out_tk = Some(ASSET_DURATION_TK + 1);

    let err = compute_patch(&prior, &args).expect_err("past asset duration");

    assert!(
        matches!(
            err,
            ClipAddError::BadTime {
                field: "source_out_tk",
                ..
            }
        ),
        "{err:?}"
    );
}

#[test]
fn negative_track_position_errors_bad_time() {
    let prior = project_with(
        vec![video_track(TRACK_VIDEO_A, "Video 1", false, vec![])],
        vec![video_asset(
            ASSET_VIDEO_NO_AUDIO,
            VIDEO_NO_AUDIO_PATH,
            false,
        )],
        0,
    );
    let mut args = happy_args();
    args.track_position_tk = -1;

    let err = compute_patch(&prior, &args).expect_err("negative position");

    assert!(
        matches!(
            err,
            ClipAddError::BadTime {
                field: "track_position_tk",
                value: -1,
                ..
            }
        ),
        "{err:?}"
    );
}

#[test]
fn image_with_nonzero_source_in_errors_schema_violation() {
    let prior = project_with(
        vec![video_track(TRACK_VIDEO_A, "Video 1", false, vec![])],
        vec![image_asset(ASSET_IMAGE, IMAGE_PATH)],
        0,
    );
    let mut args = happy_args();
    args.asset_id = ASSET_IMAGE.to_string();
    args.source_in_tk = Some(1);

    let err = compute_patch(&prior, &args).expect_err("image source_in invariant");

    assert!(
        matches!(
            err,
            ClipAddError::SchemaViolation {
                field: "source_in_tk",
                ..
            }
        ),
        "{err:?}"
    );
}

#[test]
fn locked_target_track_errors_locked() {
    let prior = project_with(
        vec![video_track(TRACK_VIDEO_A, "Video 1", true, vec![])],
        vec![video_asset(
            ASSET_VIDEO_NO_AUDIO,
            VIDEO_NO_AUDIO_PATH,
            false,
        )],
        0,
    );

    let err = compute_patch(&prior, &happy_args()).expect_err("locked track");

    assert!(matches!(err, ClipAddError::Locked { .. }), "{err:?}");
}

#[test]
fn overlap_with_existing_clip_errors_clip_overlap() {
    let existing_clip = json!({
        "id": CLIP_EXISTING,
        "name": "Existing",
        "asset_id": ASSET_VIDEO_NO_AUDIO,
        "track_position_tk": 0,
        "source_in_tk": 0,
        "source_out_tk": 120_000,
        "locked": false,
    });
    let prior = project_with(
        vec![video_track(
            TRACK_VIDEO_A,
            "Video 1",
            false,
            vec![existing_clip],
        )],
        vec![video_asset(
            ASSET_VIDEO_NO_AUDIO,
            VIDEO_NO_AUDIO_PATH,
            false,
        )],
        120_000,
    );
    let mut args = happy_args();
    args.track_position_tk = 60_000;

    let err = compute_patch(&prior, &args).expect_err("overlap");

    assert!(matches!(err, ClipAddError::ClipOverlap { .. }), "{err:?}");
}

#[test]
fn off_frame_position_on_video_track_snaps_and_warns() {
    let prior = project_with(
        vec![video_track(TRACK_VIDEO_A, "Video 1", false, vec![])],
        vec![video_asset(
            ASSET_VIDEO_NO_AUDIO,
            VIDEO_NO_AUDIO_PATH,
            false,
        )],
        0,
    );
    let mut args = happy_args();
    args.track_position_tk = 1;

    let (patch, warnings, _data) = compute_patch(&prior, &args).expect("clip.add snap");
    let post = apply_patch(&prior, patch);

    assert_eq!(warnings[0]["code"], W_TIME_SNAPPED_CODE);
    assert_eq!(warnings[0]["details"]["from_tk"], 1);
    assert_eq!(warnings[0]["details"]["to_tk"], 0);
    assert_eq!(last_clip(&post, 0).track_position_tk.get(), 0);
}

// ---------------------------------------------------------------------
// Reconstructor + harness
// ---------------------------------------------------------------------

#[test]
fn reconstructor_round_trip() {
    let prior = project_with(
        vec![video_track(TRACK_VIDEO_A, "Video 1", false, vec![])],
        vec![video_asset(
            ASSET_VIDEO_NO_AUDIO,
            VIDEO_NO_AUDIO_PATH,
            false,
        )],
        0,
    );
    let args = happy_args();
    let (patch, warnings, data) = compute_patch(&prior, &args).expect("clip.add");
    let post = apply_patch(&prior, patch.clone());

    let reconstructed = ClipAddVerb
        .reconstruct(
            &serde_json::to_value(&args).expect("args serialize"),
            &patch,
            &warnings,
            &post,
        )
        .expect("reconstruct");

    assert_eq!(reconstructed, serde_json::to_value(data).expect("data"));
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "clip.add")
        .expect("default_fixtures includes clip.add");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(ClipAddVerb))
        .expect("register clip.add verb");

    let report =
        validate_reconstructors(&registry, &[fixture]).expect("clip.add reconstruct from fixture");
    assert_eq!(report.verbs_checked, vec!["clip.add"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn default_fixture_data_matches_warning_envelope() {
    let RecordedEvent {
        warnings,
        expected_data,
        ..
    } = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "clip.add")
        .expect("default_fixtures includes clip.add");

    let data = data_envelope_from_warnings(&warnings).expect("envelope");
    assert_eq!(
        serde_json::to_value(data).expect("data serializes"),
        expected_data
    );
}

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let prior = project_with(
        vec![video_track(TRACK_VIDEO_A, "Video 1", false, vec![])],
        vec![video_asset(
            ASSET_VIDEO_NO_AUDIO,
            VIDEO_NO_AUDIO_PATH,
            false,
        )],
        0,
    );
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        prior,
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry clears gate");

    let outcome = store
        .mutate_via_verb(
            "clip.add",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "asset_id": ASSET_VIDEO_NO_AUDIO,
                "track": TRACK_VIDEO_A,
                "track_position_tk": 0,
            }),
            None,
        )
        .expect("mutate_via_verb happy path");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("happy path must return Applied, got {outcome:?}");
    };

    let data: ClipAddData = serde_json::from_value(data).expect("clip.add data");
    assert_eq!(data.track_id.to_string(), TRACK_VIDEO_A);
    assert_eq!(store.project().tracks[0].clips.len(), 1);
    assert_eq!(store.project().tracks[0].clips[0].id, data.clip_id);
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_CLIP_ADD_ENVELOPE_CODE);
}

/// Build the same minimal single-video-track ISO-BMFF fixture the
/// `asset.import` tests use, kept local so this test compiles standalone.
/// 640x360, timescale 24000, duration 48000 (2.0s) → duration_tk 480000.
#[cfg(feature = "native")]
fn minimal_mp4_fixture() -> Vec<u8> {
    fn iso_box(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let size = u32::try_from(8 + body.len()).expect("box fits u32");
        let mut out = size.to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(body);
        out
    }
    let ftyp = iso_box(b"ftyp", b"isom\x00\x00\x00\x00isom");

    let mut hdlr_body = vec![0u8; 8];
    hdlr_body.extend_from_slice(b"vide");
    hdlr_body.extend_from_slice(&[0u8; 12]);
    hdlr_body.push(0);
    let hdlr = iso_box(b"hdlr", &hdlr_body);

    let mut mdhd_body = vec![0u8; 4];
    mdhd_body.extend_from_slice(&0u32.to_be_bytes());
    mdhd_body.extend_from_slice(&0u32.to_be_bytes());
    mdhd_body.extend_from_slice(&24_000u32.to_be_bytes());
    mdhd_body.extend_from_slice(&48_000u32.to_be_bytes());
    mdhd_body.extend_from_slice(&[0u8; 4]);
    let mdhd = iso_box(b"mdhd", &mdhd_body);

    let mut avc1_body = vec![0u8; 6 + 2 + 16];
    avc1_body.extend_from_slice(&640u16.to_be_bytes());
    avc1_body.extend_from_slice(&360u16.to_be_bytes());
    let avc1 = iso_box(b"avc1", &avc1_body);
    let mut stsd_body = vec![0u8; 4];
    stsd_body.extend_from_slice(&1u32.to_be_bytes());
    stsd_body.extend_from_slice(&avc1);
    let stsd = iso_box(b"stsd", &stsd_body);

    let mut stsz_body = vec![0u8; 4];
    stsz_body.extend_from_slice(&0u32.to_be_bytes());
    stsz_body.extend_from_slice(&48u32.to_be_bytes());
    let stsz = iso_box(b"stsz", &stsz_body);

    let mut stbl_body = Vec::new();
    stbl_body.extend_from_slice(&stsd);
    stbl_body.extend_from_slice(&stsz);
    let stbl = iso_box(b"stbl", &stbl_body);
    let minf = iso_box(b"minf", &stbl);

    let mut mdia_body = Vec::new();
    mdia_body.extend_from_slice(&hdlr);
    mdia_body.extend_from_slice(&mdhd);
    mdia_body.extend_from_slice(&minf);
    let mdia = iso_box(b"mdia", &mdia_body);
    let trak = iso_box(b"trak", &mdia);
    let moov = iso_box(b"moov", &trak);

    let mut out = ftyp;
    out.extend_from_slice(&moov);
    out
}

/// Integration regression: importing an `.mp4` then `clip.add`-ing it
/// onto a video track succeeds. Before the classifier fix the mp4
/// imported as a `SubtitleAsset` and `clip.add` rejected it with
/// `E_ASSET_KIND_UNROUTABLE` (§5.1). The clip's `source_out_tk` defaults
/// to the asset's probed `duration_tk`.
#[cfg(feature = "native")]
#[test]
fn import_mp4_then_clip_add_succeeds() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let source = dir.path().join("clip.mp4");
    std::fs::write(&source, minimal_mp4_fixture()).expect("write mp4 fixture");

    let prior = project_with(
        vec![video_track(TRACK_VIDEO_A, "Video 1", false, vec![])],
        vec![],
        0,
    );
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        prior,
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry clears gate");

    // Step 1: import the mp4 — must classify as video.
    let import_outcome = store
        .mutate_via_verb(
            "asset.import",
            json!({"project_id": FIXTURE_PROJECT_ID, "paths": [source.to_string_lossy()]}),
            None,
        )
        .expect("asset.import succeeds");
    let MutateOutcome::Applied { data, .. } = import_outcome else {
        panic!("expected Applied outcome for mp4 import");
    };
    let import_data: verbreel_state::AssetImportData =
        serde_json::from_value(data).expect("asset.import data");
    let imported = import_data.assets[0].as_object().expect("asset object");
    assert_eq!(
        imported.get("kind").and_then(Value::as_str),
        Some("video"),
        "imported mp4 must be a video asset for clip.add to route it"
    );
    let asset_id = imported
        .get("id")
        .and_then(Value::as_str)
        .expect("imported asset id")
        .to_string();
    let asset_duration_tk = imported
        .get("metadata")
        .and_then(|m| m.get("duration_tk"))
        .and_then(Value::as_i64)
        .expect("video asset duration_tk");

    // Step 2: clip.add the imported video onto the video track.
    let clip_outcome = store
        .mutate_via_verb(
            "clip.add",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "asset_id": asset_id,
                "track": TRACK_VIDEO_A,
                "track_position_tk": 0,
            }),
            None,
        )
        .expect("clip.add against imported mp4 must succeed (no E_ASSET_KIND_UNROUTABLE)");
    let MutateOutcome::Applied { data, .. } = clip_outcome else {
        panic!("clip.add against imported mp4 must return Applied, got an error/no-op");
    };
    let clip_data: ClipAddData = serde_json::from_value(data).expect("clip.add data");
    assert_eq!(clip_data.track_id.to_string(), TRACK_VIDEO_A);

    let clip = &store.project().tracks[0].clips[0];
    assert_eq!(clip.id, clip_data.clip_id);
    // source_out_tk defaults to the asset's probed duration.
    assert_eq!(clip.source_out_tk.get(), asset_duration_tk);
}
