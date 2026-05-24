//! §0.13 invariant tests — currently covers fade-clamp enforcement.
//!
//! Each subsequent invariant slice (track contiguity, no-overlap,
//! duration_tk maintenance, dangling-keyframe cascade, etc.) extends
//! this file with new `check_*` coverage.

use serde_json::json;
use verbreel_state::{
    ApplyError, AssetIdState, AssetRef, BlendMode, Clip, Effect, EffectKind, FadeCurve,
    InvariantViolation, Keyframe, KeyframeProperty, Project, SourceInTkKind, SpeedCurvePoint,
    Track, TrackKind, Transform, check_asset_existence, check_asset_id_biconditional,
    check_dangling_keyframes, check_duration_tk, check_fade_clamp, check_no_overlap,
    check_source_in_tk, check_speed_curve_on_image_text, check_speed_on_image_text,
    check_track_contiguity, extract_effect_id_from_property, timeline_duration_tk,
};
use verbreel_types::{AssetId, ClipId, EffectId, KeyframeId, Tick, TrackId, UuidV7};

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
    //
    // Per the asset_id ↔ Track.kind biconditional, the appended
    // video-track clip MUST reference a real asset (not nil) — reuse
    // the keyframes fixture's video asset id.
    let p = load_three_track();
    let real_video_asset_id = *p.assets[0].id();
    let mut new_clip = clip_at(2_400_000, 480_000, 1.0);
    new_clip.asset_id = AssetRef::from_id(real_video_asset_id);
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

// ---------------------------------------------------------------------
// check_dangling_keyframes
// ---------------------------------------------------------------------

fn make_effect(id_suffix: &str, kind: &str) -> Effect {
    let raw = format!("01890000-0000-7000-8000-{id_suffix}");
    let id: EffectId = raw.parse::<UuidV7>().map(EffectId::from_uuid_v7).unwrap();
    Effect::new(id, EffectKind::new(kind.to_string()).unwrap())
}

fn make_keyframe(id_suffix: &str, property: &str) -> Keyframe {
    let raw = format!("01890000-0000-7000-8000-{id_suffix}");
    let id: KeyframeId = raw.parse::<UuidV7>().map(KeyframeId::from_uuid_v7).unwrap();
    Keyframe::new(
        id,
        KeyframeProperty::new(property.to_string()).unwrap(),
        Tick::new(0),
        json!(0),
    )
}

/// Build a clip-with-effects-and-keyframes for the dangling tests.
/// Set the effect/keyframe collections atop a vanilla `clip_at`.
fn clip_with_kf_effects(
    start: i64,
    dur: i64,
    effects: Vec<Effect>,
    keyframes: Vec<Keyframe>,
) -> Clip {
    let mut c = clip_at(start, dur, 1.0);
    c.effects = effects;
    c.keyframes = keyframes;
    c
}

#[test]
fn dangling_kf_extract_uuid_from_property_test() {
    // Effect-targeting properties: extract the uuid.
    let uuid = "01890000-0000-7000-8000-0000000000a1";
    let prop = format!("effects[{uuid}].params.radius_px");
    assert_eq!(extract_effect_id_from_property(&prop), Some(uuid));

    let prop = format!("effects[{uuid}].params.foo.bar");
    assert_eq!(extract_effect_id_from_property(&prop), Some(uuid));

    // Non-effect-targeting properties → None.
    assert_eq!(extract_effect_id_from_property("transform.x"), None);
    assert_eq!(
        extract_effect_id_from_property("transform.rotation_deg"),
        None
    );
    assert_eq!(extract_effect_id_from_property("opacity"), None);
    assert_eq!(extract_effect_id_from_property("volume"), None);
    assert_eq!(extract_effect_id_from_property("mask.feather_px"), None);
    assert_eq!(extract_effect_id_from_property("mask.params.cx"), None);
}

#[test]
fn dangling_kf_no_keyframes_passes() {
    // Clip with no keyframes — trivially overlap-free.
    let p = project_with_single_track_clips(TrackKind::Video, vec![clip_at(0, 100, 1.0)]);
    let mut p = p;
    p.duration_tk = Tick::new(100);
    check_dangling_keyframes(&p).expect("no keyframes → no dangling possible");
}

#[test]
fn dangling_kf_transform_property_passes() {
    let mut p = project_with_single_track_clips(
        TrackKind::Video,
        vec![clip_with_kf_effects(
            0,
            100,
            vec![],
            vec![make_keyframe("0000000000c1", "transform.x")],
        )],
    );
    p.duration_tk = Tick::new(100);
    check_dangling_keyframes(&p).expect("transform-targeting keyframe skips the effect check");
}

#[test]
fn dangling_kf_opacity_passes() {
    let mut p = project_with_single_track_clips(
        TrackKind::Video,
        vec![clip_with_kf_effects(
            0,
            100,
            vec![],
            vec![make_keyframe("0000000000c2", "opacity")],
        )],
    );
    p.duration_tk = Tick::new(100);
    check_dangling_keyframes(&p).expect("opacity keyframe is Clip-direct");
}

#[test]
fn dangling_kf_volume_passes() {
    let mut p = project_with_single_track_clips(
        TrackKind::Video,
        vec![clip_with_kf_effects(
            0,
            100,
            vec![],
            vec![make_keyframe("0000000000c3", "volume")],
        )],
    );
    p.duration_tk = Tick::new(100);
    check_dangling_keyframes(&p).expect("volume keyframe is Clip-direct");
}

#[test]
fn dangling_kf_mask_passes() {
    let mut p = project_with_single_track_clips(
        TrackKind::Video,
        vec![clip_with_kf_effects(
            0,
            100,
            vec![],
            vec![
                make_keyframe("0000000000c4", "mask.feather_px"),
                make_keyframe("0000000000c5", "mask.params.cx"),
            ],
        )],
    );
    p.duration_tk = Tick::new(100);
    check_dangling_keyframes(&p).expect("mask keyframes are Clip-direct");
}

#[test]
fn dangling_kf_valid_effect_ref_passes() {
    // Clip carries a blur effect with id <X>; keyframe property
    // refers to `effects[X].params.radius_px`. Must pass.
    let eff = make_effect("0000000000d0", "blur");
    let eff_id_str = eff.id.to_string();
    let prop = format!("effects[{eff_id_str}].params.radius_px");
    let kf = make_keyframe("0000000000d1", &prop);

    let mut p = project_with_single_track_clips(
        TrackKind::Video,
        vec![clip_with_kf_effects(0, 100, vec![eff], vec![kf])],
    );
    p.duration_tk = Tick::new(100);
    check_dangling_keyframes(&p).expect("valid effect-id ref must pass");
}

#[test]
fn dangling_kf_invalid_effect_ref_rejected() {
    // Clip carries blur effect <X>; keyframe targets `effects[<Y>]…`
    // where Y is a different valid v7. Must reject.
    let eff = make_effect("0000000000e0", "blur");
    let other_uuid = "01890000-0000-7000-8000-0000000000ff";
    let prop = format!("effects[{other_uuid}].params.x");
    let kf = make_keyframe("0000000000e1", &prop);

    let mut p = project_with_single_track_clips(
        TrackKind::Video,
        vec![clip_with_kf_effects(0, 100, vec![eff], vec![kf])],
    );
    p.duration_tk = Tick::new(100);
    let err = check_dangling_keyframes(&p).expect_err("dangling ref must reject");
    if let InvariantViolation::DanglingKeyframe {
        referenced_effect_id,
        property,
        ..
    } = err
    {
        assert_eq!(referenced_effect_id, other_uuid);
        assert!(property.contains("effects[01890000-0000-7000-8000-0000000000ff]"));
    } else {
        panic!("expected DanglingKeyframe, got {err:?}");
    }
}

#[test]
fn dangling_kf_no_effects_but_effect_ref_rejected() {
    // Clip carries NO effects; keyframe still points at one.
    let ref_uuid = "01890000-0000-7000-8000-0000000000f0";
    let prop = format!("effects[{ref_uuid}].params.x");
    let kf = make_keyframe("0000000000f1", &prop);

    let mut p = project_with_single_track_clips(
        TrackKind::Video,
        vec![clip_with_kf_effects(0, 100, vec![], vec![kf])],
    );
    p.duration_tk = Tick::new(100);
    let err = check_dangling_keyframes(&p).expect_err("ref to no-effects clip must reject");
    assert!(matches!(err, InvariantViolation::DanglingKeyframe { .. }));
}

#[test]
fn apply_rejects_dangling_keyframe_violation() {
    // Start from the three-track fixture (video clip with blur
    // effect + a keyframe targeting that blur). Patch the keyframe
    // property to target a different effect-id that doesn't exist.
    let p = load_three_track();
    let bogus_uuid = "01890000-0000-7000-8000-0000000000be";
    let new_prop = format!("effects[{bogus_uuid}].params.radius_px");
    // The clip has 4 keyframes, the 4th (index 3) targets the blur
    // effect — flip it to the bogus uuid.
    let patch: json_patch::Patch = serde_json::from_value(json!([
        {"op":"replace","path":"/tracks/0/clips/0/keyframes/3/property","value": new_prop},
    ]))
    .unwrap();
    let err = p
        .apply(&patch)
        .expect_err("dangling effect ref must reject");
    assert!(matches!(
        err,
        ApplyError::InvariantViolation(InvariantViolation::DanglingKeyframe { .. })
    ));
}

#[test]
fn apply_accepts_patch_with_consistent_effect_keyframe_pair() {
    // Add an effect AND a keyframe targeting that effect, in the
    // same patch. The Project must accept (effect exists on the
    // parent clip when the post-state is evaluated).
    let p = load_three_track();
    let new_eff_id = "01890000-0000-7000-8000-0000000000ab";
    let new_eff = json!({
        "id": new_eff_id,
        "kind": "blur",
        "enabled": true,
        "params": {"radius_px": 4}
    });
    let new_kf_property = format!("effects[{new_eff_id}].params.radius_px");
    let new_kf = json!({
        "id": "01890000-0000-7000-8000-0000000000ac",
        "property": new_kf_property,
        "time_tk": 1,
        "value": 8.0,
        "easing": "linear",
    });
    let patch: json_patch::Patch = serde_json::from_value(json!([
        {"op":"add","path":"/tracks/0/clips/0/effects/-","value": new_eff},
        {"op":"add","path":"/tracks/0/clips/0/keyframes/-","value": new_kf},
    ]))
    .unwrap();
    let out = p
        .apply(&patch)
        .expect("consistent effect+keyframe pair must succeed");
    assert_eq!(out.tracks[0].clips[0].effects.len(), 3);
    assert_eq!(out.tracks[0].clips[0].keyframes.len(), 5);
}

#[test]
fn dangling_keyframe_error_carries_clip_keyframe_effect_ids() {
    let eff = make_effect("0000000000a0", "blur");
    let kf_id_suffix = "0000000000a2";
    let bogus_uuid = "01890000-0000-7000-8000-0000000000a3";
    let prop = format!("effects[{bogus_uuid}].params.radius_px");
    let kf = make_keyframe(kf_id_suffix, &prop);
    let kf_id = kf.id;

    let mut p = project_with_single_track_clips(
        TrackKind::Video,
        vec![clip_with_kf_effects(0, 100, vec![eff], vec![kf])],
    );
    p.duration_tk = Tick::new(100);
    let expected_clip_id = p.tracks[0].clips[0].id;

    let err = p.apply(&json_patch::Patch(vec![])).unwrap_err();
    match err {
        ApplyError::InvariantViolation(InvariantViolation::DanglingKeyframe {
            clip_id,
            keyframe_id,
            referenced_effect_id,
            property,
        }) => {
            assert_eq!(clip_id, expected_clip_id);
            assert_eq!(keyframe_id, kf_id);
            assert_eq!(referenced_effect_id, bogus_uuid);
            assert!(property.contains(bogus_uuid));
        }
        other => panic!("expected DanglingKeyframe, got {other:?}"),
    }
}

#[test]
fn fixtures_satisfy_dangling_keyframes() {
    // Regression canary. project_with_keyframes.json has both a
    // blur effect (id 01890000-…-000000000010) and a keyframe
    // targeting `effects[01890000-…-000000000010].params.radius_px`
    // — they pair correctly per the keyframe slice (PR #28).
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
        check_dangling_keyframes(&p).unwrap_or_else(|e| {
            panic!("fixture {name:?} must satisfy dangling-keyframes: {e}");
        });
    }
}

// ---------------------------------------------------------------------
// check_source_in_tk
// ---------------------------------------------------------------------

/// Build a minimal project with one image-asset and one clip on a
/// video track referencing that image asset. Tests for image-clip
/// behavior mutate the clip's `source_in_tk` from this baseline.
fn project_with_image_clip() -> Project {
    let raw = serde_json::json!({
        "id": "0190b8d3-15e3-7000-bd00-000000000001",
        "schema_version": "1.0.0",
        "tick_rate_hz": 240000,
        "name": "img-test",
        "created_at": "2026-05-24T00:00:00Z",
        "updated_at": "2026-05-24T00:00:00Z",
        "canvas": {
            "width": 1080, "height": 1920,
            "background": "#000000ff",
            "pixel_aspect_num": 1, "pixel_aspect_den": 1
        },
        "fps_num": 30, "fps_den": 1, "duration_tk": 480000,
        "tracks": [
            {
                "id": "0190b8d3-15e3-7000-bd00-000000000002",
                "kind": "video",
                "name": "Video 1",
                "clips": [
                    {
                        "id": "0190b8d3-15e3-7000-bd00-000000000c01",
                        "name": "img clip",
                        "asset_id": "0190b8d3-15e3-7000-bd00-0000000000b1",
                        "track_position_tk": 0,
                        "source_in_tk": 0,
                        "source_out_tk": 480000
                    }
                ],
                "muted": false, "solo": false, "locked": false, "hidden": false,
                "volume": 1.0, "pan": 0.0, "effects": []
            }
        ],
        "assets": [
            {
                "id": "0190b8d3-15e3-7000-bd00-0000000000b1",
                "kind": "image",
                "hash": "53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658",
                "path": "assets/53/53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.png",
                "original_filename": "img.png",
                "imported_at": "2026-05-24T00:00:00Z",
                "metadata": {
                    "width": 1920, "height": 1080,
                    "container": "png",
                    "fingerprint": {
                        "mtime_ms": 1_700_000_000_000_i64,
                        "size_bytes": 1_048_576
                    }
                }
            }
        ],
        "markers": [], "metadata": {}, "last_saved_event_id": null, "trackers": []
    });
    serde_json::from_value(raw).expect("image-clip project deserializes")
}

#[test]
fn source_in_text_clip_zero_passes() {
    // Three-track keyframes fixture: text clip on the text track with
    // source_in_tk = 0.
    let p = load_three_track();
    check_source_in_tk(&p).expect("text clip with source_in_tk=0 passes");
}

#[test]
fn source_in_text_clip_nonzero_rejected() {
    let mut p = load_three_track();
    p.tracks[2].clips[0].source_in_tk = Tick::new(1234);
    let err = check_source_in_tk(&p).expect_err("text clip non-zero must reject");
    if let InvariantViolation::InvalidSourceInTk {
        clip_kind_indicator,
        source_in_tk,
        ..
    } = err
    {
        assert_eq!(clip_kind_indicator, SourceInTkKind::Text);
        assert_eq!(source_in_tk.get(), 1234);
    } else {
        panic!("expected InvalidSourceInTk, got {err:?}");
    }
}

#[test]
fn source_in_image_clip_zero_passes() {
    let p = project_with_image_clip();
    check_source_in_tk(&p).expect("image clip with source_in_tk=0 passes");
}

#[test]
fn source_in_image_clip_nonzero_rejected() {
    let mut p = project_with_image_clip();
    p.tracks[0].clips[0].source_in_tk = Tick::new(5000);
    let err = check_source_in_tk(&p).expect_err("image clip non-zero must reject");
    if let InvariantViolation::InvalidSourceInTk {
        clip_kind_indicator,
        source_in_tk,
        ..
    } = err
    {
        assert_eq!(clip_kind_indicator, SourceInTkKind::Image);
        assert_eq!(source_in_tk.get(), 5000);
    } else {
        panic!("expected InvalidSourceInTk, got {err:?}");
    }
}

#[test]
fn source_in_video_clip_any_value_passes() {
    // The keyframes fixture's video clip references a video asset
    // (kind="video" in assets[]). Bumping source_in_tk to a non-zero
    // value must pass — only image/text are constrained.
    let mut p = load_three_track();
    p.tracks[0].clips[0].source_in_tk = Tick::new(12000);
    // We need to update source_out_tk and duration_tk consistently
    // to avoid tripping the duration_tk + no-fade-clamp invariants
    // first when we run apply() — but for direct check we only test
    // check_source_in_tk, which doesn't care about those.
    check_source_in_tk(&p).expect("video clip source_in_tk=12000 passes");
}

#[test]
fn source_in_audio_clip_any_value_passes() {
    // Need a project with an audio clip. Build via direct mutation
    // of the keyframes fixture: switch the video asset to audio,
    // bump the clip's source_in_tk. Since we're testing check_source_in_tk
    // directly, no need to keep the rest of the project consistent.
    let mut p = load_three_track();
    // Replace the video asset's discriminator with audio. We do this
    // via serde to keep the helper simple — re-serialize, edit, re-parse.
    let mut v = serde_json::to_value(&p).unwrap();
    v["assets"][0]["kind"] = serde_json::json!("audio");
    // Audio asset needs different metadata; replace with audio shape.
    v["assets"][0]["metadata"] = serde_json::json!({
        "duration_tk": 2_400_000,
        "audio_codec": "aac",
        "audio_channels": 2,
        "audio_sample_rate_hz": 48_000,
        "container": "mp4",
        "fingerprint": {
            "mtime_ms": 1_700_000_000_000_i64,
            "size_bytes": 1_048_576
        }
    });
    p = serde_json::from_value(v).expect("audio-asset reshape parses");
    p.tracks[0].clips[0].source_in_tk = Tick::new(8000);
    check_source_in_tk(&p).expect("audio clip source_in_tk=8000 passes");
}

#[test]
fn source_in_clip_with_unresolvable_asset_skipped() {
    // Clip references an asset_id that's not in project.assets[].
    // resolve_asset_kind returns None → check skips.
    let mut p = project_with_image_clip();
    // Clear the assets array — clip still references the (now gone)
    // image asset.
    p.assets.clear();
    // Set source_in to non-zero. If the check did NOT skip
    // unresolvable refs, this would reject.
    p.tracks[0].clips[0].source_in_tk = Tick::new(999);
    check_source_in_tk(&p).expect("unresolvable asset_id skips source_in_tk check");
}

#[test]
fn apply_rejects_text_clip_nonzero_source_in() {
    // Patch the text clip's source_in_tk to non-zero. apply() must
    // reject — and also include the matching /duration_tk update
    // because the text clip's source_out_tk - source_in_tk change
    // would otherwise trip the duration_tk invariant first. Since
    // we want to specifically trip source_in_tk, keep source_out
    // unchanged so the text clip's timeline duration shrinks
    // (480000 → 477000); also bump Project.duration_tk to the new
    // video-clip max (still 2400000, unchanged because video is the
    // taller track).
    let p = load_three_track();
    let patch: json_patch::Patch = serde_json::from_value(json!([
        {"op":"replace","path":"/tracks/2/clips/0/source_in_tk","value": 3000},
    ]))
    .unwrap();
    let err = p.apply(&patch).expect_err("text clip non-zero must reject");
    match err {
        ApplyError::InvariantViolation(InvariantViolation::InvalidSourceInTk {
            clip_kind_indicator,
            ..
        }) => assert_eq!(clip_kind_indicator, SourceInTkKind::Text),
        other => panic!("expected InvalidSourceInTk(Text), got {other:?}"),
    }
}

#[test]
fn apply_rejects_image_clip_nonzero_source_in() {
    // Image clip's source span shrinks by `source_in_tk`, which the
    // duration_tk invariant (earlier in chain) would catch first.
    // Update /duration_tk in the same patch so we get to the
    // source_in_tk check.
    let p = project_with_image_clip();
    let patch: json_patch::Patch = serde_json::from_value(json!([
        {"op":"replace","path":"/tracks/0/clips/0/source_in_tk","value": 7777},
        {"op":"replace","path":"/duration_tk","value": 472_223},
    ]))
    .unwrap();
    let err = p
        .apply(&patch)
        .expect_err("image clip non-zero must reject");
    match err {
        ApplyError::InvariantViolation(InvariantViolation::InvalidSourceInTk {
            clip_kind_indicator,
            ..
        }) => assert_eq!(clip_kind_indicator, SourceInTkKind::Image),
        other => panic!("expected InvalidSourceInTk(Image), got {other:?}"),
    }
}

#[test]
fn apply_accepts_text_clip_with_zero_source_in_patch() {
    // Re-set source_in_tk to 0 on an already-zero text clip. No-op
    // but must succeed.
    let p = load_three_track();
    let patch: json_patch::Patch = serde_json::from_value(json!([
        {"op":"replace","path":"/tracks/2/clips/0/source_in_tk","value": 0},
    ]))
    .unwrap();
    let out = p.apply(&patch).expect("zero is always OK on text clip");
    assert_eq!(out.tracks[2].clips[0].source_in_tk.get(), 0);
}

#[test]
fn invalid_source_in_tk_error_carries_clip_id_and_kind() {
    let mut p = load_three_track();
    let expected_clip_id = p.tracks[2].clips[0].id;
    p.tracks[2].clips[0].source_in_tk = Tick::new(42);

    let err = p.apply(&json_patch::Patch(vec![])).unwrap_err();
    match err {
        ApplyError::InvariantViolation(InvariantViolation::InvalidSourceInTk {
            clip_id,
            clip_kind_indicator,
            source_in_tk,
        }) => {
            assert_eq!(clip_id, expected_clip_id);
            assert_eq!(clip_kind_indicator, SourceInTkKind::Text);
            assert_eq!(source_in_tk.get(), 42);
        }
        other => panic!("expected InvalidSourceInTk, got {other:?}"),
    }
}

#[test]
fn fixtures_satisfy_source_in_tk() {
    // Regression canary. All Phase 0 fixtures' text clips have
    // source_in_tk=0 (per the fixture authors). The keyframes,
    // effects, clips fixtures all have a text track with one text
    // clip at source_in_tk=0.
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
        check_source_in_tk(&p).unwrap_or_else(|e| {
            panic!("fixture {name:?} must satisfy source_in_tk: {e}");
        });
    }
    let _ = AssetId::now(); // silence unused-import warning if any
}

// ---------------------------------------------------------------------
// check_speed_on_image_text — direct walks
// ---------------------------------------------------------------------

#[test]
fn speed_text_clip_one_passes() {
    // Three-track keyframes fixture: text clip on text track with
    // speed = 1.0 (fixture default).
    let p = load_three_track();
    check_speed_on_image_text(&p).expect("text clip with speed=1.0 passes");
}

#[test]
fn speed_text_clip_non_one_rejected() {
    let mut p = load_three_track();
    p.tracks[2].clips[0].speed = 1.5;
    let err = check_speed_on_image_text(&p).expect_err("text clip speed != 1.0 must reject");
    if let InvariantViolation::InvalidSpeedOnDisplayClip {
        clip_kind_indicator,
        speed,
        ..
    } = err
    {
        assert_eq!(clip_kind_indicator, SourceInTkKind::Text);
        assert!(
            (speed - 1.5).abs() < f64::EPSILON,
            "speed surfaced: {speed}"
        );
    } else {
        panic!("expected InvalidSpeedOnDisplayClip, got {err:?}");
    }
}

#[test]
fn speed_image_clip_one_passes() {
    let p = project_with_image_clip();
    check_speed_on_image_text(&p).expect("image clip with speed=1.0 passes");
}

#[test]
fn speed_image_clip_non_one_rejected() {
    let mut p = project_with_image_clip();
    p.tracks[0].clips[0].speed = 2.0;
    let err = check_speed_on_image_text(&p).expect_err("image clip speed != 1.0 must reject");
    if let InvariantViolation::InvalidSpeedOnDisplayClip {
        clip_kind_indicator,
        speed,
        ..
    } = err
    {
        assert_eq!(clip_kind_indicator, SourceInTkKind::Image);
        assert!((speed - 2.0).abs() < f64::EPSILON);
    } else {
        panic!("expected InvalidSpeedOnDisplayClip, got {err:?}");
    }
}

#[test]
fn speed_video_clip_any_value_passes() {
    // Video clip on a video track referencing a video asset — speed
    // is meaningful here. Any value passes the display-kind invariant.
    let mut p = load_three_track();
    p.tracks[0].clips[0].speed = 2.5;
    check_speed_on_image_text(&p).expect("video clip speed=2.5 passes — not display kind");
}

#[test]
fn speed_audio_clip_any_value_passes() {
    // Reshape the keyframes fixture's video asset → audio asset
    // (same trick as `source_in_audio_clip_any_value_passes`).
    let mut p = load_three_track();
    let mut v = serde_json::to_value(&p).unwrap();
    v["assets"][0]["kind"] = serde_json::json!("audio");
    v["assets"][0]["metadata"] = serde_json::json!({
        "duration_tk": 2_400_000,
        "audio_codec": "aac",
        "audio_channels": 2,
        "audio_sample_rate_hz": 48_000,
        "container": "mp4",
        "fingerprint": {
            "mtime_ms": 1_700_000_000_000_i64,
            "size_bytes": 1_048_576
        }
    });
    p = serde_json::from_value(v).expect("audio-asset reshape parses");
    p.tracks[0].clips[0].speed = 0.5;
    check_speed_on_image_text(&p).expect("audio clip speed=0.5 passes — not display kind");
}

// ---------------------------------------------------------------------
// check_speed_curve_on_image_text — direct walks
// ---------------------------------------------------------------------

/// Build a minimal 2-point `speed_curve` for tests. Internal validity
/// (bounds, monotonicity) is enforced by a future slice; here we just
/// need `Some(...)`-ness.
fn two_point_curve() -> Vec<SpeedCurvePoint> {
    vec![
        SpeedCurvePoint {
            time_tk: Tick::new(0),
            factor: 1.0,
        },
        SpeedCurvePoint {
            time_tk: Tick::new(1000),
            factor: 1.0,
        },
    ]
}

#[test]
fn speed_curve_text_clip_none_passes() {
    let p = load_three_track();
    // Fixture default has no `speed_curve` field → None.
    assert!(p.tracks[2].clips[0].speed_curve.is_none());
    check_speed_curve_on_image_text(&p).expect("text clip with speed_curve=None passes");
}

#[test]
fn speed_curve_text_clip_some_rejected() {
    let mut p = load_three_track();
    p.tracks[2].clips[0].speed_curve = Some(two_point_curve());
    let err =
        check_speed_curve_on_image_text(&p).expect_err("text clip speed_curve Some must reject");
    if let InvariantViolation::InvalidSpeedCurveOnDisplayClip {
        clip_kind_indicator,
        point_count,
        ..
    } = err
    {
        assert_eq!(clip_kind_indicator, SourceInTkKind::Text);
        assert_eq!(point_count, 2);
    } else {
        panic!("expected InvalidSpeedCurveOnDisplayClip, got {err:?}");
    }
}

#[test]
fn speed_curve_image_clip_none_passes() {
    let p = project_with_image_clip();
    assert!(p.tracks[0].clips[0].speed_curve.is_none());
    check_speed_curve_on_image_text(&p).expect("image clip with speed_curve=None passes");
}

#[test]
fn speed_curve_image_clip_some_rejected() {
    let mut p = project_with_image_clip();
    p.tracks[0].clips[0].speed_curve = Some(two_point_curve());
    let err =
        check_speed_curve_on_image_text(&p).expect_err("image clip speed_curve Some must reject");
    if let InvariantViolation::InvalidSpeedCurveOnDisplayClip {
        clip_kind_indicator,
        point_count,
        ..
    } = err
    {
        assert_eq!(clip_kind_indicator, SourceInTkKind::Image);
        assert_eq!(point_count, 2);
    } else {
        panic!("expected InvalidSpeedCurveOnDisplayClip, got {err:?}");
    }
}

#[test]
fn speed_curve_video_clip_some_passes() {
    // Video clip on a video track referencing a video asset —
    // speed_curve is meaningful here. Setting Some must pass the
    // display-kind invariant.
    let mut p = load_three_track();
    p.tracks[0].clips[0].speed_curve = Some(two_point_curve());
    check_speed_curve_on_image_text(&p)
        .expect("video clip speed_curve Some passes — not display kind");
}

// ---------------------------------------------------------------------
// apply() integration — speed / speed_curve
// ---------------------------------------------------------------------

#[test]
fn apply_rejects_text_clip_non_one_speed() {
    // Patch the text clip's speed to 2.0. apply() must reject with
    // InvalidSpeedOnDisplayClip(Text).
    let p = load_three_track();
    let patch: json_patch::Patch = serde_json::from_value(json!([
        {"op":"replace","path":"/tracks/2/clips/0/speed","value": 2.0},
    ]))
    .unwrap();
    let err = p
        .apply(&patch)
        .expect_err("text clip speed != 1.0 must reject");
    match err {
        ApplyError::InvariantViolation(InvariantViolation::InvalidSpeedOnDisplayClip {
            clip_kind_indicator,
            ..
        }) => assert_eq!(clip_kind_indicator, SourceInTkKind::Text),
        other => panic!("expected InvalidSpeedOnDisplayClip(Text), got {other:?}"),
    }
}

#[test]
fn apply_rejects_text_clip_speed_curve_some() {
    // Patch the text clip's speed_curve to a 2-point curve. apply()
    // must reject with InvalidSpeedCurveOnDisplayClip(Text).
    let p = load_three_track();
    let curve_json = serde_json::to_value(two_point_curve()).unwrap();
    let patch: json_patch::Patch = serde_json::from_value(json!([
        {"op":"add","path":"/tracks/2/clips/0/speed_curve","value": curve_json},
    ]))
    .unwrap();
    let err = p
        .apply(&patch)
        .expect_err("text clip speed_curve Some must reject");
    match err {
        ApplyError::InvariantViolation(InvariantViolation::InvalidSpeedCurveOnDisplayClip {
            clip_kind_indicator,
            point_count,
            ..
        }) => {
            assert_eq!(clip_kind_indicator, SourceInTkKind::Text);
            assert_eq!(point_count, 2);
        }
        other => panic!("expected InvalidSpeedCurveOnDisplayClip(Text), got {other:?}"),
    }
}

#[test]
fn invalid_speed_on_display_clip_error_carries_info() {
    // Verify clip_id + indicator + speed reach the caller intact.
    let mut p = load_three_track();
    let expected_clip_id = p.tracks[2].clips[0].id;
    p.tracks[2].clips[0].speed = 0.75;

    let err = p.apply(&json_patch::Patch(vec![])).unwrap_err();
    match err {
        ApplyError::InvariantViolation(InvariantViolation::InvalidSpeedOnDisplayClip {
            clip_id,
            clip_kind_indicator,
            speed,
        }) => {
            assert_eq!(clip_id, expected_clip_id);
            assert_eq!(clip_kind_indicator, SourceInTkKind::Text);
            assert!((speed - 0.75).abs() < f64::EPSILON);
        }
        other => panic!("expected InvalidSpeedOnDisplayClip, got {other:?}"),
    }

    // Also verify Display impl carries the human-readable info.
    let msg = format!(
        "§0.13 invariant violation: {}",
        InvariantViolation::InvalidSpeedOnDisplayClip {
            clip_id: expected_clip_id,
            clip_kind_indicator: SourceInTkKind::Text,
            speed: 0.75,
        }
    );
    assert!(msg.contains("0.75"), "msg must mention speed: {msg}");
    assert!(msg.contains("text"), "msg must mention kind: {msg}");
    assert!(
        msg.contains("must be 1.0"),
        "msg must mention target: {msg}"
    );
}

#[test]
fn invalid_speed_curve_on_display_clip_error_carries_info() {
    // Verify clip_id + indicator + point_count reach the caller.
    let mut p = project_with_image_clip();
    let expected_clip_id = p.tracks[0].clips[0].id;
    p.tracks[0].clips[0].speed_curve = Some(two_point_curve());

    let err = p.apply(&json_patch::Patch(vec![])).unwrap_err();
    match err {
        ApplyError::InvariantViolation(InvariantViolation::InvalidSpeedCurveOnDisplayClip {
            clip_id,
            clip_kind_indicator,
            point_count,
        }) => {
            assert_eq!(clip_id, expected_clip_id);
            assert_eq!(clip_kind_indicator, SourceInTkKind::Image);
            assert_eq!(point_count, 2);
        }
        other => panic!("expected InvalidSpeedCurveOnDisplayClip, got {other:?}"),
    }

    // Display impl carries the human-readable info.
    let msg = format!(
        "§0.13 invariant violation: {}",
        InvariantViolation::InvalidSpeedCurveOnDisplayClip {
            clip_id: expected_clip_id,
            clip_kind_indicator: SourceInTkKind::Image,
            point_count: 2,
        }
    );
    assert!(
        msg.contains("2 points"),
        "msg must mention point count: {msg}"
    );
    assert!(msg.contains("image"), "msg must mention kind: {msg}");
    assert!(
        msg.contains("must be None"),
        "msg must mention target: {msg}"
    );
}

#[test]
fn fixtures_satisfy_speed_and_speed_curve_invariants() {
    // Regression canary. All Phase 0 fixtures' text/image clips
    // satisfy speed=1.0 and speed_curve=None (the spec defaults).
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
        check_speed_on_image_text(&p).unwrap_or_else(|e| {
            panic!("fixture {name:?} must satisfy speed-on-image-text: {e}");
        });
        check_speed_curve_on_image_text(&p).unwrap_or_else(|e| {
            panic!("fixture {name:?} must satisfy speed_curve-on-image-text: {e}");
        });
    }
}

// ---------------------------------------------------------------------
// check_asset_id_biconditional — direct walks
// ---------------------------------------------------------------------

/// Helper — borrow a known-valid AssetId from the keyframes fixture
/// so tests can construct non-nil `AssetRef`s without minting new
/// UUIDs.
fn fixture_video_asset_id() -> AssetId {
    let p = load_three_track();
    *p.assets[0].id()
}

#[test]
fn biconditional_text_clip_with_nil_passes() {
    // Keyframes fixture text clip has asset_id=nil — the canonical
    // shape per spec.
    let p = load_three_track();
    check_asset_id_biconditional(&p).expect("text clip with nil asset_id passes");
}

#[test]
fn biconditional_text_clip_with_non_nil_rejected() {
    // Mutate the text clip's asset_id to a real UUID. Even though
    // the UUID resolves to a real (video) asset, the biconditional
    // is violated — text clips must have nil.
    let mut p = load_three_track();
    let real_id = fixture_video_asset_id();
    p.tracks[2].clips[0].asset_id = AssetRef::from_id(real_id);
    let err = check_asset_id_biconditional(&p)
        .expect_err("non-nil asset_id on text-track clip must reject");
    if let InvariantViolation::AssetIdBiconditionalViolation {
        track_kind,
        asset_id_state,
        ..
    } = err
    {
        assert_eq!(track_kind, TrackKind::Text);
        assert_eq!(asset_id_state, AssetIdState::NonNil);
    } else {
        panic!("expected AssetIdBiconditionalViolation, got {err:?}");
    }
}

#[test]
fn biconditional_video_clip_with_non_nil_passes() {
    // Video clip on video track with a real asset_id resolving to a
    // video asset — canonical shape.
    let p = load_three_track();
    check_asset_id_biconditional(&p).expect("video clip with non-nil asset_id passes");
}

#[test]
fn biconditional_video_clip_with_nil_rejected() {
    let mut p = load_three_track();
    p.tracks[0].clips[0].asset_id = AssetRef::nil();
    let err =
        check_asset_id_biconditional(&p).expect_err("nil asset_id on video-track clip must reject");
    if let InvariantViolation::AssetIdBiconditionalViolation {
        track_kind,
        asset_id_state,
        ..
    } = err
    {
        assert_eq!(track_kind, TrackKind::Video);
        assert_eq!(asset_id_state, AssetIdState::Nil);
    } else {
        panic!("expected AssetIdBiconditionalViolation, got {err:?}");
    }
}

#[test]
fn biconditional_audio_clip_with_nil_rejected() {
    // Append an audio clip with nil asset_id to the (empty) audio
    // track in the keyframes fixture. Direct walk (not apply) so we
    // don't need to keep duration_tk / track structure consistent.
    let mut p = load_three_track();
    p.tracks[1].clips.push(clip_at(0, 100, 1.0)); // clip_at uses AssetRef::nil()
    let err =
        check_asset_id_biconditional(&p).expect_err("nil asset_id on audio-track clip must reject");
    if let InvariantViolation::AssetIdBiconditionalViolation {
        track_kind,
        asset_id_state,
        ..
    } = err
    {
        assert_eq!(track_kind, TrackKind::Audio);
        assert_eq!(asset_id_state, AssetIdState::Nil);
    } else {
        panic!("expected AssetIdBiconditionalViolation, got {err:?}");
    }
}

#[test]
fn biconditional_effect_track_skipped() {
    // Effect tracks have empty clips by a (separate) future
    // invariant. This slice doesn't enforce that — it just iterates
    // clips; an effect track with no clips is trivially OK for the
    // biconditional check. Synthesize a project with [V, E] where the
    // effect track has no clips and verify it passes.
    let mut p = serde_json::from_str::<Project>(include_str!("fixtures/empty_project_create.json"))
        .unwrap();
    p.tracks = vec![
        track_of(TrackKind::Video, "v0"),
        track_of(TrackKind::Effect, "e0"),
    ];
    check_asset_id_biconditional(&p)
        .expect("effect track with empty clips is trivially biconditional-OK");
}

// ---------------------------------------------------------------------
// check_asset_existence — direct walks
// ---------------------------------------------------------------------

#[test]
fn asset_existence_resolvable_passes() {
    // Keyframes fixture: video clip on video track references the
    // single video asset in project.assets[]. Resolves cleanly.
    let p = load_three_track();
    check_asset_existence(&p).expect("resolvable asset_id passes");
}

#[test]
fn asset_existence_unresolvable_rejected() {
    // Same fixture, but clear project.assets[]. The video clip's
    // asset_id is now dangling. Direct walk — we don't need to also
    // run the biconditional first; both checks are independent at the
    // unit level.
    let mut p = load_three_track();
    let original_clip_id = p.tracks[0].clips[0].id;
    let original_asset_id = *p.tracks[0].clips[0].asset_id.id().unwrap();
    p.assets.clear();
    let err = check_asset_existence(&p).expect_err("dangling asset_id must reject");
    if let InvariantViolation::AssetIdUnresolved {
        clip_id,
        referenced_asset_id,
    } = err
    {
        assert_eq!(clip_id, original_clip_id);
        assert_eq!(referenced_asset_id, original_asset_id);
    } else {
        panic!("expected AssetIdUnresolved, got {err:?}");
    }
}

#[test]
fn asset_existence_skips_nil_asset_ref() {
    // Text clips have nil asset_id. The existence check must skip
    // them (the biconditional handles the kind-mismatch case in a
    // different slice of the chain). Even with an empty assets[]
    // array, a text clip with nil asset_id passes existence.
    let mut p = load_three_track();
    // Strip the video track + its clip so we don't get a separate
    // dangling-asset failure; leave only the text track.
    p.tracks.retain(|t| t.kind == TrackKind::Text);
    p.assets.clear();
    check_asset_existence(&p).expect("nil asset_id on text clip skips existence check");
}

// ---------------------------------------------------------------------
// apply() integration — biconditional + existence
// ---------------------------------------------------------------------

#[test]
fn apply_rejects_biconditional_violation() {
    // Patch the keyframes-fixture text clip's asset_id from nil to
    // a real UUID. apply() must reject with
    // AssetIdBiconditionalViolation(Text, NonNil).
    let p = load_three_track();
    let real_id_string = fixture_video_asset_id().to_string();
    let patch: json_patch::Patch = serde_json::from_value(json!([
        {"op": "replace", "path": "/tracks/2/clips/0/asset_id", "value": real_id_string},
    ]))
    .unwrap();
    let err = p
        .apply(&patch)
        .expect_err("text-track clip with non-nil asset_id must reject");
    match err {
        ApplyError::InvariantViolation(InvariantViolation::AssetIdBiconditionalViolation {
            track_kind,
            asset_id_state,
            ..
        }) => {
            assert_eq!(track_kind, TrackKind::Text);
            assert_eq!(asset_id_state, AssetIdState::NonNil);
        }
        other => panic!("expected AssetIdBiconditionalViolation(Text, NonNil), got {other:?}"),
    }
}

#[test]
fn apply_rejects_unresolved_asset_id() {
    // Patch the keyframes-fixture video clip's asset_id to a fresh
    // UUID that doesn't exist in assets[]. apply() must reject with
    // AssetIdUnresolved. Biconditional passes (non-nil on non-text
    // track), so existence is what fires.
    let p = load_three_track();
    let dangling = "0190b8d3-15e3-7000-bd00-00000000dead";
    let patch: json_patch::Patch = serde_json::from_value(json!([
        {"op": "replace", "path": "/tracks/0/clips/0/asset_id", "value": dangling},
    ]))
    .unwrap();
    let err = p.apply(&patch).expect_err("dangling asset_id must reject");
    match err {
        ApplyError::InvariantViolation(InvariantViolation::AssetIdUnresolved {
            referenced_asset_id,
            ..
        }) => {
            assert_eq!(referenced_asset_id.to_string(), dangling);
        }
        other => panic!("expected AssetIdUnresolved, got {other:?}"),
    }
}

#[test]
fn biconditional_error_carries_clip_id_track_kind_asset_id_state() {
    // Verify clip_id + track_kind + asset_id_state reach the caller
    // intact, and that the Display impl carries the human-readable
    // info.
    let mut p = load_three_track();
    let expected_clip_id = p.tracks[2].clips[0].id;
    let real_id = fixture_video_asset_id();
    p.tracks[2].clips[0].asset_id = AssetRef::from_id(real_id);

    let err = p.apply(&json_patch::Patch(vec![])).unwrap_err();
    match err {
        ApplyError::InvariantViolation(
            v @ InvariantViolation::AssetIdBiconditionalViolation {
                clip_id,
                track_kind,
                asset_id_state,
            },
        ) => {
            assert_eq!(clip_id, expected_clip_id);
            assert_eq!(track_kind, TrackKind::Text);
            assert_eq!(asset_id_state, AssetIdState::NonNil);
            let msg = v.to_string();
            assert!(msg.contains("non-nil"), "msg must mention state: {msg}");
            assert!(msg.contains("Text"), "msg must mention track kind: {msg}");
            assert!(
                msg.contains("biconditional"),
                "msg must mention invariant name: {msg}"
            );
        }
        other => panic!("expected AssetIdBiconditionalViolation, got {other:?}"),
    }
}

#[test]
fn asset_id_unresolved_error_carries_clip_and_asset_ids() {
    // Verify the existence variant surfaces clip_id + the dangling
    // asset id + a useful Display impl.
    let mut p = load_three_track();
    let expected_clip_id = p.tracks[0].clips[0].id;
    p.assets.clear();
    let expected_asset_id = *p.tracks[0].clips[0].asset_id.id().unwrap();

    let err = p.apply(&json_patch::Patch(vec![])).unwrap_err();
    match err {
        ApplyError::InvariantViolation(
            v @ InvariantViolation::AssetIdUnresolved {
                clip_id,
                referenced_asset_id,
            },
        ) => {
            assert_eq!(clip_id, expected_clip_id);
            assert_eq!(referenced_asset_id, expected_asset_id);
            let msg = v.to_string();
            assert!(
                msg.contains(&expected_asset_id.to_string()),
                "msg must mention dangling id: {msg}"
            );
            assert!(
                msg.contains("does not exist"),
                "msg must explain the failure: {msg}"
            );
        }
        other => panic!("expected AssetIdUnresolved, got {other:?}"),
    }
}

#[test]
fn fixtures_satisfy_asset_id_biconditional_and_existence() {
    // Regression canary across all 5 fixtures. Text clips → nil
    // asset_id; non-text clips → non-nil asset_id resolving into
    // project.assets[].
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
        check_asset_id_biconditional(&p).unwrap_or_else(|e| {
            panic!("fixture {name:?} must satisfy asset_id biconditional: {e}");
        });
        check_asset_existence(&p).unwrap_or_else(|e| {
            panic!("fixture {name:?} must satisfy asset-existence: {e}");
        });
    }
}
