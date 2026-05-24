//! §0.13 invariant tests — currently covers fade-clamp enforcement.
//!
//! Each subsequent invariant slice (track contiguity, no-overlap,
//! duration_tk maintenance, dangling-keyframe cascade, etc.) extends
//! this file with new `check_*` coverage.

use serde_json::json;
use verbreel_state::{
    ApplyError, InvariantViolation, Project, Track, TrackKind, check_fade_clamp,
    check_track_contiguity, timeline_duration_tk,
};
use verbreel_types::{Tick, TrackId};

const THREE_TRACK_FIXTURE: &str = include_str!("fixtures/project_with_keyframes.json");

fn load_three_track() -> Project {
    serde_json::from_str(THREE_TRACK_FIXTURE).expect("fixture → Project")
}

// ---------------------------------------------------------------------
// timeline_duration_tk
// ---------------------------------------------------------------------

#[test]
fn timeline_duration_video_clip_with_speed_2() {
    // source_in=0, source_out=2_400_000, speed=2.0 → duration = 1_200_000
    let dur = timeline_duration_tk(Tick::new(0), Tick::new(2_400_000), 2.0);
    assert_eq!(dur.get(), 1_200_000);

    // Non-integer ceil: 100_000 / 3.0 = 33_333.33… → 33_334
    let dur = timeline_duration_tk(Tick::new(0), Tick::new(100_000), 3.0);
    assert_eq!(dur.get(), 33_334);
}

#[test]
fn timeline_duration_image_clip_speed_1() {
    // text/image: speed == 1 invariant → duration = source_out - source_in
    let dur = timeline_duration_tk(Tick::new(0), Tick::new(480_000), 1.0);
    assert_eq!(dur.get(), 480_000);

    // Same with non-zero source_in
    let dur = timeline_duration_tk(Tick::new(100), Tick::new(500), 1.0);
    assert_eq!(dur.get(), 400);
}

#[test]
fn timeline_duration_zero_speed_handled() {
    // Schema bound is >= 0.001, but hand-edited project.json can carry
    // anything. Helper must NOT panic on speed=0 or negative — it
    // saturates to i64::MAX so any downstream fade-clamp check still
    // produces a meaningful result (fade_sum is never > i64::MAX).
    let dur = timeline_duration_tk(Tick::new(0), Tick::new(1_000_000), 0.0);
    assert_eq!(dur.get(), i64::MAX, "zero speed saturates to i64::MAX");

    let dur = timeline_duration_tk(Tick::new(0), Tick::new(1_000_000), -1.0);
    assert_eq!(dur.get(), i64::MAX, "negative speed saturates to i64::MAX");

    // Inverted source bounds (source_out < source_in) clamp to 0 ticks.
    let dur = timeline_duration_tk(Tick::new(1000), Tick::new(500), 1.0);
    assert_eq!(dur.get(), 0, "source_out < source_in clamps to 0");

    // NaN speed saturates as well.
    let dur = timeline_duration_tk(Tick::new(0), Tick::new(1000), f64::NAN);
    assert_eq!(dur.get(), i64::MAX, "NaN speed saturates");
}

// ---------------------------------------------------------------------
// check_fade_clamp — direct Project walks
// ---------------------------------------------------------------------

#[test]
fn fade_clamp_violation_in_initial_project_rejected_at_apply() {
    // Hand-construct a Project that violates the invariant on a clip.
    // Apply any patch (empty) — the post-condition check runs on the
    // RESULT of apply(), which is the same as the input here.
    let mut p = load_three_track();
    // Video clip: timeline_duration = 2_400_000. Push fades to
    // 2_400_001 to violate by 1.
    p.tracks[0].clips[0].fade_in_tk = Tick::new(2_000_000);
    p.tracks[0].clips[0].fade_out_tk = Tick::new(400_001);

    // Direct check fires.
    let err = check_fade_clamp(&p).expect_err("must detect the violation");
    if let InvariantViolation::FadeClamp {
        fade_in_tk,
        fade_out_tk,
        timeline_duration_tk: dur,
        ..
    } = err
    {
        assert_eq!(fade_in_tk.get(), 2_000_000);
        assert_eq!(fade_out_tk.get(), 400_001);
        assert_eq!(dur.get(), 2_400_000);
    } else {
        panic!("expected FadeClamp, got {err:?}");
    }

    // Apply with an empty patch — still rejects (the post-state is
    // unchanged from input, which violates).
    let err = p
        .apply(&json_patch::Patch(vec![]))
        .expect_err("apply must surface the violation even with empty patch");
    assert!(matches!(
        err,
        ApplyError::InvariantViolation(InvariantViolation::FadeClamp { .. })
    ));
}

#[test]
fn fade_clamp_at_boundary_accepted() {
    // Boundary equality: fade_in + fade_out == timeline_duration → OK.
    let mut p = load_three_track();
    // Set fade sum to exactly 2_400_000 on the video clip.
    p.tracks[0].clips[0].fade_in_tk = Tick::new(1_200_000);
    p.tracks[0].clips[0].fade_out_tk = Tick::new(1_200_000);

    check_fade_clamp(&p).expect("equality is OK per spec ≤ rule");
}

// ---------------------------------------------------------------------
// apply() integration
// ---------------------------------------------------------------------

#[test]
fn apply_accepts_fade_at_boundary() {
    // Patch the fade values up to the boundary (sum == duration). Must
    // succeed.
    let p = load_three_track();
    let patch: json_patch::Patch = serde_json::from_value(json!([
        {"op":"replace","path":"/tracks/0/clips/0/fade_in_tk","value":1_200_000},
        {"op":"replace","path":"/tracks/0/clips/0/fade_out_tk","value":1_200_000},
    ]))
    .unwrap();
    let out = p.apply(&patch).expect("equality at boundary must succeed");
    assert_eq!(out.tracks[0].clips[0].fade_in_tk.get(), 1_200_000);
    assert_eq!(out.tracks[0].clips[0].fade_out_tk.get(), 1_200_000);
}

#[test]
fn apply_accepts_fade_well_under_duration() {
    // Happy path — fades well under duration.
    let p = load_three_track();
    let patch: json_patch::Patch = serde_json::from_value(json!([
        {"op":"replace","path":"/tracks/0/clips/0/fade_in_tk","value":48_000},
        {"op":"replace","path":"/tracks/0/clips/0/fade_out_tk","value":48_000},
    ]))
    .unwrap();
    let out = p.apply(&patch).expect("well under duration must succeed");
    assert_eq!(out.tracks[0].clips[0].fade_in_tk.get(), 48_000);
    assert_eq!(out.tracks[0].clips[0].fade_out_tk.get(), 48_000);
}

#[test]
fn invariant_violation_error_carries_clip_id() {
    // Surface the offending clip_id to the caller for debuggability.
    let p = load_three_track();
    let expected_clip_id = p.tracks[0].clips[0].id;

    let patch: json_patch::Patch = serde_json::from_value(json!([
        {"op":"replace","path":"/tracks/0/clips/0/fade_in_tk","value":3_000_000},
    ]))
    .unwrap();
    let err = p.apply(&patch).expect_err("must reject fade > duration");
    match err {
        ApplyError::InvariantViolation(InvariantViolation::FadeClamp { clip_id, .. }) => {
            assert_eq!(
                clip_id, expected_clip_id,
                "error must carry the offending clip_id"
            );
        }
        other => panic!("expected FadeClamp, got {other:?}"),
    }
}

#[test]
fn invariant_violation_error_message_is_informative() {
    // The Display impl must mention the three relevant numbers + the
    // clip id so logs are debuggable without re-walking the tree.
    let p = load_three_track();
    let patch: json_patch::Patch = serde_json::from_value(json!([
        {"op":"replace","path":"/tracks/0/clips/0/fade_in_tk","value":3_000_000},
    ]))
    .unwrap();
    let err = p.apply(&patch).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("fade-clamp"),
        "msg must mention fade-clamp: {msg}"
    );
    assert!(msg.contains("2400000"), "msg must mention duration: {msg}");
    assert!(msg.contains("3000000"), "msg must mention fade_in: {msg}");
}

#[test]
fn fixtures_satisfy_fade_clamp() {
    // Sanity: the existing fixtures (project_with_clips.json,
    // project_with_effects.json, project_with_keyframes.json) all
    // satisfy the fade-clamp invariant. If a future fixture edit
    // accidentally violates it, the per-fixture round-trip tests in
    // the other test files will start failing — this is the early-
    // warning canary.
    let fixtures: [(&str, &str); 3] = [
        ("clips", include_str!("fixtures/project_with_clips.json")),
        (
            "effects",
            include_str!("fixtures/project_with_effects.json"),
        ),
        (
            "keyframes",
            include_str!("fixtures/project_with_keyframes.json"),
        ),
    ];
    for (name, src) in fixtures {
        let p: Project =
            serde_json::from_str(src).unwrap_or_else(|e| panic!("fixture {name:?} parses: {e}"));
        check_fade_clamp(&p).unwrap_or_else(|e| {
            panic!("fixture {name:?} must satisfy fade-clamp: {e}");
        });
    }
}

// ---------------------------------------------------------------------
// check_track_contiguity — direct walks
// ---------------------------------------------------------------------

/// Build a minimal Track of the given kind. Empty clips, default
/// numerics — keeps the contiguity tests focused on track ordering
/// without dragging in fade-clamp / clip-shape concerns.
fn track_of(kind: TrackKind, name: &str) -> Track {
    Track {
        id: TrackId::now(),
        kind,
        name: name.to_string(),
        clips: vec![],
        muted: false,
        solo: false,
        locked: false,
        hidden: false,
        volume: 1.0,
        pan: 0.0,
        effects: vec![],
    }
}

/// Build a Project with the given track-kind sequence. Starts from
/// the empty-fixture snapshot (1 video + 1 audio track from
/// `project.create` defaults) but overrides `tracks` outright so
/// the test owns the ordering completely.
fn project_with_track_kinds(kinds: &[TrackKind]) -> Project {
    let mut p: Project =
        serde_json::from_str(include_str!("fixtures/empty_project_create.json")).unwrap();
    p.tracks = kinds
        .iter()
        .enumerate()
        .map(|(i, &k)| track_of(k, &format!("t{i}")))
        .collect();
    p
}

#[test]
fn track_contiguity_single_kind_passes() {
    let p = project_with_track_kinds(&[TrackKind::Video]);
    check_track_contiguity(&p).expect("single track is trivially contiguous");

    let p = project_with_track_kinds(&[TrackKind::Video, TrackKind::Video, TrackKind::Video]);
    check_track_contiguity(&p).expect("all-same-kind is contiguous");
}

#[test]
fn track_contiguity_two_kinds_blocked_passes() {
    let p = project_with_track_kinds(&[
        TrackKind::Video,
        TrackKind::Video,
        TrackKind::Audio,
        TrackKind::Audio,
    ]);
    check_track_contiguity(&p).expect("[V,V,A,A] is contiguous");
}

#[test]
fn track_contiguity_all_four_kinds_in_canonical_order_passes() {
    let p = project_with_track_kinds(&[
        TrackKind::Video,
        TrackKind::Audio,
        TrackKind::Text,
        TrackKind::Effect,
    ]);
    check_track_contiguity(&p).expect("canonical [V,A,T,E] is contiguous");
}

#[test]
fn track_contiguity_non_canonical_block_order_passes() {
    // The invariant is contiguity, NOT canonical block order.
    // [A,A,V,V] is valid even though `project.open` reconciliation
    // would stable-sort it to [V,V,A,A].
    let p = project_with_track_kinds(&[
        TrackKind::Audio,
        TrackKind::Audio,
        TrackKind::Video,
        TrackKind::Video,
    ]);
    check_track_contiguity(&p)
        .expect("[A,A,V,V] is contiguous — block order is reconciliation-layer territory");

    // [E,T,A,V] — also contiguous.
    let p = project_with_track_kinds(&[
        TrackKind::Effect,
        TrackKind::Text,
        TrackKind::Audio,
        TrackKind::Video,
    ]);
    check_track_contiguity(&p).expect("[E,T,A,V] is contiguous (reverse canonical)");
}

#[test]
fn track_contiguity_interleaved_rejected_video_audio_video() {
    let p = project_with_track_kinds(&[TrackKind::Video, TrackKind::Audio, TrackKind::Video]);
    let err = check_track_contiguity(&p).expect_err("[V,A,V] interleaved must reject");
    match err {
        InvariantViolation::InterleavedTracks {
            first_violation_index,
            actual_kind,
            prior_kind_block,
            expected_kind_block,
        } => {
            assert_eq!(
                first_violation_index, 2,
                "violation at index 2 (third track)"
            );
            assert_eq!(actual_kind, TrackKind::Video);
            assert_eq!(prior_kind_block, TrackKind::Video);
            assert_eq!(expected_kind_block, TrackKind::Audio);
        }
        other => panic!("expected InterleavedTracks, got {other:?}"),
    }
}

#[test]
fn track_contiguity_interleaved_rejected_audio_text_audio() {
    let p = project_with_track_kinds(&[TrackKind::Audio, TrackKind::Text, TrackKind::Audio]);
    let err = check_track_contiguity(&p).expect_err("[A,T,A] interleaved must reject");
    if let InvariantViolation::InterleavedTracks {
        first_violation_index,
        actual_kind,
        ..
    } = err
    {
        assert_eq!(first_violation_index, 2);
        assert_eq!(actual_kind, TrackKind::Audio);
    } else {
        panic!("expected InterleavedTracks, got {err:?}");
    }
}

// ---------------------------------------------------------------------
// check_track_contiguity — apply() integration
// ---------------------------------------------------------------------

#[test]
fn apply_rejects_track_contiguity_violation() {
    // Start from the 2-track empty project (V,A). Insert a video
    // track at the end → [V, A, V] → must reject.
    let p = serde_json::from_str::<Project>(include_str!("fixtures/empty_project_create.json"))
        .unwrap();
    let new_track = track_of(TrackKind::Video, "interleaver");
    let new_track_json = serde_json::to_value(&new_track).unwrap();
    let patch: json_patch::Patch = serde_json::from_value(json!([
        {"op":"add","path":"/tracks/-","value": new_track_json},
    ]))
    .unwrap();
    let err = p
        .apply(&patch)
        .expect_err("[V,A,V] from append must reject");
    assert!(matches!(
        err,
        ApplyError::InvariantViolation(InvariantViolation::InterleavedTracks { .. })
    ));
}

#[test]
fn apply_accepts_appending_to_existing_kind_block() {
    // Start from [V, A]; append another audio track → [V, A, A] is
    // contiguous, must accept.
    let p = serde_json::from_str::<Project>(include_str!("fixtures/empty_project_create.json"))
        .unwrap();
    let new_track = track_of(TrackKind::Audio, "extra-audio");
    let new_track_json = serde_json::to_value(&new_track).unwrap();
    let patch: json_patch::Patch = serde_json::from_value(json!([
        {"op":"add","path":"/tracks/-","value": new_track_json},
    ]))
    .unwrap();
    let out = p
        .apply(&patch)
        .expect("appending to the in-progress audio block must succeed");
    assert_eq!(out.tracks.len(), 3);
    assert_eq!(out.tracks[2].kind, TrackKind::Audio);
}

#[test]
fn apply_accepts_starting_new_kind_block() {
    // Start from [V, A]; append a text track → [V, A, T] is
    // contiguous (new kind block begins), must accept.
    let p = serde_json::from_str::<Project>(include_str!("fixtures/empty_project_create.json"))
        .unwrap();
    let new_track = track_of(TrackKind::Text, "text-track");
    let new_track_json = serde_json::to_value(&new_track).unwrap();
    let patch: json_patch::Patch = serde_json::from_value(json!([
        {"op":"add","path":"/tracks/-","value": new_track_json},
    ]))
    .unwrap();
    let out = p
        .apply(&patch)
        .expect("starting a new kind block at the end must succeed");
    assert_eq!(out.tracks.len(), 3);
    assert_eq!(out.tracks[2].kind, TrackKind::Text);
}

#[test]
fn interleaved_tracks_error_carries_offending_index() {
    let p = project_with_track_kinds(&[
        TrackKind::Video,
        TrackKind::Video,
        TrackKind::Audio,
        TrackKind::Video, // ← index 3 violates
    ]);
    let err = p
        .apply(&json_patch::Patch(vec![]))
        .expect_err("apply must surface the violation via the post-patch check");
    match err {
        ApplyError::InvariantViolation(InvariantViolation::InterleavedTracks {
            first_violation_index,
            actual_kind,
            prior_kind_block,
            expected_kind_block,
        }) => {
            assert_eq!(first_violation_index, 3);
            assert_eq!(actual_kind, TrackKind::Video);
            assert_eq!(prior_kind_block, TrackKind::Video);
            assert_eq!(expected_kind_block, TrackKind::Audio);
        }
        other => panic!("expected InterleavedTracks, got {other:?}"),
    }
}

#[test]
fn fixtures_satisfy_track_contiguity() {
    // All 3 existing fixtures use the canonical [video, audio, text]
    // ordering established in Phase 0 — they should already satisfy
    // this invariant. This canary fails loudly if a future fixture
    // edit interleaves tracks.
    let fixtures: [(&str, &str); 3] = [
        ("clips", include_str!("fixtures/project_with_clips.json")),
        (
            "effects",
            include_str!("fixtures/project_with_effects.json"),
        ),
        (
            "keyframes",
            include_str!("fixtures/project_with_keyframes.json"),
        ),
    ];
    for (name, src) in fixtures {
        let p: Project =
            serde_json::from_str(src).unwrap_or_else(|e| panic!("fixture {name:?} parses: {e}"));
        check_track_contiguity(&p).unwrap_or_else(|e| {
            panic!("fixture {name:?} must satisfy track contiguity: {e}");
        });
    }
}
