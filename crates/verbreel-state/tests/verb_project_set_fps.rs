//! Tests for `project.set_fps` (§2.11) — the third production verb.
//!
//! Covers `compute_patch` happy paths (minimal 30→60, fps_den-omitted
//! single-op patch, partial-update preserves NTSC den), error paths
//! (fps_num / fps_den zero), the `is_off_frame` predicate at integer
//! and NTSC rates, off-frame bucketing across all four entity classes
//! (video clip, audio clip, keyframe, marker), the
//! `list_off_frame_entities` flag matrix (true emits / false omits /
//! true-but-zero omits), `data_envelope`, the reconstructor round-trip
//! via [`validate_reconstructors`], and one end-to-end exercise
//! through [`ProjectStore::mutate_via_verb`] proving the verb is wired
//! into the kernel's default registry + fixtures.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::{
    Clip, FPS_MIN, Marker, MutateOutcome, Project, ProjectSetFpsArgs, ProjectSetFpsError,
    ProjectSetFpsVerb, ProjectStore, RecordedEvent, VerbRegistry, default_fixtures,
    default_registry, validate_reconstructors,
    verbs::project_set_fps::{compute_patch, data_envelope, is_off_frame},
};
use verbreel_types::{ProjectId, Tick};

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";

// Synthetic v7 UUIDs used as Clip / Keyframe / Marker / Asset ids in
// hand-built projects. Hard-coded so the id-list outputs in tests stay
// deterministic (calling ::now() would tie tests to wall-clock state).
const SAMPLE_CLIP_ID: &str = "0190b8d3-15e3-7000-bd00-0000000aa001";
const SAMPLE_CLIP_ID_2: &str = "0190b8d3-15e3-7000-bd00-0000000aa002";
const SAMPLE_KEYFRAME_ID: &str = "0190b8d3-15e3-7000-bd00-0000000bb001";
const SAMPLE_MARKER_ID: &str = "0190b8d3-15e3-7000-bd00-0000000cc001";
const SAMPLE_ASSET_ID: &str = "0190b8d3-15e3-7000-bd00-0000000dd001";

/// Load the canonical empty-project fixture as the prior state.
/// fps = 30/1, two tracks (`Video 1` at index 0, `Audio 1` at index 1),
/// no clips, no markers, no metadata.
fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture → Project")
}

/// The fixture project's own id — reuse so the round-trip envelope's
/// `project_id` field matches by construction.
fn fixture_project_id() -> ProjectId {
    empty_project().id
}

/// Convenience constructor: `ProjectSetFpsArgs` with the fixture
/// project id, the supplied numerator + optional denominator, and the
/// list flag defaulted off (`None`).
fn args(fps_num: u32, fps_den: Option<u32>) -> ProjectSetFpsArgs {
    ProjectSetFpsArgs {
        project_id: fixture_project_id(),
        fps_num,
        fps_den,
        list_off_frame_entities: None,
    }
}

/// Same as [`args`] but with `list_off_frame_entities` set to `Some(flag)`.
fn args_listing(fps_num: u32, fps_den: Option<u32>, flag: bool) -> ProjectSetFpsArgs {
    ProjectSetFpsArgs {
        project_id: fixture_project_id(),
        fps_num,
        fps_den,
        list_off_frame_entities: Some(flag),
    }
}

/// Build a video-track clip at the given `track_position_tk`. Source
/// range is `[0, 240000)` so timeline duration is one second at 30/1
/// (irrelevant to the off-frame walk — only `track_position_tk` is
/// inspected).
fn video_clip(id: &str, track_position_tk: i64, keyframes: Value) -> Clip {
    serde_json::from_value(json!({
        "id": id,
        "name": "test-clip",
        "asset_id": SAMPLE_ASSET_ID,
        "track_position_tk": track_position_tk,
        "source_in_tk": 0,
        "source_out_tk": 240000,
        "keyframes": keyframes,
    }))
    .expect("video clip JSON → Clip")
}

/// Build a `Marker` at the given `time_tk`.
fn marker(id: &str, time_tk: i64) -> Marker {
    serde_json::from_value(json!({
        "id": id,
        "time_tk": time_tk,
        "label": "test-marker",
    }))
    .expect("marker JSON → Marker")
}

/// Empty project with one video clip at `track_position_tk = tk` on
/// the existing "Video 1" track. Keyframes empty.
fn project_with_video_clip_at(tk: i64) -> Project {
    let mut p = empty_project();
    p.tracks[0]
        .clips
        .push(video_clip(SAMPLE_CLIP_ID, tk, json!([])));
    p
}

/// Empty project with one audio clip at `track_position_tk = tk` on
/// the existing "Audio 1" track. The clip is shape-compatible with
/// the typed `Clip` (we deliberately don't run `apply()` so the
/// asset-id biconditional / asset-existence invariants don't fire).
fn project_with_audio_clip_at(tk: i64) -> Project {
    let mut p = empty_project();
    p.tracks[1]
        .clips
        .push(video_clip(SAMPLE_CLIP_ID, tk, json!([])));
    p
}

/// Empty project with one on-frame video clip carrying a single
/// keyframe whose `time_tk = tk`.
fn project_with_keyframe_at(tk: i64) -> Project {
    let mut p = empty_project();
    let kfs = json!([{
        "id": SAMPLE_KEYFRAME_ID,
        "property": "opacity",
        "time_tk": tk,
        "value": 0.5,
    }]);
    p.tracks[0].clips.push(video_clip(SAMPLE_CLIP_ID, 0, kfs));
    p
}

/// Empty project with one marker at `time_tk = tk`.
fn project_with_marker_at(tk: i64) -> Project {
    let mut p = empty_project();
    p.markers.push(marker(SAMPLE_MARKER_ID, tk));
    p
}

// ---------------------------------------------------------------------
// Happy-path patches
// ---------------------------------------------------------------------

#[test]
fn compute_patch_minimal_30_to_60_succeeds() {
    let prior = empty_project();
    let (patch, counts, entities) =
        compute_patch(&prior, &args(60, Some(1))).expect("happy-path 30→60 compute_patch");

    let arr = patch.as_array().expect("patch is an array");
    assert_eq!(arr.len(), 2, "fps_den supplied ⇒ two-op patch");
    assert_eq!(arr[0]["op"], "replace");
    assert_eq!(arr[0]["path"], "/fps_num");
    assert_eq!(arr[0]["value"], 60);
    assert_eq!(arr[1]["op"], "replace");
    assert_eq!(arr[1]["path"], "/fps_den");
    assert_eq!(arr[1]["value"], 1);

    assert_eq!(counts.video_image_text_clips, 0);
    assert_eq!(counts.audio_clips, 0);
    assert_eq!(counts.keyframes, 0);
    assert_eq!(counts.markers, 0);
    assert!(entities.is_none(), "no entities block when counts are zero");
}

#[test]
fn compute_patch_fps_den_omitted_emits_single_op_patch() {
    let prior = empty_project();
    let (patch, _counts, _entities) =
        compute_patch(&prior, &args(60, None)).expect("happy-path fps_den-omitted compute_patch");

    let arr = patch.as_array().expect("patch is an array");
    assert_eq!(arr.len(), 1, "fps_den omitted ⇒ single-op patch");
    assert_eq!(arr[0]["op"], "replace");
    assert_eq!(arr[0]["path"], "/fps_num");
    assert_eq!(arr[0]["value"], 60);
}

#[test]
fn boundary_fps_1_1_accepted() {
    let prior = empty_project();
    let (_, counts, _) =
        compute_patch(&prior, &args(FPS_MIN, Some(FPS_MIN))).expect("1/1 is the minimum legal fps");
    assert_eq!(counts.video_image_text_clips, 0);
}

#[test]
fn partial_update_into_ntsc_preserves_den_when_omitted() {
    // Reverse companion of `compute_patch_partial_update_preserves_fps_den`:
    // prior is integer 60/1, args supply BOTH (24, Some(1)) — emits a
    // two-op patch, post-state is 24/1.
    let mut prior = empty_project();
    prior.fps_num = 60;
    prior.fps_den = 1;
    let (patch, _, _) = compute_patch(&prior, &args(24, Some(1))).expect("60→24 compute_patch");
    let arr = patch.as_array().expect("patch is an array");
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["value"], 24);
    assert_eq!(arr[1]["value"], 1);
}

// ---------------------------------------------------------------------
// Error paths
// ---------------------------------------------------------------------

#[test]
fn compute_patch_fps_num_zero_errors() {
    let prior = empty_project();
    match compute_patch(&prior, &args(0, Some(1))).expect_err("fps_num=0 must reject") {
        ProjectSetFpsError::FpsNumOutOfRange { value, min } => {
            assert_eq!(value, 0);
            assert_eq!(min, FPS_MIN);
        }
        other => panic!("expected FpsNumOutOfRange, got {other:?}"),
    }
}

#[test]
fn compute_patch_fps_den_zero_errors() {
    let prior = empty_project();
    match compute_patch(&prior, &args(30, Some(0))).expect_err("fps_den=0 must reject") {
        ProjectSetFpsError::FpsDenOutOfRange { value, min } => {
            assert_eq!(value, 0);
            assert_eq!(min, FPS_MIN);
        }
        other => panic!("expected FpsDenOutOfRange, got {other:?}"),
    }
}

// ---------------------------------------------------------------------
// is_off_frame direct predicate tests
// ---------------------------------------------------------------------

#[test]
fn is_off_frame_30fps_at_8000_is_on_frame() {
    // 30/1 fps: per-frame ticks = 240000 / 30 = 8000.
    // 8000 ticks is exactly one frame boundary.
    assert!(!is_off_frame(Tick::new(8000), 30, 1));
}

#[test]
fn is_off_frame_30fps_at_8001_is_off_frame() {
    // 8001 ticks: (8001 * 30) mod (240000 * 1) = 240030 mod 240000 = 30 ≠ 0.
    assert!(is_off_frame(Tick::new(8001), 30, 1));
}

#[test]
fn is_off_frame_ntsc_30000_1001_at_8008_is_on_frame() {
    // NTSC 30000/1001 fps: per-frame ticks = 240000 * 1001 / 30000 = 8008.
    // (8008 * 30000) mod (240000 * 1001) = 240,240,000 mod 240,240,000 = 0.
    assert!(!is_off_frame(Tick::new(8008), 30000, 1001));
}

// ---------------------------------------------------------------------
// Off-frame counts per entity class
// ---------------------------------------------------------------------

#[test]
fn compute_patch_counts_video_clip_off_frame() {
    let prior = project_with_video_clip_at(8001);
    let (_, counts, _) = compute_patch(&prior, &args(30, Some(1))).expect("compute_patch ok");
    assert_eq!(
        counts.video_image_text_clips, 1,
        "off-frame video clip lands in the visual bucket"
    );
    assert_eq!(counts.audio_clips, 0);
    assert_eq!(counts.keyframes, 0);
    assert_eq!(counts.markers, 0);
}

#[test]
fn compute_patch_counts_audio_clip_off_frame() {
    let prior = project_with_audio_clip_at(8001);
    let (_, counts, _) = compute_patch(&prior, &args(30, Some(1))).expect("compute_patch ok");
    assert_eq!(counts.video_image_text_clips, 0);
    assert_eq!(
        counts.audio_clips, 1,
        "off-frame audio clip lands in the informational bucket"
    );
}

#[test]
fn compute_patch_counts_keyframe_off_frame() {
    // Clip on-frame (track_position_tk=0), keyframe at 8001 ticks.
    let prior = project_with_keyframe_at(8001);
    let (_, counts, _) = compute_patch(&prior, &args(30, Some(1))).expect("compute_patch ok");
    assert_eq!(
        counts.video_image_text_clips, 0,
        "parent clip is on-frame; only the keyframe counts"
    );
    assert_eq!(counts.keyframes, 1);
}

#[test]
fn compute_patch_counts_marker_off_frame() {
    let prior = project_with_marker_at(8001);
    let (_, counts, _) = compute_patch(&prior, &args(30, Some(1))).expect("compute_patch ok");
    assert_eq!(counts.markers, 1);
    assert_eq!(counts.video_image_text_clips, 0);
}

// ---------------------------------------------------------------------
// `list_off_frame_entities` flag matrix
// ---------------------------------------------------------------------

#[test]
fn compute_patch_list_off_frame_entities_true_emits_block() {
    let prior = project_with_video_clip_at(8001);
    let (_, _counts, entities) =
        compute_patch(&prior, &args_listing(30, Some(1), true)).expect("compute_patch ok");
    let e = entities.expect("flag=true + off-frame entity ⇒ block present");
    assert_eq!(
        e.video_image_text_clip_ids,
        vec![SAMPLE_CLIP_ID.to_string()]
    );
    assert!(e.audio_clip_ids.is_empty());
    assert!(e.keyframe_ids.is_empty());
    assert!(e.marker_ids.is_empty());
}

#[test]
fn compute_patch_list_off_frame_entities_false_omits_block() {
    let prior = project_with_video_clip_at(8001);
    let (_, counts, entities) =
        compute_patch(&prior, &args_listing(30, Some(1), false)).expect("compute_patch ok");
    assert_eq!(counts.video_image_text_clips, 1);
    assert!(entities.is_none(), "flag=false ⇒ block omitted");
}

#[test]
fn compute_patch_list_off_frame_entities_true_but_all_zero_omits_block() {
    // All counts zero (empty project) — block omitted even with flag=true.
    let prior = empty_project();
    let (_, counts, entities) =
        compute_patch(&prior, &args_listing(60, Some(1), true)).expect("compute_patch ok");
    assert_eq!(counts.video_image_text_clips, 0);
    assert_eq!(counts.audio_clips, 0);
    assert_eq!(counts.keyframes, 0);
    assert_eq!(counts.markers, 0);
    assert!(
        entities.is_none(),
        "flag=true but all counts zero ⇒ block still omitted per §2.11"
    );
}

// ---------------------------------------------------------------------
// Partial-update NTSC-footgun-guard semantics
// ---------------------------------------------------------------------

#[test]
fn compute_patch_partial_update_preserves_fps_den() {
    // Prior is NTSC 30000/1001; args bump only fps_num.
    // Post-state must be 60/1001 (NOT 60/1 — the NTSC-footgun-guard).
    let mut prior = empty_project();
    prior.fps_num = 30000;
    prior.fps_den = 1001;

    let (patch, _, _) = compute_patch(&prior, &args(60, None)).expect("partial-update ok");
    let arr = patch.as_array().expect("patch is an array");
    assert_eq!(arr.len(), 1, "fps_den omitted ⇒ single-op patch");
    assert_eq!(arr[0]["path"], "/fps_num");
    assert_eq!(arr[0]["value"], 60);
    // /fps_den is NOT in the patch — the kernel's apply() leaves
    // prior.fps_den (1001) intact, yielding the desired 60/1001.

    // And the data envelope reads post_state fps_den (= prior.fps_den
    // since the patch doesn't touch it).
    let mut post_state = prior.clone();
    post_state.fps_num = 60; // /fps_num replace
    let env = data_envelope(&args(60, None), &post_state);
    assert_eq!(env.fps_num, 60);
    assert_eq!(
        env.fps_den, 1001,
        "NTSC den preserved when fps_den arg is omitted"
    );
}

// ---------------------------------------------------------------------
// data_envelope sanity
// ---------------------------------------------------------------------

#[test]
fn data_envelope_returns_post_state_fps_and_counts() {
    let mut post_state = empty_project();
    post_state.fps_num = 60;
    post_state.fps_den = 1;
    let a = args(60, Some(1));
    let env = data_envelope(&a, &post_state);
    assert_eq!(env.project_id, a.project_id);
    assert_eq!(env.fps_num, 60);
    assert_eq!(env.fps_den, 1);
    assert_eq!(env.off_frame_count.video_image_text_clips, 0);
    assert!(env.off_frame_entities.is_none());
}

// ---------------------------------------------------------------------
// Reconstructor round-trip — the §0.8 startup-gate exercise
// ---------------------------------------------------------------------

#[test]
fn reconstructor_round_trip() {
    // Build a prior with off-frame entities across multiple classes so
    // the round-trip exercises a non-trivial walk + id-list output,
    // not just the empty-counts fixture path.
    let mut prior = empty_project();
    prior.tracks[0]
        .clips
        .push(video_clip(SAMPLE_CLIP_ID, 8001, json!([])));
    prior.tracks[1]
        .clips
        .push(video_clip(SAMPLE_CLIP_ID_2, 8001, json!([])));
    prior.markers.push(marker(SAMPLE_MARKER_ID, 8001));

    let a = args_listing(30, Some(1), true);
    let (patch, _, _) = compute_patch(&prior, &a).expect("compute_patch ok");

    let mut post_state = prior.clone();
    post_state.fps_num = a.fps_num;
    post_state.fps_den = a.fps_den.expect("test supplies fps_den");

    let expected_envelope = data_envelope(&a, &post_state);
    let expected_data = serde_json::to_value(&expected_envelope).expect("envelope → Value");

    let recorded = RecordedEvent {
        verb: "project.set_fps".to_owned(),
        args: serde_json::to_value(&a).expect("args → Value"),
        patch,
        warnings: vec![],
        post_state,
        expected_data,
    };

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(ProjectSetFpsVerb))
        .expect("register ok");

    let report = validate_reconstructors(&registry, &[recorded])
        .expect("reconstructor round-trip must pass");
    assert_eq!(report.verbs_checked, vec!["project.set_fps"]);
    assert_eq!(report.fixtures_run, 1);
}

// ---------------------------------------------------------------------
// End-to-end through ProjectStore::mutate_via_verb — native only
// ---------------------------------------------------------------------

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
    .expect("create_with_registry clears the gate and writes project.json");

    let args = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "fps_num": 60,
        "fps_den": 1,
    });

    let outcome = store
        .mutate_via_verb("project.set_fps", args, None)
        .expect("mutate_via_verb happy path");

    let MutateOutcome::Applied { event_id, data } = outcome else {
        panic!("happy path must return Applied, got {outcome:?}");
    };

    assert_eq!(
        store.last_applied_event_id(),
        Some(event_id),
        "store tracks the just-applied event"
    );

    // In-memory project reflects the new fps.
    assert_eq!(store.project().fps_num, 60);
    assert_eq!(store.project().fps_den, 1);

    // Data envelope shape: { project_id, fps_num, fps_den, off_frame_count }.
    let expected = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "fps_num": 60,
        "fps_den": 1,
        "off_frame_count": {
            "video_image_text_clips": 0,
            "audio_clips": 0,
            "keyframes": 0,
            "markers": 0,
        },
    });
    assert_eq!(
        data, expected,
        "data envelope is the verb's typed shape (off_frame_entities omitted when flag false)"
    );
}
