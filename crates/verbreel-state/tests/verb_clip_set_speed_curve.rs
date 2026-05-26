//! Tests for `clip.set_speed_curve` (§5.20) — fifty-second production verb.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::clip_set_speed_curve::{
    W_NOOP_CODE, compute_patch, data_envelope_from_post_state,
};
use verbreel_state::{
    ClipSetSpeedCurveArgs, ClipSetSpeedCurveData, ClipSetSpeedCurveError, ClipSetSpeedCurveVerb,
    MutateOutcome, Project, Track, VerbRegistry, default_fixtures, default_registry,
    validate_reconstructors,
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

    project.duration_tk = Tick::new((480_000.0_f64 / options.speed).ceil() as i64);
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
    assert_eq!(ops.last().expect("replace op")["op"], "replace");
    assert_eq!(
        ops.last().expect("replace op")["path"],
        "/tracks/0/clips/0/speed_curve"
    );
    assert_eq!(data.clip_id.to_string(), CLIP_ID);
    assert_eq!(data.speed_curve.expect("curve").len(), 2);
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
fn data_envelope_effective_duration_tk_uses_scalar_formula() {
    let prior = project_with_options(ProjectOptions {
        speed: 2.0,
        ..ProjectOptions::default()
    });
    let (_patch, _warnings, data) =
        compute_patch(&prior, &args(Some(two_point_curve()))).expect("set curve");

    assert_eq!(data.effective_duration_tk, 240_000);
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
