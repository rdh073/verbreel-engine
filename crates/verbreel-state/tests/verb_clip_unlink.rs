//! Tests for `clip.unlink` (§5.16) — twenty-fifth production verb.

use serde_json::{Value, json};
use std::sync::Arc;

use verbreel_state::verbs::clip_unlink::{compute_patch, data_envelope_from_patch_and_post_state};
use verbreel_state::{
    ClipUnlinkArgs, ClipUnlinkData, ClipUnlinkError, ClipUnlinkVerb, MutateOutcome, Project, Track,
    Verb, VerbError, VerbRegistry, default_fixtures, default_registry, validate_reconstructors,
};
use verbreel_types::{ClipId, ProjectId, Tick};

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const TRACK_VIDEO_A: &str = "01900000-0000-7000-8000-0000000aa201";
const TRACK_VIDEO_B: &str = "01900000-0000-7000-8000-0000000aa202";
const TRACK_AUDIO_A: &str = "01900000-0000-7000-8000-0000000aa301";
const TRACK_AUDIO_B: &str = "01900000-0000-7000-8000-0000000aa302";
const CLIP_VIDEO_A: &str = "01900000-0000-7000-8000-0000000bb301";
const CLIP_VIDEO_B: &str = "01900000-0000-7000-8000-0000000bb101";
const CLIP_AUDIO_A: &str = "01900000-0000-7000-8000-0000000bb202";
const CLIP_AUDIO_B: &str = "01900000-0000-7000-8000-0000000bb203";
const CLIP_VIDEO_SORT_LOW: &str = "01900000-0000-7000-8000-0000000bb901";
const CLIP_VIDEO_SORT_HIGH: &str = "01900000-0000-7000-8000-0000000bb903";
const CLIP_AUDIO_SORT: &str = "01900000-0000-7000-8000-0000000bb902";
const MISSING_CLIP: &str = "01900000-0000-7000-8000-0000000cc101";
const LINK_GROUP_ID: &str = "01900000-0000-7000-8000-00000000aaaa";
const LINK_GROUP_OTHER: &str = "01900000-0000-7000-8000-00000000bbbb";
const ASSET_VIDEO_ID: &str = "01900000-0000-7000-8000-0000000dd101";
const ASSET_AUDIO_ID: &str = "01900000-0000-7000-8000-0000000dd102";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture → Project")
}

fn fixture_project_id() -> ProjectId {
    empty_project().id
}

fn video_track(
    id: &str,
    name: &str,
    track_locked: bool,
    clip_id: &str,
    clip_locked: bool,
    link_group: Option<&str>,
) -> Track {
    serde_json::from_value(json!({
        "id": id,
        "kind": "video",
        "name": name,
        "locked": track_locked,
        "clips": [{
            "id": clip_id,
            "name": "Video Clip",
            "asset_id": ASSET_VIDEO_ID,
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 240_000,
            "locked": clip_locked,
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
            "locked": clip_locked,
            "volume": 1.0,
            "link_group": link_group,
        }],
    }))
    .expect("audio track fixture parses")
}

fn project_with_tracks(tracks: Vec<Track>) -> Project {
    let mut project = empty_project();
    project.tracks = tracks;
    project.duration_tk = Tick::new(240_000);
    project
}

fn add_assets(project: &mut Project) {
    project.assets.push(
        serde_json::from_value(json!({
            "id": ASSET_VIDEO_ID,
            "kind": "video",
            "hash": "36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da",
            "path": "assets/36/36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da.mp4",
            "original_filename": "video-clip-set-volume.mp4",
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
        .expect("video asset parses"),
    );
    project.assets.push(
        serde_json::from_value(json!({
            "id": ASSET_AUDIO_ID,
            "kind": "audio",
            "hash": "53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658",
            "path": "assets/53/53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.m4a",
            "original_filename": "audio-clip-set-volume.m4a",
            "imported_at": "2026-05-24T00:00:00Z",
            "metadata": {
                "duration_tk": 240_000,
                "audio_codec": "aac",
                "audio_channels": 2,
                "audio_sample_rate_hz": 48000,
                "container": "m4a",
                "fingerprint": {
                    "mtime_ms": 1_700_000_000_000_i64,
                    "size_bytes": 1024,
                }
            }
        }))
        .expect("audio asset parses"),
    );
}

fn assert_link_group_paths(patch: &Value, expected: &[&str]) {
    let ops = patch.as_array().expect("patch is an array");
    let paths: Vec<&str> = ops
        .iter()
        .filter_map(|op| op.get("path").and_then(Value::as_str))
        .collect();
    assert_eq!(paths, expected);
}

#[test]
fn compute_patch_unlink_two_member_group_clears_link_group() {
    let prior = project_with_tracks(vec![
        video_track(
            TRACK_VIDEO_A,
            "Video",
            false,
            CLIP_VIDEO_A,
            false,
            Some(LINK_GROUP_ID),
        ),
        audio_track(
            TRACK_AUDIO_A,
            "Audio",
            false,
            CLIP_AUDIO_A,
            false,
            Some(LINK_GROUP_ID),
        ),
    ]);

    let args = ClipUnlinkArgs {
        project_id: fixture_project_id(),
        clip: CLIP_VIDEO_A.to_string(),
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("happy path should unlink");
    assert!(warnings.is_empty());
    assert_eq!(data.link_group.to_string(), LINK_GROUP_ID);
    assert_eq!(
        data.cleared_clip_ids
            .iter()
            .map(ClipId::to_string)
            .collect::<Vec<_>>(),
        [CLIP_AUDIO_A, CLIP_VIDEO_A]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>()
    );

    assert_eq!(patch.as_array().expect("patch is array").len(), 4);
    assert_eq!(
        patch.as_array().expect("patch is array")[0]["op"]
            .as_str()
            .expect("op is str"),
        "test"
    );
    assert_eq!(
        patch.as_array().expect("patch is array")[0]["path"]
            .as_str()
            .expect("path is str"),
        "/tracks/0/clips/0/link_group"
    );
    assert_eq!(
        patch.as_array().expect("patch is array")[1]["op"]
            .as_str()
            .expect("op is str"),
        "replace"
    );
    assert_eq!(
        patch.as_array().expect("patch is array")[2]["path"]
            .as_str()
            .expect("path is str"),
        "/tracks/1/clips/0/link_group"
    );
}

#[test]
fn compute_patch_unlink_three_member_group_clears_all_members() {
    let prior = project_with_tracks(vec![
        video_track(
            TRACK_VIDEO_A,
            "Video A",
            false,
            CLIP_VIDEO_A,
            false,
            Some(LINK_GROUP_ID),
        ),
        audio_track(
            TRACK_AUDIO_A,
            "Audio A",
            false,
            CLIP_AUDIO_A,
            false,
            Some(LINK_GROUP_ID),
        ),
        audio_track(
            TRACK_AUDIO_B,
            "Audio B",
            false,
            CLIP_AUDIO_B,
            false,
            Some(LINK_GROUP_ID),
        ),
    ]);

    let args = ClipUnlinkArgs {
        project_id: fixture_project_id(),
        clip: CLIP_VIDEO_A.to_string(),
    };

    let (patch, _warnings, data) = compute_patch(&prior, &args).expect("happy path should unlink");
    assert_eq!(patch.as_array().expect("patch is array").len(), 6);
    assert_eq!(data.cleared_clip_ids.len(), 3,);
    let expected = [CLIP_AUDIO_A, CLIP_AUDIO_B, CLIP_VIDEO_A];
    assert_eq!(
        data.cleared_clip_ids
            .iter()
            .map(ClipId::to_string)
            .collect::<Vec<_>>(),
        expected.iter().map(|id| id.to_string()).collect::<Vec<_>>()
    );
}

#[test]
fn compute_patch_unlink_not_linked_errors() {
    let prior = project_with_tracks(vec![video_track(
        TRACK_VIDEO_A,
        "Video",
        false,
        CLIP_VIDEO_A,
        false,
        None,
    )]);

    let args = ClipUnlinkArgs {
        project_id: fixture_project_id(),
        clip: CLIP_VIDEO_A.to_string(),
    };

    let err = compute_patch(&prior, &args).expect_err("not linked should fail");
    match err {
        ClipUnlinkError::NotLinked { clip_id } => assert_eq!(clip_id, CLIP_VIDEO_A),
        other => panic!("expected NotLinked, got {other:?}"),
    }
}

#[test]
fn compute_patch_unlink_target_locked_returns_member_locked() {
    let prior = project_with_tracks(vec![
        video_track(
            TRACK_VIDEO_A,
            "Video",
            false,
            CLIP_VIDEO_A,
            true,
            Some(LINK_GROUP_ID),
        ),
        audio_track(
            TRACK_AUDIO_A,
            "Audio",
            false,
            CLIP_AUDIO_A,
            false,
            Some(LINK_GROUP_ID),
        ),
    ]);

    let args = ClipUnlinkArgs {
        project_id: fixture_project_id(),
        clip: CLIP_VIDEO_A.to_string(),
    };

    let err = compute_patch(&prior, &args).expect_err("target locked should fail");
    match err {
        ClipUnlinkError::MemberLocked { clip_id } => assert_eq!(clip_id, CLIP_VIDEO_A),
        other => panic!("expected MemberLocked, got {other:?}"),
    }
}

#[test]
fn compute_patch_unlink_sibling_locked_returns_sibling() {
    let prior = project_with_tracks(vec![
        video_track(
            TRACK_VIDEO_A,
            "Video",
            false,
            CLIP_VIDEO_A,
            false,
            Some(LINK_GROUP_ID),
        ),
        audio_track(
            TRACK_AUDIO_A,
            "Audio",
            false,
            CLIP_AUDIO_A,
            true,
            Some(LINK_GROUP_ID),
        ),
    ]);

    let args = ClipUnlinkArgs {
        project_id: fixture_project_id(),
        clip: CLIP_VIDEO_A.to_string(),
    };

    let err = compute_patch(&prior, &args).expect_err("sibling locked should fail");
    match err {
        ClipUnlinkError::MemberLocked { clip_id } => assert_eq!(clip_id, CLIP_AUDIO_A),
        other => panic!("expected MemberLocked, got {other:?}"),
    }
}

#[test]
fn compute_patch_unlink_not_linked_takes_precedence_over_locked_members() {
    let prior = project_with_tracks(vec![
        video_track(TRACK_VIDEO_A, "Video", false, CLIP_VIDEO_A, true, None),
        audio_track(
            TRACK_AUDIO_A,
            "Audio",
            false,
            CLIP_AUDIO_A,
            true,
            Some(LINK_GROUP_ID),
        ),
    ]);

    let args = ClipUnlinkArgs {
        project_id: fixture_project_id(),
        clip: CLIP_VIDEO_A.to_string(),
    };

    let err = compute_patch(&prior, &args).expect_err("not linked should take precedence");
    match err {
        ClipUnlinkError::NotLinked { clip_id } => assert_eq!(clip_id, CLIP_VIDEO_A),
        other => panic!("expected NotLinked, got {other:?}"),
    }
}

#[test]
fn compute_patch_unlink_bad_selector_errors() {
    let prior = project_with_tracks(vec![video_track(
        TRACK_VIDEO_A,
        "Video",
        false,
        CLIP_VIDEO_A,
        false,
        Some(LINK_GROUP_ID),
    )]);

    let err = compute_patch(
        &prior,
        &ClipUnlinkArgs {
            project_id: fixture_project_id(),
            clip: "not-a-uuid".to_string(),
        },
    )
    .expect_err("bad selector should fail");

    match err {
        ClipUnlinkError::BadSelector { detail } => assert!(detail.contains("UUID")),
        other => panic!("expected BadSelector, got {other:?}"),
    }
}

#[test]
fn compute_patch_unlink_clip_not_found_errors() {
    let prior = project_with_tracks(vec![video_track(
        TRACK_VIDEO_A,
        "Video",
        false,
        CLIP_VIDEO_A,
        false,
        Some(LINK_GROUP_ID),
    )]);

    let err = compute_patch(
        &prior,
        &ClipUnlinkArgs {
            project_id: fixture_project_id(),
            clip: MISSING_CLIP.to_string(),
        },
    )
    .expect_err("missing clip should fail");

    match err {
        ClipUnlinkError::ClipNotFound { clip_id } => assert_eq!(clip_id, MISSING_CLIP),
        other => panic!("expected ClipNotFound, got {other:?}"),
    }
}

#[test]
fn compute_patch_unlink_cleared_ids_are_lex_sorted() {
    let prior = project_with_tracks(vec![
        audio_track(
            TRACK_AUDIO_A,
            "Audio",
            false,
            CLIP_VIDEO_SORT_LOW,
            false,
            Some(LINK_GROUP_ID),
        ),
        video_track(
            TRACK_VIDEO_A,
            "Video",
            false,
            CLIP_AUDIO_SORT,
            false,
            Some(LINK_GROUP_ID),
        ),
        audio_track(
            TRACK_AUDIO_B,
            "Audio",
            false,
            CLIP_VIDEO_SORT_HIGH,
            false,
            Some(LINK_GROUP_ID),
        ),
    ]);

    let args = ClipUnlinkArgs {
        project_id: fixture_project_id(),
        clip: CLIP_VIDEO_SORT_LOW.to_string(),
    };

    let (_, _, data) = compute_patch(&prior, &args).expect("scan in reverse insertion order");
    assert_eq!(
        data.cleared_clip_ids
            .iter()
            .map(ClipId::to_string)
            .collect::<Vec<_>>(),
        [CLIP_VIDEO_SORT_LOW, CLIP_AUDIO_SORT, CLIP_VIDEO_SORT_HIGH]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>()
    );
}

#[test]
fn compute_patch_unlink_ops_are_deterministic_by_track_then_clip() {
    let prior = project_with_tracks(vec![
        video_track(
            TRACK_VIDEO_A,
            "Video A",
            false,
            CLIP_VIDEO_A,
            false,
            Some(LINK_GROUP_ID),
        ),
        audio_track(
            TRACK_AUDIO_A,
            "Audio A",
            false,
            CLIP_AUDIO_A,
            false,
            Some(LINK_GROUP_ID),
        ),
        video_track(
            TRACK_VIDEO_B,
            "Video B",
            false,
            CLIP_VIDEO_B,
            false,
            Some(LINK_GROUP_ID),
        ),
    ]);

    let args = ClipUnlinkArgs {
        project_id: fixture_project_id(),
        clip: CLIP_VIDEO_B.to_string(),
    };

    let (patch, _, _) =
        compute_patch(&prior, &args).expect("scan order should stabilize patch order");
    assert_link_group_paths(
        &patch,
        &[
            "/tracks/0/clips/0/link_group",
            "/tracks/0/clips/0/link_group",
            "/tracks/1/clips/0/link_group",
            "/tracks/1/clips/0/link_group",
            "/tracks/2/clips/0/link_group",
            "/tracks/2/clips/0/link_group",
        ],
    );
}

#[test]
fn compute_patch_unlink_patch_is_rfc6902_applyable() {
    let mut prior = project_with_tracks(vec![
        audio_track(
            TRACK_AUDIO_A,
            "Audio",
            false,
            CLIP_AUDIO_A,
            false,
            Some(LINK_GROUP_ID),
        ),
        video_track(
            TRACK_VIDEO_A,
            "Video",
            false,
            CLIP_VIDEO_A,
            false,
            Some(LINK_GROUP_ID),
        ),
    ]);
    add_assets(&mut prior);

    let args = ClipUnlinkArgs {
        project_id: fixture_project_id(),
        clip: CLIP_AUDIO_A.to_string(),
    };

    let (patch, _, _) = compute_patch(&prior, &args).expect("happy path");
    let typed: json_patch::Patch =
        serde_json::from_value(patch).expect("patch value should be RFC6902-compatible");
    prior.apply(&typed).expect("clip.unlink patch should apply");
}

#[test]
fn compute_patch_unlink_post_state_members_cleared() {
    let mut prior = project_with_tracks(vec![
        video_track(
            TRACK_VIDEO_A,
            "Video",
            false,
            CLIP_VIDEO_A,
            false,
            Some(LINK_GROUP_ID),
        ),
        audio_track(
            TRACK_AUDIO_A,
            "Audio",
            false,
            CLIP_AUDIO_A,
            false,
            Some(LINK_GROUP_ID),
        ),
    ]);
    add_assets(&mut prior);

    let args = ClipUnlinkArgs {
        project_id: fixture_project_id(),
        clip: CLIP_AUDIO_A.to_string(),
    };

    let (patch, _, _) = compute_patch(&prior, &args).expect("happy path");
    let typed: json_patch::Patch = serde_json::from_value(patch).expect("patch parses");
    let post_state = prior.apply(&typed).expect("apply succeeds");

    assert!(post_state.tracks[0].clips[0].link_group.is_none());
    assert!(post_state.tracks[1].clips[0].link_group.is_none());
}

#[test]
fn compute_patch_unlink_other_groups_are_untouched() {
    let prior = project_with_tracks(vec![
        video_track(
            TRACK_VIDEO_A,
            "Video A",
            false,
            CLIP_VIDEO_A,
            false,
            Some(LINK_GROUP_ID),
        ),
        video_track(
            TRACK_VIDEO_B,
            "Video B",
            false,
            CLIP_VIDEO_B,
            false,
            Some(LINK_GROUP_ID),
        ),
        audio_track(
            TRACK_AUDIO_A,
            "Audio A",
            false,
            CLIP_AUDIO_A,
            false,
            Some(LINK_GROUP_OTHER),
        ),
    ]);
    let mut prior = prior;
    add_assets(&mut prior);

    let args = ClipUnlinkArgs {
        project_id: fixture_project_id(),
        clip: CLIP_VIDEO_A.to_string(),
    };

    let (patch, _, _) = compute_patch(&prior, &args).expect("happy path");
    let typed: json_patch::Patch = serde_json::from_value(patch).expect("patch parses");
    let post_state = prior.apply(&typed).expect("apply succeeds");

    assert!(post_state.tracks[0].clips[0].link_group.is_none());
    assert!(post_state.tracks[1].clips[0].link_group.is_none());
    assert_eq!(
        post_state.tracks[2].clips[0]
            .link_group
            .expect("other group remains")
            .to_string(),
        LINK_GROUP_OTHER,
    );
}

#[test]
fn reconstruct_round_trip_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "clip.unlink")
        .expect("default_fixtures includes clip.unlink");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(ClipUnlinkVerb))
        .expect("register clip.unlink verb");

    let report = validate_reconstructors(&registry, &[fixture])
        .expect("clip.unlink reconstruct from fixture");
    assert_eq!(report.verbs_checked, vec!["clip.unlink"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn data_envelope_from_patch_and_post_state_recovers_group_and_ids() {
    let mut prior = project_with_tracks(vec![
        audio_track(
            TRACK_AUDIO_A,
            "Audio",
            false,
            CLIP_AUDIO_A,
            false,
            Some(LINK_GROUP_ID),
        ),
        audio_track(
            TRACK_AUDIO_B,
            "Audio B",
            false,
            CLIP_AUDIO_B,
            false,
            Some(LINK_GROUP_ID),
        ),
        video_track(
            TRACK_VIDEO_A,
            "Video",
            false,
            CLIP_VIDEO_A,
            false,
            Some(LINK_GROUP_ID),
        ),
    ]);
    add_assets(&mut prior);

    let args = ClipUnlinkArgs {
        project_id: fixture_project_id(),
        clip: CLIP_VIDEO_A.to_string(),
    };

    let (patch, _, _) = compute_patch(&prior, &args).expect("happy path");
    let typed_patch: json_patch::Patch =
        serde_json::from_value(patch.clone()).expect("patch parses");
    let post_state = prior.apply(&typed_patch).expect("apply succeeds");

    let data = data_envelope_from_patch_and_post_state(&patch, &post_state)
        .expect("envelope from patch+post-state");
    assert_eq!(data.link_group.to_string(), LINK_GROUP_ID);
    assert_eq!(
        data.cleared_clip_ids
            .iter()
            .map(ClipId::to_string)
            .collect::<Vec<_>>(),
        [CLIP_AUDIO_A, CLIP_AUDIO_B, CLIP_VIDEO_A]
            .into_iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
    );
}

#[test]
fn clip_unlink_errors_map_to_bad_args() {
    let prior = project_with_tracks(vec![
        video_track(
            TRACK_VIDEO_A,
            "Video",
            false,
            CLIP_VIDEO_A,
            false,
            Some(LINK_GROUP_ID),
        ),
        audio_track(
            TRACK_AUDIO_A,
            "Audio",
            false,
            CLIP_AUDIO_A,
            false,
            Some(LINK_GROUP_ID),
        ),
    ]);
    let verb = ClipUnlinkVerb;

    let bad_selector = serde_json::to_value(ClipUnlinkArgs {
        project_id: fixture_project_id(),
        clip: "not-a-uuid".to_string(),
    })
    .expect("bad selector args serialize");
    let err = verb
        .compute_patch(&prior, &bad_selector)
        .expect_err("bad selector maps");
    assert!(matches!(err, VerbError::BadArgs { .. }));

    let not_linked = serde_json::to_value(ClipUnlinkArgs {
        project_id: fixture_project_id(),
        clip: CLIP_VIDEO_B.to_string(),
    })
    .expect("not linked args serialize");
    let err = verb
        .compute_patch(
            &project_with_tracks(vec![video_track(
                TRACK_VIDEO_A,
                "Video",
                false,
                CLIP_VIDEO_A,
                false,
                None,
            )]),
            &not_linked,
        )
        .expect_err("not linked maps");
    assert!(matches!(err, VerbError::BadArgs { .. }));

    let member_locked = serde_json::to_value(ClipUnlinkArgs {
        project_id: fixture_project_id(),
        clip: CLIP_VIDEO_A.to_string(),
    })
    .expect("member locked args serialize");
    let err = verb
        .compute_patch(
            &project_with_tracks(vec![video_track(
                TRACK_VIDEO_A,
                "Video",
                false,
                CLIP_VIDEO_A,
                true,
                Some(LINK_GROUP_ID),
            )]),
            &member_locked,
        )
        .expect_err("member locked maps");
    assert!(matches!(err, VerbError::BadArgs { .. }));

    let clip_not_found = serde_json::to_value(ClipUnlinkArgs {
        project_id: fixture_project_id(),
        clip: MISSING_CLIP.to_string(),
    })
    .expect("clip not found args serialize");
    let err = verb
        .compute_patch(&prior, &clip_not_found)
        .expect_err("clip not found maps");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb() {
    use tempfile::TempDir;

    let mut base_project = project_with_tracks(vec![
        video_track(
            TRACK_VIDEO_A,
            "Video",
            false,
            CLIP_VIDEO_A,
            false,
            Some(LINK_GROUP_ID),
        ),
        audio_track(
            TRACK_AUDIO_A,
            "Audio",
            false,
            CLIP_AUDIO_A,
            false,
            Some(LINK_GROUP_ID),
        ),
    ]);
    add_assets(&mut base_project);

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        base_project,
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry clears gate");

    let args = json!({"project_id": FIXTURE_PROJECT_ID, "clip": CLIP_VIDEO_A});
    let outcome = store
        .mutate_via_verb("clip.unlink", args, None)
        .expect("mutate_via_verb happy path");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("happy path must return Applied, got {outcome:?}");
    };

    let data: ClipUnlinkData = serde_json::from_value(data).expect("clip.unlink data");
    assert_eq!(data.link_group.to_string(), LINK_GROUP_ID);
    assert_eq!(warnings, Vec::<Value>::new());
    assert!(store.project().tracks[0].clips[0].link_group.is_none());
    assert!(store.project().tracks[1].clips[0].link_group.is_none());
}
