//! Tests for `clip.rename` (§5.17) — nineteenth production verb.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::clip_rename::{
    W_NOOP_CODE, compute_patch, data_envelope_from_post_state,
};
use verbreel_state::{
    ClipRenameArgs, ClipRenameData, ClipRenameError, ClipRenameVerb, MutateOutcome, Project,
    RecordedEvent, Track, TrackKind, Verb, VerbError, VerbRegistry, default_fixtures,
    default_registry, validate_reconstructors,
};
use verbreel_types::ProjectId;
use verbreel_types::Tick;

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

fn text_track(
    id: &str,
    name: &str,
    track_locked: bool,
    clip_id: &str,
    clip_name: &str,
    clip_locked: bool,
) -> Track {
    serde_json::from_value(json!({
        "id": id,
        "kind": TrackKind::Text,
        "name": name,
        "locked": track_locked,
        "clips": [{
            "id": clip_id,
            "name": clip_name,
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

fn patch_rename_name(patch: &Value) -> &Value {
    let arr = patch.as_array().expect("patch is an array");
    assert_eq!(arr.len(), 1, "rename emits one replace op");
    let op = arr[0].as_object().expect("patch op is object");
    assert_eq!(op.get("op").and_then(Value::as_str), Some("replace"));
    op.get("value").expect("replace op carries a value")
}

#[test]
fn compute_patch_simple_rename_succeeds() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        "Clip 1",
        false,
    )]);

    let args = ClipRenameArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_A.to_string(),
        name: "Renamed".to_string(),
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("happy-path rename");
    assert_eq!(patch_rename_name(&patch), "Renamed");
    assert!(warnings.is_empty());
    assert_eq!(data.clip_id.to_string(), CLIP_TEXT_A);
    assert_eq!(data.name, "Renamed");
    assert_eq!(
        patch.as_array().expect("patch is array")[0]
            .get("path")
            .and_then(Value::as_str),
        Some("/tracks/0/clips/0/name")
    );
}

#[test]
fn compute_patch_1_char_name_ok() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        "Clip 1",
        false,
    )]);

    let args = ClipRenameArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_A.to_string(),
        name: "A".to_string(),
    };

    let (patch, warnings, data) =
        compute_patch(&prior, &args).expect("single-char name should pass");
    assert_eq!(patch_rename_name(&patch), "A");
    assert!(warnings.is_empty());
    assert_eq!(data.name, "A");
}

#[test]
fn compute_patch_128_unicode_chars_ok() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        "Clip 1",
        false,
    )]);

    let name = "界".repeat(128);
    assert_eq!(name.chars().count(), 128);

    let args = ClipRenameArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_A.to_string(),
        name: name.clone(),
    };

    let (patch, warnings, data) =
        compute_patch(&prior, &args).expect("unicode 128 chars should pass");
    assert_eq!(patch_rename_name(&patch), name.as_str());
    assert!(warnings.is_empty());
    assert_eq!(data.name, args.name);
}

#[test]
fn compute_patch_128_ascii_name_ok() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        "Clip 1",
        false,
    )]);
    let name = "a".repeat(128);
    assert_eq!(name.chars().count(), 128);

    let args = ClipRenameArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_A.to_string(),
        name: name.clone(),
    };

    let (patch, warnings, data) =
        compute_patch(&prior, &args).expect("ascii 128 chars should pass");
    assert_eq!(patch_rename_name(&patch), name.as_str());
    assert!(warnings.is_empty());
    assert_eq!(data.name, args.name);
}

#[test]
fn compute_patch_idempotent_same_name_emits_w_noop() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        "Clip 1",
        false,
    )]);

    let args = ClipRenameArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_A.to_string(),
        name: "Clip 1".to_string(),
    };

    let (patch, warnings, data) =
        compute_patch(&prior, &args).expect("same-name rename should no-op");
    assert!(patch.as_array().expect("patch is array").is_empty());
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_NOOP_CODE);
    assert_eq!(warnings[0]["message"], "clip name unchanged");
    assert_eq!(warnings[0]["details"]["clip_id"], CLIP_TEXT_A);
    assert_eq!(warnings[0]["details"]["name"], "Clip 1");
    assert_eq!(data.clip_id.to_string(), CLIP_TEXT_A);
    assert_eq!(data.name, "Clip 1");
}

#[test]
fn compute_patch_empty_name_errors() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        "Clip 1",
        false,
    )]);

    let err = compute_patch(
        &prior,
        &ClipRenameArgs {
            project_id: fixture_project_id(),
            clip: CLIP_TEXT_A.to_string(),
            name: "".to_string(),
        },
    )
    .expect_err("empty names must reject");

    match err {
        ClipRenameError::SchemaViolation { detail } => {
            assert_eq!(detail, "name length out of range [1, 128]");
        }
        other => panic!("expected SchemaViolation, got {other:?}"),
    }
}

#[test]
fn compute_patch_129_ascii_name_errors() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        "Clip 1",
        false,
    )]);

    let err = compute_patch(
        &prior,
        &ClipRenameArgs {
            project_id: fixture_project_id(),
            clip: CLIP_TEXT_A.to_string(),
            name: "a".repeat(129),
        },
    )
    .expect_err("too-long ASCII names must reject");

    match err {
        ClipRenameError::SchemaViolation { detail } => {
            assert_eq!(detail, "name length out of range [1, 128]");
        }
        other => panic!("expected SchemaViolation, got {other:?}"),
    }
}

#[test]
fn compute_patch_129_unicode_name_errors() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        "Clip 1",
        false,
    )]);
    let name = "界".repeat(129);
    assert_eq!(name.chars().count(), 129);

    let err = compute_patch(
        &prior,
        &ClipRenameArgs {
            project_id: fixture_project_id(),
            clip: CLIP_TEXT_A.to_string(),
            name,
        },
    )
    .expect_err("unicode too-long names must reject");

    match err {
        ClipRenameError::SchemaViolation { detail } => {
            assert_eq!(detail, "name length out of range [1, 128]");
        }
        other => panic!("expected SchemaViolation, got {other:?}"),
    }
}

#[test]
fn compute_patch_locked_clip_errors() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        "Clip 1",
        true,
    )]);

    let err = compute_patch(
        &prior,
        &ClipRenameArgs {
            project_id: fixture_project_id(),
            clip: CLIP_TEXT_A.to_string(),
            name: "Renamed".to_string(),
        },
    )
    .expect_err("locked clip must reject");

    match err {
        ClipRenameError::Locked { clip_id } => assert_eq!(clip_id, CLIP_TEXT_A),
        other => panic!("expected Locked, got {other:?}"),
    }
}

#[test]
fn compute_patch_track_lock_true_does_not_block_clip_rename() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        true,
        CLIP_TEXT_A,
        "Clip 1",
        false,
    )]);

    let args = ClipRenameArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_A.to_string(),
        name: "Renamed".to_string(),
    };

    let (patch, warnings, data) =
        compute_patch(&prior, &args).expect("track lock should not block clip.rename");
    assert_eq!(patch_rename_name(&patch), "Renamed");
    assert!(warnings.is_empty());
    assert_eq!(data.clip_id.to_string(), CLIP_TEXT_A);
    assert_eq!(data.name, "Renamed");
}

#[test]
fn compute_patch_bad_uuid_errors() {
    let prior = project_with_tracks(vec![]);

    let err = compute_patch(
        &prior,
        &ClipRenameArgs {
            project_id: fixture_project_id(),
            clip: "not-a-uuid".to_string(),
            name: "Renamed".to_string(),
        },
    )
    .expect_err("bad clip selector must reject");

    match err {
        ClipRenameError::BadSelector { detail } => {
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
        "Clip 1",
        false,
    )]);

    let err = compute_patch(
        &prior,
        &ClipRenameArgs {
            project_id: fixture_project_id(),
            clip: MISSING_CLIP.to_string(),
            name: "Renamed".to_string(),
        },
    )
    .expect_err("missing clip must reject");

    match err {
        ClipRenameError::ClipNotFound { clip_id } => assert_eq!(clip_id, MISSING_CLIP),
        other => panic!("expected ClipNotFound, got {other:?}"),
    }
}

#[test]
fn reconstructor_round_trip() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        "Clip 1",
        false,
    )]);
    let args = ClipRenameArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_A.to_string(),
        name: "Renamed".to_string(),
    };

    let (patch, _warnings, _data) = compute_patch(&prior, &args).expect("compute_patch ok");
    let typed_patch: json_patch::Patch =
        serde_json::from_value(patch.clone()).expect("patch parses to RFC 6902");
    let post_state = prior
        .apply(&typed_patch)
        .expect("clip.rename patch should apply cleanly");

    let expected_data = serde_json::to_value(
        data_envelope_from_post_state(&args, &post_state)
            .expect("envelope from post-state should be readable"),
    )
    .expect("envelope serializes");

    let recorded = RecordedEvent {
        verb: "clip.rename".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch,
        warnings: vec![],
        post_state,
        expected_data,
    };

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(ClipRenameVerb))
        .expect("register clip.rename verb");

    let report = validate_reconstructors(&registry, std::slice::from_ref(&recorded))
        .expect("reconstructor validation should pass");
    assert_eq!(report.verbs_checked, vec!["clip.rename"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn compute_patch_error_variants_map_to_bad_args() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        "Clip 1",
        false,
    )]);
    let verb = ClipRenameVerb;

    let bad_selector = serde_json::to_value(ClipRenameArgs {
        project_id: fixture_project_id(),
        clip: "not-a-uuid".to_string(),
        name: "Renamed".to_string(),
    })
    .expect("bad selector args serialize");
    let err = verb
        .compute_patch(&prior, &bad_selector)
        .expect_err("bad selector maps");
    assert!(matches!(err, VerbError::BadArgs { .. }));

    let clip_not_found = serde_json::to_value(ClipRenameArgs {
        project_id: fixture_project_id(),
        clip: MISSING_CLIP.to_string(),
        name: "Renamed".to_string(),
    })
    .expect("missing clip args serialize");
    let err = verb
        .compute_patch(&prior, &clip_not_found)
        .expect_err("missing clip maps");
    assert!(matches!(err, VerbError::BadArgs { .. }));

    let locked_clip = serde_json::to_value(ClipRenameArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_A.to_string(),
        name: "Renamed".to_string(),
    })
    .expect("locked clip args serialize");
    let locked_state = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        "Clip 1",
        true,
    )]);
    let err = verb
        .compute_patch(&locked_state, &locked_clip)
        .expect_err("locked clip maps");
    assert!(matches!(err, VerbError::BadArgs { .. }));

    let schema_violation = serde_json::to_value(ClipRenameArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_A.to_string(),
        name: "".to_string(),
    })
    .expect("schema violation args serialize");
    let err = verb
        .compute_patch(&prior, &schema_violation)
        .expect_err("schema violation maps");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "clip.rename")
        .expect("default_fixtures includes clip.rename");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(ClipRenameVerb))
        .expect("register clip.rename verb");

    let report = validate_reconstructors(&registry, &[fixture])
        .expect("clip.rename reconstruct from fixture");
    assert_eq!(report.verbs_checked, vec!["clip.rename"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn data_envelope_from_post_state_returns_post_name() {
    let post_state = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        "Renamed",
        false,
    )]);
    let args = ClipRenameArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_A.to_string(),
        name: "Renamed".to_string(),
    };

    let data = data_envelope_from_post_state(&args, &post_state)
        .expect("post-state envelope should be readable");
    assert_eq!(data.clip_id.to_string(), CLIP_TEXT_A);
    assert_eq!(data.name, "Renamed");
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
        "Clip 1",
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
        "name": "Renamed",
    });

    let outcome = store
        .mutate_via_verb("clip.rename", args, None)
        .expect("mutate_via_verb happy path");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("happy path must return Applied, got {outcome:?}");
    };

    let data: ClipRenameData =
        serde_json::from_value(data).expect("clip.rename data is ClipRenameData");
    assert_eq!(data.clip_id.to_string(), CLIP_TEXT_A);
    assert_eq!(store.project().tracks[0].clips[0].name, "Renamed");
    assert_eq!(data.name, "Renamed");
    assert_eq!(warnings, Vec::<Value>::new());
}

#[test]
fn multi_track_clip_resolution_uses_track_index() {
    let prior = project_with_tracks(vec![
        text_track(TRACK_TEXT_A, "Text 1", false, CLIP_TEXT_A, "Clip 1", false),
        text_track(TRACK_TEXT_B, "Text 2", false, CLIP_TEXT_B, "Clip 2", false),
    ]);

    let args = ClipRenameArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_B.to_string(),
        name: "Renamed".to_string(),
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("search in second track");
    assert_eq!(patch_rename_name(&patch), "Renamed");
    assert!(warnings.is_empty());
    assert_eq!(
        patch.as_array().expect("patch is array")[0]
            .get("path")
            .and_then(Value::as_str),
        Some("/tracks/1/clips/0/name")
    );
    assert_eq!(data.clip_id.to_string(), CLIP_TEXT_B);
    assert_eq!(data.name, "Renamed");
}

#[test]
fn duplicate_names_allowed() {
    let prior = project_with_tracks(vec![
        text_track(
            TRACK_TEXT_A,
            "Text 1",
            false,
            CLIP_TEXT_A,
            "Shared Name",
            false,
        ),
        text_track(
            TRACK_TEXT_B,
            "Text 2",
            false,
            CLIP_TEXT_B,
            "Unique Name",
            false,
        ),
    ]);

    let args = ClipRenameArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_B.to_string(),
        name: "Shared Name".to_string(),
    };

    let (patch, warnings, data) =
        compute_patch(&prior, &args).expect("duplicate names are allowed");
    assert_eq!(patch_rename_name(&patch), "Shared Name");
    assert!(warnings.is_empty());
    assert_eq!(data.clip_id.to_string(), CLIP_TEXT_B);
    assert_eq!(data.name, "Shared Name");
}
