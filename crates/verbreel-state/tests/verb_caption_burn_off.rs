//! Tests for `caption.burn_off` (§10.5) — fifty-first production verb.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::caption_burn_off::{
    CaptionBurnOffArgs, CaptionBurnOffData, CaptionBurnOffError, CaptionBurnOffVerb,
    W_KEYFRAMES_REMOVED_CODE, W_NOOP_CODE, compute_patch, data_envelope_from_args_warnings,
};
use verbreel_state::{
    Project, TrackKind, Verb, VerbRegistry, default_fixtures, validate_reconstructors,
};
use verbreel_types::{ProjectId, Tick};

#[cfg(feature = "native")]
use verbreel_state::{MutateOutcome, ProjectStore, default_registry};

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";

const TEXT_TRACK_A: &str = "01900000-0000-7000-8000-0000000aa901";
const TEXT_TRACK_B: &str = "01900000-0000-7000-8000-0000000aa902";
const VIDEO_TRACK_A: &str = "01900000-0000-7000-8000-0000000aa903";
const MISSING_TRACK: &str = "01900000-0000-7000-8000-0000000aa999";

const TEXT_CLIP_A: &str = "01900000-0000-7000-8000-0000000bb901";
const TEXT_CLIP_B: &str = "01900000-0000-7000-8000-0000000bb902";
const VIDEO_CLIP_A: &str = "01900000-0000-7000-8000-0000000bb903";
const VIDEO_CLIP_B: &str = "01900000-0000-7000-8000-0000000bb904";
const VIDEO_CLIP_C: &str = "01900000-0000-7000-8000-0000000bb905";
const MISSING_CLIP: &str = "01900000-0000-7000-8000-0000000bb999";

const ASSET_ID: &str = "01900000-0000-7000-8000-0000000cc901";
const BURNED_A: &str = "01900000-0000-7000-8000-0000000ee903";
const BURNED_B: &str = "01900000-0000-7000-8000-0000000ee901";
const BURNED_C: &str = "01900000-0000-7000-8000-0000000ee902";
const BLUR_EFFECT: &str = "01900000-0000-7000-8000-0000000ef901";

const KEYFRAME_A: &str = "01900000-0000-7000-8000-0000000ff903";
const KEYFRAME_B: &str = "01900000-0000-7000-8000-0000000ff901";
const KEYFRAME_C: &str = "01900000-0000-7000-8000-0000000ff902";

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
        "original_filename": "caption-burn-off.mp4",
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

fn burned_effect(id: &str, source_text_track_id: &str) -> Value {
    json!({
        "id": id,
        "kind": "burned_caption",
        "enabled": true,
        "params": {
            "source_text_track_id": source_text_track_id,
        },
    })
}

fn blur_effect() -> Value {
    json!({
        "id": BLUR_EFFECT,
        "kind": "blur",
        "enabled": true,
        "params": { "radius_px": 4 },
    })
}

fn effect_keyframe(id: &str, effect_id: &str) -> Value {
    json!({
        "id": id,
        "property": format!("effects[{effect_id}].params.opacity"),
        "time_tk": 0,
        "value": 1.0,
        "easing": "linear",
    })
}

fn opacity_keyframe(id: &str) -> Value {
    json!({
        "id": id,
        "property": "opacity",
        "time_tk": 0,
        "value": 1.0,
        "easing": "linear",
    })
}

fn text_clip(id: &str) -> Value {
    json!({
        "id": id,
        "name": "Text Clip",
        "asset_id": "00000000-0000-0000-0000-000000000000",
        "track_position_tk": 0,
        "source_in_tk": 0,
        "source_out_tk": 480_000,
        "locked": false,
        "text": {
            "content": "Caption",
            "font_family": "Arial",
            "font_size_px": 24,
        },
    })
}

fn video_clip(id: &str, locked: bool, effects: Vec<Value>, keyframes: Vec<Value>) -> Value {
    json!({
        "id": id,
        "name": "Video Clip",
        "asset_id": ASSET_ID,
        "track_position_tk": 0,
        "source_in_tk": 0,
        "source_out_tk": 480_000,
        "locked": locked,
        "effects": effects,
        "keyframes": keyframes,
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

fn project_with_tracks(tracks: Vec<Value>) -> Project {
    let mut project = empty_project();
    project.tracks = tracks
        .into_iter()
        .map(|track| serde_json::from_value(track).expect("track fixture parses"))
        .collect();
    project.assets.clear();
    project
        .assets
        .push(serde_json::from_value(video_asset()).expect("asset fixture parses"));
    project.duration_tk = Tick::new(480_000);
    project
}

fn base_project(video_clips: Vec<Value>) -> Project {
    let mut spaced_video_clips = video_clips;
    for (idx, clip) in spaced_video_clips.iter_mut().enumerate() {
        clip["track_position_tk"] = json!(idx as i64 * 480_000);
    }
    let video_clip_count = spaced_video_clips.len();
    let mut project = project_with_tracks(vec![
        track(
            TEXT_TRACK_A,
            TrackKind::Text,
            false,
            vec![text_clip(TEXT_CLIP_A)],
        ),
        track(
            TEXT_TRACK_B,
            TrackKind::Text,
            false,
            vec![text_clip(TEXT_CLIP_B)],
        ),
        track(VIDEO_TRACK_A, TrackKind::Video, false, spaced_video_clips),
    ]);
    project.duration_tk = Tick::new(std::cmp::max(1, video_clip_count) as i64 * 480_000);
    project
}

fn args(text_track: Option<&str>, clip: Option<&str>) -> CaptionBurnOffArgs {
    CaptionBurnOffArgs {
        project_id: fixture_project_id(),
        text_track: text_track.map(str::to_string),
        clip: clip.map(str::to_string),
    }
}

fn apply_patch(prior: &Project, patch: Value) -> Project {
    let patch: json_patch::Patch = serde_json::from_value(patch).expect("patch parses");
    prior.apply(&patch).expect("patch applies")
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

fn keyframes_on(project: &Project, clip_id: &str) -> Vec<String> {
    project
        .tracks
        .iter()
        .flat_map(|track| track.clips.iter())
        .find(|clip| clip.id.to_string() == clip_id)
        .expect("clip exists")
        .keyframes
        .iter()
        .map(|keyframe| keyframe.id.to_string())
        .collect()
}

fn strings<T: ToString>(ids: &[T]) -> Vec<String> {
    ids.iter().map(ToString::to_string).collect()
}

fn warning_codes(warnings: &[Value]) -> Vec<String> {
    warnings
        .iter()
        .map(|warning| warning["code"].as_str().expect("code string").to_string())
        .collect()
}

#[test]
fn both_args_none_returns_args_incompatible() {
    let prior = base_project(vec![]);

    let err = compute_patch(&prior, &args(None, None)).expect_err("missing selectors rejects");

    match err {
        CaptionBurnOffError::ArgsIncompatible { hint } => {
            assert_eq!(hint, "supply at least one of text_track or clip");
        }
        other => panic!("expected ArgsIncompatible, got {other:?}"),
    }
}

#[test]
fn text_track_alone_removes_all_matching_burned_caption_effects() {
    let prior = base_project(vec![
        video_clip(
            VIDEO_CLIP_A,
            false,
            vec![burned_effect(BURNED_A, TEXT_TRACK_A), blur_effect()],
            vec![],
        ),
        video_clip(
            VIDEO_CLIP_B,
            false,
            vec![burned_effect(BURNED_B, TEXT_TRACK_A)],
            vec![],
        ),
    ]);

    let (patch, warnings, data) =
        compute_patch(&prior, &args(Some(TEXT_TRACK_A), None)).expect("happy path");
    let post = apply_patch(&prior, patch);

    assert_eq!(
        warning_codes(&warnings),
        vec!["W_CAPTION_BURN_OFF_ENVELOPE"]
    );
    assert_eq!(
        strings(&data.removed_effect_ids),
        vec![BURNED_B.to_string(), BURNED_A.to_string()]
    );
    assert_eq!(
        strings(&data.affected_clip_ids),
        vec![VIDEO_CLIP_A.to_string(), VIDEO_CLIP_B.to_string()]
    );
    assert_eq!(
        data.resolved_text_track_id.map(|id| id.to_string()),
        Some(TEXT_TRACK_A.to_string())
    );
    assert_eq!(
        effects_on(&post, VIDEO_CLIP_A),
        vec![BLUR_EFFECT.to_string()]
    );
    assert!(effects_on(&post, VIDEO_CLIP_B).is_empty());
}

#[test]
fn clip_alone_removes_all_burned_caption_effects_on_that_clip() {
    let prior = base_project(vec![
        video_clip(
            VIDEO_CLIP_A,
            false,
            vec![
                burned_effect(BURNED_A, TEXT_TRACK_A),
                burned_effect(BURNED_B, TEXT_TRACK_B),
                blur_effect(),
            ],
            vec![],
        ),
        video_clip(
            VIDEO_CLIP_B,
            false,
            vec![burned_effect(BURNED_C, TEXT_TRACK_A)],
            vec![],
        ),
    ]);

    let (patch, _warnings, data) =
        compute_patch(&prior, &args(None, Some(VIDEO_CLIP_A))).expect("happy path");
    let post = apply_patch(&prior, patch);

    assert_eq!(
        strings(&data.removed_effect_ids),
        vec![BURNED_B.to_string(), BURNED_A.to_string()]
    );
    assert_eq!(data.resolved_text_track_id, None);
    assert_eq!(
        effects_on(&post, VIDEO_CLIP_A),
        vec![BLUR_EFFECT.to_string()]
    );
    assert_eq!(effects_on(&post, VIDEO_CLIP_B), vec![BURNED_C.to_string()]);
}

#[test]
fn both_supplied_remove_only_intersection() {
    let prior = base_project(vec![
        video_clip(
            VIDEO_CLIP_A,
            false,
            vec![
                burned_effect(BURNED_A, TEXT_TRACK_A),
                burned_effect(BURNED_B, TEXT_TRACK_B),
            ],
            vec![],
        ),
        video_clip(
            VIDEO_CLIP_B,
            false,
            vec![burned_effect(BURNED_C, TEXT_TRACK_A)],
            vec![],
        ),
    ]);

    let (patch, _warnings, data) =
        compute_patch(&prior, &args(Some(TEXT_TRACK_A), Some(VIDEO_CLIP_A))).expect("happy path");
    let post = apply_patch(&prior, patch);

    assert_eq!(
        strings(&data.removed_effect_ids),
        vec![BURNED_A.to_string()]
    );
    assert_eq!(effects_on(&post, VIDEO_CLIP_A), vec![BURNED_B.to_string()]);
    assert_eq!(effects_on(&post, VIDEO_CLIP_B), vec![BURNED_C.to_string()]);
}

#[test]
fn noop_when_text_track_has_no_references() {
    let prior = base_project(vec![video_clip(
        VIDEO_CLIP_A,
        false,
        vec![burned_effect(BURNED_A, TEXT_TRACK_A)],
        vec![],
    )]);

    let (patch, warnings, data) =
        compute_patch(&prior, &args(Some(TEXT_TRACK_B), None)).expect("no-op ok");

    assert!(patch.as_array().expect("patch array").is_empty());
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_NOOP_CODE);
    assert_eq!(
        warnings[0]["details"]["message"],
        "no matching burned_caption effects"
    );
    assert_eq!(
        warnings[0]["details"]["resolved_text_track_id"],
        TEXT_TRACK_B
    );
    assert!(data.removed_effect_ids.is_empty());
    assert_eq!(
        data.resolved_text_track_id.map(|id| id.to_string()),
        Some(TEXT_TRACK_B.to_string())
    );
}

#[test]
fn noop_when_clip_has_no_burned_caption_effects() {
    let prior = base_project(vec![video_clip(
        VIDEO_CLIP_A,
        false,
        vec![blur_effect()],
        vec![],
    )]);

    let (patch, warnings, data) =
        compute_patch(&prior, &args(None, Some(VIDEO_CLIP_A))).expect("no-op ok");

    assert!(patch.as_array().expect("patch array").is_empty());
    assert_eq!(warnings[0]["code"], W_NOOP_CODE);
    assert_eq!(warnings[0]["details"]["clip_id"], VIDEO_CLIP_A);
    assert!(data.removed_effect_ids.is_empty());
    assert_eq!(data.resolved_text_track_id, None);
}

#[test]
fn malformed_text_track_uuid_returns_bad_selector() {
    let prior = base_project(vec![]);

    let err =
        compute_patch(&prior, &args(Some("not-a-uuid"), None)).expect_err("bad text_track rejects");

    match err {
        CaptionBurnOffError::BadSelector { field, detail } => {
            assert_eq!(field, "text_track");
            assert!(detail.contains("UUID"));
        }
        other => panic!("expected BadSelector, got {other:?}"),
    }
}

#[test]
fn malformed_clip_uuid_returns_bad_selector() {
    let prior = base_project(vec![]);

    let err = compute_patch(&prior, &args(None, Some("not-a-uuid"))).expect_err("bad clip rejects");

    match err {
        CaptionBurnOffError::BadSelector { field, detail } => {
            assert_eq!(field, "clip");
            assert!(detail.contains("UUID"));
        }
        other => panic!("expected BadSelector, got {other:?}"),
    }
}

#[test]
fn unknown_text_track_returns_track_not_found() {
    let prior = base_project(vec![]);

    let err = compute_patch(&prior, &args(Some(MISSING_TRACK), None))
        .expect_err("missing text track rejects");

    match err {
        CaptionBurnOffError::TrackNotFound { track_id } => assert_eq!(track_id, MISSING_TRACK),
        other => panic!("expected TrackNotFound, got {other:?}"),
    }
}

#[test]
fn text_track_resolving_to_video_track_returns_kind_mismatch() {
    let prior = base_project(vec![]);

    let err = compute_patch(&prior, &args(Some(VIDEO_TRACK_A), None))
        .expect_err("video track rejects as text_track");

    match err {
        CaptionBurnOffError::TrackKindMismatch {
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
fn unknown_clip_returns_not_found() {
    let prior = base_project(vec![]);

    let err =
        compute_patch(&prior, &args(None, Some(MISSING_CLIP))).expect_err("missing clip rejects");

    match err {
        CaptionBurnOffError::ClipNotFound { clip_id } => assert_eq!(clip_id, MISSING_CLIP),
        other => panic!("expected ClipNotFound, got {other:?}"),
    }
}

#[test]
fn matched_locked_clip_returns_locked_without_partial_patch() {
    let prior = base_project(vec![
        video_clip(
            VIDEO_CLIP_A,
            true,
            vec![burned_effect(BURNED_A, TEXT_TRACK_A)],
            vec![],
        ),
        video_clip(
            VIDEO_CLIP_B,
            false,
            vec![burned_effect(BURNED_B, TEXT_TRACK_A)],
            vec![],
        ),
    ]);

    let err = compute_patch(&prior, &args(Some(TEXT_TRACK_A), None))
        .expect_err("locked clip rejects all removals");

    match err {
        CaptionBurnOffError::Locked { failed_clip } => assert_eq!(failed_clip, VIDEO_CLIP_A),
        other => panic!("expected Locked, got {other:?}"),
    }
    assert_eq!(effects_on(&prior, VIDEO_CLIP_B), vec![BURNED_B.to_string()]);
}

#[test]
fn matched_locked_parent_track_returns_locked() {
    let prior = project_with_tracks(vec![
        track(
            TEXT_TRACK_A,
            TrackKind::Text,
            false,
            vec![text_clip(TEXT_CLIP_A)],
        ),
        track(
            VIDEO_TRACK_A,
            TrackKind::Video,
            true,
            vec![video_clip(
                VIDEO_CLIP_A,
                false,
                vec![burned_effect(BURNED_A, TEXT_TRACK_A)],
                vec![],
            )],
        ),
    ]);

    let err = compute_patch(&prior, &args(Some(TEXT_TRACK_A), None))
        .expect_err("locked parent track rejects");

    match err {
        CaptionBurnOffError::Locked { failed_clip } => assert_eq!(failed_clip, VIDEO_CLIP_A),
        other => panic!("expected Locked, got {other:?}"),
    }
}

#[test]
fn keyframes_targeting_removed_effect_are_cascade_removed() {
    let prior = base_project(vec![video_clip(
        VIDEO_CLIP_A,
        false,
        vec![burned_effect(BURNED_A, TEXT_TRACK_A)],
        vec![effect_keyframe(KEYFRAME_A, BURNED_A)],
    )]);

    let (patch, warnings, data) =
        compute_patch(&prior, &args(Some(TEXT_TRACK_A), None)).expect("happy path");
    let post = apply_patch(&prior, patch);

    assert_eq!(
        strings(&data.removed_keyframe_ids),
        vec![KEYFRAME_A.to_string()]
    );
    assert_eq!(
        warning_codes(&warnings),
        vec!["W_CAPTION_BURN_OFF_ENVELOPE", W_KEYFRAMES_REMOVED_CODE]
    );
    assert_eq!(warnings[1]["details"]["clip_id"], VIDEO_CLIP_A);
    assert!(keyframes_on(&post, VIDEO_CLIP_A).is_empty());
}

#[test]
fn keyframes_targeting_other_effects_survive() {
    let prior = base_project(vec![video_clip(
        VIDEO_CLIP_A,
        false,
        vec![
            burned_effect(BURNED_A, TEXT_TRACK_A),
            burned_effect(BURNED_B, TEXT_TRACK_B),
        ],
        vec![
            effect_keyframe(KEYFRAME_A, BURNED_A),
            effect_keyframe(KEYFRAME_B, BURNED_B),
            opacity_keyframe(KEYFRAME_C),
        ],
    )]);

    let (patch, _warnings, data) =
        compute_patch(&prior, &args(Some(TEXT_TRACK_A), None)).expect("happy path");
    let post = apply_patch(&prior, patch);

    assert_eq!(
        strings(&data.removed_keyframe_ids),
        vec![KEYFRAME_A.to_string()]
    );
    assert_eq!(
        keyframes_on(&post, VIDEO_CLIP_A),
        vec![KEYFRAME_B.to_string(), KEYFRAME_C.to_string()]
    );
}

#[test]
fn effects_of_other_kinds_survive_on_same_clip() {
    let prior = base_project(vec![video_clip(
        VIDEO_CLIP_A,
        false,
        vec![burned_effect(BURNED_A, TEXT_TRACK_A), blur_effect()],
        vec![],
    )]);

    let (patch, _warnings, _data) =
        compute_patch(&prior, &args(Some(TEXT_TRACK_A), None)).expect("happy path");
    let post = apply_patch(&prior, patch);

    assert_eq!(
        effects_on(&post, VIDEO_CLIP_A),
        vec![BLUR_EFFECT.to_string()]
    );
}

#[test]
fn data_envelope_sorts_removed_effects_and_affected_clips() {
    let prior = base_project(vec![
        video_clip(
            VIDEO_CLIP_B,
            false,
            vec![burned_effect(BURNED_A, TEXT_TRACK_A)],
            vec![],
        ),
        video_clip(
            VIDEO_CLIP_A,
            false,
            vec![burned_effect(BURNED_B, TEXT_TRACK_A)],
            vec![],
        ),
    ]);

    let (_patch, _warnings, data) =
        compute_patch(&prior, &args(Some(TEXT_TRACK_A), None)).expect("happy path");

    assert_eq!(
        strings(&data.removed_effect_ids),
        vec![BURNED_B.to_string(), BURNED_A.to_string()]
    );
    assert_eq!(
        strings(&data.affected_clip_ids),
        vec![VIDEO_CLIP_A.to_string(), VIDEO_CLIP_B.to_string()]
    );
}

#[test]
fn resolved_text_track_id_present_iff_text_track_was_supplied() {
    let prior = base_project(vec![video_clip(
        VIDEO_CLIP_A,
        false,
        vec![burned_effect(BURNED_A, TEXT_TRACK_A)],
        vec![],
    )]);

    let (_patch, warnings, with_track) =
        compute_patch(&prior, &args(Some(TEXT_TRACK_A), None)).expect("track path");
    let from_warning = data_envelope_from_args_warnings(&args(Some(TEXT_TRACK_A), None), &warnings)
        .expect("data from warning");
    let (_patch, _warnings, clip_only) =
        compute_patch(&prior, &args(None, Some(VIDEO_CLIP_A))).expect("clip path");

    assert_eq!(
        with_track.resolved_text_track_id.map(|id| id.to_string()),
        Some(TEXT_TRACK_A.to_string())
    );
    assert_eq!(
        from_warning.resolved_text_track_id.map(|id| id.to_string()),
        Some(TEXT_TRACK_A.to_string())
    );
    assert_eq!(clip_only.resolved_text_track_id, None);
}

#[test]
fn reconstructor_round_trip_text_track_removal() {
    let prior = base_project(vec![video_clip(
        VIDEO_CLIP_A,
        false,
        vec![burned_effect(BURNED_A, TEXT_TRACK_A)],
        vec![],
    )]);
    let args = args(Some(TEXT_TRACK_A), None);

    let (patch, warnings, recorded_data) = compute_patch(&prior, &args).expect("compute patch");
    let typed_patch: json_patch::Patch =
        serde_json::from_value(patch.clone()).expect("patch parses to typed JSON patch");
    let post_state = prior.apply(&typed_patch).expect("patch applies");
    let verb = CaptionBurnOffVerb;

    let reconstructed = verb
        .reconstruct(
            &serde_json::to_value(&args).expect("args serialize"),
            &patch,
            &warnings,
            &post_state,
        )
        .expect("verb reconstructs");
    let reconstructed: CaptionBurnOffData =
        serde_json::from_value(reconstructed).expect("caption.burn_off data parses");

    assert_eq!(recorded_data, reconstructed);
}

#[test]
fn reconstructor_round_trip_with_keyframe_cascade() {
    let prior = base_project(vec![video_clip(
        VIDEO_CLIP_A,
        false,
        vec![burned_effect(BURNED_A, TEXT_TRACK_A)],
        vec![effect_keyframe(KEYFRAME_A, BURNED_A)],
    )]);
    let args = args(Some(TEXT_TRACK_A), None);

    let (patch, warnings, recorded_data) = compute_patch(&prior, &args).expect("compute patch");
    let typed_patch: json_patch::Patch =
        serde_json::from_value(patch.clone()).expect("patch parses to typed JSON patch");
    let post_state = prior.apply(&typed_patch).expect("patch applies");

    let expected_data = serde_json::to_value(&recorded_data).expect("data serializes");
    let recorded = verbreel_state::RecordedEvent {
        verb: "caption.burn_off".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch,
        warnings,
        post_state,
        expected_data,
    };

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(CaptionBurnOffVerb))
        .expect("register caption.burn_off verb");

    let report = validate_reconstructors(&registry, &[recorded]).expect("validation passes");
    assert_eq!(report.verbs_checked, vec!["caption.burn_off"]);
}

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let base_project = base_project(vec![video_clip(
        VIDEO_CLIP_A,
        false,
        vec![burned_effect(BURNED_A, TEXT_TRACK_A)],
        vec![],
    )]);
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        base_project,
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry clears the gate");

    let outcome = store
        .mutate_via_verb(
            "caption.burn_off",
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

    let data: CaptionBurnOffData =
        serde_json::from_value(data).expect("caption.burn_off data parses");
    assert_eq!(
        strings(&data.removed_effect_ids),
        vec![BURNED_A.to_string()]
    );
    assert!(effects_on(store.project(), VIDEO_CLIP_A).is_empty());
}

#[test]
fn multi_clip_removal_removes_all_three_atomically() {
    let prior = base_project(vec![
        video_clip(
            VIDEO_CLIP_A,
            false,
            vec![burned_effect(BURNED_A, TEXT_TRACK_A)],
            vec![],
        ),
        video_clip(
            VIDEO_CLIP_B,
            false,
            vec![burned_effect(BURNED_B, TEXT_TRACK_A)],
            vec![],
        ),
        video_clip(
            VIDEO_CLIP_C,
            false,
            vec![burned_effect(BURNED_C, TEXT_TRACK_A)],
            vec![],
        ),
    ]);

    let (patch, _warnings, data) =
        compute_patch(&prior, &args(Some(TEXT_TRACK_A), None)).expect("happy path");
    let post = apply_patch(&prior, patch);

    assert_eq!(
        strings(&data.affected_clip_ids),
        vec![
            VIDEO_CLIP_A.to_string(),
            VIDEO_CLIP_B.to_string(),
            VIDEO_CLIP_C.to_string()
        ]
    );
    assert!(effects_on(&post, VIDEO_CLIP_A).is_empty());
    assert!(effects_on(&post, VIDEO_CLIP_B).is_empty());
    assert!(effects_on(&post, VIDEO_CLIP_C).is_empty());
}

#[test]
fn default_fixture_reconstructs() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "caption.burn_off")
        .expect("default_fixtures includes caption.burn_off");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(CaptionBurnOffVerb))
        .expect("register caption.burn_off verb");

    let report =
        validate_reconstructors(&registry, &[fixture]).expect("reconstruction from fixture");
    assert_eq!(report.verbs_checked, vec!["caption.burn_off"]);
}

#[test]
fn patch_replaces_effects_and_keyframes_arrays_atomically_when_cascade_fires() {
    let prior = base_project(vec![video_clip(
        VIDEO_CLIP_A,
        false,
        vec![burned_effect(BURNED_A, TEXT_TRACK_A)],
        vec![effect_keyframe(KEYFRAME_A, BURNED_A)],
    )]);

    let (patch, _warnings, _data) =
        compute_patch(&prior, &args(Some(TEXT_TRACK_A), None)).expect("happy path");
    let ops = patch.as_array().expect("patch array");

    assert_eq!(ops.len(), 2);
    assert_eq!(ops[0]["op"], "replace");
    assert_eq!(ops[0]["path"], "/tracks/2/clips/0/effects");
    assert_eq!(ops[1]["op"], "replace");
    assert_eq!(ops[1]["path"], "/tracks/2/clips/0/keyframes");
}
