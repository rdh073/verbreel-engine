//! Tests for `text.edit` (§7.2) — thirtieth production verb.

use serde_json::{Value, json};
use std::sync::Arc;

use verbreel_state::verbs::text_edit::{
    MAX_CONTENT_LEN, W_NOOP_CODE, compute_patch, data_envelope_from_post_state,
};
use verbreel_state::{
    MutateOutcome, Project, TextEditArgs, TextEditData, TextEditError, TextEditVerb, Track,
    TrackKind, Verb, VerbError, VerbRegistry, default_fixtures, default_registry,
    validate_reconstructors,
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
const ASSET_VIDEO_ID: &str = "0190b8d3-15e3-7000-bd00-0000000dd001";
const ASSET_AUDIO_ID: &str = "0190b8d3-15e3-7000-bd00-0000000dd002";

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
    content: &str,
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
            "locked": clip_locked,
            "text": {
                "content": content,
                "font_family": "Arial",
                "font_size_px": 24
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
            "source_out_tk": 480_000,
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

fn patch_content_path(patch: &Value) -> &str {
    let ops = patch.as_array().expect("patch is an array");
    assert_eq!(ops.len(), 1);
    ops[0]
        .get("path")
        .and_then(Value::as_str)
        .expect("patch op has path")
}

#[test]
fn compute_patch_text_track_updates_content() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
        "Hello",
    )]);

    let args = TextEditArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_A.to_string(),
        content: "World".to_string(),
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("text edit happy path");
    assert_eq!(patch.as_array().expect("patch is array").len(), 1);
    assert!(warnings.is_empty());
    assert_eq!(patch_content_path(&patch), "/tracks/0/clips/0/text/content");
    assert_eq!(patch[0].get("value").and_then(Value::as_str), Some("World"),);
    assert_eq!(data.clip_id.to_string(), CLIP_TEXT_A);
    assert_eq!(data.content, "World");
}

#[test]
fn compute_patch_text_track_empty_content_is_allowed() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
        "Hello",
    )]);

    let args = TextEditArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_A.to_string(),
        content: "".to_string(),
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("empty content allowed");
    assert_eq!(patch.as_array().expect("patch is array").len(), 1);
    assert!(warnings.is_empty());
    assert_eq!(patch_content_path(&patch), "/tracks/0/clips/0/text/content");
    assert_eq!(data.content, "");
    assert_eq!(patch[0].get("value").and_then(Value::as_str), Some(""));
}

#[test]
fn compute_patch_text_track_boundary_8192_chars_succeeds() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
        "Hello",
    )]);

    let content = "a".repeat(MAX_CONTENT_LEN);
    assert_eq!(content.chars().count(), MAX_CONTENT_LEN);

    let args = TextEditArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_A.to_string(),
        content: content.clone(),
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("max chars allowed");
    assert_eq!(patch.as_array().expect("patch is array").len(), 1);
    assert!(warnings.is_empty());
    assert_eq!(patch_content_path(&patch), "/tracks/0/clips/0/text/content");
    assert_eq!(data.content, content);
    assert_eq!(data.content.chars().count(), MAX_CONTENT_LEN);
}

#[test]
fn compute_patch_text_track_unicode_8193_chars_is_schema_violation() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
        "Hello",
    )]);

    let content = "界".repeat(MAX_CONTENT_LEN + 1);
    assert_eq!(content.chars().count(), MAX_CONTENT_LEN + 1);

    let err = compute_patch(
        &prior,
        &TextEditArgs {
            project_id: fixture_project_id(),
            clip: CLIP_TEXT_A.to_string(),
            content,
        },
    )
    .expect_err("unicode count should exceed schema bound");

    match err {
        TextEditError::SchemaViolation { .. } => {}
        other => panic!("expected schema violation, got {other:?}"),
    }
}

#[test]
fn compute_patch_text_track_8193_chars_is_schema_violation() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
        "Hello",
    )]);

    let content = "a".repeat(MAX_CONTENT_LEN + 1);
    assert_eq!(content.chars().count(), MAX_CONTENT_LEN + 1);

    let err = compute_patch(
        &prior,
        &TextEditArgs {
            project_id: fixture_project_id(),
            clip: CLIP_TEXT_A.to_string(),
            content,
        },
    )
    .expect_err("ascii count should exceed schema bound");

    match err {
        TextEditError::SchemaViolation { detail } => {
            assert_eq!(
                detail,
                format!(
                    "content length {} exceeds max {MAX_CONTENT_LEN}",
                    MAX_CONTENT_LEN + 1
                )
            );
        }
        other => panic!("expected schema violation, got {other:?}"),
    }
}

#[test]
fn compute_patch_video_track_is_not_text_clip() {
    let prior = project_with_tracks(vec![video_track(
        TRACK_VIDEO_A,
        "Video 1",
        false,
        CLIP_VIDEO_A,
        false,
    )]);

    let err = compute_patch(
        &prior,
        &TextEditArgs {
            project_id: fixture_project_id(),
            clip: CLIP_VIDEO_A.to_string(),
            content: "World".to_string(),
        },
    )
    .expect_err("video clip rejects text.edit");

    assert!(matches!(
        err,
        TextEditError::ClipKindMismatch {
            clip_id,
            found_kind: TrackKind::Video,
        } if clip_id == CLIP_VIDEO_A
    ));
}

#[test]
fn compute_patch_audio_track_is_not_text_clip() {
    let prior = project_with_tracks(vec![audio_track(
        TRACK_AUDIO_A,
        "Audio 1",
        false,
        CLIP_AUDIO_A,
        false,
    )]);

    let err = compute_patch(
        &prior,
        &TextEditArgs {
            project_id: fixture_project_id(),
            clip: CLIP_AUDIO_A.to_string(),
            content: "World".to_string(),
        },
    )
    .expect_err("audio clip rejects text.edit");

    assert!(matches!(
        err,
        TextEditError::ClipKindMismatch {
            clip_id,
            found_kind: TrackKind::Audio,
        } if clip_id == CLIP_AUDIO_A
    ));
}

#[test]
fn compute_patch_kind_check_precedes_lock() {
    let prior = project_with_tracks(vec![video_track(
        TRACK_VIDEO_A,
        "Video 1",
        true,
        CLIP_VIDEO_A,
        true,
    )]);

    let err = compute_patch(
        &prior,
        &TextEditArgs {
            project_id: fixture_project_id(),
            clip: CLIP_VIDEO_A.to_string(),
            content: "World".to_string(),
        },
    )
    .expect_err("kind check should precede lock");

    assert!(matches!(
        err,
        TextEditError::ClipKindMismatch { clip_id, .. } if clip_id == CLIP_VIDEO_A
    ));
}

#[test]
fn compute_patch_text_track_noop_warning_when_unchanged() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
        "Hello",
    )]);

    let args = TextEditArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_A.to_string(),
        content: "Hello".to_string(),
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("no-op path");
    assert!(patch.as_array().expect("patch is array").is_empty());
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_NOOP_CODE);
    assert_eq!(warnings[0]["message"], "text content unchanged");
    assert_eq!(warnings[0]["details"]["clip_id"], CLIP_TEXT_A);
    assert_eq!(warnings[0]["details"]["content"], "Hello");
    assert_eq!(data.clip_id.to_string(), CLIP_TEXT_A);
    assert_eq!(data.content, "Hello");
}

#[test]
fn compute_patch_text_track_locked_fails() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        true,
        "Hello",
    )]);

    let err = compute_patch(
        &prior,
        &TextEditArgs {
            project_id: fixture_project_id(),
            clip: CLIP_TEXT_A.to_string(),
            content: "World".to_string(),
        },
    )
    .expect_err("locked clip fails");

    assert!(matches!(
        err,
        TextEditError::Locked { clip_id } if clip_id == CLIP_TEXT_A
    ));
}

#[test]
fn compute_patch_locked_text_track_with_oversized_content_still_reports_locked() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        true,
        "Hello",
    )]);

    let err = compute_patch(
        &prior,
        &TextEditArgs {
            project_id: fixture_project_id(),
            clip: CLIP_TEXT_A.to_string(),
            content: "a".repeat(9000),
        },
    )
    .expect_err("lock checks before schema");

    assert!(matches!(
        err,
        TextEditError::Locked { clip_id } if clip_id == CLIP_TEXT_A
    ));
}

#[test]
fn compute_patch_track_locked_does_not_block_text_edit() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        true,
        CLIP_TEXT_A,
        false,
        "Hello",
    )]);

    let (patch, warnings, data) = compute_patch(
        &prior,
        &TextEditArgs {
            project_id: fixture_project_id(),
            clip: CLIP_TEXT_A.to_string(),
            content: "World".to_string(),
        },
    )
    .expect("track lock should not block text.edit");

    assert_eq!(patch.as_array().expect("patch is array").len(), 1);
    assert!(warnings.is_empty());
    assert_eq!(data.content, "World");
}

#[test]
fn compute_patch_not_found() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
        "Hello",
    )]);

    let err = compute_patch(
        &prior,
        &TextEditArgs {
            project_id: fixture_project_id(),
            clip: MISSING_CLIP.to_string(),
            content: "World".to_string(),
        },
    )
    .expect_err("not found maps");

    assert!(matches!(
        err,
        TextEditError::ClipNotFound { clip_id } if clip_id == MISSING_CLIP
    ));
}

#[test]
fn compute_patch_bad_selector() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
        "Hello",
    )]);

    let err = compute_patch(
        &prior,
        &TextEditArgs {
            project_id: fixture_project_id(),
            clip: "bad-uuid".to_string(),
            content: "World".to_string(),
        },
    )
    .expect_err("bad selector maps");

    assert!(matches!(err, TextEditError::BadSelector { .. }));
}

#[test]
fn round_trip_text_edit() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
        "Hello",
    )]);

    let args = TextEditArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_A.to_string(),
        content: "World".to_string(),
    };

    let (patch_value, _warnings, _data) = compute_patch(&prior, &args).expect("happy path");
    let patch: json_patch::Patch = serde_json::from_value(patch_value).expect("patch parses");
    let post_state = prior.apply(&patch).expect("patch applies");
    let round_trip = serde_json::to_value(&post_state).expect("post-state serializes");
    let from_json: Project = serde_json::from_value(round_trip).expect("post-state deserializes");

    assert_eq!(post_state, from_json);
    assert_eq!(
        post_state.tracks[0].clips[0]
            .text
            .as_ref()
            .expect("text exists")
            .content,
        "World"
    );
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "text.edit")
        .expect("default_fixtures includes text.edit");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(TextEditVerb))
        .expect("register text.edit verb");

    let report =
        validate_reconstructors(&registry, &[fixture]).expect("text.edit reconstruct from fixture");
    assert_eq!(report.verbs_checked, vec!["text.edit"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn data_envelope_from_post_state_reads_content() {
    let post_state = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
        "World",
    )]);

    let args = TextEditArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_A.to_string(),
        content: "".to_string(),
    };

    let data = data_envelope_from_post_state(&args, &post_state).expect("data envelope");
    assert_eq!(data.clip_id.to_string(), CLIP_TEXT_A);
    assert_eq!(data.content, "World");
}

#[test]
fn verb_error_variants_are_bad_args() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
        "Hello",
    )]);
    let verb = TextEditVerb;

    let bad_selector = serde_json::to_value(TextEditArgs {
        project_id: fixture_project_id(),
        clip: "not-a-uuid".to_string(),
        content: "World".to_string(),
    })
    .expect("bad selector args serialize");
    assert!(matches!(
        verb.compute_patch(&prior, &bad_selector)
            .expect_err("bad selector maps"),
        VerbError::BadArgs { .. }
    ));

    let not_found = serde_json::to_value(TextEditArgs {
        project_id: fixture_project_id(),
        clip: MISSING_CLIP.to_string(),
        content: "World".to_string(),
    })
    .expect("not found args serialize");
    assert!(matches!(
        verb.compute_patch(&prior, &not_found)
            .expect_err("not found maps"),
        VerbError::BadArgs { .. }
    ));

    let wrong_kind = serde_json::to_value(TextEditArgs {
        project_id: fixture_project_id(),
        clip: CLIP_VIDEO_A.to_string(),
        content: "World".to_string(),
    })
    .expect("wrong kind args serialize");
    let prior_wrong_kind = project_with_tracks(vec![video_track(
        TRACK_VIDEO_A,
        "Video 1",
        false,
        CLIP_VIDEO_A,
        false,
    )]);
    assert!(matches!(
        verb.compute_patch(&prior_wrong_kind, &wrong_kind)
            .expect_err("wrong kind maps"),
        VerbError::BadArgs { .. }
    ));

    let locked = serde_json::to_value(TextEditArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_A.to_string(),
        content: "World".to_string(),
    })
    .expect("locked args serialize");
    let prior_locked = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        true,
        "Hello",
    )]);
    assert!(matches!(
        verb.compute_patch(&prior_locked, &locked)
            .expect_err("locked maps"),
        VerbError::BadArgs { .. }
    ));

    let schema_violation = serde_json::to_value(TextEditArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_A.to_string(),
        content: "a".repeat(MAX_CONTENT_LEN + 1),
    })
    .expect("schema violation args serialize");
    assert!(matches!(
        verb.compute_patch(&prior, &schema_violation)
            .expect_err("schema violation maps"),
        VerbError::BadArgs { .. }
    ));
}

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        project_with_tracks(vec![text_track(
            TRACK_TEXT_A,
            "Text 1",
            false,
            CLIP_TEXT_A,
            false,
            "Hello",
        )]),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry clears gate and writes project.json");

    let outcome = store
        .mutate_via_verb(
            "text.edit",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "clip": CLIP_TEXT_A,
                "content": "World",
            }),
            None,
        )
        .expect("mutate_via_verb happy path");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("happy path must return Applied, got {outcome:?}");
    };

    let data: TextEditData = serde_json::from_value(data).expect("text.edit data is TextEditData");
    assert_eq!(data.clip_id.to_string(), CLIP_TEXT_A);
    assert_eq!(data.content, "World");
    assert_eq!(warnings, Vec::<Value>::new());
    assert_eq!(
        store.project().tracks[0].clips[0]
            .text
            .as_ref()
            .expect("text exists")
            .content,
        "World"
    );
}

#[test]
fn verb_compute_patch_text_track_with_none_text_is_no_match() {
    let mut prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
        "Hello",
    )]);
    prior.tracks[0].clips[0].text = None;

    let (patch, warnings, data) = compute_patch(
        &prior,
        &TextEditArgs {
            project_id: fixture_project_id(),
            clip: CLIP_TEXT_A.to_string(),
            content: "World".to_string(),
        },
    )
    .expect("missing text field should still patch");
    assert_eq!(patch.as_array().expect("patch is array").len(), 1);
    assert!(warnings.is_empty());
    assert_eq!(data.content, "World");
    assert_eq!(
        patch[0].get("path").and_then(Value::as_str),
        Some("/tracks/0/clips/0/text/content")
    );

    let patch: json_patch::Patch = serde_json::from_value(patch).expect("patch parses");
    assert!(prior.apply(&patch).is_err());
}
