//! §0.13 invariant tests — currently covers fade-clamp enforcement.
//!
//! Each subsequent invariant slice (track contiguity, no-overlap,
//! duration_tk maintenance, dangling-keyframe cascade, etc.) extends
//! this file with new `check_*` coverage.

use serde_json::json;
use verbreel_state::{
    ApplyError, InvariantViolation, Project, check_fade_clamp, timeline_duration_tk,
};
use verbreel_types::Tick;

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
    match err {
        InvariantViolation::FadeClamp {
            fade_in_tk,
            fade_out_tk,
            timeline_duration_tk: dur,
            ..
        } => {
            assert_eq!(fade_in_tk.get(), 2_000_000);
            assert_eq!(fade_out_tk.get(), 400_001);
            assert_eq!(dur.get(), 2_400_000);
        }
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
