//! Tests for `clip.color_match` (issue #479).

use serde_json::{Value, json};
use verbreel_state::color_dsp::reinhard_channel;
use verbreel_state::verbs::clip_color_match::{W_COLOR_MATCH_ENVELOPE_CODE, compute_patch};
use verbreel_state::{
    ClipColorMatchArgs, ClipColorMatchError, FrameStats, Project, Track, default_fixtures,
    default_registry, validate_reconstructors,
};
use verbreel_types::Tick;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const TRACK_A: &str = "01900000-0000-7000-8000-0000000aa701";
const CLIP_A: &str = "01900000-0000-7000-8000-0000000bb701";
const MISSING_CLIP: &str = "01900000-0000-7000-8000-0000000dd701";

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

fn stats(r: f64, g: f64, b: f64, sr: f64, sg: f64, sb: f64) -> FrameStats {
    FrameStats {
        mean_r: r,
        mean_g: g,
        mean_b: b,
        std_r: sr,
        std_g: sg,
        std_b: sb,
        black_point: 0.02,
        white_point: 0.98,
    }
}

fn args(target: &str) -> ClipColorMatchArgs {
    ClipColorMatchArgs {
        project_id: empty_project().id,
        target: target.to_string(),
        reference: "asset:01900000-0000-7000-8000-0000000ff701".to_string(),
        reference_stats: stats(0.60, 0.50, 0.35, 0.22, 0.20, 0.16),
        target_stats: stats(0.35, 0.45, 0.60, 0.12, 0.14, 0.18),
    }
}

fn apply(prior: &Project, patch: Value) -> Project {
    let patch: json_patch::Patch = serde_json::from_value(patch).expect("patch parses");
    prior.apply(&patch).expect("patch applies")
}

#[test]
fn mints_color_correct_effect_and_echoes_reference() {
    let prior = project_with_clip(false, false);
    let (patch, warnings, data) =
        compute_patch(&prior, &args(&format!("clip:{CLIP_A}"))).expect("happy path");
    let post = apply(&prior, patch);

    let effect = post.tracks[0].clips[0]
        .effects
        .last()
        .expect("effect minted");
    assert_eq!(effect.kind.to_string(), "color_correct");
    assert_eq!(effect.id, data.effect_id);
    assert_eq!(data.reference, "asset:01900000-0000-7000-8000-0000000ff701");
    assert_eq!(
        warnings.last().and_then(|w| w["code"].as_str()),
        Some(W_COLOR_MATCH_ENVELOPE_CODE)
    );
}

#[test]
fn reinhard_luma_transfer_matches_closed_form() {
    // The emitted contrast equals the Rec.709 luma std ratio
    // ref_luma_std / tgt_luma_std, and brightness the luma mean shift —
    // i.e. the Reinhard transfer on the luma channel. Assert against the
    // closed form computed independently here.
    let reference = stats(0.60, 0.50, 0.35, 0.22, 0.20, 0.16);
    let target = stats(0.35, 0.45, 0.60, 0.12, 0.14, 0.18);

    let luma = |r: f64, g: f64, b: f64| 0.2126 * r + 0.7152 * g + 0.0722 * b;
    let ref_luma = luma(reference.mean_r, reference.mean_g, reference.mean_b);
    let tgt_luma = luma(target.mean_r, target.mean_g, target.mean_b);
    let ref_luma_std = luma(reference.std_r, reference.std_g, reference.std_b);
    let tgt_luma_std = luma(target.std_r, target.std_g, target.std_b);

    let prior = project_with_clip(false, false);
    let a = args(&format!("clip:{CLIP_A}"));
    let (_patch, _warnings, data) = compute_patch(&prior, &a).expect("happy path");

    let expect_contrast = ref_luma_std / tgt_luma_std;
    let expect_brightness = ref_luma - tgt_luma;
    assert!(
        (data.contrast - expect_contrast).abs() < 1e-5,
        "{} vs {expect_contrast}",
        data.contrast
    );
    assert!(
        (data.brightness - expect_brightness).abs() < 1e-5,
        "{} vs {expect_brightness}",
        data.brightness
    );

    // Sanity: feeding the target mean luma through the Reinhard channel
    // transfer lands on the reference mean luma.
    let transferred = reinhard_channel(tgt_luma, tgt_luma, tgt_luma_std, ref_luma, ref_luma_std);
    assert!((transferred - ref_luma).abs() < 1e-9, "{transferred}");
}

#[test]
fn matching_a_frame_to_itself_is_approximately_identity() {
    let same = stats(0.45, 0.5, 0.55, 0.18, 0.2, 0.22);
    let prior = project_with_clip(false, false);
    let a = ClipColorMatchArgs {
        reference_stats: same,
        target_stats: same,
        ..args(&format!("clip:{CLIP_A}"))
    };
    let (_patch, _warnings, data) = compute_patch(&prior, &a).expect("happy path");
    assert!(data.brightness.abs() < 1e-6, "{}", data.brightness);
    assert!((data.contrast - 1.0).abs() < 1e-6, "{}", data.contrast);
    assert!((data.saturation - 1.0).abs() < 1e-6, "{}", data.saturation);
    assert!(data.temperature.abs() < 1e-6, "{}", data.temperature);
    assert!(data.tint.abs() < 1e-6, "{}", data.tint);
}

#[test]
fn determinism_byte_identical_params() {
    let prior = project_with_clip(false, false);
    let (patch1, _w1, _d1) = compute_patch(&prior, &args(&format!("clip:{CLIP_A}"))).expect("a");
    let (patch2, _w2, _d2) = compute_patch(&prior, &args(&format!("clip:{CLIP_A}"))).expect("b");
    // The params object is the load-bearing determinism surface; the
    // effect id differs per mint but params must be byte-identical.
    let p1 = &patch1[0]["value"];
    let p2 = &patch2[0]["value"];
    assert_eq!(p1[0]["params"], p2[0]["params"]);
}

#[test]
fn unqualified_target_is_bad_selector() {
    let prior = project_with_clip(false, false);
    let err = compute_patch(&prior, &args(CLIP_A)).expect_err("reject");
    assert!(
        matches!(err, ClipColorMatchError::BadSelector { .. }),
        "{err:?}"
    );
}

#[test]
fn empty_reference_is_bad_selector() {
    let prior = project_with_clip(false, false);
    let a = ClipColorMatchArgs {
        reference: "   ".to_string(),
        ..args(&format!("clip:{CLIP_A}"))
    };
    let err = compute_patch(&prior, &a).expect_err("reject");
    assert!(
        matches!(err, ClipColorMatchError::EmptyReference { .. }),
        "{err:?}"
    );
}

#[test]
fn missing_clip_is_not_found() {
    let prior = project_with_clip(false, false);
    let err = compute_patch(&prior, &args(&format!("clip:{MISSING_CLIP}"))).expect_err("reject");
    assert!(
        matches!(err, ClipColorMatchError::NotFound { .. }),
        "{err:?}"
    );
}

#[test]
fn locked_clip_is_locked() {
    let prior = project_with_clip(false, true);
    let err = compute_patch(&prior, &args(&format!("clip:{CLIP_A}"))).expect_err("reject");
    assert!(
        matches!(err, ClipColorMatchError::Locked { kind: "clip", .. }),
        "{err:?}"
    );
}

#[test]
fn locked_track_is_locked() {
    let prior = project_with_clip(true, false);
    let err = compute_patch(&prior, &args(&format!("clip:{CLIP_A}"))).expect_err("reject");
    assert!(
        matches!(err, ClipColorMatchError::Locked { kind: "track", .. }),
        "{err:?}"
    );
}

#[test]
fn out_of_range_reference_stat_is_bad_range() {
    let prior = project_with_clip(false, false);
    let a = ClipColorMatchArgs {
        reference_stats: stats(1.5, 0.5, 0.35, 0.22, 0.20, 0.16),
        ..args(&format!("clip:{CLIP_A}"))
    };
    let err = compute_patch(&prior, &a).expect_err("reject");
    assert!(
        matches!(
            err,
            ClipColorMatchError::BadRange {
                field: "reference_stats.mean_r",
                ..
            }
        ),
        "{err:?}"
    );
}

#[test]
fn default_fixtures_round_trip_through_reconstructors() {
    validate_reconstructors(&default_registry(), &default_fixtures())
        .expect("default fixtures (incl. clip.color_match) round-trip");
}
