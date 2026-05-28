//! Tests for `text.add` (§7.1) — fifty-fourth production verb.

use serde_json::{Value, json};
use std::sync::Arc;
use unicode_segmentation::UnicodeSegmentation;

use verbreel_state::verbs::clip_set_fade::W_TIME_SNAPPED_CODE;
use verbreel_state::verbs::text_add::{
    StyleArg, TextAddArgs, TextAddData, TextAddError, TextAddVerb, W_TEXT_ADD_ENVELOPE_CODE,
    compute_patch, data_envelope_from_warnings,
};
use verbreel_state::{
    MutateOutcome, Project, RecordedEvent, TextElement, Track, TrackKind, Verb, VerbRegistry,
    default_fixtures, default_registry, validate_reconstructors,
};
use verbreel_types::{ProjectId, Tick};

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const TRACK_TEXT_A: &str = "0190b8d3-15e3-7000-bd00-0000000aa701";
const TRACK_VIDEO_A: &str = "0190b8d3-15e3-7000-bd00-0000000aa702";
const CLIP_TEXT_A: &str = "0190b8d3-15e3-7000-bd00-0000000bb701";
const CLIP_TEXT_B: &str = "0190b8d3-15e3-7000-bd00-0000000bb702";
const MISSING_TRACK: &str = "0190b8d3-15e3-7000-bd00-0000000cc701";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture -> Project")
}

fn fixture_project_id() -> ProjectId {
    FIXTURE_PROJECT_ID
        .parse()
        .expect("fixture project id parses")
}

fn text_add_args() -> TextAddArgs {
    TextAddArgs {
        project_id: fixture_project_id(),
        content: "Hello world".to_string(),
        track_position_tk: 0,
        duration_tk: 240_000,
        track: None,
        style: None,
        name: None,
    }
}

fn style_object(value: Value) -> StyleArg {
    StyleArg::Object(value.as_object().expect("style object").clone())
}

fn text_clip(id: &str, name: &str, position_tk: i64, duration_tk: i64) -> Value {
    json!({
        "id": id,
        "name": name,
        "asset_id": "00000000-0000-0000-0000-000000000000",
        "track_position_tk": position_tk,
        "source_in_tk": 0,
        "source_out_tk": duration_tk,
        "locked": false,
        "text": {
            "content": name,
        },
    })
}

fn text_track(id: &str, name: &str, locked: bool, clips: Vec<Value>) -> Track {
    serde_json::from_value(json!({
        "id": id,
        "kind": TrackKind::Text,
        "name": name,
        "locked": locked,
        "clips": clips,
    }))
    .expect("text track parses")
}

fn video_track() -> Track {
    serde_json::from_value(json!({
        "id": TRACK_VIDEO_A,
        "kind": TrackKind::Video,
        "name": "Video 1",
        "locked": false,
        "clips": [],
    }))
    .expect("video track parses")
}

fn project_with_tracks(tracks: Vec<Track>) -> Project {
    let mut project = empty_project();
    let duration_tk = tracks
        .iter()
        .flat_map(|track| track.clips.iter())
        .map(|clip| {
            clip.track_position_tk
                .get()
                .saturating_add(clip.source_out_tk.get())
        })
        .max()
        .unwrap_or(0);
    project.tracks = tracks;
    project.duration_tk = Tick::new(duration_tk);
    project
}

fn apply_patch(prior: &Project, patch: Value) -> Project {
    let patch: json_patch::Patch = serde_json::from_value(patch).expect("patch parses");
    prior.apply(&patch).expect("patch applies")
}

fn added_text_clip(project: &Project, track_idx: usize) -> &verbreel_state::Clip {
    project.tracks[track_idx]
        .clips
        .last()
        .expect("added text clip exists")
}

#[test]
fn track_omitted_without_text_track_mints_track_and_clip() {
    let prior = empty_project();

    let (patch, warnings, data) = compute_patch(&prior, &text_add_args()).expect("text.add");
    let post = apply_patch(&prior, patch);

    assert_eq!(post.tracks.len(), 3);
    assert_eq!(post.tracks[2].kind, TrackKind::Text);
    assert_eq!(post.tracks[2].name, "Text 1");
    assert_eq!(post.tracks[2].id, data.text_track_id);
    let clip = added_text_clip(&post, 2);
    assert_eq!(clip.id, data.clip_id);
    assert_eq!(clip.name, "Hello world");
    assert_eq!(clip.source_out_tk.get(), 240_000);
    assert_eq!(clip.text.as_ref().expect("text").content, "Hello world");
    assert_eq!(
        warnings.last().expect("envelope")["code"],
        W_TEXT_ADD_ENVELOPE_CODE
    );
}

#[test]
fn track_omitted_with_existing_text_track_uses_first_text_track() {
    let prior = project_with_tracks(vec![
        video_track(),
        text_track(TRACK_TEXT_A, "Text 1", false, vec![]),
    ]);

    let (patch, _warnings, data) = compute_patch(&prior, &text_add_args()).expect("text.add");
    let post = apply_patch(&prior, patch);

    assert_eq!(post.tracks.len(), 2);
    assert_eq!(data.text_track_id.to_string(), TRACK_TEXT_A);
    assert_eq!(post.tracks[1].clips.len(), 1);
}

#[test]
fn track_supplied_as_bare_uuid_uses_that_track() {
    let prior = project_with_tracks(vec![text_track(TRACK_TEXT_A, "Text 1", false, vec![])]);
    let mut args = text_add_args();
    args.track = Some(TRACK_TEXT_A.to_string());

    let (_patch, _warnings, data) = compute_patch(&prior, &args).expect("text.add");

    assert_eq!(data.text_track_id.to_string(), TRACK_TEXT_A);
}

#[test]
fn track_supplied_as_qualified_track_uuid_uses_that_track() {
    let prior = project_with_tracks(vec![text_track(TRACK_TEXT_A, "Text 1", false, vec![])]);
    let mut args = text_add_args();
    args.track = Some(format!("track:{TRACK_TEXT_A}"));

    let (_patch, _warnings, data) = compute_patch(&prior, &args).expect("text.add");

    assert_eq!(data.text_track_id.to_string(), TRACK_TEXT_A);
}

#[test]
fn supplied_name_is_used_verbatim() {
    let prior = project_with_tracks(vec![text_track(TRACK_TEXT_A, "Text 1", false, vec![])]);
    let mut args = text_add_args();
    args.name = Some("Custom Caption".to_string());

    let (patch, _warnings, _data) = compute_patch(&prior, &args).expect("text.add");
    let post = apply_patch(&prior, patch);

    assert_eq!(added_text_clip(&post, 0).name, "Custom Caption");
}

#[test]
fn name_auto_derives_from_ascii_content() {
    let prior = project_with_tracks(vec![text_track(TRACK_TEXT_A, "Text 1", false, vec![])]);
    let mut args = text_add_args();
    args.content = "  Hello\t\tfrom\nVerbreel  ".to_string();

    let (patch, _warnings, _data) = compute_patch(&prior, &args).expect("text.add");
    let post = apply_patch(&prior, patch);

    assert_eq!(added_text_clip(&post, 0).name, "Hello from Verbreel");
}

#[test]
fn name_auto_derives_from_grapheme_cluster_content() {
    let family = "👨‍👩‍👧‍👦";
    let prior = project_with_tracks(vec![text_track(TRACK_TEXT_A, "Text 1", false, vec![])]);
    let mut args = text_add_args();
    args.content = format!("{family} family");

    let (patch, _warnings, _data) = compute_patch(&prior, &args).expect("text.add");
    let post = apply_patch(&prior, patch);
    let name = &added_text_clip(&post, 0).name;

    assert_eq!(name, "👨‍👩‍👧‍👦 family");
    assert_eq!(name.graphemes(true).count(), 8);
}

#[test]
fn empty_content_auto_names_text_one() {
    let prior = project_with_tracks(vec![text_track(TRACK_TEXT_A, "Text 1", false, vec![])]);
    let mut args = text_add_args();
    args.content.clear();

    let (patch, _warnings, _data) = compute_patch(&prior, &args).expect("text.add");
    let post = apply_patch(&prior, patch);

    assert_eq!(added_text_clip(&post, 0).name, "Text 1");
}

#[test]
fn empty_content_uses_max_suffix_plus_one_on_target_track() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        vec![
            text_clip(CLIP_TEXT_A, "Text 1", 0, 10_000),
            text_clip(CLIP_TEXT_B, "Text 2", 20_000, 10_000),
        ],
    )]);
    let mut args = text_add_args();
    args.content = "   \n\t  ".to_string();
    args.track_position_tk = 40_000;

    let (patch, _warnings, _data) = compute_patch(&prior, &args).expect("text.add");
    let post = apply_patch(&prior, patch);

    assert_eq!(added_text_clip(&post, 0).name, "Text 3");
}

#[test]
fn style_object_merges_over_default_text_element() {
    let prior = project_with_tracks(vec![text_track(TRACK_TEXT_A, "Text 1", false, vec![])]);
    let mut args = text_add_args();
    args.style = Some(style_object(json!({
        "font_size_px": 96.0,
        "color": "#ff0000ff"
    })));

    let (patch, _warnings, _data) = compute_patch(&prior, &args).expect("text.add");
    let post = apply_patch(&prior, patch);
    let text = added_text_clip(&post, 0).text.as_ref().expect("text");

    assert_eq!(text.content, "Hello world");
    assert_eq!(text.font_family, TextElement::default().font_family);
    assert_eq!(text.font_size_px, 96.0);
    assert_eq!(text.color.as_str(), "#ff0000ff");
}

#[test]
fn content_longer_than_8192_chars_errors() {
    let prior = empty_project();
    let mut args = text_add_args();
    args.content = "x".repeat(8193);

    let err = compute_patch(&prior, &args).expect_err("content too long");

    assert!(matches!(
        err,
        TextAddError::SchemaViolation {
            field: "content",
            ..
        }
    ));
}

#[test]
fn duration_zero_errors() {
    let prior = empty_project();
    let mut args = text_add_args();
    args.duration_tk = 0;

    let err = compute_patch(&prior, &args).expect_err("bad duration");

    assert!(matches!(
        err,
        TextAddError::SchemaViolation {
            field: "duration_tk",
            ..
        }
    ));
}

#[test]
fn negative_track_position_errors_bad_time() {
    let prior = empty_project();
    let mut args = text_add_args();
    args.track_position_tk = -1;

    let err = compute_patch(&prior, &args).expect_err("bad time");

    assert!(matches!(err, TextAddError::BadTime { value: -1 }));
}

#[test]
fn clip_prefix_selector_errors_bad_selector() {
    let prior = empty_project();
    let mut args = text_add_args();
    args.track = Some(format!("clip:{TRACK_TEXT_A}"));

    let err = compute_patch(&prior, &args).expect_err("bad selector prefix");

    assert!(matches!(err, TextAddError::BadSelector { .. }));
}

#[test]
fn malformed_track_uuid_errors_bad_selector() {
    let prior = empty_project();
    let mut args = text_add_args();
    args.track = Some("not-a-uuid".to_string());

    let err = compute_patch(&prior, &args).expect_err("bad selector uuid");

    assert!(matches!(err, TextAddError::BadSelector { .. }));
}

#[test]
fn unknown_track_uuid_errors_track_not_found() {
    let prior = empty_project();
    let mut args = text_add_args();
    args.track = Some(MISSING_TRACK.to_string());

    let err = compute_patch(&prior, &args).expect_err("missing track");

    assert!(matches!(err, TextAddError::TrackNotFound { .. }));
}

#[test]
fn video_track_selector_errors_kind_mismatch() {
    let prior = project_with_tracks(vec![video_track()]);
    let mut args = text_add_args();
    args.track = Some(TRACK_VIDEO_A.to_string());

    let err = compute_patch(&prior, &args).expect_err("kind mismatch");

    assert!(matches!(
        err,
        TextAddError::TrackKindMismatch {
            found_kind: TrackKind::Video,
            ..
        }
    ));
}

#[test]
fn locked_text_track_errors_locked() {
    let prior = project_with_tracks(vec![text_track(TRACK_TEXT_A, "Text 1", true, vec![])]);
    let mut args = text_add_args();
    args.track = Some(TRACK_TEXT_A.to_string());

    let err = compute_patch(&prior, &args).expect_err("locked");

    assert!(matches!(err, TextAddError::Locked { .. }));
}

#[test]
fn preset_style_errors_preset_unknown() {
    let prior = empty_project();
    let mut args = text_add_args();
    args.style = Some(StyleArg::Preset("bold-yellow".to_string()));

    let err = compute_patch(&prior, &args).expect_err("preset deferred");

    assert!(matches!(
        err,
        TextAddError::PresetUnknown {
            preset,
            hint: _
        } if preset == "bold-yellow"
    ));
}

#[test]
fn unknown_font_family_errors_font_unknown_with_available_list() {
    let prior = empty_project();
    let mut args = text_add_args();
    args.style = Some(style_object(json!({
        "font_family": "__no_such_font_family__",
    })));

    let err = compute_patch(&prior, &args).expect_err("unknown family must fail");

    assert!(matches!(
        err,
        TextAddError::FontUnknown { ref family, .. } if family == "__no_such_font_family__"
    ));
    let TextAddError::FontUnknown { available, .. } = err else {
        panic!("expected TextAddError::FontUnknown");
    };
    assert!(
        available.iter().any(|name| name == "Inter"),
        "available list should include bundled Inter"
    );
}

#[test]
fn style_font_family_is_stored_as_canonical_registry_name() {
    let prior = project_with_tracks(vec![text_track(TRACK_TEXT_A, "Text 1", false, vec![])]);
    let mut args = text_add_args();
    args.style = Some(style_object(json!({
        "font_family": "  inter  ",
    })));

    let (patch, _warnings, _data) = compute_patch(&prior, &args).expect("text.add");
    let post = apply_patch(&prior, patch);

    assert_eq!(
        added_text_clip(&post, 0)
            .text
            .as_ref()
            .expect("text")
            .font_family,
        "Inter"
    );
}

#[test]
fn overlapping_clip_errors_clip_overlap() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        vec![text_clip(CLIP_TEXT_A, "Existing", 0, 10_000)],
    )]);
    let mut args = text_add_args();
    args.track_position_tk = 8_000;
    args.duration_tk = 10_000;

    let err = compute_patch(&prior, &args).expect_err("overlap");

    assert!(matches!(err, TextAddError::ClipOverlap { .. }));
}

#[test]
fn off_frame_track_position_snaps_and_warns() {
    let prior = project_with_tracks(vec![text_track(TRACK_TEXT_A, "Text 1", false, vec![])]);
    let mut args = text_add_args();
    args.track_position_tk = 1;

    let (patch, warnings, _data) = compute_patch(&prior, &args).expect("text.add");
    let post = apply_patch(&prior, patch);

    assert_eq!(warnings[0]["code"], W_TIME_SNAPPED_CODE);
    assert_eq!(warnings[0]["details"]["from_tk"], 1);
    assert_eq!(warnings[0]["details"]["to_tk"], 0);
    assert_eq!(added_text_clip(&post, 0).track_position_tk.get(), 0);
}

#[test]
fn data_envelope_warning_contains_clip_and_track_ids() {
    let prior = empty_project();

    let (_patch, warnings, data) = compute_patch(&prior, &text_add_args()).expect("text.add");
    let details = &warnings.last().expect("envelope")["details"];

    assert_eq!(details["clip_id"], data.clip_id.to_string());
    assert_eq!(details["text_track_id"], data.text_track_id.to_string());
    assert_eq!(details["auto_created_track"], true);
    assert_eq!(
        data_envelope_from_warnings(&warnings).expect("warning envelope"),
        data
    );
}

#[test]
fn reconstructor_round_trip_existing_track() {
    let prior = project_with_tracks(vec![text_track(TRACK_TEXT_A, "Text 1", false, vec![])]);
    let args = text_add_args();
    let (patch, warnings, data) = compute_patch(&prior, &args).expect("text.add");
    let post = apply_patch(&prior, patch.clone());

    let reconstructed = TextAddVerb
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
fn reconstructor_round_trip_newly_minted_track() {
    let prior = empty_project();
    let args = text_add_args();
    let (patch, warnings, data) = compute_patch(&prior, &args).expect("text.add");
    let post = apply_patch(&prior, patch.clone());

    let reconstructed = TextAddVerb
        .reconstruct(
            &serde_json::to_value(&args).expect("args serialize"),
            &patch,
            &warnings,
            &post,
        )
        .expect("reconstruct");

    assert_eq!(reconstructed, serde_json::to_value(data).expect("data"));
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
    .expect("create_with_registry clears gate and writes project.json");

    let outcome = store
        .mutate_via_verb(
            "text.add",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "content": "Hello world",
                "track_position_tk": 0,
                "duration_tk": 240_000
            }),
            None,
        )
        .expect("mutate_via_verb happy path");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("happy path must return Applied, got {outcome:?}");
    };

    let data: TextAddData = serde_json::from_value(data).expect("text.add data");
    assert_eq!(
        store
            .project()
            .tracks
            .iter()
            .find(|track| track.id == data.text_track_id)
            .expect("text track")
            .clips[0]
            .id,
        data.clip_id
    );
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_TEXT_ADD_ENVELOPE_CODE);
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "text.add")
        .expect("default_fixtures includes text.add");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(TextAddVerb))
        .expect("register text.add verb");

    let report =
        validate_reconstructors(&registry, &[fixture]).expect("text.add reconstruct from fixture");
    assert_eq!(report.verbs_checked, vec!["text.add"]);
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
        .find(|event| event.verb == "text.add")
        .expect("default_fixtures includes text.add");

    let data = data_envelope_from_warnings(&warnings).expect("envelope");
    assert_eq!(
        serde_json::to_value(data).expect("data serializes"),
        expected_data
    );
}

#[test]
fn default_fixtures_include_unknown_font_error_case() {
    let fixtures: Vec<RecordedEvent> = default_fixtures()
        .into_iter()
        .filter(|event| event.verb == "text.add")
        .collect();

    assert!(
        fixtures.iter().any(|event| {
            event.expected_data == Value::Null
                && event.patch == json!([])
                && event.warnings.is_empty()
                && event.args["style"]["font_family"] == "__no_such_font_family__"
        }),
        "default_fixtures must include text.add unknown-font error case"
    );
}
