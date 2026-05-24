//! §0.13 invariant tests — currently covers fade-clamp enforcement.
//!
//! Each subsequent invariant slice (track contiguity, no-overlap,
//! duration_tk maintenance, dangling-keyframe cascade, etc.) extends
//! this file with new `check_*` coverage.

use serde_json::json;
use verbreel_state::{
    ApplyError, AssetRef, BlendMode, Clip, FadeCurve, InvariantViolation, Project, Track,
    TrackKind, Transform, check_duration_tk, check_fade_clamp, check_no_overlap,
    check_track_contiguity, timeline_duration_tk,
};
use verbreel_types::{ClipId, Tick, TrackId, UuidV7};

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

// ---------------------------------------------------------------------
// check_no_overlap — synthetic projects
// ---------------------------------------------------------------------

/// Build a minimal Clip for the no-overlap tests. `start` is
/// `track_position_tk`; `dur` is the source duration (with `speed`
/// passed separately so callers can exercise the speed-affected
/// duration codepath).
fn clip_at(start: i64, dur: i64, speed: f64) -> Clip {
    Clip {
        id: ClipId::now(),
        name: "test".to_string(),
        asset_id: AssetRef::nil(),
        track_position_tk: Tick::new(start),
        source_in_tk: Tick::new(0),
        source_out_tk: Tick::new(dur),
        speed,
        reversed: false,
        transform: Transform::default(),
        opacity: 1.0,
        volume: 1.0,
        fade_in_tk: Tick::ZERO,
        fade_out_tk: Tick::ZERO,
        fade_in_curve: FadeCurve::Linear,
        fade_out_curve: FadeCurve::Linear,
        effects: vec![],
        keyframes: vec![],
        text: None,
        locked: false,
        link_group: None,
        blend_mode: BlendMode::Normal,
        mask: None,
        speed_curve: None,
    }
}

/// Build a clip with an explicit id (for tests that need to assert
/// which clip appears as `earlier` / `later` in the error).
fn clip_at_with_id(id: ClipId, start: i64, dur: i64, speed: f64) -> Clip {
    let mut c = clip_at(start, dur, speed);
    c.id = id;
    c
}

/// Build a Project with a single track populated by the given clips.
/// Starts from the empty-fixture snapshot and substitutes the first
/// track's clips outright.
fn project_with_single_track_clips(kind: TrackKind, clips: Vec<Clip>) -> Project {
    let mut p: Project =
        serde_json::from_str(include_str!("fixtures/empty_project_create.json")).unwrap();
    p.tracks = vec![Track {
        id: TrackId::now(),
        kind,
        name: "t0".to_string(),
        clips,
        muted: false,
        solo: false,
        locked: false,
        hidden: false,
        volume: 1.0,
        pan: 0.0,
        effects: vec![],
    }];
    p
}

#[test]
fn no_overlap_empty_track_passes() {
    let p = project_with_single_track_clips(TrackKind::Video, vec![]);
    check_no_overlap(&p).expect("empty track is trivially overlap-free");
}

#[test]
fn no_overlap_single_clip_passes() {
    let p = project_with_single_track_clips(TrackKind::Video, vec![clip_at(0, 100, 1.0)]);
    check_no_overlap(&p).expect("single clip is trivially overlap-free");
}

#[test]
fn no_overlap_two_adjacent_clips_pass() {
    // [0,100) and [100,200) — sharing endpoint is NOT overlap.
    let p = project_with_single_track_clips(
        TrackKind::Video,
        vec![clip_at(0, 100, 1.0), clip_at(100, 100, 1.0)],
    );
    check_no_overlap(&p).expect("adjacent half-open intervals must pass");
}

#[test]
fn no_overlap_two_separated_clips_pass() {
    // [0,100) and [200,300) — clear gap.
    let p = project_with_single_track_clips(
        TrackKind::Video,
        vec![clip_at(0, 100, 1.0), clip_at(200, 100, 1.0)],
    );
    check_no_overlap(&p).expect("separated clips must pass");
}

#[test]
fn no_overlap_overlapping_clips_rejected() {
    // [0,100) and [50,150) — overlap on [50,100).
    let p = project_with_single_track_clips(
        TrackKind::Video,
        vec![clip_at(0, 100, 1.0), clip_at(50, 100, 1.0)],
    );
    let err = check_no_overlap(&p).expect_err("overlap must reject");
    assert!(matches!(err, InvariantViolation::ClipOverlap { .. }));
}

#[test]
fn no_overlap_contained_clip_rejected() {
    // [0,100) and [20,80) — second fully contained in first.
    let p = project_with_single_track_clips(
        TrackKind::Video,
        vec![clip_at(0, 100, 1.0), clip_at(20, 60, 1.0)],
    );
    let err = check_no_overlap(&p).expect_err("contained clip must reject");
    if let InvariantViolation::ClipOverlap {
        earlier_end_tk,
        later_start_tk,
        ..
    } = err
    {
        assert_eq!(earlier_end_tk.get(), 100);
        assert_eq!(later_start_tk.get(), 20);
    } else {
        panic!("expected ClipOverlap, got {err:?}");
    }
}

#[test]
fn no_overlap_identical_clips_rejected() {
    // Two clips both at [0,100). Construct with explicit ids so we
    // can assert which one appears as `earlier`. Stable sort
    // preserves input order on ties — `id_a` was pushed first.
    let id_a: ClipId = "01890000-0000-7000-8000-0000000000a1"
        .parse::<UuidV7>()
        .map(ClipId::from_uuid_v7)
        .unwrap();
    let id_b: ClipId = "01890000-0000-7000-8000-0000000000a2"
        .parse::<UuidV7>()
        .map(ClipId::from_uuid_v7)
        .unwrap();
    let p = project_with_single_track_clips(
        TrackKind::Video,
        vec![
            clip_at_with_id(id_a, 0, 100, 1.0),
            clip_at_with_id(id_b, 0, 100, 1.0),
        ],
    );
    let err = check_no_overlap(&p).expect_err("identical intervals must reject");
    if let InvariantViolation::ClipOverlap {
        earlier_clip_id,
        later_clip_id,
        ..
    } = err
    {
        assert_eq!(
            earlier_clip_id, id_a,
            "stable sort: id_a (input order) is earlier"
        );
        assert_eq!(later_clip_id, id_b);
    } else {
        panic!("expected ClipOverlap, got {err:?}");
    }
}

#[test]
fn no_overlap_unsorted_input_passes_if_actually_non_overlapping() {
    // Array order [200,300) then [100,200) — the algorithm sorts
    // by position before pairwise scan, so this is contiguous and
    // must pass.
    let p = project_with_single_track_clips(
        TrackKind::Video,
        vec![clip_at(200, 100, 1.0), clip_at(100, 100, 1.0)],
    );
    check_no_overlap(&p).expect("unsorted but non-overlapping must pass");
}

#[test]
fn no_overlap_speed_affected_duration_used_correctly() {
    // speed=2 halves the timeline duration. A clip with source
    // duration 200 at speed 2 has timeline duration 100. So:
    // clip A: [0, 100) (source dur 200, speed 2 → timeline 100)
    // clip B: [100, 300) (source dur 200, speed 1 → timeline 200)
    // No overlap.
    let p = project_with_single_track_clips(
        TrackKind::Video,
        vec![clip_at(0, 200, 2.0), clip_at(100, 200, 1.0)],
    );
    check_no_overlap(&p).expect("speed-affected duration must be used");

    // Now make them overlap: clip A speed=1 (timeline 200), clip B at 100.
    // A: [0, 200), B: [100, 300) → overlap on [100,200).
    let p = project_with_single_track_clips(
        TrackKind::Video,
        vec![clip_at(0, 200, 1.0), clip_at(100, 200, 1.0)],
    );
    check_no_overlap(&p).expect_err("speed=1 changes timeline duration, overlap appears");
}

// ---------------------------------------------------------------------
// check_no_overlap — apply() integration
// ---------------------------------------------------------------------

#[test]
fn apply_rejects_no_overlap_violation() {
    // Start from the keyframes fixture (3 tracks, video clip
    // [0, 2_400_000)). Append a second video clip overlapping at
    // [1_000_000, 2_400_000) by patching it onto the same track.
    let p = load_three_track();
    let new_clip = clip_at(1_000_000, 1_400_000, 1.0);
    let new_clip_json = serde_json::to_value(&new_clip).unwrap();
    let patch: json_patch::Patch = serde_json::from_value(json!([
        {"op":"add","path":"/tracks/0/clips/-","value": new_clip_json},
    ]))
    .unwrap();
    let err = p.apply(&patch).expect_err("overlap must reject");
    assert!(matches!(
        err,
        ApplyError::InvariantViolation(InvariantViolation::ClipOverlap { track_index: 0, .. })
    ));
}

#[test]
fn apply_accepts_adjacent_non_overlapping_patch() {
    // Same setup — append a second video clip starting RIGHT WHERE
    // the first ends (2_400_000). Half-open intervals → adjacent,
    // not overlapping. Also includes the /duration_tk replace op
    // the spec §0.13 maintenance rule requires for duration-
    // extending mutations (post-state max becomes 2_880_000).
    let p = load_three_track();
    let new_clip = clip_at(2_400_000, 480_000, 1.0);
    let new_clip_json = serde_json::to_value(&new_clip).unwrap();
    let patch: json_patch::Patch = serde_json::from_value(json!([
        {"op":"add","path":"/tracks/0/clips/-","value": new_clip_json},
        {"op":"replace","path":"/duration_tk","value": 2_880_000},
    ]))
    .unwrap();
    let out = p.apply(&patch).expect("adjacent clips must succeed");
    assert_eq!(out.tracks[0].clips.len(), 2);
    assert_eq!(out.tracks[0].clips[1].track_position_tk.get(), 2_400_000);
    assert_eq!(out.duration_tk.get(), 2_880_000);
}

#[test]
fn clip_overlap_error_carries_both_clip_ids_and_tk_values() {
    let id_a: ClipId = "01890000-0000-7000-8000-0000000000b1"
        .parse::<UuidV7>()
        .map(ClipId::from_uuid_v7)
        .unwrap();
    let id_b: ClipId = "01890000-0000-7000-8000-0000000000b2"
        .parse::<UuidV7>()
        .map(ClipId::from_uuid_v7)
        .unwrap();
    let p = project_with_single_track_clips(
        TrackKind::Video,
        vec![
            clip_at_with_id(id_a, 0, 100, 1.0),
            clip_at_with_id(id_b, 50, 100, 1.0),
        ],
    );
    let err = p.apply(&json_patch::Patch(vec![])).unwrap_err();
    match err {
        ApplyError::InvariantViolation(InvariantViolation::ClipOverlap {
            track_index,
            earlier_clip_id,
            earlier_end_tk,
            later_clip_id,
            later_start_tk,
        }) => {
            assert_eq!(track_index, 0);
            assert_eq!(earlier_clip_id, id_a);
            assert_eq!(later_clip_id, id_b);
            assert_eq!(earlier_end_tk.get(), 100);
            assert_eq!(later_start_tk.get(), 50);
        }
        other => panic!("expected ClipOverlap, got {other:?}"),
    }
}

#[test]
fn fixtures_satisfy_no_overlap() {
    // Regression canary against the 3 existing fixtures.
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
        check_no_overlap(&p).unwrap_or_else(|e| {
            panic!("fixture {name:?} must satisfy no-overlap: {e}");
        });
    }
}

// ---------------------------------------------------------------------
// check_duration_tk
// ---------------------------------------------------------------------

/// Helper — synthesize a single-track project with `clips` and set
/// `Project.duration_tk` to `expected`. Used to exercise both the
/// happy and unhappy paths of `check_duration_tk` without dragging in
/// fade/contiguity/overlap concerns.
fn project_with_duration(kind: TrackKind, clips: Vec<Clip>, expected: i64) -> Project {
    let mut p = project_with_single_track_clips(kind, clips);
    p.duration_tk = Tick::new(expected);
    p
}

#[test]
fn duration_tk_empty_project_zero_passes() {
    // No clips at all — max = 0; persisted must equal 0.
    let p = project_with_duration(TrackKind::Video, vec![], 0);
    check_duration_tk(&p).expect("empty project with duration_tk=0 passes");
}

#[test]
fn duration_tk_empty_project_nonzero_rejected() {
    // No clips but stale `duration_tk = 1000`.
    let p = project_with_duration(TrackKind::Video, vec![], 1000);
    let err = check_duration_tk(&p).expect_err("nonzero on empty must reject");
    if let InvariantViolation::ProjectDurationStale {
        stored_duration_tk,
        computed_duration_tk,
    } = err
    {
        assert_eq!(stored_duration_tk.get(), 1000);
        assert_eq!(computed_duration_tk.get(), 0);
    } else {
        panic!("expected ProjectDurationStale, got {err:?}");
    }
}

#[test]
fn duration_tk_single_clip_correct_passes() {
    // Single clip [0, 100). max = 100. duration_tk = 100.
    let p = project_with_duration(TrackKind::Video, vec![clip_at(0, 100, 1.0)], 100);
    check_duration_tk(&p).expect("correct single-clip duration_tk passes");

    // Same with a non-zero track_position_tk. clip [200, 300).
    let p = project_with_duration(TrackKind::Video, vec![clip_at(200, 100, 1.0)], 300);
    check_duration_tk(&p).expect("clip with non-zero start passes");
}

#[test]
fn duration_tk_single_clip_stale_rejected() {
    // Clip [0, 100). duration_tk = 50 (stale-under).
    let p = project_with_duration(TrackKind::Video, vec![clip_at(0, 100, 1.0)], 50);
    let err = check_duration_tk(&p).expect_err("stale-under must reject");
    if let InvariantViolation::ProjectDurationStale {
        stored_duration_tk,
        computed_duration_tk,
    } = err
    {
        assert_eq!(stored_duration_tk.get(), 50);
        assert_eq!(computed_duration_tk.get(), 100);
    } else {
        panic!("expected ProjectDurationStale, got {err:?}");
    }
}

#[test]
fn duration_tk_single_clip_overshoot_rejected() {
    // Clip [0, 100). duration_tk = 999 (overshoot — no clip extends
    // that far). Must reject — duration must equal, not bound.
    let p = project_with_duration(TrackKind::Video, vec![clip_at(0, 100, 1.0)], 999);
    let err = check_duration_tk(&p).expect_err("overshoot must reject");
    if let InvariantViolation::ProjectDurationStale {
        stored_duration_tk,
        computed_duration_tk,
    } = err
    {
        assert_eq!(stored_duration_tk.get(), 999);
        assert_eq!(computed_duration_tk.get(), 100);
    } else {
        panic!("expected ProjectDurationStale, got {err:?}");
    }
}

#[test]
fn duration_tk_multi_clip_correct() {
    // Clips [0, 100) and [200, 350). max = 350.
    let p = project_with_duration(
        TrackKind::Video,
        vec![clip_at(0, 100, 1.0), clip_at(200, 150, 1.0)],
        350,
    );
    check_duration_tk(&p).expect("multi-clip max correct passes");
}

#[test]
fn duration_tk_speed_affected_max_uses_speed_adjusted_duration() {
    // Clip source duration = 200, speed = 2 → timeline duration = 100.
    // Track position = 0 → end = 100.
    let p = project_with_duration(TrackKind::Video, vec![clip_at(0, 200, 2.0)], 100);
    check_duration_tk(&p).expect("speed-adjusted timeline duration in max");

    // Same configuration but persisted as 200 (raw source span — bug).
    let p = project_with_duration(TrackKind::Video, vec![clip_at(0, 200, 2.0)], 200);
    let err = check_duration_tk(&p).expect_err("speed not accounted for must reject");
    if let InvariantViolation::ProjectDurationStale {
        computed_duration_tk,
        ..
    } = err
    {
        assert_eq!(computed_duration_tk.get(), 100, "speed-adjusted = 100");
    } else {
        panic!("expected ProjectDurationStale, got {err:?}");
    }
}

#[test]
fn duration_tk_clips_on_different_tracks_max_across_all() {
    // Start from the 3-track keyframes fixture (video clip
    // [0, 2_400_000), text clip [0, 480_000)). max = 2_400_000.
    let mut p = load_three_track();
    p.duration_tk = Tick::new(2_400_000);
    check_duration_tk(&p).expect("max across all tracks");

    // Stale to 480_000 (text-only max) → reject.
    p.duration_tk = Tick::new(480_000);
    let err = check_duration_tk(&p).expect_err("stale to text-only max must reject");
    if let InvariantViolation::ProjectDurationStale {
        stored_duration_tk,
        computed_duration_tk,
    } = err
    {
        assert_eq!(stored_duration_tk.get(), 480_000);
        assert_eq!(computed_duration_tk.get(), 2_400_000);
    } else {
        panic!("expected ProjectDurationStale, got {err:?}");
    }
}

// ---------------------------------------------------------------------
// check_duration_tk — apply() integration
// ---------------------------------------------------------------------

#[test]
fn apply_rejects_duration_tk_stale_after_patch() {
    // Patch that extends a clip's source_out (and thus timeline
    // duration) WITHOUT including the matching /duration_tk replace
    // op must reject. Three-track fixture's video clip has
    // source_out=2_400_000; bump to 3_000_000 without touching
    // duration_tk → post-state has duration_tk=2_400_000 but max
    // = 3_000_000.
    let p = load_three_track();
    let patch: json_patch::Patch = serde_json::from_value(json!([
        {"op":"replace","path":"/tracks/0/clips/0/source_out_tk","value": 3_000_000},
    ]))
    .unwrap();
    let err = p
        .apply(&patch)
        .expect_err("missing /duration_tk update must reject");
    assert!(matches!(
        err,
        ApplyError::InvariantViolation(InvariantViolation::ProjectDurationStale { .. })
    ));
}

#[test]
fn apply_accepts_patch_with_consistent_duration_tk_update() {
    // Same patch as above, but WITH the matching /duration_tk replace
    // op. Spec §0.13: duration-extending mutations must include the
    // op in the same patch.
    let p = load_three_track();
    let patch: json_patch::Patch = serde_json::from_value(json!([
        {"op":"replace","path":"/tracks/0/clips/0/source_out_tk","value": 3_000_000},
        {"op":"replace","path":"/duration_tk","value": 3_000_000},
    ]))
    .unwrap();
    let out = p.apply(&patch).expect("consistent patch must succeed");
    assert_eq!(out.duration_tk.get(), 3_000_000);
    assert_eq!(out.tracks[0].clips[0].source_out_tk.get(), 3_000_000);
}

#[test]
fn project_duration_stale_error_carries_both_values() {
    // The Err must surface both `stored` and `computed` for
    // debuggability. Hand-construct via direct check.
    let p = project_with_duration(TrackKind::Video, vec![clip_at(0, 100, 1.0)], 42);
    let err = check_duration_tk(&p).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("42"), "msg must mention stored value: {msg}");
    assert!(
        msg.contains("100"),
        "msg must mention computed value: {msg}"
    );
    assert!(
        msg.contains("duration_tk"),
        "msg must mention the field: {msg}"
    );
}

#[test]
fn fixtures_satisfy_duration_tk() {
    // Regression canary against ALL 4 fixtures (incl. empty + assets).
    let fixtures: [(&str, &str); 5] = [
        ("empty", include_str!("fixtures/empty_project_create.json")),
        ("assets", include_str!("fixtures/project_with_assets.json")),
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
        check_duration_tk(&p).unwrap_or_else(|e| {
            panic!("fixture {name:?} must satisfy duration_tk: {e}");
        });
    }
}
