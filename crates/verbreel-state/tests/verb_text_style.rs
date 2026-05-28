//! Tests for `text.style` (§7.3) — thirty-second production verb.

use serde_json::{Value, json};
use std::sync::Arc;

use verbreel_state::verbs::text_style::{
    TextStyleArgs, TextStyleData, TextStyleError, TextStyleVerb, W_NOOP_CODE, compute_patch,
    data_envelope_from_post_state,
};
use verbreel_state::{
    Color, MutateOutcome, Project, Shadow, TextElement, Track, TrackKind, VerbRegistry,
    default_fixtures, default_registry, validate_reconstructors,
};
use verbreel_types::{ProjectId, Tick};

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const TRACK_TEXT_A: &str = "0190b8d3-15e3-7000-bd00-0000000aa101";
const TRACK_VIDEO_A: &str = "0190b8d3-15e3-7000-bd00-0000000aa201";
const CLIP_TEXT_A: &str = "0190b8d3-15e3-7000-bd00-0000000bb101";
const CLIP_VIDEO_A: &str = "0190b8d3-15e3-7000-bd00-0000000bb201";
const ASSET_VIDEO_ID: &str = "0190b8d3-15e3-7000-bd00-0000000dd001";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture -> Project")
}

fn fixture_project_id() -> ProjectId {
    empty_project().id
}

fn rgba(value: &str) -> Color {
    Color::new(value.to_string()).expect("valid color literal")
}

fn text_track(clip_locked: bool, text: TextElement) -> Track {
    serde_json::from_value(json!({
        "id": TRACK_TEXT_A,
        "kind": TrackKind::Text,
        "name": "Text 1",
        "locked": false,
        "clips": [{
            "id": CLIP_TEXT_A,
            "name": "Text Clip",
            "asset_id": "00000000-0000-0000-0000-000000000000",
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 480_000,
            "locked": clip_locked,
            "text": text,
        }],
    }))
    .expect("text track fixture parses")
}

fn video_track(clip_locked: bool) -> Track {
    serde_json::from_value(json!({
        "id": TRACK_VIDEO_A,
        "kind": TrackKind::Video,
        "name": "Video 1",
        "locked": false,
        "clips": [{
            "id": CLIP_VIDEO_A,
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

fn base_text() -> TextElement {
    TextElement {
        content: "Hello".to_string(),
        font_family: "Arial".to_string(),
        font_size_px: 24.0,
        color: rgba("#ffffffff"),
        ..TextElement::default()
    }
}

fn text_with_shadow() -> TextElement {
    TextElement {
        shadow: Some(Shadow {
            color: rgba("#000000aa"),
            blur_px: 4.0,
            offset_x: 1.0,
            offset_y: 2.0,
        }),
        ..base_text()
    }
}

fn project_with_tracks(tracks: Vec<Track>) -> Project {
    let mut project = empty_project();
    project.tracks = tracks;
    project.duration_tk = Tick::new(480_000);
    project
}

fn text_style_args(style: Value) -> TextStyleArgs {
    TextStyleArgs {
        project_id: fixture_project_id(),
        clip: CLIP_TEXT_A.to_string(),
        style,
    }
}

fn apply_patch(prior: &Project, patch: Value) -> Project {
    let patch: json_patch::Patch = serde_json::from_value(patch).expect("patch parses");
    prior.apply(&patch).expect("patch applies")
}

#[test]
fn compute_patch_text_track_updates_multiple_leaves() {
    let prior = project_with_tracks(vec![text_track(false, base_text())]);
    let args = text_style_args(json!({
        "color": "#ff0000ff",
        "font_size_px": 96.0
    }));

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("text.style happy path");
    let ops = patch.as_array().expect("patch is array");
    assert_eq!(ops.len(), 2);
    assert!(warnings.is_empty());
    assert_eq!(ops[0]["op"], "replace");
    assert_eq!(ops[0]["path"], "/tracks/0/clips/0/text/font_size_px");
    assert_eq!(ops[0]["value"], 96.0);
    assert_eq!(ops[1]["op"], "replace");
    assert_eq!(ops[1]["path"], "/tracks/0/clips/0/text/color");
    assert_eq!(ops[1]["value"], "#ff0000ff");
    assert_eq!(data.clip_id.to_string(), CLIP_TEXT_A);
    assert_eq!(data.text.font_size_px, 96.0);
    assert_eq!(data.text.color.as_str(), "#ff0000ff");
}

#[test]
fn compute_patch_shadow_null_removes_field() {
    let prior = project_with_tracks(vec![text_track(false, text_with_shadow())]);
    let args = text_style_args(json!({ "shadow": null }));

    let (patch, warnings, _data) = compute_patch(&prior, &args).expect("remove shadow");
    assert!(warnings.is_empty());
    assert_eq!(patch.as_array().expect("patch is array").len(), 1);
    assert_eq!(patch[0]["op"], "remove");
    assert_eq!(patch[0]["path"], "/tracks/0/clips/0/text/shadow");

    let post_state = apply_patch(&prior, patch);
    assert!(
        post_state.tracks[0].clips[0]
            .text
            .as_ref()
            .expect("text exists")
            .shadow
            .is_none()
    );
}

#[test]
fn compute_patch_shadow_null_on_no_shadow_clip_is_noop() {
    let prior = project_with_tracks(vec![text_track(false, base_text())]);
    let args = text_style_args(json!({ "shadow": null }));

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("shadow noop");
    assert!(patch.as_array().expect("patch is array").is_empty());
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_NOOP_CODE);
    assert_eq!(warnings[0]["details"]["verb"], "text.style");
    assert!(data.text.shadow.is_none());
}

#[test]
fn compute_patch_shadow_object_sets_shadow() {
    let prior = project_with_tracks(vec![text_track(false, base_text())]);
    let args = text_style_args(json!({
        "shadow": {
            "color": "#12345678",
            "blur_px": 8.0,
            "offset_x": 3.0,
            "offset_y": 4.0
        }
    }));

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("set shadow");
    assert!(warnings.is_empty());
    assert_eq!(patch.as_array().expect("patch is array").len(), 1);
    assert_eq!(patch[0]["path"], "/tracks/0/clips/0/text/shadow");

    let post_state = apply_patch(&prior, patch);
    let shadow = post_state.tracks[0].clips[0]
        .text
        .as_ref()
        .expect("text exists")
        .shadow
        .as_ref()
        .expect("shadow set");
    assert_eq!(shadow.color.as_str(), "#12345678");
    assert_eq!(shadow.blur_px, 8.0);
    assert_eq!(shadow.offset_x, 3.0);
    assert_eq!(shadow.offset_y, 4.0);
    assert_eq!(data.text.shadow.as_ref().expect("data shadow").blur_px, 8.0);
}

#[test]
fn compute_patch_font_size_below_one_is_schema_violation() {
    let prior = project_with_tracks(vec![text_track(false, base_text())]);
    let err = compute_patch(&prior, &text_style_args(json!({ "font_size_px": 0.0 })))
        .expect_err("font size below minimum fails");
    assert!(matches!(err, TextStyleError::SchemaViolation { .. }));
}

#[test]
fn compute_patch_font_weight_out_of_range_is_schema_violation() {
    let prior = project_with_tracks(vec![text_track(false, base_text())]);
    for font_weight in [50_u32, 1000_u32] {
        let err = compute_patch(
            &prior,
            &text_style_args(json!({ "font_weight": font_weight })),
        )
        .expect_err("font weight outside schema range fails");
        assert!(matches!(err, TextStyleError::SchemaViolation { .. }));
    }
}

#[test]
fn compute_patch_unknown_font_family_is_font_unknown() {
    let prior = project_with_tracks(vec![text_track(false, base_text())]);
    let err = compute_patch(
        &prior,
        &text_style_args(json!({ "font_family": "__no_such_font_family__" })),
    )
    .expect_err("unknown family must fail");
    let TextStyleError::FontUnknown { family, available } = err else {
        panic!("expected TextStyleError::FontUnknown");
    };
    assert_eq!(family, "__no_such_font_family__");
    assert!(
        available.iter().any(|name| name == "Inter"),
        "available list should include bundled Inter"
    );
}

#[test]
fn compute_patch_line_height_below_half_is_schema_violation() {
    let prior = project_with_tracks(vec![text_track(false, base_text())]);
    let err = compute_patch(&prior, &text_style_args(json!({ "line_height": 0.49 })))
        .expect_err("line height below minimum fails");
    assert!(matches!(err, TextStyleError::SchemaViolation { .. }));
}

#[test]
fn compute_patch_nan_or_infinity_is_schema_violation() {
    let prior = project_with_tracks(vec![text_track(false, base_text())]);
    let non_finite_value = serde_json::to_value(f64::INFINITY).unwrap_or(Value::Null);
    let err = compute_patch(
        &prior,
        &text_style_args(json!({ "font_size_px": non_finite_value })),
    )
    .expect_err("non-finite JSON boundary value fails schema validation");
    assert!(matches!(err, TextStyleError::SchemaViolation { .. }));
}

#[test]
fn compute_patch_video_track_is_not_text_clip() {
    let prior = project_with_tracks(vec![video_track(false)]);
    let err = compute_patch(
        &prior,
        &TextStyleArgs {
            project_id: fixture_project_id(),
            clip: CLIP_VIDEO_A.to_string(),
            style: json!({ "color": "#ff0000ff" }),
        },
    )
    .expect_err("video clip rejects text.style");

    assert!(matches!(
        err,
        TextStyleError::ClipKindMismatch {
            clip_id,
            found_kind: TrackKind::Video,
        } if clip_id == CLIP_VIDEO_A
    ));
}

#[test]
fn compute_patch_locked_text_clip_rejects_with_e_locked() {
    let prior = project_with_tracks(vec![text_track(true, base_text())]);
    let err = compute_patch(&prior, &text_style_args(json!({ "color": "#ff0000ff" })))
        .expect_err("locked clip fails");

    assert!(matches!(
        err,
        TextStyleError::Locked { clip_id } if clip_id == CLIP_TEXT_A
    ));
}

#[test]
fn compute_patch_locked_clip_takes_precedence_over_schema_violation() {
    let prior = project_with_tracks(vec![text_track(true, base_text())]);
    let err = compute_patch(&prior, &text_style_args(json!({ "font_size_px": 0.0 })))
        .expect_err("lock checks before schema");

    assert!(matches!(
        err,
        TextStyleError::Locked { clip_id } if clip_id == CLIP_TEXT_A
    ));
}

#[test]
fn compute_patch_empty_style_is_noop() {
    let prior = project_with_tracks(vec![text_track(false, base_text())]);
    let (patch, warnings, data) =
        compute_patch(&prior, &text_style_args(json!({}))).expect("empty style noop");

    assert!(patch.as_array().expect("patch is array").is_empty());
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_NOOP_CODE);
    assert_eq!(data.text, base_text());
}

#[test]
fn compute_patch_noop_when_all_values_unchanged() {
    let prior = project_with_tracks(vec![text_track(false, base_text())]);
    let (patch, warnings, data) = compute_patch(
        &prior,
        &text_style_args(json!({ "color": base_text().color })),
    )
    .expect("unchanged color noop");

    assert!(patch.as_array().expect("patch is array").is_empty());
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_NOOP_CODE);
    assert_eq!(data.text.color.as_str(), "#ffffffff");
}

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        project_with_tracks(vec![text_track(false, base_text())]),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry clears gate and writes project.json");

    let outcome = store
        .mutate_via_verb(
            "text.style",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "clip": CLIP_TEXT_A,
                "style": {
                    "color": "#ff0000ff",
                    "font_size_px": 96.0
                }
            }),
            None,
        )
        .expect("mutate_via_verb happy path");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("happy path must return Applied, got {outcome:?}");
    };

    let data: TextStyleData =
        serde_json::from_value(data).expect("text.style data is TextStyleData");
    assert_eq!(data.clip_id.to_string(), CLIP_TEXT_A);
    assert_eq!(data.text.color.as_str(), "#ff0000ff");
    assert_eq!(data.text.font_size_px, 96.0);
    assert_eq!(warnings, Vec::<Value>::new());
    assert_eq!(
        store.project().tracks[0].clips[0]
            .text
            .as_ref()
            .expect("text exists")
            .color
            .as_str(),
        "#ff0000ff"
    );
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "text.style")
        .expect("default_fixtures includes text.style");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(TextStyleVerb))
        .expect("register text.style verb");

    let report = validate_reconstructors(&registry, &[fixture])
        .expect("text.style reconstruct from fixture");
    assert_eq!(report.verbs_checked, vec!["text.style"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn round_trip_text_style() {
    let prior = project_with_tracks(vec![text_track(false, base_text())]);
    let args = text_style_args(json!({
        "color": "#ff0000ff",
        "font_size_px": 96.0
    }));

    let (patch_value, _warnings, data) = compute_patch(&prior, &args).expect("happy path");
    let post_state = apply_patch(&prior, patch_value);
    let round_trip = serde_json::to_value(&post_state).expect("post-state serializes");
    let from_json: Project = serde_json::from_value(round_trip).expect("post-state deserializes");

    assert_eq!(post_state, from_json);
    assert_eq!(
        post_state.tracks[0].clips[0]
            .text
            .as_ref()
            .expect("text exists")
            .color
            .as_str(),
        "#ff0000ff"
    );
    assert_eq!(data.text.font_size_px, 96.0);
}

#[test]
fn data_envelope_from_post_state_returns_text_element() {
    let post_state = project_with_tracks(vec![text_track(
        false,
        TextElement {
            color: rgba("#ff0000ff"),
            font_size_px: 96.0,
            ..base_text()
        },
    )]);

    let data = data_envelope_from_post_state(&text_style_args(json!({})), &post_state)
        .expect("data envelope");
    assert_eq!(data.clip_id.to_string(), CLIP_TEXT_A);
    assert_eq!(data.text.color.as_str(), "#ff0000ff");
    assert_eq!(data.text.font_size_px, 96.0);
}
