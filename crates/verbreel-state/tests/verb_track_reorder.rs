//! Tests for `track.reorder` (§4.3) — seventeenth production verb.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::{
    MutateOutcome, Project, Track, TrackAddData, TrackKind, TrackReorderArgs, TrackReorderData,
    TrackReorderError, TrackReorderVerb, Verb, VerbError, VerbRegistry, default_fixtures,
    default_registry, validate_reconstructors,
};
use verbreel_types::ProjectId;

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";

const TRACK_VIDEO_A: &str = "0190b8d3-15e3-7000-bd00-0000000aa101";
const TRACK_VIDEO_B: &str = "0190b8d3-15e3-7000-bd00-0000000aa102";
const TRACK_VIDEO_C: &str = "0190b8d3-15e3-7000-bd00-0000000aa103";
const TRACK_AUDIO_A: &str = "0190b8d3-15e3-7000-bd00-0000000bb101";
const TRACK_AUDIO_B: &str = "0190b8d3-15e3-7000-bd00-0000000bb102";
const MISSING_TRACK: &str = "0190b8d3-15e3-7000-bd00-0000000cc101";

use verbreel_state::verbs::track_reorder::{
    W_NOOP_CODE, compute_patch, data_envelope_from_patch_and_post_state,
};

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture → Project")
}

fn fixture_project_id() -> ProjectId {
    empty_project().id
}

fn track(id: &str, kind: TrackKind, name: &str) -> Track {
    serde_json::from_value(json!({
        "id": id,
        "kind": kind,
        "name": name,
        "clips": [],
    }))
    .expect("track fixture value should deserialize")
}

fn project_with_tracks(tracks: Vec<Track>) -> Project {
    let mut project = empty_project();
    project.tracks = tracks;
    project
}

fn reorder_patch_indices(patch: &Value) -> (usize, usize) {
    let arr = patch.as_array().expect("patch is array");
    assert_eq!(arr.len(), 1, "reorder emits one move op");
    let op = arr[0].as_object().expect("patch op is object");
    assert_eq!(op.get("op").and_then(Value::as_str), Some("move"));

    let parse = |field| {
        let path = op
            .get(field)
            .and_then(Value::as_str)
            .expect("patch path exists");
        path.strip_prefix("/tracks/")
            .expect("path under /tracks")
            .parse::<usize>()
            .expect("path index parses")
    };

    (parse("from"), parse("path"))
}

#[test]
fn compute_patch_move_second_video_to_zero() {
    let prior = project_with_tracks(vec![
        track(TRACK_VIDEO_A, TrackKind::Video, "Video A"),
        track(TRACK_VIDEO_B, TrackKind::Video, "Video B"),
    ]);

    let args = TrackReorderArgs {
        project_id: fixture_project_id(),
        track: TRACK_VIDEO_B.to_string(),
        to_index: 0,
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("happy-path reorder");
    let (from_global, to_global) = reorder_patch_indices(&patch);
    assert_eq!(from_global, 1);
    assert_eq!(to_global, 0);
    assert!(warnings.is_empty());
    assert_eq!(data.track_id.to_string(), TRACK_VIDEO_B);
    assert_eq!(data.kind, "video");
    assert_eq!(data.from_index, 1);
    assert_eq!(data.to_index, 0);
}

#[test]
fn compute_patch_move_first_video_to_tail() {
    let prior = project_with_tracks(vec![
        track(TRACK_VIDEO_A, TrackKind::Video, "Video A"),
        track(TRACK_VIDEO_B, TrackKind::Video, "Video B"),
        track(TRACK_VIDEO_C, TrackKind::Video, "Video C"),
    ]);

    let args = TrackReorderArgs {
        project_id: fixture_project_id(),
        track: TRACK_VIDEO_A.to_string(),
        to_index: 2,
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("happy-path reorder");
    let (from_global, to_global) = reorder_patch_indices(&patch);
    assert_eq!(from_global, 0);
    assert_eq!(to_global, 2);
    assert!(warnings.is_empty());
    assert_eq!(data.track_id.to_string(), TRACK_VIDEO_A);
    assert_eq!(data.kind, "video");
    assert_eq!(data.from_index, 0);
    assert_eq!(data.to_index, 2);
}

#[test]
fn compute_patch_noop_emits_w_noop() {
    let prior = project_with_tracks(vec![track(TRACK_VIDEO_A, TrackKind::Video, "Video A")]);

    let args = TrackReorderArgs {
        project_id: fixture_project_id(),
        track: TRACK_VIDEO_A.to_string(),
        to_index: 0,
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("single-track no-op");
    assert!(patch.as_array().expect("patch is array").is_empty());
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_NOOP_CODE);
    assert_eq!(warnings[0]["message"], "track position unchanged");
    assert_eq!(warnings[0]["details"]["track_id"], TRACK_VIDEO_A);
    assert_eq!(warnings[0]["details"]["kind"], "video");
    assert_eq!(warnings[0]["details"]["index"], 0);
    assert_eq!(data.from_index, 0);
    assert_eq!(data.to_index, 0);
}

#[test]
fn compute_patch_mixed_kinds_preserves_other_kind_contiguity() {
    let prior = project_with_tracks(vec![
        track(TRACK_VIDEO_A, TrackKind::Video, "Video A"),
        track(TRACK_VIDEO_B, TrackKind::Video, "Video B"),
        track(TRACK_AUDIO_A, TrackKind::Audio, "Audio A"),
        track(TRACK_AUDIO_B, TrackKind::Audio, "Audio B"),
    ]);

    let args = TrackReorderArgs {
        project_id: fixture_project_id(),
        track: TRACK_VIDEO_A.to_string(),
        to_index: 1,
    };

    let (_patch, warnings, _) = compute_patch(&prior, &args).expect("mixed-kind reorder");
    let typed_patch: json_patch::Patch = serde_json::from_value(_patch).expect("patch parses");
    let post_state = prior
        .apply(&typed_patch)
        .expect("track.reorder patch should apply");

    assert!(warnings.is_empty());
    assert_eq!(post_state.tracks[0].kind, TrackKind::Video);
    assert_eq!(post_state.tracks[1].kind, TrackKind::Video);
    assert_eq!(post_state.tracks[2].kind, TrackKind::Audio);
    assert_eq!(post_state.tracks[3].kind, TrackKind::Audio);
}

#[test]
fn compute_patch_bad_index_negative() {
    let prior = project_with_tracks(vec![
        track(TRACK_VIDEO_A, TrackKind::Video, "Video A"),
        track(TRACK_VIDEO_B, TrackKind::Video, "Video B"),
    ]);

    let err = compute_patch(
        &prior,
        &TrackReorderArgs {
            project_id: fixture_project_id(),
            track: TRACK_VIDEO_B.to_string(),
            to_index: -1,
        },
    )
    .expect_err("negative index must reject");

    match err {
        TrackReorderError::BadIndex {
            to_index,
            kind_count,
        } => {
            assert_eq!(to_index, -1);
            assert_eq!(kind_count, 2);
        }
        other => panic!("expected BadIndex, got {other:?}"),
    }
}

#[test]
fn compute_patch_bad_index_too_large() {
    let prior = project_with_tracks(vec![
        track(TRACK_VIDEO_A, TrackKind::Video, "Video A"),
        track(TRACK_VIDEO_B, TrackKind::Video, "Video B"),
    ]);

    let err = compute_patch(
        &prior,
        &TrackReorderArgs {
            project_id: fixture_project_id(),
            track: TRACK_VIDEO_B.to_string(),
            to_index: 2,
        },
    )
    .expect_err("too-large index must reject");

    match err {
        TrackReorderError::BadIndex {
            to_index,
            kind_count,
        } => {
            assert_eq!(to_index, 2);
            assert_eq!(kind_count, 2);
        }
        other => panic!("expected BadIndex, got {other:?}"),
    }
}

#[test]
fn compute_patch_single_video_noop() {
    let prior = project_with_tracks(vec![track(TRACK_VIDEO_A, TrackKind::Video, "Video A")]);

    let (patch, warnings, data) = compute_patch(
        &prior,
        &TrackReorderArgs {
            project_id: fixture_project_id(),
            track: TRACK_VIDEO_A.to_string(),
            to_index: 0,
        },
    )
    .expect("single-track case should be no-op");

    assert!(patch.as_array().expect("patch is array").is_empty());
    assert_eq!(warnings.len(), 1);
    assert_eq!(data.from_index, 0);
    assert_eq!(data.to_index, 0);
}

#[test]
fn compute_patch_bad_selector() {
    let prior = project_with_tracks(vec![]);

    let err = compute_patch(
        &prior,
        &TrackReorderArgs {
            project_id: fixture_project_id(),
            track: "not-a-uuid".to_string(),
            to_index: 0,
        },
    )
    .expect_err("bad selector must reject");

    match err {
        TrackReorderError::BadSelector { detail } => assert!(detail.contains("UUID")),
        other => panic!("expected BadSelector, got {other:?}"),
    }
}

#[test]
fn compute_patch_track_not_found() {
    let prior = project_with_tracks(vec![track(TRACK_VIDEO_A, TrackKind::Video, "Video A")]);

    let err = compute_patch(
        &prior,
        &TrackReorderArgs {
            project_id: fixture_project_id(),
            track: MISSING_TRACK.to_string(),
            to_index: 0,
        },
    )
    .expect_err("missing track must reject");

    match err {
        TrackReorderError::TrackNotFound { track_id } => assert_eq!(track_id, MISSING_TRACK),
        other => panic!("expected TrackNotFound, got {other:?}"),
    }
}

#[test]
fn track_contiguity_invariant_holds() {
    let prior = project_with_tracks(vec![
        track(TRACK_VIDEO_A, TrackKind::Video, "Video A"),
        track(TRACK_VIDEO_B, TrackKind::Video, "Video B"),
        track(TRACK_AUDIO_A, TrackKind::Audio, "Audio A"),
        track(TRACK_AUDIO_B, TrackKind::Audio, "Audio B"),
    ]);

    let args = TrackReorderArgs {
        project_id: fixture_project_id(),
        track: TRACK_VIDEO_B.to_string(),
        to_index: 0,
    };

    let (patch, _, _) = compute_patch(&prior, &args).expect("mixed-kind reorder");
    let typed_patch: json_patch::Patch = serde_json::from_value(patch).expect("patch parses");
    let post_state = prior
        .apply(&typed_patch)
        .expect("reorder patch should apply");

    let video_positions: Vec<usize> = post_state
        .tracks
        .iter()
        .enumerate()
        .filter_map(|(idx, track)| (track.kind == TrackKind::Video).then_some(idx))
        .collect();

    assert_eq!(video_positions, vec![0, 1]);
}

#[test]
fn track_identities_are_preserved() {
    let prior = project_with_tracks(vec![
        track(TRACK_VIDEO_A, TrackKind::Video, "Video A"),
        track(TRACK_VIDEO_B, TrackKind::Video, "Video B"),
        track(TRACK_AUDIO_A, TrackKind::Audio, "Audio A"),
    ]);

    let ids_before: Vec<_> = prior
        .tracks
        .iter()
        .map(|track| track.id.to_string())
        .collect();

    let args = TrackReorderArgs {
        project_id: fixture_project_id(),
        track: TRACK_VIDEO_B.to_string(),
        to_index: 0,
    };

    let (patch, _, _) = compute_patch(&prior, &args).expect("happy-path reorder");
    let patch: json_patch::Patch = serde_json::from_value(patch).expect("patch parses");
    let post_state = prior.apply(&patch).expect("patch applies");

    let ids_after: Vec<_> = post_state
        .tracks
        .iter()
        .map(|track| track.id.to_string())
        .collect();

    let mut before = ids_before;
    let mut after = ids_after;
    before.sort_unstable();
    after.sort_unstable();
    assert_eq!(before, after);
}

#[test]
fn round_trip_reconstruction_from_compute_patch() {
    let prior = project_with_tracks(vec![
        track(TRACK_VIDEO_A, TrackKind::Video, "Video A"),
        track(TRACK_VIDEO_B, TrackKind::Video, "Video B"),
        track(TRACK_VIDEO_C, TrackKind::Video, "Video C"),
    ]);

    let args = TrackReorderArgs {
        project_id: fixture_project_id(),
        track: TRACK_VIDEO_A.to_string(),
        to_index: 2,
    };

    let (patch, _warnings, recorded) = compute_patch(&prior, &args).expect("compute patch");
    let patch_ops: json_patch::Patch = serde_json::from_value(patch.clone()).expect("typed patch");
    let post_state = prior.apply(&patch_ops).expect("patch applies");
    let reconstructed = data_envelope_from_patch_and_post_state(&patch, &args, &post_state)
        .expect("reconstructed envelope");
    assert_eq!(recorded, reconstructed);
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "track.reorder")
        .expect("default_fixtures includes track.reorder");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(TrackReorderVerb))
        .expect("register track.reorder verb");

    let report =
        validate_reconstructors(&registry, &[fixture]).expect("reconstruction from fixture");
    assert_eq!(report.verbs_checked, vec!["track.reorder"]);
}

#[test]
fn compute_patch_errors_map_to_bad_args() {
    let prior = project_with_tracks(vec![track(TRACK_VIDEO_A, TrackKind::Video, "Video A")]);

    let verb = TrackReorderVerb;

    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": fixture_project_id(),
                "track": "not-a-uuid",
                "to_index": 0,
            }),
        )
        .expect_err("bad selector maps");
    assert!(matches!(err, VerbError::BadArgs { .. }));

    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": fixture_project_id(),
                "track": MISSING_TRACK,
                "to_index": 0,
            }),
        )
        .expect_err("missing track maps");
    assert!(matches!(err, VerbError::BadArgs { .. }));

    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": fixture_project_id(),
                "track": TRACK_VIDEO_A,
                "to_index": 2,
            }),
        )
        .expect_err("bad index maps");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        empty_project(),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create with registry");

    let first_add = store
        .mutate_via_verb(
            "track.add",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "kind": "video",
            }),
            None,
        )
        .expect("add first track");

    let MutateOutcome::Applied {
        data: first_data, ..
    } = first_add
    else {
        panic!("track.add should apply, got {first_add:?}");
    };

    let TrackAddData {
        track_id: _track_id,
        ..
    } = serde_json::from_value(first_data).expect("track.add data should be TrackAddData");

    let second_add = store
        .mutate_via_verb(
            "track.add",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "kind": "video",
            }),
            None,
        )
        .expect("add second track");

    let MutateOutcome::Applied {
        data: second_data, ..
    } = second_add
    else {
        panic!("track.add should apply, got {second_add:?}");
    };

    let TrackAddData {
        track_id: second_track_id,
        ..
    } = serde_json::from_value(second_data).expect("track.add data should be TrackAddData");

    let outcome = store
        .mutate_via_verb(
            "track.reorder",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "track": second_track_id.to_string(),
                "to_index": 0,
            }),
            None,
        )
        .expect("reorder track");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("reorder should apply, got {outcome:?}");
    };

    let data: TrackReorderData = serde_json::from_value(data).expect("reorder data");
    assert_eq!(data.track_id, second_track_id);
    assert_eq!(data.kind, "video");
    assert_eq!(data.from_index, 2);
    assert_eq!(data.to_index, 0);
    assert!(warnings.is_empty());

    let reordered_first = store
        .project()
        .tracks
        .first()
        .expect("project has at least one track after reorder");
    assert_eq!(reordered_first.id, second_track_id);
}

#[test]
fn data_kind_is_lowercase_audio() {
    let prior = project_with_tracks(vec![
        track(TRACK_AUDIO_A, TrackKind::Audio, "Audio A"),
        track(TRACK_AUDIO_B, TrackKind::Audio, "Audio B"),
    ]);

    let args = TrackReorderArgs {
        project_id: fixture_project_id(),
        track: TRACK_AUDIO_B.to_string(),
        to_index: 0,
    };

    let (_patch, _, data) = compute_patch(&prior, &args).expect("audio reorder");
    assert_eq!(data.kind, "audio");
}
