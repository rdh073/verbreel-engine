//! Tests for `caption.burn_in` (§10.4) — fifty-fifth production verb.

use std::sync::Arc;

use serde_json::{Map, Value, json};
use verbreel_state::verbs::caption_burn_in::{
    CaptionBurnInArgs, CaptionBurnInData, CaptionBurnInError, CaptionBurnInVerb,
    W_CAPTION_BURN_DEDUP_CODE, W_CAPTION_BURN_IN_ENVELOPE_CODE, W_NOOP_CODE, compute_patch,
    data_envelope_from_warnings,
};
use verbreel_state::verbs::text_add::StyleArg;
use verbreel_state::{
    Project, TrackKind, Verb, VerbRegistry, default_fixtures, validate_reconstructors,
};
use verbreel_types::{ProjectId, Tick};

#[cfg(feature = "native")]
use verbreel_state::{MutateOutcome, ProjectStore, default_registry};

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";

const TEXT_TRACK_A: &str = "01900000-0000-7000-8000-0000000aa701";
const VIDEO_TRACK_A: &str = "01900000-0000-7000-8000-0000000aa702";
const VIDEO_TRACK_B: &str = "01900000-0000-7000-8000-0000000aa703";
const MISSING_TRACK: &str = "01900000-0000-7000-8000-0000000aa799";

const TEXT_CLIP_A: &str = "01900000-0000-7000-8000-0000000bb701";
const TEXT_CLIP_B: &str = "01900000-0000-7000-8000-0000000bb702";
const VIDEO_CLIP_A: &str = "01900000-0000-7000-8000-0000000bb703";
const VIDEO_CLIP_B: &str = "01900000-0000-7000-8000-0000000bb704";
const VIDEO_CLIP_C: &str = "01900000-0000-7000-8000-0000000bb705";

const ASSET_ID: &str = "01900000-0000-7000-8000-0000000cc701";

const BURNED_A: &str = "01900000-0000-7000-8000-0000000ee701";
const BURNED_B: &str = "01900000-0000-7000-8000-0000000ee702";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture -> Project")
}

fn fixture_project_id() -> ProjectId {
    FIXTURE_PROJECT_ID
        .parse()
        .expect("fixture project id parses")
}

fn video_asset() -> Value {
    json!({
        "id": ASSET_ID,
        "kind": "video",
        "hash": "53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658",
        "path": "assets/53/53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.mp4",
        "original_filename": "caption-burn-in.mp4",
        "imported_at": "2026-05-24T00:00:00Z",
        "metadata": {
            "duration_tk": 480_000,
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
    })
}

fn text_clip(id: &str, track_position_tk: i64, duration_tk: i64) -> Value {
    json!({
        "id": id,
        "name": "Caption",
        "asset_id": "00000000-0000-0000-0000-000000000000",
        "track_position_tk": track_position_tk,
        "source_in_tk": 0,
        "source_out_tk": duration_tk,
        "locked": false,
        "text": {
            "content": "Caption",
            "font_family": "Arial",
            "font_size_px": 24,
        },
    })
}

fn video_clip(
    id: &str,
    track_position_tk: i64,
    duration_tk: i64,
    locked: bool,
    effects: Vec<Value>,
) -> Value {
    json!({
        "id": id,
        "name": "Video",
        "asset_id": ASSET_ID,
        "track_position_tk": track_position_tk,
        "source_in_tk": 0,
        "source_out_tk": duration_tk,
        "locked": locked,
        "effects": effects,
    })
}

fn burned_caption_effect(id: &str, source_track: &str) -> Value {
    json!({
        "id": id,
        "kind": "burned_caption",
        "enabled": true,
        "params": {
            "source_text_track_id": source_track,
        },
    })
}

fn track(id: &str, kind: TrackKind, locked: bool, clips: Vec<Value>) -> Value {
    json!({
        "id": id,
        "kind": kind,
        "name": "Track",
        "locked": locked,
        "clips": clips,
    })
}

fn project_with_tracks(tracks: Vec<Value>, duration_tk: i64) -> Project {
    let mut project = empty_project();
    project.tracks = tracks
        .into_iter()
        .map(|track| serde_json::from_value(track).expect("track fixture parses"))
        .collect();
    project.assets.clear();
    project
        .assets
        .push(serde_json::from_value(video_asset()).expect("asset fixture parses"));
    project.duration_tk = Tick::new(duration_tk);
    project
}

fn args(text_track: &str, style: Option<StyleArg>) -> CaptionBurnInArgs {
    CaptionBurnInArgs {
        project_id: fixture_project_id(),
        text_track: text_track.to_string(),
        style,
    }
}

fn apply_patch(prior: &Project, patch: Value) -> Project {
    let typed: json_patch::Patch = serde_json::from_value(patch).expect("patch parses");
    prior.apply(&typed).expect("patch applies")
}

fn effects_on(project: &Project, clip_id: &str) -> Vec<String> {
    project
        .tracks
        .iter()
        .flat_map(|track| track.clips.iter())
        .find(|clip| clip.id.to_string() == clip_id)
        .expect("clip exists")
        .effects
        .iter()
        .map(|effect| effect.id.to_string())
        .collect()
}

fn warning_codes(warnings: &[Value]) -> Vec<String> {
    warnings
        .iter()
        .map(|warning| warning["code"].as_str().expect("code string").to_string())
        .collect()
}

fn strings<T: ToString>(ids: &[T]) -> Vec<String> {
    ids.iter().map(ToString::to_string).collect()
}

// --- Happy paths -----------------------------------------------------

#[test]
fn full_overlap_creates_one_burned_caption() {
    let prior = project_with_tracks(
        vec![
            track(
                TEXT_TRACK_A,
                TrackKind::Text,
                false,
                vec![text_clip(TEXT_CLIP_A, 0, 240_000)],
            ),
            track(
                VIDEO_TRACK_A,
                TrackKind::Video,
                false,
                vec![video_clip(VIDEO_CLIP_A, 0, 240_000, false, vec![])],
            ),
        ],
        240_000,
    );

    let (patch, _warnings, data) =
        compute_patch(&prior, &args(TEXT_TRACK_A, None)).expect("happy path");
    let post = apply_patch(&prior, patch);

    assert_eq!(data.effect_ids.len(), 1);
    assert!(data.updated_effect_ids.is_empty());
    assert!(data.deduped_effect_ids.is_empty());
    assert_eq!(strings(&data.affected_clip_ids), vec![VIDEO_CLIP_A]);

    let new_effects = effects_on(&post, VIDEO_CLIP_A);
    assert_eq!(new_effects.len(), 1);
    let effect = &post.tracks[1].clips[0].effects[0];
    assert_eq!(effect.kind.as_str(), "burned_caption");
    assert!(effect.enabled);
    assert_eq!(
        effect
            .params
            .get("source_text_track_id")
            .and_then(Value::as_str),
        Some(TEXT_TRACK_A)
    );
    let window = effect.window.expect("window present");
    assert_eq!(window.in_tk.get(), 0);
    assert_eq!(window.out_tk.get(), 240_000);
}

#[test]
fn partial_overlap_yields_window_clamped_to_text_clip_range() {
    let prior = project_with_tracks(
        vec![
            track(
                TEXT_TRACK_A,
                TrackKind::Text,
                false,
                vec![text_clip(TEXT_CLIP_A, 100, 100)],
            ),
            track(
                VIDEO_TRACK_A,
                TrackKind::Video,
                false,
                vec![video_clip(VIDEO_CLIP_A, 0, 240_000, false, vec![])],
            ),
        ],
        240_000,
    );

    let (patch, _warnings, _data) =
        compute_patch(&prior, &args(TEXT_TRACK_A, None)).expect("happy path");
    let post = apply_patch(&prior, patch);

    let window = post.tracks[1].clips[0].effects[0]
        .window
        .expect("window present");
    assert_eq!(window.in_tk.get(), 100);
    assert_eq!(window.out_tk.get(), 200);
}

#[test]
fn two_text_clips_yield_bounding_intersection_per_video_clip() {
    let prior = project_with_tracks(
        vec![
            track(
                TEXT_TRACK_A,
                TrackKind::Text,
                false,
                vec![
                    text_clip(TEXT_CLIP_A, 10, 40),
                    text_clip(TEXT_CLIP_B, 100, 50),
                ],
            ),
            track(
                VIDEO_TRACK_A,
                TrackKind::Video,
                false,
                vec![video_clip(VIDEO_CLIP_A, 0, 240_000, false, vec![])],
            ),
        ],
        240_000,
    );

    let (patch, _warnings, data) =
        compute_patch(&prior, &args(TEXT_TRACK_A, None)).expect("happy path");
    let post = apply_patch(&prior, patch);
    assert_eq!(data.effect_ids.len(), 1);

    let window = post.tracks[1].clips[0].effects[0]
        .window
        .expect("window present");
    assert_eq!(window.in_tk.get(), 10);
    assert_eq!(window.out_tk.get(), 150);
}

#[test]
fn two_video_clips_both_overlap_yield_two_effects() {
    let prior = project_with_tracks(
        vec![
            track(
                TEXT_TRACK_A,
                TrackKind::Text,
                false,
                vec![text_clip(TEXT_CLIP_A, 0, 480_000)],
            ),
            track(
                VIDEO_TRACK_A,
                TrackKind::Video,
                false,
                vec![video_clip(VIDEO_CLIP_A, 0, 240_000, false, vec![])],
            ),
            track(
                VIDEO_TRACK_B,
                TrackKind::Video,
                false,
                vec![video_clip(VIDEO_CLIP_B, 240_000, 240_000, false, vec![])],
            ),
        ],
        480_000,
    );

    let (patch, _warnings, data) =
        compute_patch(&prior, &args(TEXT_TRACK_A, None)).expect("happy path");
    let post = apply_patch(&prior, patch);

    assert_eq!(data.effect_ids.len(), 2);
    assert_eq!(data.affected_clip_ids.len(), 2);
    assert_eq!(effects_on(&post, VIDEO_CLIP_A).len(), 1);
    assert_eq!(effects_on(&post, VIDEO_CLIP_B).len(), 1);
}

#[test]
fn video_clip_outside_text_track_emits_w_noop() {
    let prior = project_with_tracks(
        vec![
            track(
                TEXT_TRACK_A,
                TrackKind::Text,
                false,
                vec![text_clip(TEXT_CLIP_A, 0, 100_000)],
            ),
            // Video clip lives past the text-clip range.
            track(
                VIDEO_TRACK_A,
                TrackKind::Video,
                false,
                vec![video_clip(VIDEO_CLIP_A, 200_000, 40_000, false, vec![])],
            ),
        ],
        240_000,
    );

    let (patch, warnings, data) =
        compute_patch(&prior, &args(TEXT_TRACK_A, None)).expect("no-op ok");

    assert!(patch.as_array().expect("array").is_empty());
    assert!(data.effect_ids.is_empty());
    assert!(data.updated_effect_ids.is_empty());
    assert!(data.deduped_effect_ids.is_empty());
    assert!(data.affected_clip_ids.is_empty());
    let codes = warning_codes(&warnings);
    assert!(codes.contains(&W_NOOP_CODE.to_string()));
    let noop = warnings
        .iter()
        .find(|w| w["code"] == W_NOOP_CODE)
        .expect("noop warning");
    assert_eq!(
        noop["details"]["text_track_id"].as_str(),
        Some(TEXT_TRACK_A)
    );
}

#[test]
fn bare_uuid_selector_resolves() {
    let prior = simple_overlap_project();

    let (_patch, _warnings, data) =
        compute_patch(&prior, &args(TEXT_TRACK_A, None)).expect("happy path");
    assert_eq!(data.text_track_id.to_string(), TEXT_TRACK_A);
}

#[test]
fn qualified_track_selector_resolves() {
    let prior = simple_overlap_project();
    let qualified = format!("track:{TEXT_TRACK_A}");

    let (_patch, _warnings, data) =
        compute_patch(&prior, &args(&qualified, None)).expect("happy path");
    assert_eq!(data.text_track_id.to_string(), TEXT_TRACK_A);
}

#[test]
fn style_object_supplied_is_embedded_in_params() {
    let prior = simple_overlap_project();

    let mut style = Map::new();
    style.insert("color".to_string(), json!("#ff0000ff"));
    style.insert("font_size_px".to_string(), json!(48.0));

    let (patch, _warnings, _data) = compute_patch(
        &prior,
        &args(TEXT_TRACK_A, Some(StyleArg::Object(style.clone()))),
    )
    .expect("happy path");
    let post = apply_patch(&prior, patch);

    let effect = &post.tracks[1].clips[0].effects[0];
    let style_value = effect
        .params
        .get("style")
        .expect("style key present")
        .as_object()
        .expect("style object");
    assert_eq!(
        style_value.get("color").and_then(Value::as_str),
        Some("#ff0000ff")
    );
    assert_eq!(
        style_value.get("font_size_px").and_then(Value::as_f64),
        Some(48.0)
    );
}

#[test]
fn idempotent_second_call_updates_existing_effect_returning_updated_id() {
    let prior = simple_overlap_project();

    let (patch_one, _warnings, first) =
        compute_patch(&prior, &args(TEXT_TRACK_A, None)).expect("first call");
    let after_first = apply_patch(&prior, patch_one);
    assert_eq!(first.effect_ids.len(), 1);
    let minted_effect_id = first.effect_ids[0];

    let (patch_two, _warnings, second) =
        compute_patch(&after_first, &args(TEXT_TRACK_A, None)).expect("second call");
    let after_second = apply_patch(&after_first, patch_two);

    assert!(second.effect_ids.is_empty());
    assert_eq!(second.updated_effect_ids, vec![minted_effect_id]);
    assert!(second.deduped_effect_ids.is_empty());
    // One effect total on the clip, ID preserved.
    let effects = effects_on(&after_second, VIDEO_CLIP_A);
    assert_eq!(effects, vec![minted_effect_id.to_string()]);
}

#[test]
fn multi_match_dedup_removes_extras_and_emits_warning() {
    // Hand-crafted: two burned_caption effects on the same clip
    // referencing the same source text track.
    let prior = project_with_tracks(
        vec![
            track(
                TEXT_TRACK_A,
                TrackKind::Text,
                false,
                vec![text_clip(TEXT_CLIP_A, 0, 240_000)],
            ),
            track(
                VIDEO_TRACK_A,
                TrackKind::Video,
                false,
                vec![video_clip(
                    VIDEO_CLIP_A,
                    0,
                    240_000,
                    false,
                    vec![
                        burned_caption_effect(BURNED_A, TEXT_TRACK_A),
                        burned_caption_effect(BURNED_B, TEXT_TRACK_A),
                    ],
                )],
            ),
        ],
        240_000,
    );

    let (patch, warnings, data) =
        compute_patch(&prior, &args(TEXT_TRACK_A, None)).expect("dedup ok");
    let post = apply_patch(&prior, patch);

    assert!(data.effect_ids.is_empty());
    assert_eq!(strings(&data.updated_effect_ids), vec![BURNED_A]);
    assert_eq!(strings(&data.deduped_effect_ids), vec![BURNED_B]);
    assert_eq!(strings(&data.affected_clip_ids), vec![VIDEO_CLIP_A]);

    let codes = warning_codes(&warnings);
    assert!(codes.contains(&W_CAPTION_BURN_DEDUP_CODE.to_string()));
    let dedup = warnings
        .iter()
        .find(|w| w["code"] == W_CAPTION_BURN_DEDUP_CODE)
        .expect("dedup warning");
    assert_eq!(dedup["details"]["clip_id"].as_str(), Some(VIDEO_CLIP_A));
    assert_eq!(
        dedup["details"]["removed_effect_ids"]
            .as_array()
            .expect("array")
            .iter()
            .map(|v| v.as_str().expect("string").to_string())
            .collect::<Vec<_>>(),
        vec![BURNED_B]
    );

    let remaining = effects_on(&post, VIDEO_CLIP_A);
    assert_eq!(remaining, vec![BURNED_A]);
}

// --- Error paths -----------------------------------------------------

#[test]
fn clip_prefixed_selector_returns_bad_selector() {
    let prior = simple_overlap_project();
    let qualified = format!("clip:{VIDEO_CLIP_A}");

    let err = compute_patch(&prior, &args(&qualified, None)).expect_err("clip prefix rejected");
    match err {
        CaptionBurnInError::BadSelector { detail } => {
            assert!(detail.contains("clip"));
        }
        other => panic!("expected BadSelector, got {other:?}"),
    }
}

#[test]
fn malformed_uuid_returns_bad_selector() {
    let prior = simple_overlap_project();

    let err = compute_patch(&prior, &args("not-a-uuid", None)).expect_err("malformed rejected");
    match err {
        CaptionBurnInError::BadSelector { detail } => {
            assert!(detail.to_ascii_lowercase().contains("uuid"));
        }
        other => panic!("expected BadSelector, got {other:?}"),
    }
}

#[test]
fn unknown_text_track_returns_track_not_found() {
    let prior = simple_overlap_project();

    let err = compute_patch(&prior, &args(MISSING_TRACK, None)).expect_err("missing rejected");
    match err {
        CaptionBurnInError::TrackNotFound { track_id } => {
            assert_eq!(track_id, MISSING_TRACK);
        }
        other => panic!("expected TrackNotFound, got {other:?}"),
    }
}

#[test]
fn video_track_supplied_returns_track_kind_mismatch() {
    let prior = simple_overlap_project();

    let err = compute_patch(&prior, &args(VIDEO_TRACK_A, None)).expect_err("video track rejected");
    match err {
        CaptionBurnInError::TrackKindMismatch {
            expected_kind,
            actual_kind,
        } => {
            assert_eq!(expected_kind, "text");
            assert_eq!(actual_kind, "video");
        }
        other => panic!("expected TrackKindMismatch, got {other:?}"),
    }
}

#[test]
fn preset_name_returns_preset_unknown() {
    let prior = simple_overlap_project();

    let err = compute_patch(
        &prior,
        &args(TEXT_TRACK_A, Some(StyleArg::Preset("captions".to_string()))),
    )
    .expect_err("preset rejected");
    match err {
        CaptionBurnInError::PresetUnknown { preset, hint } => {
            assert_eq!(preset, "captions");
            assert!(!hint.is_empty());
        }
        other => panic!("expected PresetUnknown, got {other:?}"),
    }
}

#[test]
fn locked_affected_video_clip_returns_locked() {
    let prior = project_with_tracks(
        vec![
            track(
                TEXT_TRACK_A,
                TrackKind::Text,
                false,
                vec![text_clip(TEXT_CLIP_A, 0, 240_000)],
            ),
            track(
                VIDEO_TRACK_A,
                TrackKind::Video,
                false,
                vec![video_clip(VIDEO_CLIP_A, 0, 240_000, true, vec![])],
            ),
        ],
        240_000,
    );

    let err = compute_patch(&prior, &args(TEXT_TRACK_A, None)).expect_err("locked clip rejects");
    match err {
        CaptionBurnInError::Locked { failed_clip } => {
            assert_eq!(failed_clip, VIDEO_CLIP_A);
        }
        other => panic!("expected Locked, got {other:?}"),
    }
}

#[test]
fn locked_parent_video_track_returns_locked() {
    let prior = project_with_tracks(
        vec![
            track(
                TEXT_TRACK_A,
                TrackKind::Text,
                false,
                vec![text_clip(TEXT_CLIP_A, 0, 240_000)],
            ),
            track(
                VIDEO_TRACK_A,
                TrackKind::Video,
                true,
                vec![video_clip(VIDEO_CLIP_A, 0, 240_000, false, vec![])],
            ),
        ],
        240_000,
    );

    let err = compute_patch(&prior, &args(TEXT_TRACK_A, None)).expect_err("locked track rejects");
    match err {
        CaptionBurnInError::Locked { failed_clip } => {
            assert_eq!(failed_clip, VIDEO_CLIP_A);
        }
        other => panic!("expected Locked, got {other:?}"),
    }
}

// --- Envelope / determinism / round trips ----------------------------

#[test]
fn data_envelope_arrays_are_sorted_by_uuidv7_lex() {
    // Three video clips each get their own burned_caption.
    let prior = project_with_tracks(
        vec![
            track(
                TEXT_TRACK_A,
                TrackKind::Text,
                false,
                vec![text_clip(TEXT_CLIP_A, 0, 720_000)],
            ),
            track(
                VIDEO_TRACK_A,
                TrackKind::Video,
                false,
                vec![
                    video_clip(VIDEO_CLIP_C, 480_000, 240_000, false, vec![]),
                    video_clip(VIDEO_CLIP_A, 0, 240_000, false, vec![]),
                    video_clip(VIDEO_CLIP_B, 240_000, 240_000, false, vec![]),
                ],
            ),
        ],
        720_000,
    );

    let (_patch, _warnings, data) =
        compute_patch(&prior, &args(TEXT_TRACK_A, None)).expect("happy path");

    let clip_strings = strings(&data.affected_clip_ids);
    let mut sorted = clip_strings.clone();
    sorted.sort();
    assert_eq!(clip_strings, sorted);

    let effect_strings = strings(&data.effect_ids);
    let mut sorted_effects = effect_strings.clone();
    sorted_effects.sort();
    assert_eq!(effect_strings, sorted_effects);
}

#[test]
fn reconstructor_round_trip_create_new() {
    let prior = simple_overlap_project();
    let args = args(TEXT_TRACK_A, None);

    let (patch, warnings, recorded_data) = compute_patch(&prior, &args).expect("compute");
    let typed: json_patch::Patch = serde_json::from_value(patch.clone()).expect("patch parses");
    let post_state = prior.apply(&typed).expect("apply");
    let verb = CaptionBurnInVerb;

    let reconstructed = verb
        .reconstruct(
            &serde_json::to_value(&args).expect("args serialize"),
            &patch,
            &warnings,
            &post_state,
        )
        .expect("verb reconstructs");
    let reconstructed: CaptionBurnInData =
        serde_json::from_value(reconstructed).expect("data parses");
    assert_eq!(reconstructed, recorded_data);
}

#[test]
fn reconstructor_round_trip_idempotent_update() {
    let prior = simple_overlap_project();
    let args_first = args(TEXT_TRACK_A, None);
    let (patch_one, _warnings, first) = compute_patch(&prior, &args_first).expect("first compute");
    let after_first = apply_patch(&prior, patch_one);

    let args_second = args(TEXT_TRACK_A, None);
    let (patch_two, warnings_two, recorded_two) =
        compute_patch(&after_first, &args_second).expect("second compute");
    let typed_two: json_patch::Patch =
        serde_json::from_value(patch_two.clone()).expect("patch parses");
    let post_state = after_first.apply(&typed_two).expect("apply");

    let verb = CaptionBurnInVerb;
    let reconstructed = verb
        .reconstruct(
            &serde_json::to_value(&args_second).expect("args serialize"),
            &patch_two,
            &warnings_two,
            &post_state,
        )
        .expect("verb reconstructs");
    let reconstructed: CaptionBurnInData =
        serde_json::from_value(reconstructed).expect("data parses");
    assert_eq!(reconstructed, recorded_two);
    assert_eq!(reconstructed.updated_effect_ids, vec![first.effect_ids[0]]);
}

#[test]
fn reconstructor_round_trip_dedup() {
    let prior = project_with_tracks(
        vec![
            track(
                TEXT_TRACK_A,
                TrackKind::Text,
                false,
                vec![text_clip(TEXT_CLIP_A, 0, 240_000)],
            ),
            track(
                VIDEO_TRACK_A,
                TrackKind::Video,
                false,
                vec![video_clip(
                    VIDEO_CLIP_A,
                    0,
                    240_000,
                    false,
                    vec![
                        burned_caption_effect(BURNED_A, TEXT_TRACK_A),
                        burned_caption_effect(BURNED_B, TEXT_TRACK_A),
                    ],
                )],
            ),
        ],
        240_000,
    );
    let args = args(TEXT_TRACK_A, None);
    let (patch, warnings, recorded) = compute_patch(&prior, &args).expect("dedup compute");
    let typed: json_patch::Patch = serde_json::from_value(patch.clone()).expect("patch parses");
    let post_state = prior.apply(&typed).expect("apply");
    let verb = CaptionBurnInVerb;
    let reconstructed = verb
        .reconstruct(
            &serde_json::to_value(&args).expect("args serialize"),
            &patch,
            &warnings,
            &post_state,
        )
        .expect("verb reconstructs");
    let reconstructed: CaptionBurnInData =
        serde_json::from_value(reconstructed).expect("data parses");
    assert_eq!(reconstructed, recorded);
}

#[test]
fn data_envelope_from_warnings_round_trip() {
    let prior = simple_overlap_project();
    let (_patch, warnings, recorded) =
        compute_patch(&prior, &args(TEXT_TRACK_A, None)).expect("compute");

    let recovered = data_envelope_from_warnings(&warnings).expect("envelope decodes");
    assert_eq!(recovered, recorded);
    let codes = warning_codes(&warnings);
    assert!(codes.contains(&W_CAPTION_BURN_IN_ENVELOPE_CODE.to_string()));
}

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let base = simple_overlap_project();
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        base,
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry clears the gate");

    let outcome = store
        .mutate_via_verb(
            "caption.burn_in",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "text_track": TEXT_TRACK_A,
            }),
            None,
        )
        .expect("mutate_via_verb happy path");

    let MutateOutcome::Applied { data, .. } = outcome else {
        panic!("happy path must return Applied, got {outcome:?}");
    };

    let data: CaptionBurnInData = serde_json::from_value(data).expect("data parses");
    assert_eq!(data.effect_ids.len(), 1);
    assert_eq!(effects_on(store.project(), VIDEO_CLIP_A).len(), 1);
}

#[test]
fn default_fixture_reconstructs() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "caption.burn_in")
        .expect("default_fixtures includes caption.burn_in");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(CaptionBurnInVerb))
        .expect("register caption.burn_in verb");

    let report =
        validate_reconstructors(&registry, &[fixture]).expect("reconstruction from fixture");
    assert_eq!(report.verbs_checked, vec!["caption.burn_in"]);
}

// --- Helpers ---------------------------------------------------------

fn simple_overlap_project() -> Project {
    project_with_tracks(
        vec![
            track(
                TEXT_TRACK_A,
                TrackKind::Text,
                false,
                vec![text_clip(TEXT_CLIP_A, 0, 240_000)],
            ),
            track(
                VIDEO_TRACK_A,
                TrackKind::Video,
                false,
                vec![video_clip(VIDEO_CLIP_A, 0, 240_000, false, vec![])],
            ),
        ],
        240_000,
    )
}

/// Resolve the `style` map embedded in the single burned-caption effect
/// after applying the patch — i.e. the color values as they enter
/// canonical event data.
fn burned_style(prior: &Project, style: Map<String, Value>) -> Map<String, Value> {
    let (patch, _warnings, _data) =
        compute_patch(prior, &args(TEXT_TRACK_A, Some(StyleArg::Object(style))))
            .expect("happy path");
    let post = apply_patch(prior, patch);
    post.tracks[1].clips[0].effects[0]
        .params
        .get("style")
        .expect("style key present")
        .as_object()
        .expect("style object")
        .clone()
}

// All remaining color inputs must normalize before entering canonical
// event data. `caption.burn_in` previously embedded the raw caller-
// supplied `style` map into the burned-caption effect params, so an
// uppercase color was written verbatim into canonical event data —
// bypassing the typed `Color` newtype that every sibling color-bearing
// verb routes through.

#[test]
fn caption_burn_in_normalizes_uppercase_color_into_event_data() {
    let prior = simple_overlap_project();
    let mut style = Map::new();
    style.insert("color".to_string(), json!("#FF0000FF"));

    let embedded = burned_style(&prior, style);
    assert_eq!(
        embedded.get("color").and_then(Value::as_str),
        Some("#ff0000ff"),
        "uppercase `color` must be lowercased before entering effect params"
    );
}

#[test]
fn caption_burn_in_normalizes_uppercase_bg_and_stroke_color_into_event_data() {
    let prior = simple_overlap_project();
    let mut style = Map::new();
    style.insert("bg_color".to_string(), json!("#11AABBCC"));
    style.insert("stroke_color".to_string(), json!("#DDeeFF00"));

    let embedded = burned_style(&prior, style);
    assert_eq!(
        embedded.get("bg_color").and_then(Value::as_str),
        Some("#11aabbcc")
    );
    assert_eq!(
        embedded.get("stroke_color").and_then(Value::as_str),
        Some("#ddeeff00")
    );
}

#[test]
fn caption_burn_in_normalizes_uppercase_shadow_color_into_event_data() {
    let prior = simple_overlap_project();
    let mut style = Map::new();
    style.insert(
        "shadow".to_string(),
        json!({ "color": "#000000AA", "blur_px": 4.0 }),
    );

    let embedded = burned_style(&prior, style);
    let shadow = embedded
        .get("shadow")
        .and_then(Value::as_object)
        .expect("shadow object embedded");
    assert_eq!(
        shadow.get("color").and_then(Value::as_str),
        Some("#000000aa"),
        "uppercase `shadow.color` must be lowercased before entering effect params"
    );
    // Normalization touches only the color leaf — the partial shadow map
    // keeps exactly the keys the caller sent, no default fields injected.
    let mut keys = shadow.keys().map(String::as_str).collect::<Vec<_>>();
    keys.sort_unstable();
    assert_eq!(keys, vec!["blur_px", "color"]);
}

#[test]
fn caption_burn_in_rejects_malformed_color_before_event_data() {
    let prior = simple_overlap_project();
    let mut style = Map::new();
    style.insert("color".to_string(), json!("red"));

    let err = compute_patch(&prior, &args(TEXT_TRACK_A, Some(StyleArg::Object(style))))
        .expect_err("a malformed color must be rejected, not passed through");
    assert!(matches!(
        err,
        CaptionBurnInError::StyleSchemaViolation { .. }
    ));
}
