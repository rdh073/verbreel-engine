//! Tests for `clip.lock` (§5.13) — eighteenth production verb.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::clip_lock::{
    DEFAULT_LOCKED, W_NOOP_CODE, compute_patch, data_envelope_from_post_state,
};
use verbreel_state::{
    ClipLockArgs, ClipLockData, ClipLockError, ClipLockVerb, MutateOutcome, Project, RecordedEvent,
    Track, Verb, VerbError, VerbRegistry, default_fixtures, default_registry,
    validate_reconstructors,
};
use verbreel_types::{ProjectId, Tick};

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const TRACK_TEXT_A: &str = "0190b8d3-15e3-7000-bd00-0000000aa101";
const TRACK_TEXT_B: &str = "0190b8d3-15e3-7000-bd00-0000000aa102";
const CLIP_TEXT_A: &str = "0190b8d3-15e3-7000-bd00-0000000bb101";
const CLIP_TEXT_B: &str = "0190b8d3-15e3-7000-bd00-0000000bb102";
const MISSING_CLIP: &str = "0190b8d3-15e3-7000-bd00-0000000cc101";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture → Project")
}

fn fixture_project_id() -> ProjectId {
    empty_project().id
}

fn text_track(id: &str, name: &str, track_locked: bool, clip_id: &str, clip_locked: bool) -> Track {
    serde_json::from_value(json!({
        "id": id,
        "kind": "text",
        "name": name,
        "locked": track_locked,
        "clips": [{
            "id": clip_id,
            "name": "Clip 1",
            "asset_id": "00000000-0000-0000-0000-000000000000",
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 480_000,
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
    project.duration_tk = Tick::new(480_000);
    project
}

fn patch_lock_value(patch: &Value) -> bool {
    let arr = patch.as_array().expect("patch is an array");
    assert_eq!(arr.len(), 1, "clip.lock emits one replace op");
    let op = arr[0].as_object().expect("patch op is object");
    assert_eq!(op.get("op").and_then(Value::as_str), Some("replace"));
    op.get("value")
        .and_then(Value::as_bool)
        .expect("replace op value is bool")
}

#[test]
fn compute_patch_lock_unlocked_clip() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
    )]);

    let args = ClipLockArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_A.to_string(),
        locked: Some(true),
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("happy-path lock");
    assert!(patch_lock_value(&patch));
    assert!(warnings.is_empty());
    assert_eq!(data.clip_id.to_string(), CLIP_TEXT_A);
    assert!(data.locked);
    assert_eq!(
        patch.as_array().expect("patch is array")[0]
            .get("path")
            .and_then(Value::as_str),
        Some("/tracks/0/clips/0/locked")
    );
}

#[test]
fn compute_patch_unlock_locked_clip() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        true,
    )]);

    let args = ClipLockArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_A.to_string(),
        locked: Some(false),
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("happy-path unlock");
    assert!(!patch_lock_value(&patch));
    assert!(warnings.is_empty());
    assert_eq!(data.clip_id.to_string(), CLIP_TEXT_A);
    assert!(!data.locked);
}

#[test]
fn compute_patch_defaulted_to_true() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
    )]);

    let args = ClipLockArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_A.to_string(),
        locked: None,
    };

    let (patch, warnings, data) =
        compute_patch(&prior, &args).expect("omitted locked defaults true");
    assert_eq!(patch_lock_value(&patch), DEFAULT_LOCKED);
    assert!(warnings.is_empty());
    assert_eq!(data.clip_id.to_string(), CLIP_TEXT_A);
    assert!(data.locked);
}

#[test]
fn compute_patch_idempotent_already_locked_emits_w_noop() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        true,
    )]);

    let args = ClipLockArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_A.to_string(),
        locked: Some(true),
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("idempotent lock is a no-op");
    assert!(patch.as_array().expect("patch is array").is_empty());
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_NOOP_CODE);
    assert_eq!(warnings[0]["message"], "clip lock state unchanged");
    assert_eq!(warnings[0]["details"]["clip_id"], CLIP_TEXT_A);
    assert_eq!(warnings[0]["details"]["locked"], true);
    assert_eq!(data.clip_id.to_string(), CLIP_TEXT_A);
    assert!(data.locked);
}

#[test]
fn compute_patch_idempotent_already_unlocked_emits_w_noop() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
    )]);

    let args = ClipLockArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_A.to_string(),
        locked: Some(false),
    };

    let (patch, warnings, data) =
        compute_patch(&prior, &args).expect("idempotent unlock is a no-op");
    assert!(patch.as_array().expect("patch is array").is_empty());
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_NOOP_CODE);
    assert_eq!(warnings[0]["message"], "clip lock state unchanged");
    assert_eq!(warnings[0]["details"]["clip_id"], CLIP_TEXT_A);
    assert_eq!(warnings[0]["details"]["locked"], false);
    assert_eq!(data.clip_id.to_string(), CLIP_TEXT_A);
    assert!(!data.locked);
}

#[test]
fn compute_patch_none_on_already_unlocked_still_patches() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
    )]);

    let args = ClipLockArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_A.to_string(),
        locked: None,
    };

    let (patch, warnings, data) =
        compute_patch(&prior, &args).expect("defaulted true should mutate");
    assert!(patch_lock_value(&patch));
    assert!(warnings.is_empty());
    assert_eq!(data.clip_id.to_string(), CLIP_TEXT_A);
    assert!(data.locked);
}

#[test]
fn compute_patch_clip_lock_true_when_locked_but_not_checked() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        true,
    )]);

    let args = ClipLockArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_A.to_string(),
        locked: Some(false),
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("locked clip can be toggled");
    assert!(!patch_lock_value(&patch));
    assert!(warnings.is_empty());
    assert_eq!(data.clip_id.to_string(), CLIP_TEXT_A);
    assert!(!data.locked);
}

#[test]
fn compute_patch_track_lock_true_does_not_block_clip_lock() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        true,
        CLIP_TEXT_A,
        false,
    )]);

    let args = ClipLockArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_A.to_string(),
        locked: Some(true),
    };

    let (patch, warnings, data) =
        compute_patch(&prior, &args).expect("track lock should not block clip.lock");
    assert!(patch_lock_value(&patch));
    assert!(warnings.is_empty());
    assert_eq!(data.clip_id.to_string(), CLIP_TEXT_A);
    assert!(data.locked);
}

#[test]
fn compute_patch_bad_uuid_errors() {
    let prior = project_with_tracks(vec![]);

    let err = compute_patch(
        &prior,
        &ClipLockArgs {
            project_id: fixture_project_id(),
            clip: "not-a-uuid".to_string(),
            locked: Some(true),
        },
    )
    .expect_err("bad clip selector must reject");

    match err {
        ClipLockError::BadSelector { detail } => {
            assert!(detail.contains("UUID"), "{detail}");
        }
        other => panic!("expected BadSelector, got {other:?}"),
    }
}

#[test]
fn compute_patch_clip_not_found_errors() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
    )]);

    let err = compute_patch(
        &prior,
        &ClipLockArgs {
            project_id: fixture_project_id(),
            clip: MISSING_CLIP.to_string(),
            locked: Some(true),
        },
    )
    .expect_err("missing clip must reject");

    match err {
        ClipLockError::ClipNotFound { clip_id } => assert_eq!(clip_id, MISSING_CLIP),
        other => panic!("expected ClipNotFound, got {other:?}"),
    }
}

#[test]
fn data_envelope_returns_post_state_locked() {
    let mut post_state = empty_project();
    post_state.tracks = vec![text_track(TRACK_TEXT_A, "Text 1", false, CLIP_TEXT_A, true)];

    let args = ClipLockArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_A.to_string(),
        locked: Some(false),
    };

    let data = data_envelope_from_post_state(&args, &post_state)
        .expect("post-state envelope should be readable");
    assert_eq!(data.clip_id.to_string(), CLIP_TEXT_A);
    assert!(data.locked);
}

#[test]
fn reconstructor_round_trip() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
    )]);
    let args = ClipLockArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_A.to_string(),
        locked: Some(true),
    };

    let (patch, _warnings, _data) = compute_patch(&prior, &args).expect("compute_patch ok");
    let typed_patch: json_patch::Patch =
        serde_json::from_value(patch.clone()).expect("patch parses to typed JSON patch");
    let post_state = prior
        .apply(&typed_patch)
        .expect("clip.lock patch should apply cleanly");

    let expected_data =
        serde_json::to_value(data_envelope_from_post_state(&args, &post_state).expect("envelope"))
            .expect("envelope serializes");

    let recorded = RecordedEvent {
        verb: "clip.lock".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch,
        warnings: vec![],
        post_state,
        expected_data,
    };

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(ClipLockVerb))
        .expect("register clip.lock verb");

    let report = validate_reconstructors(&registry, std::slice::from_ref(&recorded))
        .expect("reconstructor validation should pass");
    assert_eq!(report.verbs_checked, vec!["clip.lock"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn compute_patch_error_variants_map_to_bad_args() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
    )]);
    let verb = ClipLockVerb;

    let bad_selector = serde_json::to_value(ClipLockArgs {
        project_id: fixture_project_id(),
        clip: "not-a-uuid".to_string(),
        locked: Some(true),
    })
    .expect("bad selector args serialize");
    let err = verb
        .compute_patch(&prior, &bad_selector)
        .expect_err("bad selector maps");
    assert!(matches!(err, VerbError::BadArgs { .. }));

    let clip_not_found = serde_json::to_value(ClipLockArgs {
        project_id: fixture_project_id(),
        clip: MISSING_CLIP.to_string(),
        locked: Some(true),
    })
    .expect("missing clip args serialize");
    let err = verb
        .compute_patch(&prior, &clip_not_found)
        .expect_err("missing clip maps");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "clip.lock")
        .expect("default_fixtures includes clip.lock");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(ClipLockVerb))
        .expect("register clip.lock verb");

    let report =
        validate_reconstructors(&registry, &[fixture]).expect("clip.lock reconstruct from fixture");
    assert_eq!(report.verbs_checked, vec!["clip.lock"]);
    assert_eq!(report.fixtures_run, 1);
}

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let base_project = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
    )]);
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        base_project,
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry clears the gate and writes project.json");

    let args = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "clip": CLIP_TEXT_A,
        "locked": true,
    });

    let outcome = store
        .mutate_via_verb("clip.lock", args, None)
        .expect("mutate_via_verb happy path");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("happy path must return Applied, got {outcome:?}");
    };

    let data: ClipLockData = serde_json::from_value(data).expect("clip.lock data is ClipLockData");
    assert_eq!(data.clip_id.to_string(), CLIP_TEXT_A);
    assert!(store.project().tracks[0].clips[0].locked);
    assert!(data.locked);
    assert_eq!(warnings, Vec::<Value>::new());
}

#[test]
fn multi_track_clip_resolution_uses_track_index() {
    let prior = project_with_tracks(vec![
        text_track(TRACK_TEXT_A, "Text 1", false, CLIP_TEXT_A, false),
        text_track(TRACK_TEXT_B, "Text 2", false, CLIP_TEXT_B, false),
    ]);

    let args = ClipLockArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_B.to_string(),
        locked: Some(true),
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("search in second track");
    assert!(patch_lock_value(&patch));
    assert!(warnings.is_empty());
    assert_eq!(
        patch.as_array().expect("patch is array")[0]
            .get("path")
            .and_then(Value::as_str),
        Some("/tracks/1/clips/0/locked")
    );
    assert_eq!(data.clip_id.to_string(), CLIP_TEXT_B);
    assert!(data.locked);
}
