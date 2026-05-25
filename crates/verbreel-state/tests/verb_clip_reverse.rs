//! Tests for `clip.reverse` (§5.8) — forty-fifth production verb.

use serde_json::{Value, json};
use std::sync::Arc;

use verbreel_state::verbs::clip_reverse::{
    LINK_GROUP_SEMANTICS_MIX_HINT, W_NOOP_CODE, compute_patch, data_envelope_from_post_state,
};
use verbreel_state::{
    ClipReverseArgs, ClipReverseData, ClipReverseError, ClipReverseVerb, MutateOutcome, Project,
    RecordedEvent, Track, Verb, VerbError, VerbRegistry, default_fixtures, default_registry,
    validate_reconstructors,
};
use verbreel_types::{ClipId, ProjectId, Tick};

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const TRACK_VIDEO_A: &str = "01900000-0000-7000-8000-0000000aa101";
const TRACK_VIDEO_B: &str = "01900000-0000-7000-8000-0000000aa102";
const TRACK_AUDIO_A: &str = "01900000-0000-7000-8000-0000000aa201";
const TRACK_TEXT_A: &str = "01900000-0000-7000-8000-0000000aa301";
const CLIP_VIDEO_A: &str = "01900000-0000-7000-8000-0000000bb301";
const CLIP_VIDEO_B: &str = "01900000-0000-7000-8000-0000000bb302";
const CLIP_AUDIO_A: &str = "01900000-0000-7000-8000-0000000bb201";
const CLIP_TEXT_A: &str = "01900000-0000-7000-8000-0000000bb401";
const MISSING_CLIP: &str = "01900000-0000-7000-8000-0000000cc101";
const LINK_GROUP_ID: &str = "01900000-0000-7000-8000-00000000aaaa";
const ASSET_VIDEO_ID: &str = "01900000-0000-7000-8000-0000000dd101";
const ASSET_AUDIO_ID: &str = "01900000-0000-7000-8000-0000000dd102";
const ASSET_IMAGE_ID: &str = "01900000-0000-7000-8000-0000000dd103";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture -> Project")
}

fn fixture_project_id() -> ProjectId {
    empty_project().id
}

#[allow(clippy::too_many_arguments)]
fn video_track(
    id: &str,
    name: &str,
    track_locked: bool,
    clip_id: &str,
    clip_locked: bool,
    reversed: bool,
    link_group: Option<&str>,
    asset_id: &str,
) -> Track {
    serde_json::from_value(json!({
        "id": id,
        "kind": "video",
        "name": name,
        "locked": track_locked,
        "clips": [{
            "id": clip_id,
            "name": "Video Clip",
            "asset_id": asset_id,
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 240_000,
            "locked": clip_locked,
            "reversed": reversed,
            "link_group": link_group,
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
    reversed: bool,
    link_group: Option<&str>,
) -> Track {
    serde_json::from_value(json!({
        "id": id,
        "kind": "audio",
        "name": name,
        "locked": track_locked,
        "clips": [{
            "id": clip_id,
            "name": "Audio Clip",
            "asset_id": ASSET_AUDIO_ID,
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 240_000,
            "volume": 1.0,
            "locked": clip_locked,
            "reversed": reversed,
            "link_group": link_group,
        }],
    }))
    .expect("audio track fixture parses")
}

fn text_track(
    id: &str,
    name: &str,
    track_locked: bool,
    clip_id: &str,
    clip_locked: bool,
    reversed: bool,
    link_group: Option<&str>,
) -> Track {
    serde_json::from_value(json!({
        "id": id,
        "kind": "text",
        "name": name,
        "locked": track_locked,
        "clips": [{
            "id": clip_id,
            "name": "Text Clip",
            "asset_id": "00000000-0000-0000-0000-000000000000",
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 240_000,
            "locked": clip_locked,
            "reversed": reversed,
            "link_group": link_group,
            "text": {
                "content": "Hello",
                "font_family": "Arial",
                "font_size_px": 24
            },
        }],
    }))
    .expect("text track fixture parses")
}

fn project_with_tracks(tracks: Vec<Track>) -> Project {
    let mut project = empty_project();
    project.tracks = tracks;
    project.duration_tk = Tick::new(240_000);
    add_assets(&mut project);
    project
}

fn add_assets(project: &mut Project) {
    project.assets.push(
        serde_json::from_value(json!({
            "id": ASSET_VIDEO_ID,
            "kind": "video",
            "hash": "36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da",
            "path": "assets/36/36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da.mp4",
            "original_filename": "video-clip-reverse.mp4",
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
                    "size_bytes": 1024
                }
            }
        }))
        .expect("video asset parses"),
    );
    project.assets.push(
        serde_json::from_value(json!({
            "id": ASSET_AUDIO_ID,
            "kind": "audio",
            "hash": "53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658",
            "path": "assets/53/53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.m4a",
            "original_filename": "audio-clip-reverse.m4a",
            "imported_at": "2026-05-24T00:00:00Z",
            "metadata": {
                "duration_tk": 240_000,
                "audio_codec": "aac",
                "audio_channels": 2,
                "audio_sample_rate_hz": 48000,
                "container": "m4a",
                "fingerprint": {
                    "mtime_ms": 1_700_000_000_000_i64,
                    "size_bytes": 1024
                }
            }
        }))
        .expect("audio asset parses"),
    );
    project.assets.push(
        serde_json::from_value(json!({
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
                    "size_bytes": 1024
                }
            }
        }))
        .expect("image asset parses"),
    );
}

fn reverse_args(clip: &str, reversed: Option<bool>) -> ClipReverseArgs {
    ClipReverseArgs {
        project_id: fixture_project_id(),
        clip: clip.to_string(),
        reversed,
    }
}

fn apply_patch(prior: &Project, patch: &Value) -> Project {
    let typed_patch: json_patch::Patch =
        serde_json::from_value(patch.clone()).expect("patch parses to typed JSON patch");
    prior
        .apply(&typed_patch)
        .expect("clip.reverse patch applies cleanly")
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

fn linked_project(video_reversed: bool, audio_reversed: bool) -> Project {
    project_with_tracks(vec![
        video_track(
            TRACK_VIDEO_A,
            "Video",
            false,
            CLIP_VIDEO_A,
            false,
            video_reversed,
            Some(LINK_GROUP_ID),
            ASSET_VIDEO_ID,
        ),
        audio_track(
            TRACK_AUDIO_A,
            "Audio",
            false,
            CLIP_AUDIO_A,
            false,
            audio_reversed,
            Some(LINK_GROUP_ID),
        ),
    ])
}

#[test]
fn singleton_omitted_reversed_defaults_true() {
    let prior = project_with_tracks(vec![video_track(
        TRACK_VIDEO_A,
        "Video",
        false,
        CLIP_VIDEO_A,
        false,
        false,
        None,
        ASSET_VIDEO_ID,
    )]);
    let args = reverse_args(CLIP_VIDEO_A, None);

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("singleton reverse");
    assert!(warnings.is_empty());
    assert_eq!(patch_paths(&patch), ["/tracks/0/clips/0/reversed"]);
    assert!(data.reversed);
    assert!(data.linked_clip_ids.is_empty());
}

#[test]
fn singleton_explicit_false_sets_false() {
    let prior = project_with_tracks(vec![video_track(
        TRACK_VIDEO_A,
        "Video",
        false,
        CLIP_VIDEO_A,
        false,
        true,
        None,
        ASSET_VIDEO_ID,
    )]);
    let args = reverse_args(CLIP_VIDEO_A, Some(false));

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("explicit false");
    assert!(warnings.is_empty());
    assert_eq!(patch.as_array().expect("patch is array")[0]["value"], false);
    assert!(!data.reversed);
}

#[test]
fn linked_video_target_reverses_video_and_audio() {
    let prior = linked_project(false, false);
    let args = reverse_args(CLIP_VIDEO_A, None);

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("linked reverse");
    let post = apply_patch(&prior, &patch);

    assert!(warnings.is_empty());
    assert_eq!(
        patch_paths(&patch),
        ["/tracks/0/clips/0/reversed", "/tracks/1/clips/0/reversed"]
    );
    assert!(post.tracks[0].clips[0].reversed);
    assert!(post.tracks[1].clips[0].reversed);
    assert_eq!(data.linked_clip_ids, vec![CLIP_AUDIO_A.parse().unwrap()]);
}

#[test]
fn linked_audio_target_reverses_audio_and_video_symmetrically() {
    let prior = linked_project(false, false);
    let args = reverse_args(CLIP_AUDIO_A, None);

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("linked reverse");
    let post = apply_patch(&prior, &patch);

    assert!(warnings.is_empty());
    assert!(post.tracks[0].clips[0].reversed);
    assert!(post.tracks[1].clips[0].reversed);
    assert_eq!(data.clip_id.to_string(), CLIP_AUDIO_A);
    assert_eq!(data.linked_clip_ids, vec![CLIP_VIDEO_A.parse().unwrap()]);
}

#[test]
fn noop_when_singleton_already_true() {
    let prior = project_with_tracks(vec![video_track(
        TRACK_VIDEO_A,
        "Video",
        false,
        CLIP_VIDEO_A,
        false,
        true,
        None,
        ASSET_VIDEO_ID,
    )]);
    let args = reverse_args(CLIP_VIDEO_A, None);

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("noop");
    assert!(patch.as_array().expect("patch is array").is_empty());
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_NOOP_CODE);
    assert_eq!(warnings[0]["details"]["clip_id"], CLIP_VIDEO_A);
    assert_eq!(warnings[0]["details"]["reversed"], true);
    assert!(data.reversed);
}

#[test]
fn noop_when_all_linked_members_already_true() {
    let prior = linked_project(true, true);
    let args = reverse_args(CLIP_VIDEO_A, None);

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("noop");
    assert!(patch.as_array().expect("patch is array").is_empty());
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_NOOP_CODE);
    assert_eq!(data.linked_clip_ids, vec![CLIP_AUDIO_A.parse().unwrap()]);
}

#[test]
fn linked_clip_ids_empty_for_singleton() {
    let prior = project_with_tracks(vec![video_track(
        TRACK_VIDEO_A,
        "Video",
        false,
        CLIP_VIDEO_A,
        false,
        false,
        None,
        ASSET_VIDEO_ID,
    )]);
    let (_patch, _warnings, data) =
        compute_patch(&prior, &reverse_args(CLIP_VIDEO_A, None)).expect("singleton");
    assert!(data.linked_clip_ids.is_empty());
}

#[test]
fn linked_clip_ids_are_sorted_and_exclude_target() {
    let prior = project_with_tracks(vec![
        video_track(
            TRACK_VIDEO_A,
            "Video A",
            false,
            CLIP_VIDEO_A,
            false,
            false,
            Some(LINK_GROUP_ID),
            ASSET_VIDEO_ID,
        ),
        video_track(
            TRACK_VIDEO_B,
            "Video B",
            false,
            CLIP_VIDEO_B,
            false,
            false,
            Some(LINK_GROUP_ID),
            ASSET_VIDEO_ID,
        ),
        audio_track(
            TRACK_AUDIO_A,
            "Audio",
            false,
            CLIP_AUDIO_A,
            false,
            false,
            Some(LINK_GROUP_ID),
        ),
    ]);
    let (_patch, _warnings, data) =
        compute_patch(&prior, &reverse_args(CLIP_VIDEO_B, None)).expect("linked");
    let actual = data
        .linked_clip_ids
        .iter()
        .map(ClipId::to_string)
        .collect::<Vec<_>>();
    assert_eq!(actual, vec![CLIP_AUDIO_A, CLIP_VIDEO_A]);
}

#[test]
fn missing_clip_returns_not_found() {
    let prior = linked_project(false, false);
    let err = compute_patch(&prior, &reverse_args(MISSING_CLIP, None)).expect_err("missing");
    match err {
        ClipReverseError::ClipNotFound { clip_id } => assert_eq!(clip_id, MISSING_CLIP),
        other => panic!("expected ClipNotFound, got {other:?}"),
    }
}

#[test]
fn malformed_uuid_returns_bad_selector() {
    let prior = linked_project(false, false);
    let err = compute_patch(&prior, &reverse_args("not-a-uuid", None)).expect_err("bad uuid");
    match err {
        ClipReverseError::BadSelector { detail } => assert!(detail.contains("UUID"), "{detail}"),
        other => panic!("expected BadSelector, got {other:?}"),
    }
}

#[test]
fn target_clip_lock_returns_locked() {
    let prior = project_with_tracks(vec![video_track(
        TRACK_VIDEO_A,
        "Video",
        false,
        CLIP_VIDEO_A,
        true,
        false,
        None,
        ASSET_VIDEO_ID,
    )]);
    let err = compute_patch(&prior, &reverse_args(CLIP_VIDEO_A, None)).expect_err("locked");
    match err {
        ClipReverseError::Locked { failed_clip } => assert_eq!(failed_clip, CLIP_VIDEO_A),
        other => panic!("expected Locked, got {other:?}"),
    }
}

#[test]
fn target_track_lock_returns_locked() {
    let prior = project_with_tracks(vec![video_track(
        TRACK_VIDEO_A,
        "Video",
        true,
        CLIP_VIDEO_A,
        false,
        false,
        None,
        ASSET_VIDEO_ID,
    )]);
    let err = compute_patch(&prior, &reverse_args(CLIP_VIDEO_A, None)).expect_err("locked track");
    match err {
        ClipReverseError::Locked { failed_clip } => assert_eq!(failed_clip, CLIP_VIDEO_A),
        other => panic!("expected Locked, got {other:?}"),
    }
}

#[test]
fn linked_member_track_lock_returns_locked_and_does_not_mutate() {
    let prior = project_with_tracks(vec![
        video_track(
            TRACK_VIDEO_A,
            "Video",
            false,
            CLIP_VIDEO_A,
            false,
            false,
            Some(LINK_GROUP_ID),
            ASSET_VIDEO_ID,
        ),
        audio_track(
            TRACK_AUDIO_A,
            "Audio",
            true,
            CLIP_AUDIO_A,
            false,
            false,
            Some(LINK_GROUP_ID),
        ),
    ]);
    let err = compute_patch(&prior, &reverse_args(CLIP_VIDEO_A, None)).expect_err("locked member");
    match err {
        ClipReverseError::Locked { failed_clip } => assert_eq!(failed_clip, CLIP_AUDIO_A),
        other => panic!("expected Locked, got {other:?}"),
    }
    assert!(!prior.tracks[0].clips[0].reversed);
    assert!(!prior.tracks[1].clips[0].reversed);
}

#[test]
fn video_image_link_group_returns_semantics_mix() {
    let prior = project_with_tracks(vec![
        video_track(
            TRACK_VIDEO_A,
            "Video",
            false,
            CLIP_VIDEO_A,
            false,
            false,
            Some(LINK_GROUP_ID),
            ASSET_VIDEO_ID,
        ),
        video_track(
            TRACK_VIDEO_B,
            "Image",
            false,
            CLIP_VIDEO_B,
            false,
            false,
            Some(LINK_GROUP_ID),
            ASSET_IMAGE_ID,
        ),
    ]);
    let err = compute_patch(&prior, &reverse_args(CLIP_VIDEO_A, None)).expect_err("mix");
    match err {
        ClipReverseError::LinkGroupSemanticsMix {
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
fn audio_text_link_group_returns_semantics_mix() {
    let prior = project_with_tracks(vec![
        audio_track(
            TRACK_AUDIO_A,
            "Audio",
            false,
            CLIP_AUDIO_A,
            false,
            false,
            Some(LINK_GROUP_ID),
        ),
        text_track(
            TRACK_TEXT_A,
            "Text",
            false,
            CLIP_TEXT_A,
            false,
            false,
            Some(LINK_GROUP_ID),
        ),
    ]);
    let err = compute_patch(&prior, &reverse_args(CLIP_AUDIO_A, None)).expect_err("mix");
    match err {
        ClipReverseError::LinkGroupSemanticsMix {
            member_kinds,
            semantics_classes,
            ..
        } => {
            assert_eq!(member_kinds.audio, 1);
            assert_eq!(member_kinds.text, 1);
            assert_eq!(semantics_classes.source_slice, 1);
            assert_eq!(semantics_classes.display_duration, 1);
        }
        other => panic!("expected LinkGroupSemanticsMix, got {other:?}"),
    }
}

#[test]
fn homogeneous_video_audio_group_does_not_trigger_semantics_mix() {
    let prior = linked_project(false, false);
    let (patch, warnings, data) =
        compute_patch(&prior, &reverse_args(CLIP_VIDEO_A, None)).expect("homogeneous group");
    assert_eq!(patch.as_array().expect("patch is array").len(), 2);
    assert!(warnings.is_empty());
    assert!(data.reversed);
}

#[test]
fn mismatched_link_group_members_all_get_new_value() {
    let prior = linked_project(true, false);
    let (patch, warnings, _data) =
        compute_patch(&prior, &reverse_args(CLIP_AUDIO_A, Some(false))).expect("partial mismatch");
    let post = apply_patch(&prior, &patch);

    assert!(warnings.is_empty());
    assert_eq!(patch_paths(&patch), ["/tracks/0/clips/0/reversed"]);
    assert!(!post.tracks[0].clips[0].reversed);
    assert!(!post.tracks[1].clips[0].reversed);
}

#[test]
fn reconstructor_round_trip_singleton() {
    let prior = project_with_tracks(vec![video_track(
        TRACK_VIDEO_A,
        "Video",
        false,
        CLIP_VIDEO_A,
        false,
        false,
        None,
        ASSET_VIDEO_ID,
    )]);
    let args = reverse_args(CLIP_VIDEO_A, None);
    let (patch, warnings, _data) = compute_patch(&prior, &args).expect("compute_patch ok");
    let post_state = apply_patch(&prior, &patch);
    let expected_data =
        serde_json::to_value(data_envelope_from_post_state(&args, &post_state).expect("envelope"))
            .expect("envelope serializes");
    let recorded = RecordedEvent {
        verb: "clip.reverse".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch,
        warnings,
        post_state,
        expected_data,
    };

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(ClipReverseVerb))
        .expect("register clip.reverse");
    let report = validate_reconstructors(&registry, &[recorded]).expect("round trip");
    assert_eq!(report.verbs_checked, vec!["clip.reverse"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn reconstructor_round_trip_linked_group() {
    let prior = linked_project(false, false);
    let args = reverse_args(CLIP_AUDIO_A, None);
    let (patch, warnings, _data) = compute_patch(&prior, &args).expect("compute_patch ok");
    let post_state = apply_patch(&prior, &patch);
    let expected_data =
        serde_json::to_value(data_envelope_from_post_state(&args, &post_state).expect("envelope"))
            .expect("envelope serializes");
    let recorded = RecordedEvent {
        verb: "clip.reverse".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch,
        warnings,
        post_state,
        expected_data,
    };

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(ClipReverseVerb))
        .expect("register clip.reverse");
    let report = validate_reconstructors(&registry, &[recorded]).expect("round trip");
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "clip.reverse")
        .expect("default_fixtures includes clip.reverse");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(ClipReverseVerb))
        .expect("register clip.reverse");
    let report = validate_reconstructors(&registry, &[fixture]).expect("default fixture");
    assert_eq!(report.verbs_checked, vec!["clip.reverse"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn compute_patch_error_variants_map_to_bad_args() {
    let prior = linked_project(false, false);
    let verb = ClipReverseVerb;
    let bad_selector = serde_json::to_value(reverse_args("not-a-uuid", None)).unwrap();
    let err = verb
        .compute_patch(&prior, &bad_selector)
        .expect_err("bad selector maps");
    assert!(matches!(err, VerbError::BadArgs { .. }));

    let missing = serde_json::to_value(reverse_args(MISSING_CLIP, None)).unwrap();
    let err = verb
        .compute_patch(&prior, &missing)
        .expect_err("missing clip maps");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let base_project = linked_project(false, false);
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        base_project,
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry clears the gate");

    let outcome = store
        .mutate_via_verb(
            "clip.reverse",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "clip": CLIP_VIDEO_A,
            }),
            None,
        )
        .expect("mutate_via_verb happy path");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("happy path must return Applied, got {outcome:?}");
    };

    let data: ClipReverseData =
        serde_json::from_value(data).expect("clip.reverse data is ClipReverseData");
    assert!(store.project().tracks[0].clips[0].reversed);
    assert!(store.project().tracks[1].clips[0].reversed);
    assert_eq!(data.linked_clip_ids, vec![CLIP_AUDIO_A.parse().unwrap()]);
    assert_eq!(warnings, Vec::<Value>::new());
}
