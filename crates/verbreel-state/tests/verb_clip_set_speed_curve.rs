//! Tests for `clip.set_speed_curve` (§5.20) — fifty-second production verb.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::clip_set_speed_curve::{
    W_NOOP_CODE, compute_patch, data_envelope_from_post_state,
};
use verbreel_state::{
    ClipSetSpeedCurveArgs, ClipSetSpeedCurveData, ClipSetSpeedCurveError, ClipSetSpeedCurveVerb,
    MutateOutcome, Project, Track, VerbRegistry, clip_timeline_duration_tk, default_fixtures,
    default_registry, validate_reconstructors,
};
use verbreel_types::{ProjectId, Tick};

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const TRACK_ID: &str = "01900000-0000-7000-8000-0000000aa601";
const TEXT_TRACK_ID: &str = "01900000-0000-7000-8000-0000000aa602";
const CLIP_ID: &str = "01900000-0000-7000-8000-0000000bb601";
const TEXT_CLIP_ID: &str = "01900000-0000-7000-8000-0000000bb602";
const MISSING_CLIP_ID: &str = "01900000-0000-7000-8000-0000000bb699";
const VIDEO_ASSET_ID: &str = "01900000-0000-7000-8000-0000000cc601";
const IMAGE_ASSET_ID: &str = "01900000-0000-7000-8000-0000000cc602";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture -> Project")
}

fn fixture_project_id() -> ProjectId {
    FIXTURE_PROJECT_ID
        .parse()
        .expect("fixture project id parses")
}

#[derive(Debug, Clone)]
struct ProjectOptions {
    track_locked: bool,
    clip_locked: bool,
    prior_curve: Option<Vec<verbreel_state::SpeedCurvePoint>>,
    image_clip: bool,
    text_clip: bool,
    speed: f64,
}

impl Default for ProjectOptions {
    fn default() -> Self {
        Self {
            track_locked: false,
            clip_locked: false,
            prior_curve: None,
            image_clip: false,
            text_clip: false,
            speed: 1.0,
        }
    }
}

fn video_asset() -> Value {
    json!({
        "id": VIDEO_ASSET_ID,
        "kind": "video",
        "hash": "53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658",
        "path": "assets/53/53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.mp4",
        "original_filename": "clip-set-speed-curve.mp4",
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

fn image_asset() -> Value {
    json!({
        "id": IMAGE_ASSET_ID,
        "kind": "image",
        "hash": "8d969eef6ecad3c29a3a629280e686cf0c3f5d5a86aff3ca12020c923adc6c92",
        "path": "assets/8d/8d969eef6ecad3c29a3a629280e686cf0c3f5d5a86aff3ca12020c923adc6c92.png",
        "original_filename": "speed-curve.png",
        "imported_at": "2026-05-24T00:00:00Z",
        "metadata": {
            "width": 640,
            "height": 360,
            "container": "png",
            "has_alpha": true,
            "fingerprint": {
                "mtime_ms": 1_700_000_000_000_i64,
                "size_bytes": 512,
            }
        }
    })
}

fn project_with_options(options: ProjectOptions) -> Project {
    let mut project = empty_project();
    project.tracks.clear();
    project.assets.clear();
    project
        .assets
        .push(serde_json::from_value(video_asset()).expect("video asset parses"));
    if options.image_clip {
        project
            .assets
            .push(serde_json::from_value(image_asset()).expect("image asset parses"));
    }

    if options.text_clip {
        let track: Track = serde_json::from_value(json!({
            "id": TEXT_TRACK_ID,
            "kind": "text",
            "name": "Text",
            "locked": options.track_locked,
            "clips": [{
                "id": TEXT_CLIP_ID,
                "name": "Text Clip",
                "asset_id": "00000000-0000-0000-0000-000000000000",
                "track_position_tk": 0,
                "source_in_tk": 0,
                "source_out_tk": 480_000,
                "locked": options.clip_locked,
                "text": {
                    "content": "Speed",
                    "font_family": "Arial",
                    "font_size_px": 24
                },
            }],
        }))
        .expect("text track fixture parses");
        project.tracks.push(track);
    } else {
        let mut clip = json!({
            "id": CLIP_ID,
            "name": "Speed Curve Clip",
            "asset_id": if options.image_clip { IMAGE_ASSET_ID } else { VIDEO_ASSET_ID },
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 480_000,
            "speed": options.speed,
            "locked": options.clip_locked,
        });
        if let Some(curve) = options.prior_curve {
            clip.as_object_mut().expect("clip object").insert(
                "speed_curve".to_string(),
                serde_json::to_value(curve).expect("curve serializes"),
            );
        }

        let track: Track = serde_json::from_value(json!({
            "id": TRACK_ID,
            "kind": "video",
            "name": "Video",
            "locked": options.track_locked,
            "clips": [clip],
        }))
        .expect("video track fixture parses");
        project.tracks.push(track);
    }

    // Use the curve-aware helper so the fixture's `duration_tk`
    // matches `check_duration_tk` for both scalar and curve-bearing
    // clips. Important: the noop test path applies an empty patch over
    // the prior, which means the prior itself must already satisfy the
    // §0.13 duration invariant.
    project.duration_tk = clip_timeline_duration_tk(&project.tracks[0].clips[0]);
    project
}

fn project() -> Project {
    project_with_options(ProjectOptions::default())
}

fn point(time_tk: i64, factor: f64) -> verbreel_state::SpeedCurvePoint {
    verbreel_state::SpeedCurvePoint {
        time_tk: Tick::new(time_tk),
        factor,
    }
}

fn two_point_curve() -> Vec<verbreel_state::SpeedCurvePoint> {
    vec![point(0, 1.0), point(480_000, 2.0)]
}

fn boundary_curve(count: usize) -> Vec<verbreel_state::SpeedCurvePoint> {
    (0..count)
        .map(|index| {
            let time_tk = if count == 1 {
                0
            } else {
                (480_000_i64 * index as i64) / (count as i64 - 1)
            };
            point(time_tk, 1.0)
        })
        .collect()
}

fn args(points: Option<Vec<verbreel_state::SpeedCurvePoint>>) -> ClipSetSpeedCurveArgs {
    ClipSetSpeedCurveArgs {
        project_id: fixture_project_id(),
        clip: CLIP_ID.to_string(),
        points,
    }
}

fn apply_patch(prior: &Project, patch: Value) -> Project {
    let patch: json_patch::Patch = serde_json::from_value(patch).expect("patch parses");
    prior.apply(&patch).expect("patch applies")
}

#[test]
fn compute_patch_sets_two_point_monotonic_curve_on_video_clip() {
    let prior = project();
    let (patch, warnings, data) =
        compute_patch(&prior, &args(Some(two_point_curve()))).expect("set curve");

    assert!(warnings.is_empty());
    let ops = patch.as_array().expect("patch array");
    let speed_curve_replace = ops
        .iter()
        .find(|op| op["path"] == "/tracks/0/clips/0/speed_curve" && op["op"] == "replace")
        .expect("speed_curve replace op present");
    assert_eq!(speed_curve_replace["op"], "replace");
    assert_eq!(data.clip_id.to_string(), CLIP_ID);
    assert_eq!(data.speed_curve.expect("curve").len(), 2);
    // §5.20 closed-form integral: speed=1.0, points (0, 1.0) → (480000, 2.0)
    // contribution = 480000/(1*(2-1)) * ln(2/1) = 480000 * ln(2) ≈ 332710.7
    assert_eq!(data.effective_duration_tk, 332_711);
}

#[test]
fn compute_patch_clears_curve_via_null() {
    let prior = project_with_options(ProjectOptions {
        prior_curve: Some(two_point_curve()),
        ..ProjectOptions::default()
    });

    let (patch, warnings, data) = compute_patch(&prior, &args(None)).expect("clear curve");

    assert!(warnings.is_empty());
    assert_eq!(patch[0]["op"], "replace");
    assert_eq!(patch[0]["value"], Value::Null);
    assert!(data.speed_curve.is_none());
}

#[test]
fn compute_patch_accepts_256_point_curve() {
    let (_patch, warnings, data) =
        compute_patch(&project(), &args(Some(boundary_curve(256)))).expect("256 points");

    assert!(warnings.is_empty());
    assert_eq!(data.speed_curve.expect("curve").len(), 256);
}

#[test]
fn compute_patch_accepts_factor_lower_bound() {
    let curve = vec![point(0, 0.001), point(480_000, 1.0)];
    let (_patch, warnings, data) =
        compute_patch(&project(), &args(Some(curve))).expect("factor lower bound");

    assert!(warnings.is_empty());
    assert_eq!(data.speed_curve.expect("curve")[0].factor, 0.001);
}

#[test]
fn compute_patch_accepts_factor_upper_bound() {
    let curve = vec![point(0, 1.0), point(480_000, 100.0)];
    let (_patch, warnings, data) =
        compute_patch(&project(), &args(Some(curve))).expect("factor upper bound");

    assert!(warnings.is_empty());
    assert_eq!(data.speed_curve.expect("curve")[1].factor, 100.0);
}

#[test]
fn compute_patch_warns_noop_when_curve_already_matches() {
    let curve = two_point_curve();
    let prior = project_with_options(ProjectOptions {
        prior_curve: Some(curve.clone()),
        ..ProjectOptions::default()
    });

    let (patch, warnings, data) = compute_patch(&prior, &args(Some(curve))).expect("noop");

    assert_eq!(patch, json!([]));
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_NOOP_CODE);
    assert_eq!(data.speed_curve.expect("curve").len(), 2);
}

#[test]
fn compute_patch_missing_clip_errors() {
    let mut args = args(Some(two_point_curve()));
    args.clip = MISSING_CLIP_ID.to_string();

    let err = compute_patch(&project(), &args).expect_err("missing clip");
    assert!(matches!(err, ClipSetSpeedCurveError::ClipNotFound { .. }));
}

#[test]
fn compute_patch_bad_selector_errors() {
    let mut args = args(Some(two_point_curve()));
    args.clip = "not-a-uuid".to_string();

    let err = compute_patch(&project(), &args).expect_err("bad selector");
    assert!(matches!(err, ClipSetSpeedCurveError::BadSelector { .. }));
}

#[test]
fn compute_patch_locked_clip_errors() {
    let prior = project_with_options(ProjectOptions {
        clip_locked: true,
        ..ProjectOptions::default()
    });

    let err = compute_patch(&prior, &args(Some(two_point_curve()))).expect_err("locked clip");
    assert!(matches!(
        err,
        ClipSetSpeedCurveError::Locked { kind: "clip", .. }
    ));
}

#[test]
fn compute_patch_locked_track_errors() {
    let prior = project_with_options(ProjectOptions {
        track_locked: true,
        ..ProjectOptions::default()
    });

    let err = compute_patch(&prior, &args(Some(two_point_curve()))).expect_err("locked track");
    assert!(matches!(
        err,
        ClipSetSpeedCurveError::Locked { kind: "track", .. }
    ));
}

#[test]
fn compute_patch_rejects_text_clip() {
    let prior = project_with_options(ProjectOptions {
        text_clip: true,
        ..ProjectOptions::default()
    });
    let mut args = args(Some(two_point_curve()));
    args.clip = TEXT_CLIP_ID.to_string();

    let err = compute_patch(&prior, &args).expect_err("text mismatch");
    assert!(matches!(
        err,
        ClipSetSpeedCurveError::ClipKindMismatch {
            actual_kind: "text",
            ..
        }
    ));
}

#[test]
fn compute_patch_rejects_image_clip() {
    let prior = project_with_options(ProjectOptions {
        image_clip: true,
        ..ProjectOptions::default()
    });

    let err = compute_patch(&prior, &args(Some(two_point_curve()))).expect_err("image mismatch");
    assert!(matches!(
        err,
        ClipSetSpeedCurveError::ClipKindMismatch {
            actual_kind: "image",
            ..
        }
    ));
}

#[test]
fn compute_patch_rejects_length_one() {
    let err = compute_patch(&project(), &args(Some(vec![point(0, 1.0)]))).expect_err("length 1");

    assert!(matches!(
        err,
        ClipSetSpeedCurveError::BadSpeedCurveLength {
            violation: "length",
            length: 1,
            bound: "[2, 256]",
        }
    ));
}

#[test]
fn compute_patch_rejects_length_257() {
    let err = compute_patch(&project(), &args(Some(boundary_curve(257)))).expect_err("length 257");

    assert!(matches!(
        err,
        ClipSetSpeedCurveError::BadSpeedCurveLength {
            violation: "length",
            length: 257,
            bound: "[2, 256]",
        }
    ));
}

#[test]
fn compute_patch_rejects_factor_below_lower_bound() {
    let err = compute_patch(
        &project(),
        &args(Some(vec![point(0, 0.0), point(480_000, 1.0)])),
    )
    .expect_err("factor 0");

    assert!(matches!(
        err,
        ClipSetSpeedCurveError::BadSpeedCurveFactor {
            violation: "factor_out_of_range",
            index: 0,
            factor: 0.0,
            bound: "[0.001, 100]",
        }
    ));
}

#[test]
fn compute_patch_rejects_factor_above_upper_bound() {
    let err = compute_patch(
        &project(),
        &args(Some(vec![point(0, 1.0), point(480_000, 100.001)])),
    )
    .expect_err("factor 100.001");

    assert!(matches!(
        err,
        ClipSetSpeedCurveError::BadSpeedCurveFactor {
            violation: "factor_out_of_range",
            index: 1,
            factor,
            bound: "[0.001, 100]",
        } if (factor - 100.001).abs() < f64::EPSILON
    ));
}

#[test]
fn compute_patch_rejects_negative_time_tk() {
    let err = compute_patch(
        &project(),
        &args(Some(vec![point(-1, 1.0), point(480_000, 1.0)])),
    )
    .expect_err("negative time");

    assert!(matches!(
        err,
        ClipSetSpeedCurveError::BadSpeedCurveTime {
            violation: "time_tk_out_of_range",
            index: 0,
            time_tk: -1,
            ..
        }
    ));
}

#[test]
fn compute_patch_rejects_time_tk_above_source_range() {
    let err = compute_patch(
        &project(),
        &args(Some(vec![point(0, 1.0), point(480_001, 1.0)])),
    )
    .expect_err("time beyond range");

    assert!(matches!(
        err,
        ClipSetSpeedCurveError::BadSpeedCurveTime {
            violation: "time_tk_out_of_range",
            index: 1,
            time_tk: 480_001,
            ..
        }
    ));
}

#[test]
fn compute_patch_rejects_equal_consecutive_time_tk() {
    let err = compute_patch(&project(), &args(Some(vec![point(0, 1.0), point(0, 2.0)])))
        .expect_err("equal time");

    assert!(matches!(
        err,
        ClipSetSpeedCurveError::BadSpeedCurveMonotonic {
            violation: "monotonic",
            index: 1,
            time_tk: 0,
            previous_time_tk: 0,
        }
    ));
}

#[test]
fn compute_patch_rejects_descending_time_tk() {
    let err = compute_patch(&project(), &args(Some(vec![point(10, 1.0), point(9, 2.0)])))
        .expect_err("descending time");

    assert!(matches!(
        err,
        ClipSetSpeedCurveError::BadSpeedCurveMonotonic {
            violation: "monotonic",
            index: 1,
            time_tk: 9,
            previous_time_tk: 10,
        }
    ));
}

#[test]
fn data_envelope_linked_audio_clip_ids_is_empty() {
    let (_patch, _warnings, data) =
        compute_patch(&project(), &args(Some(two_point_curve()))).expect("set curve");

    assert!(data.linked_audio_clip_ids.is_empty());
}

#[test]
fn data_envelope_effective_duration_tk_uses_integral_when_curve_set() {
    // §5.20 closed-form integral: speed=2.0, curve (0, 1.0) → (480_000, 2.0)
    // contribution = (480000 / (2*(2-1))) * ln(2/1) = 240000 * ln(2) ≈ 166355.32
    let prior = project_with_options(ProjectOptions {
        speed: 2.0,
        ..ProjectOptions::default()
    });
    let (_patch, _warnings, data) =
        compute_patch(&prior, &args(Some(two_point_curve()))).expect("set curve");

    assert_eq!(data.effective_duration_tk, 166_356);
}

#[test]
fn reconstructor_round_trips_with_some_curve() {
    let prior = project();
    let args = args(Some(two_point_curve()));
    let (patch, _warnings, data) = compute_patch(&prior, &args).expect("set curve");
    let post = apply_patch(&prior, patch);

    let reconstructed = data_envelope_from_post_state(&args, &post).expect("reconstructs");

    assert_eq!(reconstructed, data);
    assert_eq!(reconstructed.speed_curve.expect("curve").len(), 2);
}

#[test]
fn reconstructor_round_trips_with_none_curve() {
    let prior = project_with_options(ProjectOptions {
        prior_curve: Some(two_point_curve()),
        ..ProjectOptions::default()
    });
    let args = args(None);
    let (patch, _warnings, data) = compute_patch(&prior, &args).expect("clear curve");
    let post = apply_patch(&prior, patch);

    let reconstructed = data_envelope_from_post_state(&args, &post).expect("reconstructs");

    assert_eq!(reconstructed, data);
    assert!(reconstructed.speed_curve.is_none());
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "clip.set_speed_curve")
        .expect("default_fixtures includes clip.set_speed_curve");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(ClipSetSpeedCurveVerb))
        .expect("register clip.set_speed_curve verb");

    let report = validate_reconstructors(&registry, &[fixture])
        .expect("clip.set_speed_curve reconstruct from fixture");
    assert_eq!(report.verbs_checked, vec!["clip.set_speed_curve"]);
    assert_eq!(report.fixtures_run, 1);
}

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        project(),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry clears the gate and writes project.json");

    let outcome = store
        .mutate_via_verb(
            "clip.set_speed_curve",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "clip": CLIP_ID,
                "points": [
                    { "time_tk": 0, "factor": 1.0 },
                    { "time_tk": 480_000, "factor": 2.0 }
                ]
            }),
            None,
        )
        .expect("mutate_via_verb happy path");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("happy path must return Applied, got {outcome:?}");
    };

    let data: ClipSetSpeedCurveData =
        serde_json::from_value(data).expect("clip.set_speed_curve data parses");
    assert_eq!(data.clip_id.to_string(), CLIP_ID);
    assert_eq!(data.speed_curve.expect("curve").len(), 2);
    assert_eq!(
        store.project().tracks[0].clips[0]
            .speed_curve
            .as_ref()
            .expect("speed curve")
            .len(),
        2
    );
    assert!(warnings.is_empty());
}

// ---------------------------------------------------------------------
// Integration math (§5.20 closed-form integral)
// ---------------------------------------------------------------------

/// `ln(2)` to f64 precision — bound to the same library function the
/// engine uses so byte-identical expected values fall out of `ceil()`.
fn ln_2() -> f64 {
    2.0_f64.ln()
}

#[test]
fn integration_single_linear_segment_two_points() {
    // speed=1, points (0, 1.0) → (480_000, 2.0)
    // contribution = (480000/(1*(2-1))) * ln(2/1) = 480000 * ln(2)
    let prior = project();
    let curve = vec![point(0, 1.0), point(480_000, 2.0)];
    let (_patch, warnings, data) =
        compute_patch(&prior, &args(Some(curve))).expect("single segment");

    let expected = (480_000.0_f64 * ln_2()).ceil() as i64;
    assert_eq!(data.effective_duration_tk, expected);
    assert_eq!(data.effective_duration_tk, 332_711);
    assert!(warnings.is_empty());
}

#[test]
fn integration_two_segments_shared_middle() {
    // speed=1, points (0,1) → (240_000, 2) → (480_000, 4)
    // seg1 = 240000 * ln(2); seg2 = 120000 * ln(2); sum = 360000 * ln(2)
    let prior = project();
    let curve = vec![point(0, 1.0), point(240_000, 2.0), point(480_000, 4.0)];
    let (_patch, _warnings, data) =
        compute_patch(&prior, &args(Some(curve))).expect("two segments");

    let expected = (360_000.0_f64 * ln_2()).ceil() as i64;
    assert_eq!(data.effective_duration_tk, expected);
    assert_eq!(data.effective_duration_tk, 249_533);
}

#[test]
fn integration_three_segments_constant_middle_uses_scalar_branch() {
    // speed=1, points (0, 2.0), (160_000, 1.0), (320_000, 1.0), (480_000, 0.5)
    // seg1 (2→1)   = 160000 * ln(1/2) / (1-2) = 160000 * ln(2)
    // seg2 (1=1)   = (320000-160000) / (1*1) = 160000  ← constant branch
    // seg3 (1→0.5) = 160000 * ln(0.5/1) / (0.5-1) = 160000 * ln(2) / 0.5
    //              = 320000 * ln(2)
    let prior = project();
    let curve = vec![
        point(0, 2.0),
        point(160_000, 1.0),
        point(320_000, 1.0),
        point(480_000, 0.5),
    ];
    let (_patch, _warnings, data) =
        compute_patch(&prior, &args(Some(curve))).expect("three segments");

    let seg1 = 160_000.0_f64 * ln_2();
    let seg2 = 160_000.0_f64; // constant branch — must not call .ln()
    let seg3 = 320_000.0_f64 * ln_2();
    let expected = (seg1 + seg2 + seg3).ceil() as i64;
    assert_eq!(data.effective_duration_tk, expected);
}

#[test]
fn integration_boundary_held_left_includes_pre_first_point_segment() {
    // points (100_000, 1.0) → (480_000, 2.0) with source [0, 480_000]
    // left-held [0, 100000) at f_0=1: 100000 / (1*1) = 100000
    // segment: (380000/(1*1)) * ln(2/1) = 380000 * ln(2)
    let prior = project();
    let curve = vec![point(100_000, 1.0), point(480_000, 2.0)];
    let (_patch, _warnings, data) =
        compute_patch(&prior, &args(Some(curve))).expect("boundary held left");

    let expected = (100_000.0_f64 + 380_000.0_f64 * ln_2()).ceil() as i64;
    assert_eq!(data.effective_duration_tk, expected);
}

#[test]
fn integration_boundary_held_right_includes_post_last_point_segment() {
    // points (0, 1.0) → (380_000, 2.0) with source [0, 480_000]
    // segment: 380000 * ln(2)
    // right-held (380000, 480000] at f_n=2: 100000 / (1*2) = 50000
    let prior = project();
    let curve = vec![point(0, 1.0), point(380_000, 2.0)];
    let (_patch, _warnings, data) =
        compute_patch(&prior, &args(Some(curve))).expect("boundary held right");

    let expected = (380_000.0_f64 * ln_2() + 50_000.0_f64).ceil() as i64;
    assert_eq!(data.effective_duration_tk, expected);
}

// ---------------------------------------------------------------------
// Cascade success — fades / keyframes / effects clamped by integration
// ---------------------------------------------------------------------

fn project_with_fades(fade_in_tk: i64, fade_out_tk: i64) -> Project {
    let mut project = empty_project();
    project.tracks.clear();
    project.assets.clear();
    project
        .assets
        .push(serde_json::from_value(video_asset()).expect("video asset"));

    let track: Track = serde_json::from_value(json!({
        "id": TRACK_ID,
        "kind": "video",
        "name": "Video",
        "locked": false,
        "clips": [{
            "id": CLIP_ID,
            "name": "Faded Clip",
            "asset_id": VIDEO_ASSET_ID,
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 480_000,
            "speed": 1.0,
            "fade_in_tk": fade_in_tk,
            "fade_out_tk": fade_out_tk,
            "locked": false,
        }],
    }))
    .expect("fade fixture parses");
    project.tracks.push(track);
    project.duration_tk = Tick::new(480_000);
    project
}

#[test]
fn cascade_fade_clamped_when_curve_shrinks_duration() {
    // Prior: fades 200_000 + 200_000 = 400_000; clip duration 480_000.
    // Curve increases speed → new duration 332_711, less than fade sum.
    let prior = project_with_fades(200_000, 200_000);
    let (patch, warnings, data) =
        compute_patch(&prior, &args(Some(two_point_curve()))).expect("fade cascade");

    let fade_warn = warnings
        .iter()
        .find(|w| w["code"] == "W_FADE_CLAMPED")
        .expect("W_FADE_CLAMPED emitted");
    assert_eq!(fade_warn["details"]["clip_id"], CLIP_ID);
    assert_eq!(fade_warn["details"]["from_in_tk"], 200_000);
    assert_eq!(fade_warn["details"]["from_out_tk"], 200_000);

    let new_in = fade_warn["details"]["to_in_tk"].as_i64().expect("i64");
    let new_out = fade_warn["details"]["to_out_tk"].as_i64().expect("i64");
    assert!(new_in + new_out <= data.effective_duration_tk);
    assert_eq!(new_in + new_out, data.effective_duration_tk);

    // Apply must succeed — post-state must satisfy fade-clamp invariant.
    let post = apply_patch(&prior, patch);
    let post_clip = &post.tracks[0].clips[0];
    assert_eq!(
        post_clip.fade_in_tk.get() + post_clip.fade_out_tk.get(),
        new_in + new_out
    );
}

fn project_with_keyframes(times_tk: &[i64]) -> Project {
    let mut project = empty_project();
    project.tracks.clear();
    project.assets.clear();
    project
        .assets
        .push(serde_json::from_value(video_asset()).expect("video asset"));

    let keyframes: Vec<Value> = times_tk
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let uuid_suffix = format!("{:012x}", 0xdd00 + i);
            json!({
                "id": format!("01900000-0000-7000-8000-{uuid_suffix}"),
                "property": "opacity",
                "time_tk": t,
                "value": 0.5,
                "easing": "linear",
            })
        })
        .collect();

    let track: Track = serde_json::from_value(json!({
        "id": TRACK_ID,
        "kind": "video",
        "name": "Video",
        "locked": false,
        "clips": [{
            "id": CLIP_ID,
            "name": "Keyframed Clip",
            "asset_id": VIDEO_ASSET_ID,
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 480_000,
            "speed": 1.0,
            "keyframes": keyframes,
            "locked": false,
        }],
    }))
    .expect("keyframe fixture parses");
    project.tracks.push(track);
    project.duration_tk = Tick::new(480_000);
    project
}

#[test]
fn cascade_keyframes_removed_when_time_exceeds_new_duration() {
    // Two keyframes — one at 100_000 (kept), one at 400_000 (removed,
    // > 332_711 integrated duration).
    let prior = project_with_keyframes(&[100_000, 400_000]);
    let (patch, warnings, _data) =
        compute_patch(&prior, &args(Some(two_point_curve()))).expect("keyframe cascade");

    let kf_warn = warnings
        .iter()
        .find(|w| w["code"] == "W_KEYFRAMES_REMOVED")
        .expect("W_KEYFRAMES_REMOVED emitted");
    assert_eq!(kf_warn["details"]["clip_id"], CLIP_ID);
    let removed = kf_warn["details"]["removed_keyframe_ids"]
        .as_array()
        .expect("removed ids array");
    assert_eq!(removed.len(), 1);

    let post = apply_patch(&prior, patch);
    assert_eq!(post.tracks[0].clips[0].keyframes.len(), 1);
}

fn project_with_effect(window_in_tk: i64, window_out_tk: i64, kind: &str) -> Project {
    let mut project = empty_project();
    project.tracks.clear();
    project.assets.clear();
    project
        .assets
        .push(serde_json::from_value(video_asset()).expect("video asset"));

    let effect = json!({
        "id": "01900000-0000-7000-8000-0000000ee601",
        "kind": kind,
        "enabled": true,
        "params": {},
        "in_tk": window_in_tk,
        "out_tk": window_out_tk,
    });

    let track: Track = serde_json::from_value(json!({
        "id": TRACK_ID,
        "kind": "video",
        "name": "Video",
        "locked": false,
        "clips": [{
            "id": CLIP_ID,
            "name": "Effect Clip",
            "asset_id": VIDEO_ASSET_ID,
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 480_000,
            "speed": 1.0,
            "effects": [effect],
            "locked": false,
        }],
    }))
    .expect("effect fixture parses");
    project.tracks.push(track);
    project.duration_tk = Tick::new(480_000);
    project
}

#[test]
fn cascade_effect_window_clamped_when_out_tk_exceeds_new_duration() {
    // Effect window [100_000, 400_000); new duration 332_711.
    // → clamp out_tk to 332_711, effect retained.
    let prior = project_with_effect(100_000, 400_000, "blur");
    let (patch, warnings, _data) =
        compute_patch(&prior, &args(Some(two_point_curve()))).expect("effect cascade");

    let win_warn = warnings
        .iter()
        .find(|w| w["code"] == "W_EFFECT_WINDOW_CLAMPED")
        .expect("W_EFFECT_WINDOW_CLAMPED emitted");
    assert_eq!(win_warn["details"]["from_out_tk"], 400_000);
    assert_eq!(win_warn["details"]["to_out_tk"], 332_711);
    assert!(win_warn["details"]["removed"].is_null());

    let post = apply_patch(&prior, patch);
    let effect = &post.tracks[0].clips[0].effects[0];
    assert_eq!(
        effect.window.as_ref().expect("window present").out_tk.get(),
        332_711
    );
}

#[test]
fn cascade_effect_removed_when_in_tk_collapses_past_new_duration() {
    // Effect window [400_000, 470_000) — in_tk past 332_711 new duration.
    // → effect removed, removed:true in warning details.
    let prior = project_with_effect(400_000, 470_000, "blur");
    let (patch, warnings, _data) =
        compute_patch(&prior, &args(Some(two_point_curve()))).expect("effect remove cascade");

    let win_warn = warnings
        .iter()
        .find(|w| w["code"] == "W_EFFECT_WINDOW_CLAMPED")
        .expect("W_EFFECT_WINDOW_CLAMPED emitted");
    assert_eq!(win_warn["details"]["removed"], true);
    assert_eq!(win_warn["details"]["from_in_tk"], 400_000);

    let post = apply_patch(&prior, patch);
    assert!(post.tracks[0].clips[0].effects.is_empty());
}

// ---------------------------------------------------------------------
// Error paths — E_BAD_TIME and E_CLIP_OVERLAP
// ---------------------------------------------------------------------

#[test]
fn error_bad_time_when_integration_overflows_max_safe_integer() {
    // speed=0.001, source range ~240 GT (about 11.5 days), curve
    // flat at factor=0.001: integral = 240e9 / (0.001 * 0.001) = 2.4e17,
    // far above MAX_SAFE_INTEGER (9.007e15).
    let mut project = empty_project();
    project.tracks.clear();
    project.assets.clear();
    project
        .assets
        .push(serde_json::from_value(video_asset()).expect("video asset"));

    let track: Track = serde_json::from_value(json!({
        "id": TRACK_ID,
        "kind": "video",
        "name": "Video",
        "locked": false,
        "clips": [{
            "id": CLIP_ID,
            "name": "Huge Clip",
            "asset_id": VIDEO_ASSET_ID,
            "track_position_tk": 0,
            "source_in_tk": 0_i64,
            "source_out_tk": 240_000_000_000_i64,
            "speed": 0.001,
            "locked": false,
        }],
    }))
    .expect("overflow fixture parses");
    project.tracks.push(track);
    // Scalar duration = source_out / speed = 240e9 / 0.001 = 2.4e14 (< MSI).
    project.duration_tk = Tick::new(240_000_000_000_000);

    let curve = vec![point(0, 0.001), point(240_000_000_000_i64, 0.001)];
    let mut args = args(None);
    args.points = Some(curve);
    let err = compute_patch(&project, &args).expect_err("overflow rejection");
    assert!(matches!(
        err,
        ClipSetSpeedCurveError::BadTime {
            field: "speed_curve",
            ..
        }
    ));
    if let ClipSetSpeedCurveError::BadTime {
        computed_duration_tk,
        ..
    } = err
    {
        assert!(computed_duration_tk > 9_007_199_254_740_991.0);
    }
}

fn project_with_sibling(target_speed: f64, sibling_position_tk: i64) -> Project {
    let mut project = empty_project();
    project.tracks.clear();
    project.assets.clear();
    project
        .assets
        .push(serde_json::from_value(video_asset()).expect("video asset"));

    let target_duration = (480_000.0_f64 / target_speed).ceil() as i64;
    let sibling_duration = 100_000;
    let project_duration = (target_duration).max(sibling_position_tk + sibling_duration);

    let track: Track = serde_json::from_value(json!({
        "id": TRACK_ID,
        "kind": "video",
        "name": "Video",
        "locked": false,
        "clips": [
            {
                "id": CLIP_ID,
                "name": "Target",
                "asset_id": VIDEO_ASSET_ID,
                "track_position_tk": 0,
                "source_in_tk": 0,
                "source_out_tk": 480_000,
                "speed": target_speed,
                "locked": false,
            },
            {
                "id": "01900000-0000-7000-8000-0000000bb777",
                "name": "Sibling",
                "asset_id": VIDEO_ASSET_ID,
                "track_position_tk": sibling_position_tk,
                "source_in_tk": 0,
                "source_out_tk": sibling_duration,
                "speed": 1.0,
                "locked": false,
            }
        ],
    }))
    .expect("sibling fixture parses");
    project.tracks.push(track);
    project.duration_tk = Tick::new(project_duration);
    project
}

#[test]
fn error_clip_overlap_when_curve_slows_target_into_sibling() {
    // Target ends at 480_000 (speed=1). Sibling at 500_000.
    // Curve halves the rate everywhere → new duration 960_000.
    // 960_000 > 500_000 → overlap.
    let prior = project_with_sibling(1.0, 500_000);
    let slow_curve = vec![point(0, 0.5), point(480_000, 0.5)];
    let err = compute_patch(&prior, &args(Some(slow_curve))).expect_err("overlap");
    assert!(matches!(
        err,
        ClipSetSpeedCurveError::ClipOverlap { ref failed_clip } if failed_clip == CLIP_ID
    ));
}

#[test]
fn error_clip_overlap_skipped_when_clearing_curve_shortens() {
    // Prior has slow curve → integrated duration 960_000, but the
    // fixture's scalar `project.duration_tk` is 480_000 (helper limit).
    // Sibling lives at 500_000 — would conflict if curve stayed.
    // Clearing reverts to scalar 480_000 < 500_000 → no overlap check.
    let mut prior = project_with_sibling(1.0, 500_000);
    prior.tracks[0].clips[0].speed_curve = Some(vec![point(0, 0.5), point(480_000, 0.5)]);
    let _ = compute_patch(&prior, &args(None)).expect("clear curve shortening is allowed");
}

// ---------------------------------------------------------------------
// W_SPEED_CURVE_EXTREME (§5.20 quality warning)
// ---------------------------------------------------------------------

#[test]
fn extreme_warning_emitted_when_max_factor_exceeds_16_with_time_stretch() {
    let mut prior = project_with_effect(0, 480_000, "time_stretch");
    // Speed=1; curve max factor 20 → max effective 20 > 16.
    prior.tracks[0].clips[0].speed = 1.0;
    let curve = vec![point(0, 1.0), point(480_000, 20.0)];
    let (_patch, warnings, _data) =
        compute_patch(&prior, &args(Some(curve))).expect("extreme curve");

    let extreme = warnings
        .iter()
        .find(|w| w["code"] == "W_SPEED_CURVE_EXTREME")
        .expect("W_SPEED_CURVE_EXTREME emitted");
    assert_eq!(extreme["details"]["clip_id"], CLIP_ID);
    assert_eq!(extreme["details"]["segment_index"], 1);
    assert!(
        extreme["details"]["max_effective_factor"]
            .as_f64()
            .expect("f64")
            > 16.0
    );
}

#[test]
fn extreme_warning_not_emitted_without_time_stretch_effect() {
    // Same extreme curve but no time_stretch → no warning per §5.20.
    let prior = project_with_effect(0, 480_000, "blur");
    let curve = vec![point(0, 1.0), point(480_000, 20.0)];
    let (_patch, warnings, _data) =
        compute_patch(&prior, &args(Some(curve))).expect("non-time-stretch");

    assert!(
        !warnings
            .iter()
            .any(|w| w["code"] == "W_SPEED_CURVE_EXTREME"),
        "W_SPEED_CURVE_EXTREME must not fire without a time_stretch effect"
    );
}

// ---------------------------------------------------------------------
// Reconstructor round-trip
// ---------------------------------------------------------------------

#[test]
fn reconstructor_round_trips_after_full_cascade() {
    // Prior has fades that will clamp under integration; apply patch
    // and re-derive the envelope from post-state — must match the
    // forward-emitted envelope.
    let prior = project_with_fades(200_000, 200_000);
    let args = args(Some(two_point_curve()));
    let (patch, _warnings, data) =
        compute_patch(&prior, &args).expect("forward integration + cascade");
    let post = apply_patch(&prior, patch);

    let reconstructed = data_envelope_from_post_state(&args, &post).expect("reconstruct");
    assert_eq!(reconstructed, data);
    assert_eq!(reconstructed.effective_duration_tk, 332_711);
}

#[test]
fn reconstructor_round_trips_on_noop_path() {
    // W_NOOP path — patch is empty, but reconstructor still rebuilds
    // the envelope from post-state.
    let prior = project_with_options(ProjectOptions {
        prior_curve: Some(two_point_curve()),
        ..ProjectOptions::default()
    });
    let args = args(Some(two_point_curve()));
    let (patch, _warnings, data) = compute_patch(&prior, &args).expect("noop");
    // Empty patch — apply yields the same project shape.
    let post = apply_patch(&prior, patch);
    let reconstructed = data_envelope_from_post_state(&args, &post).expect("reconstruct noop");
    assert_eq!(reconstructed.speed_curve, data.speed_curve);
    assert_eq!(
        reconstructed.effective_duration_tk,
        data.effective_duration_tk
    );
}

// ---------------------------------------------------------------------
// W_NOOP — integration short-circuit
// ---------------------------------------------------------------------

#[test]
fn noop_short_circuits_integration_with_empty_patch() {
    // Identical curve → W_NOOP and no /duration_tk op (integration
    // path is never invoked).
    let curve = two_point_curve();
    let prior = project_with_options(ProjectOptions {
        prior_curve: Some(curve.clone()),
        ..ProjectOptions::default()
    });
    let (patch, warnings, _data) = compute_patch(&prior, &args(Some(curve))).expect("noop");

    assert_eq!(patch, json!([]));
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_NOOP_CODE);
}

// ---------------------------------------------------------------------
// Determinism — stable summation order
// ---------------------------------------------------------------------

#[test]
fn determinism_two_computes_produce_identical_effective_duration_tk() {
    // Same (prior, args) → byte-identical `effective_duration_tk` and
    // byte-identical patch values.
    let prior = project();
    let curve = vec![
        point(0, 1.0),
        point(120_000, 1.5),
        point(240_000, 2.5),
        point(360_000, 3.0),
        point(480_000, 4.0),
    ];
    let (patch_a, _warnings_a, data_a) =
        compute_patch(&prior, &args(Some(curve.clone()))).expect("first compute");
    let (patch_b, _warnings_b, data_b) =
        compute_patch(&prior, &args(Some(curve))).expect("second compute");

    assert_eq!(data_a.effective_duration_tk, data_b.effective_duration_tk);
    assert_eq!(patch_a, patch_b);
}
