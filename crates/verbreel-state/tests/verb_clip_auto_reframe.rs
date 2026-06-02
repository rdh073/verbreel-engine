//! Tests for `clip.auto_reframe` (issue #481).
//!
//! `clip.auto_reframe` is a new, additive composition verb — not yet in the
//! published spec — so these tests pin the wire surface (args schema,
//! emitted `transform.*` keyframe track, `W_TRACKER_OUT_OF_BOUNDS` clamp
//! reuse, determinism) rather than mirroring a spec section.

use serde_json::{Value, json};
use verbreel_state::verbs::clip_auto_reframe::{
    W_AUTO_REFRAME_ENVELOPE_CODE, W_TRACKER_OUT_OF_BOUNDS, compute_patch,
};
use verbreel_state::{
    ClipAutoReframeArgs, ClipAutoReframeError, Project, ReframeSmoothing, SubjectSample,
    TargetAspect, Track, default_fixtures, default_registry, validate_reconstructors,
};
use verbreel_types::Tick;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const TRACK_A: &str = "01900000-0000-7000-8000-0000000aa481";
const CLIP_A: &str = "01900000-0000-7000-8000-0000000bb481";
const MISSING_CLIP: &str = "01900000-0000-7000-8000-0000000dd481";

const CANVAS_W: u32 = 1920;
const CANVAS_H: u32 = 1080;

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
            "text": { "content": "X", "font_family": "Arial", "font_size_px": 24 },
            "effects": [],
        }],
    }))
    .expect("text track fixture parses")
}

/// 16:9 canvas with a single unlocked clip.
fn project_with_clip(track_locked: bool, clip_locked: bool) -> Project {
    let mut project = empty_project();
    project.canvas.width = CANVAS_W;
    project.canvas.height = CANVAS_H;
    project.tracks = vec![text_track(track_locked, clip_locked)];
    project.duration_tk = Tick::new(480_000);
    project
}

fn aspect_9_16() -> TargetAspect {
    TargetAspect { num: 9, den: 16 }
}

fn args(target: &str, trace: Vec<SubjectSample>) -> ClipAutoReframeArgs {
    ClipAutoReframeArgs {
        project_id: empty_project().id,
        target: target.to_string(),
        target_aspect: aspect_9_16(),
        subject_trace: trace,
        smoothing: ReframeSmoothing::default(),
    }
}

fn apply(prior: &Project, patch: Value) -> Project {
    let patch: json_patch::Patch = serde_json::from_value(patch).expect("patch parses");
    prior.apply(&patch).expect("patch applies")
}

/// Cover fit-scale for a 9:16 crop on a 1920x1080 canvas, recomputed here
/// so the test pins the geometry independently of the verb's internals.
fn expected_fit_scale() -> f64 {
    let width = f64::from(CANVAS_W);
    let height = f64::from(CANVAS_H);
    let target_ratio = 9.0 / 16.0;
    let crop_w = height * target_ratio;
    let crop_h = height;
    (width / crop_w).max(height / crop_h)
}

fn keyframes_for(post: &Project, property: &str) -> Vec<(i64, f64)> {
    post.tracks[0].clips[0]
        .keyframes
        .iter()
        .filter(|k| k.property.as_str() == property)
        .map(|k| (k.time_tk.get(), k.value.as_f64().expect("numeric keyframe")))
        .collect()
}

#[test]
fn centers_subject_with_cover_scale() {
    let prior = project_with_clip(false, false);
    let trace = vec![
        SubjectSample {
            time_tk: 0,
            cx: 960.0,
            cy: 540.0,
        },
        SubjectSample {
            time_tk: 480_000,
            cx: 1240.0,
            cy: 540.0,
        },
    ];
    // window=1 disables damping so the centering math is exact per sample.
    let (patch, warnings, data) = compute_patch(
        &prior,
        &ClipAutoReframeArgs {
            smoothing: ReframeSmoothing {
                window: 1,
                ..ReframeSmoothing::default()
            },
            ..args(&format!("clip:{CLIP_A}"), trace)
        },
    )
    .expect("happy path");
    let post = apply(&prior, patch);

    let fit_scale = expected_fit_scale();
    assert!((data.fit_scale - fit_scale).abs() < 1e-9);
    assert_eq!(data.clamped_sample_count, 0);

    // Exactly one scale keyframe per axis (the fit zoom is constant), keyed
    // at the first sample's tick.
    let scale_x = keyframes_for(&post, "transform.scale_x");
    let scale_y = keyframes_for(&post, "transform.scale_y");
    assert_eq!(scale_x, vec![(0, fit_scale)]);
    assert_eq!(scale_y, vec![(0, fit_scale)]);

    // Pan keyframes land the subject at the canvas center under the model
    // `x = center - fit_scale * subject`.
    let center_x = f64::from(CANVAS_W) / 2.0;
    let center_y = f64::from(CANVAS_H) / 2.0;
    let xs = keyframes_for(&post, "transform.x");
    let ys = keyframes_for(&post, "transform.y");
    assert_eq!(xs.len(), 2);
    assert_eq!(ys.len(), 2);
    assert_eq!(xs[0], (0, center_x - fit_scale * 960.0));
    assert_eq!(xs[1], (480_000, center_x - fit_scale * 1240.0));
    assert_eq!(ys[0], (0, center_y - fit_scale * 540.0));
    assert_eq!(ys[1], (480_000, center_y - fit_scale * 540.0));

    // envelope-derived data is exact and self-consistent.
    assert_eq!(data.emitted_sample_count, 2);
    assert_eq!(data.emitted_keyframe_count, 2 + 2 * 2);

    // Only the internal envelope warning (no clamp).
    assert_eq!(warnings.len(), 1);
    assert_eq!(
        warnings[0].get("code").and_then(Value::as_str),
        Some(W_AUTO_REFRAME_ENVELOPE_CODE)
    );
}

#[test]
fn clamps_off_screen_subject_and_warns() {
    let prior = project_with_clip(false, false);
    // Second sample walks the subject past the right and bottom edges.
    let trace = vec![
        SubjectSample {
            time_tk: 0,
            cx: 960.0,
            cy: 540.0,
        },
        SubjectSample {
            time_tk: 480_000,
            cx: 5000.0,
            cy: 4000.0,
        },
    ];
    let (patch, warnings, data) = compute_patch(
        &prior,
        &ClipAutoReframeArgs {
            // Disable damping so the clamp lands on the single off-screen
            // sample rather than being averaged back in-bounds.
            smoothing: ReframeSmoothing {
                window: 1,
                ..ReframeSmoothing::default()
            },
            ..args(&format!("clip:{CLIP_A}"), Vec::new())
        }
        .with_trace(trace),
    )
    .expect("happy path");
    let post = apply(&prior, patch);

    assert_eq!(data.clamped_sample_count, 1);

    // The clamped pan never places the anchor outside the canvas: the
    // subject center was pinned to the largest f64 below width/height.
    let xs = keyframes_for(&post, "transform.x");
    let ys = keyframes_for(&post, "transform.y");
    let fit_scale = expected_fit_scale();
    let center_x = f64::from(CANVAS_W) / 2.0;
    let center_y = f64::from(CANVAS_H) / 2.0;
    let max_x = f64::from(CANVAS_W).next_down();
    let max_y = f64::from(CANVAS_H).next_down();
    assert_eq!(
        xs.last().copied(),
        Some((480_000, center_x - fit_scale * max_x))
    );
    assert_eq!(
        ys.last().copied(),
        Some((480_000, center_y - fit_scale * max_y))
    );

    // W_TRACKER_OUT_OF_BOUNDS reuses the tracker.apply details shape.
    let oob = warnings
        .iter()
        .find(|w| w.get("code").and_then(Value::as_str) == Some(W_TRACKER_OUT_OF_BOUNDS))
        .expect("clamp warning present");
    let details = oob.get("details").expect("details present");
    assert_eq!(
        details.get("clamped_sample_count").and_then(Value::as_i64),
        Some(1)
    );
    assert_eq!(
        details.get("to_clip_id").and_then(Value::as_str),
        Some(CLIP_A)
    );
    assert_eq!(
        details.get("bound").and_then(|b| b.get("x")),
        Some(&json!([0, CANVAS_W]))
    );
    assert_eq!(
        details.get("bound").and_then(|b| b.get("y")),
        Some(&json!([0, CANVAS_H]))
    );
}

#[test]
fn determinism_byte_identical_keyframe_values() {
    let prior = project_with_clip(false, false);
    let trace = vec![
        SubjectSample {
            time_tk: 0,
            cx: 900.0,
            cy: 500.0,
        },
        SubjectSample {
            time_tk: 160_000,
            cx: 1000.0,
            cy: 560.0,
        },
        SubjectSample {
            time_tk: 480_000,
            cx: 1100.0,
            cy: 520.0,
        },
    ];
    let (patch_a, warnings_a, data_a) =
        compute_patch(&prior, &args(&format!("clip:{CLIP_A}"), trace.clone())).expect("run a");
    let (patch_b, warnings_b, data_b) =
        compute_patch(&prior, &args(&format!("clip:{CLIP_A}"), trace)).expect("run b");

    // Keyframe *values* + times are byte-identical; ids are minted fresh and
    // intentionally excluded from the determinism contract.
    let strip_ids = |patch: &Value| -> Vec<Value> {
        patch
            .as_array()
            .expect("patch array")
            .iter()
            .map(|op| {
                let mut value = op.get("value").cloned().expect("op value");
                if let Some(obj) = value.as_object_mut() {
                    obj.remove("id");
                }
                json!({
                    "op": op.get("op").cloned(),
                    "path": op.get("path").cloned(),
                    "value": value,
                })
            })
            .collect()
    };
    assert_eq!(strip_ids(&patch_a), strip_ids(&patch_b));
    assert_eq!(warnings_a, warnings_b);
    assert_eq!(data_a, data_b);
}

#[test]
fn min_hold_holds_window_against_jitter() {
    let prior = project_with_clip(false, false);
    // A jittery trace: small oscillations under the re-key threshold,
    // sampled every 8000 ticks (one per frame at 30fps).
    let mut trace = Vec::new();
    for i in 0..40_i64 {
        let jitter = if i % 2 == 0 { 0.5 } else { -0.5 };
        trace.push(SubjectSample {
            time_tk: i * 8_000,
            cx: 960.0 + jitter,
            cy: 540.0 + jitter,
        });
    }
    let last = trace.len() - 1;
    let (patch, _warnings, data) =
        compute_patch(&prior, &args(&format!("clip:{CLIP_A}"), trace.clone())).expect("happy path");
    let post = apply(&prior, patch);

    // With sub-threshold jitter the hysteresis emits only the first and last
    // pan keyframes, not one per sample.
    let xs = keyframes_for(&post, "transform.x");
    assert_eq!(xs.len(), 2);
    assert_eq!(xs[0].0, 0);
    assert_eq!(xs[1].0, trace[last].time_tk);
    assert_eq!(data.emitted_sample_count, 2);
}

#[test]
fn args_round_trip_with_smoothing_defaults() {
    // Omitting `smoothing` deserializes to the documented defaults.
    let raw = json!({
        "project_id": empty_project().id,
        "target": format!("clip:{CLIP_A}"),
        "target_aspect": { "num": 9, "den": 16 },
        "subject_trace": [{ "time_tk": 0, "cx": 960.0, "cy": 540.0 }],
    });
    let parsed: ClipAutoReframeArgs = serde_json::from_value(raw).expect("args deserialize");
    assert_eq!(parsed.smoothing, ReframeSmoothing::default());
    assert_eq!(parsed.target_aspect, TargetAspect { num: 9, den: 16 });

    // Full round-trip is value-stable.
    let re = serde_json::to_value(&parsed).expect("serialize");
    let again: ClipAutoReframeArgs = serde_json::from_value(re).expect("re-deserialize");
    assert_eq!(again.smoothing, parsed.smoothing);
}

#[test]
fn rejects_unqualified_and_missing_and_locked() {
    let prior = project_with_clip(false, false);
    let trace = || {
        vec![SubjectSample {
            time_tk: 0,
            cx: 960.0,
            cy: 540.0,
        }]
    };

    // Unqualified selector -> E_BAD_SELECTOR.
    let err = compute_patch(&prior, &args(CLIP_A, trace())).expect_err("bare selector");
    assert!(matches!(err, ClipAutoReframeError::BadSelector { .. }));

    // Missing clip -> E_NOT_FOUND.
    let err = compute_patch(&prior, &args(&format!("clip:{MISSING_CLIP}"), trace()))
        .expect_err("missing clip");
    assert!(matches!(err, ClipAutoReframeError::NotFound { .. }));

    // Locked clip -> E_LOCKED.
    let locked = project_with_clip(false, true);
    let err =
        compute_patch(&locked, &args(&format!("clip:{CLIP_A}"), trace())).expect_err("locked clip");
    assert!(matches!(
        err,
        ClipAutoReframeError::Locked { kind: "clip", .. }
    ));

    // Locked track -> E_LOCKED.
    let locked_track = project_with_clip(true, false);
    let err = compute_patch(&locked_track, &args(&format!("clip:{CLIP_A}"), trace()))
        .expect_err("locked track");
    assert!(matches!(
        err,
        ClipAutoReframeError::Locked { kind: "track", .. }
    ));
}

#[test]
fn rejects_malformed_params() {
    let prior = project_with_clip(false, false);
    let base = || args(&format!("clip:{CLIP_A}"), Vec::new());

    // Empty trace.
    let err = compute_patch(&prior, &base()).expect_err("empty trace");
    assert!(matches!(
        err,
        ClipAutoReframeError::BadParams {
            field: "subject_trace",
            ..
        }
    ));

    // Zero aspect denominator.
    let mut a = base().with_trace(vec![SubjectSample {
        time_tk: 0,
        cx: 1.0,
        cy: 1.0,
    }]);
    a.target_aspect = TargetAspect { num: 9, den: 0 };
    let err = compute_patch(&prior, &a).expect_err("zero den");
    assert!(matches!(
        err,
        ClipAutoReframeError::BadParams {
            field: "target_aspect",
            ..
        }
    ));

    // Non-monotonic sample ticks.
    let a = base().with_trace(vec![
        SubjectSample {
            time_tk: 100,
            cx: 1.0,
            cy: 1.0,
        },
        SubjectSample {
            time_tk: 50,
            cx: 1.0,
            cy: 1.0,
        },
    ]);
    let err = compute_patch(&prior, &a).expect_err("non-monotonic");
    assert!(matches!(
        err,
        ClipAutoReframeError::BadParams {
            field: "subject_trace",
            ..
        }
    ));

    // Sample tick past the clip window.
    let a = base().with_trace(vec![SubjectSample {
        time_tk: 999_999_999,
        cx: 1.0,
        cy: 1.0,
    }]);
    let err = compute_patch(&prior, &a).expect_err("out of window");
    assert!(matches!(
        err,
        ClipAutoReframeError::BadParams {
            field: "subject_trace",
            ..
        }
    ));
}

#[test]
fn default_registry_and_fixtures_include_auto_reframe() {
    let registry = default_registry();
    assert!(registry.get("clip.auto_reframe").is_some());

    let fixtures = default_fixtures();
    assert!(
        fixtures
            .iter()
            .any(|event| event.verb == "clip.auto_reframe")
    );

    // The §0.8 reconstructor-purity gate passes with the new verb in place.
    validate_reconstructors(&registry, &fixtures).expect("reconstructor gate");
}

/// Small ergonomic helper for tests that build args then swap the trace.
trait WithTrace {
    fn with_trace(self, trace: Vec<SubjectSample>) -> ClipAutoReframeArgs;
}

impl WithTrace for ClipAutoReframeArgs {
    fn with_trace(mut self, trace: Vec<SubjectSample>) -> ClipAutoReframeArgs {
        self.subject_trace = trace;
        self
    }
}
