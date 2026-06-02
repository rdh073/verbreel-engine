//! Tests for `clip.auto_color` (issue #479).

use serde_json::{Value, json};
use verbreel_state::color_dsp::GradeParams;
use verbreel_state::verbs::clip_auto_color::{
    EMITTED_EFFECT_KIND, W_AUTO_COLOR_ENVELOPE_CODE, compute_patch, params_for,
};
use verbreel_state::{
    ClipAutoColorArgs, ClipAutoColorError, Project, Track, default_fixtures, default_registry,
    validate_reconstructors,
};
use verbreel_types::Tick;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const TRACK_A: &str = "01900000-0000-7000-8000-0000000aa601";
const CLIP_A: &str = "01900000-0000-7000-8000-0000000bb601";
const MISSING_CLIP: &str = "01900000-0000-7000-8000-0000000dd601";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture -> Project")
}

fn text_track(track_locked: bool, clip_locked: bool) -> Track {
    serde_json::from_value(json!({
        "id": TRACK_A,
        "kind": "text",
        "name": "Text",
        "locked": track_locked,
        "clips": [{
            "id": CLIP_A,
            "name": "Clip",
            "asset_id": "00000000-0000-0000-0000-000000000000",
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 480_000,
            "locked": clip_locked,
            "text": { "content": "Hello", "font_family": "Arial", "font_size_px": 24 },
            "effects": [],
        }],
    }))
    .expect("text track fixture parses")
}

fn project_with_clip(track_locked: bool, clip_locked: bool) -> Project {
    let mut project = empty_project();
    project.tracks = vec![text_track(track_locked, clip_locked)];
    project.duration_tk = Tick::new(480_000);
    project
}

fn args(target: &str) -> ClipAutoColorArgs {
    ClipAutoColorArgs {
        project_id: empty_project().id,
        target: target.to_string(),
        mean_r: 0.40,
        mean_g: 0.45,
        mean_b: 0.55,
        std_r: 0.18,
        std_g: 0.18,
        std_b: 0.18,
        black_point: 0.05,
        white_point: 0.90,
    }
}

fn neutral_args() -> ClipAutoColorArgs {
    ClipAutoColorArgs {
        mean_r: 0.5,
        mean_g: 0.5,
        mean_b: 0.5,
        std_r: 0.2,
        std_g: 0.2,
        std_b: 0.2,
        black_point: 0.0,
        white_point: 1.0,
        ..args(&format!("clip:{CLIP_A}"))
    }
}

fn apply(prior: &Project, patch: Value) -> Project {
    let patch: json_patch::Patch = serde_json::from_value(patch).expect("patch parses");
    prior.apply(&patch).expect("patch applies")
}

#[test]
fn mints_color_correct_effect_with_grade_params() {
    let prior = project_with_clip(false, false);
    let (patch, warnings, data) =
        compute_patch(&prior, &args(&format!("clip:{CLIP_A}"))).expect("happy path");
    let post = apply(&prior, patch);

    let effect = post.tracks[0].clips[0]
        .effects
        .last()
        .expect("effect minted");
    assert_eq!(effect.kind.to_string(), "color_correct");
    assert_eq!(effect.kind.to_string(), EMITTED_EFFECT_KIND);
    assert_eq!(effect.id, data.effect_id);
    assert!(effect.enabled);
    for key in [
        "brightness",
        "contrast",
        "saturation",
        "temperature",
        "tint",
    ] {
        assert!(effect.params.contains_key(key), "missing param {key}");
    }
    assert_eq!(
        warnings.last().and_then(|w| w["code"].as_str()),
        Some(W_AUTO_COLOR_ENVELOPE_CODE)
    );
}

#[test]
fn neutral_full_range_frame_is_approximately_identity() {
    let prior = project_with_clip(false, false);
    let (_patch, _warnings, data) = compute_patch(&prior, &neutral_args()).expect("happy path");
    let id = GradeParams::identity();
    assert!(
        (data.brightness - id.brightness).abs() < 1e-6,
        "{}",
        data.brightness
    );
    assert!(
        (data.contrast - id.contrast).abs() < 1e-6,
        "{}",
        data.contrast
    );
    assert!(
        (data.saturation - id.saturation).abs() < 1e-6,
        "{}",
        data.saturation
    );
    assert!(
        (data.temperature - id.temperature).abs() < 1e-6,
        "{}",
        data.temperature
    );
    assert!((data.tint - id.tint).abs() < 1e-6, "{}", data.tint);
}

#[test]
fn determinism_byte_identical_params() {
    let prior = project_with_clip(false, false);
    let (_p1, _w1, d1) = compute_patch(&prior, &args(&format!("clip:{CLIP_A}"))).expect("first");
    let (_p2, _w2, d2) = compute_patch(&prior, &args(&format!("clip:{CLIP_A}"))).expect("second");

    let m1 = params_for(GradeParams {
        brightness: d1.brightness,
        contrast: d1.contrast,
        saturation: d1.saturation,
        temperature: d1.temperature,
        tint: d1.tint,
    });
    let m2 = params_for(GradeParams {
        brightness: d2.brightness,
        contrast: d2.contrast,
        saturation: d2.saturation,
        temperature: d2.temperature,
        tint: d2.tint,
    });
    assert_eq!(
        serde_json::to_string(&m1).unwrap(),
        serde_json::to_string(&m2).unwrap()
    );
}

#[test]
fn unqualified_target_is_bad_selector() {
    let prior = project_with_clip(false, false);
    let err = compute_patch(&prior, &args(CLIP_A)).expect_err("must reject");
    assert!(
        matches!(err, ClipAutoColorError::BadSelector { .. }),
        "{err:?}"
    );
}

#[test]
fn wrong_prefix_is_bad_selector() {
    let prior = project_with_clip(false, false);
    let err = compute_patch(&prior, &args(&format!("track:{TRACK_A}"))).expect_err("reject");
    assert!(
        matches!(err, ClipAutoColorError::BadSelector { .. }),
        "{err:?}"
    );
}

#[test]
fn missing_clip_is_not_found() {
    let prior = project_with_clip(false, false);
    let err = compute_patch(&prior, &args(&format!("clip:{MISSING_CLIP}"))).expect_err("reject");
    assert!(
        matches!(err, ClipAutoColorError::NotFound { .. }),
        "{err:?}"
    );
}

#[test]
fn locked_clip_is_locked() {
    let prior = project_with_clip(false, true);
    let err = compute_patch(&prior, &args(&format!("clip:{CLIP_A}"))).expect_err("reject");
    assert!(
        matches!(err, ClipAutoColorError::Locked { kind: "clip", .. }),
        "{err:?}"
    );
}

#[test]
fn locked_track_is_locked() {
    let prior = project_with_clip(true, false);
    let err = compute_patch(&prior, &args(&format!("clip:{CLIP_A}"))).expect_err("reject");
    assert!(
        matches!(err, ClipAutoColorError::Locked { kind: "track", .. }),
        "{err:?}"
    );
}

#[test]
fn out_of_range_stat_is_bad_range() {
    let prior = project_with_clip(false, false);
    let bad = ClipAutoColorArgs {
        mean_r: 1.5,
        ..args(&format!("clip:{CLIP_A}"))
    };
    let err = compute_patch(&prior, &bad).expect_err("reject");
    assert!(
        matches!(
            err,
            ClipAutoColorError::BadRange {
                field: "mean_r",
                ..
            }
        ),
        "{err:?}"
    );
}

#[test]
fn default_fixtures_round_trip_through_reconstructors() {
    validate_reconstructors(&default_registry(), &default_fixtures())
        .expect("default fixtures (incl. clip.auto_color) round-trip");
}
