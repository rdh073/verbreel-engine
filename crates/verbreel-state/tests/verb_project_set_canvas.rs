//! Tests for `project.set_canvas` (§2.10) — the second production verb.
//!
//! Covers `compute_patch` happy paths (minimal, all-optionals,
//! partial-update), the malformed-arg matrix (canvas string,
//! width/height range, background hex, pixel-aspect floor), the
//! `data_envelope` helper, the reconstructor round-trip via
//! [`validate_reconstructors`], and one end-to-end exercise through
//! [`ProjectStore::mutate_via_verb`] proving the verb is wired into
//! the kernel's default registry + fixtures.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::{
    CANVAS_MAX_DIM, CANVAS_MIN_DIM, Canvas, MutateOutcome, PIXEL_ASPECT_MIN, Project,
    ProjectSetCanvasArgs, ProjectSetCanvasError, ProjectSetCanvasVerb, ProjectStore, RecordedEvent,
    VerbRegistry, default_fixtures, default_registry, validate_reconstructors,
    verbs::project_set_canvas::{compute_patch, data_envelope},
};
use verbreel_types::ProjectId;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";

/// Load the canonical empty-project fixture as the prior state.
fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture → Project")
}

/// The fixture project's own id — every test reuses it as
/// `args.project_id` so the round-trip envelope's `project_id` field
/// matches by construction.
fn fixture_project_id() -> ProjectId {
    empty_project().id
}

/// Build a [`Project`] whose `canvas` field is set to `c`. Everything
/// else mirrors `empty_project_create.json`.
fn project_with_canvas(c: Canvas) -> Project {
    let mut p = empty_project();
    p.canvas = c;
    p
}

/// Convenience: build `ProjectSetCanvasArgs` with the fixture project
/// id and the supplied `canvas`. All optional fields default to `None`
/// (partial-update: leave the prior canvas's background/pixel-aspect
/// unchanged).
fn minimal_args(canvas: &str) -> ProjectSetCanvasArgs {
    ProjectSetCanvasArgs {
        project_id: fixture_project_id(),
        canvas: canvas.to_string(),
        background: None,
        pixel_aspect_num: None,
        pixel_aspect_den: None,
    }
}

/// Convenience: extract the `replace`-op value off the patch. Panics
/// (test-only) if the patch shape isn't the wholesale-replace form.
fn patch_canvas_value(patch: &Value) -> Value {
    let arr = patch.as_array().expect("patch is an array");
    assert_eq!(arr.len(), 1, "wholesale-replace patch has exactly one op");
    let op = arr[0].as_object().expect("op is an object");
    assert_eq!(op.get("op").and_then(Value::as_str), Some("replace"));
    assert_eq!(op.get("path").and_then(Value::as_str), Some("/canvas"));
    op.get("value")
        .cloned()
        .expect("replace op carries a value")
}

// ---------------------------------------------------------------------
// Happy paths
// ---------------------------------------------------------------------

#[test]
fn compute_patch_minimal_canvas_succeeds() {
    // No optionals supplied — width/height update, background and
    // pixel-aspect stay at the prior values (default `#000000ff` / 1/1).
    let prior = empty_project();
    let args = minimal_args("1920x1080");
    let (patch, new_canvas) =
        compute_patch(&prior, &args).expect("happy-path minimal compute_patch");

    assert_eq!(new_canvas.width, 1920);
    assert_eq!(new_canvas.height, 1080);
    assert_eq!(new_canvas.background, prior.canvas.background);
    assert_eq!(new_canvas.pixel_aspect_num, prior.canvas.pixel_aspect_num);
    assert_eq!(new_canvas.pixel_aspect_den, prior.canvas.pixel_aspect_den);

    // Patch's /canvas value carries the new dims.
    let patch_canvas: Canvas =
        serde_json::from_value(patch_canvas_value(&patch)).expect("patch /canvas → Canvas");
    assert_eq!(patch_canvas, new_canvas);
}

#[test]
fn compute_patch_with_all_optionals_succeeds() {
    let prior = empty_project();
    let args = ProjectSetCanvasArgs {
        project_id: fixture_project_id(),
        canvas: "1280x720".to_string(),
        background: Some("#abcdef12".to_string()),
        pixel_aspect_num: Some(4),
        pixel_aspect_den: Some(3),
    };
    let (_, new_canvas) =
        compute_patch(&prior, &args).expect("happy-path all-optionals compute_patch");

    assert_eq!(new_canvas.width, 1280);
    assert_eq!(new_canvas.height, 720);
    assert_eq!(new_canvas.background, "#abcdef12");
    assert_eq!(new_canvas.pixel_aspect_num, 4);
    assert_eq!(new_canvas.pixel_aspect_den, 3);
}

#[test]
fn compute_patch_partial_update_preserves_existing() {
    // Prior carries a non-default background; args supplies only the
    // canvas string; new canvas must keep the prior background.
    let prior = project_with_canvas(Canvas {
        width: 1080,
        height: 1920,
        background: "#ff0000ff".to_string(),
        pixel_aspect_num: 2,
        pixel_aspect_den: 1,
    });
    let args = minimal_args("640x480");
    let (_, new_canvas) = compute_patch(&prior, &args).expect("partial-update compute_patch ok");

    assert_eq!(new_canvas.width, 640);
    assert_eq!(new_canvas.height, 480);
    // Untouched fields preserved.
    assert_eq!(new_canvas.background, "#ff0000ff");
    assert_eq!(new_canvas.pixel_aspect_num, 2);
    assert_eq!(new_canvas.pixel_aspect_den, 1);
}

#[test]
fn compute_patch_background_uppercase_normalized_to_lowercase() {
    // The Color newtype's regex `^#[0-9a-fA-F]{8}$` accepts mixed case
    // and lowercases on construction (§0.5.2 canonical form). The verb
    // routes through Color::try_from so an uppercase background string
    // is accepted but stored as lowercase. This deviates from the
    // task-prompt's "uppercase rejected" expectation — see the
    // module-level rustdoc and the task response's Deviations section
    // for the rationale (Color/spec both allow mixed case).
    let prior = empty_project();
    let args = ProjectSetCanvasArgs {
        project_id: fixture_project_id(),
        canvas: "320x240".to_string(),
        background: Some("#FFFFFFFF".to_string()),
        pixel_aspect_num: None,
        pixel_aspect_den: None,
    };
    let (_, new_canvas) =
        compute_patch(&prior, &args).expect("uppercase background normalizes, not rejects");
    assert_eq!(new_canvas.background, "#ffffffff");
}

// ---------------------------------------------------------------------
// Canvas-string error paths
// ---------------------------------------------------------------------

#[test]
fn compute_patch_canvas_malformed_errors() {
    let prior = empty_project();
    let args = minimal_args("1920");
    let err = compute_patch(&prior, &args).expect_err("missing-x must reject");
    assert_eq!(
        err,
        ProjectSetCanvasError::CanvasMalformed {
            value: "1920".to_string()
        }
    );
}

#[test]
fn compute_patch_canvas_lowercase_x_only() {
    // Uppercase `X` is NOT in the regex `^[0-9]+x[0-9]+$` — must
    // reject (matches §2.10's literal pattern).
    let prior = empty_project();
    let args = minimal_args("1920X1080");
    let err = compute_patch(&prior, &args).expect_err("uppercase X must reject");
    assert_eq!(
        err,
        ProjectSetCanvasError::CanvasMalformed {
            value: "1920X1080".to_string()
        }
    );
}

// ---------------------------------------------------------------------
// Width / height range error paths
// ---------------------------------------------------------------------

#[test]
fn compute_patch_width_below_min() {
    let prior = empty_project();
    let args = minimal_args("8x100");
    match compute_patch(&prior, &args).expect_err("width below min must reject") {
        ProjectSetCanvasError::WidthOutOfRange { value, min, max } => {
            assert_eq!(value, 8);
            assert_eq!(min, CANVAS_MIN_DIM);
            assert_eq!(max, CANVAS_MAX_DIM);
        }
        other => panic!("expected WidthOutOfRange, got {other:?}"),
    }
}

#[test]
fn compute_patch_width_above_max() {
    let prior = empty_project();
    let args = minimal_args("9000x100");
    match compute_patch(&prior, &args).expect_err("width above max must reject") {
        ProjectSetCanvasError::WidthOutOfRange { value, min, max } => {
            assert_eq!(value, 9000);
            assert_eq!(min, CANVAS_MIN_DIM);
            assert_eq!(max, CANVAS_MAX_DIM);
        }
        other => panic!("expected WidthOutOfRange, got {other:?}"),
    }
}

#[test]
fn compute_patch_height_below_min() {
    let prior = empty_project();
    let args = minimal_args("100x8");
    match compute_patch(&prior, &args).expect_err("height below min must reject") {
        ProjectSetCanvasError::HeightOutOfRange { value, min, max } => {
            assert_eq!(value, 8);
            assert_eq!(min, CANVAS_MIN_DIM);
            assert_eq!(max, CANVAS_MAX_DIM);
        }
        other => panic!("expected HeightOutOfRange, got {other:?}"),
    }
}

#[test]
fn compute_patch_height_above_max() {
    let prior = empty_project();
    let args = minimal_args("100x9000");
    match compute_patch(&prior, &args).expect_err("height above max must reject") {
        ProjectSetCanvasError::HeightOutOfRange { value, min, max } => {
            assert_eq!(value, 9000);
            assert_eq!(min, CANVAS_MIN_DIM);
            assert_eq!(max, CANVAS_MAX_DIM);
        }
        other => panic!("expected HeightOutOfRange, got {other:?}"),
    }
}

#[test]
fn compute_patch_width_overflow_routes_to_out_of_range() {
    // A capture group above u32::MAX surfaces as WidthOutOfRange with
    // value: u32::MAX (it's, by definition, above the 8192 cap).
    // Documented design choice — see the module rustdoc on
    // parse_canvas_dims.
    let prior = empty_project();
    let args = minimal_args("99999999999x100");
    match compute_patch(&prior, &args).expect_err("width overflow must reject") {
        ProjectSetCanvasError::WidthOutOfRange { value, max, .. } => {
            assert_eq!(value, u32::MAX);
            assert_eq!(max, CANVAS_MAX_DIM);
        }
        other => panic!("expected WidthOutOfRange on overflow, got {other:?}"),
    }
}

// ---------------------------------------------------------------------
// Background error paths
// ---------------------------------------------------------------------

#[test]
fn compute_patch_background_invalid_hex() {
    let prior = empty_project();
    let args = ProjectSetCanvasArgs {
        project_id: fixture_project_id(),
        canvas: "640x480".to_string(),
        background: Some("rgb(0,0,0)".to_string()),
        pixel_aspect_num: None,
        pixel_aspect_den: None,
    };
    match compute_patch(&prior, &args).expect_err("non-hex background must reject") {
        ProjectSetCanvasError::BackgroundInvalid { detail } => {
            assert!(detail.contains("Color"), "detail mentions Color: {detail}");
        }
        other => panic!("expected BackgroundInvalid, got {other:?}"),
    }
}

#[test]
fn compute_patch_background_missing_alpha() {
    // 6-digit hex doesn't match the schema's 8-digit Color pattern.
    let prior = empty_project();
    let args = ProjectSetCanvasArgs {
        project_id: fixture_project_id(),
        canvas: "640x480".to_string(),
        background: Some("#ffffff".to_string()),
        pixel_aspect_num: None,
        pixel_aspect_den: None,
    };
    let err = compute_patch(&prior, &args).expect_err("missing-alpha background must reject");
    assert!(
        matches!(err, ProjectSetCanvasError::BackgroundInvalid { .. }),
        "expected BackgroundInvalid, got {err:?}"
    );
}

// ---------------------------------------------------------------------
// Pixel-aspect error paths
// ---------------------------------------------------------------------

#[test]
fn compute_patch_pixel_aspect_num_zero() {
    let prior = empty_project();
    let args = ProjectSetCanvasArgs {
        project_id: fixture_project_id(),
        canvas: "640x480".to_string(),
        background: None,
        pixel_aspect_num: Some(0),
        pixel_aspect_den: None,
    };
    match compute_patch(&prior, &args).expect_err("pixel_aspect_num=0 must reject") {
        ProjectSetCanvasError::PixelAspectNumOutOfRange { value, min } => {
            assert_eq!(value, 0);
            assert_eq!(min, PIXEL_ASPECT_MIN);
        }
        other => panic!("expected PixelAspectNumOutOfRange, got {other:?}"),
    }
}

#[test]
fn compute_patch_pixel_aspect_den_zero() {
    let prior = empty_project();
    let args = ProjectSetCanvasArgs {
        project_id: fixture_project_id(),
        canvas: "640x480".to_string(),
        background: None,
        pixel_aspect_num: None,
        pixel_aspect_den: Some(0),
    };
    match compute_patch(&prior, &args).expect_err("pixel_aspect_den=0 must reject") {
        ProjectSetCanvasError::PixelAspectDenOutOfRange { value, min } => {
            assert_eq!(value, 0);
            assert_eq!(min, PIXEL_ASPECT_MIN);
        }
        other => panic!("expected PixelAspectDenOutOfRange, got {other:?}"),
    }
}

// ---------------------------------------------------------------------
// Boundary dimensions
// ---------------------------------------------------------------------

#[test]
fn boundary_dimensions_16x16_accepted() {
    let prior = empty_project();
    let args = minimal_args("16x16");
    let (_, new_canvas) = compute_patch(&prior, &args).expect("16x16 must be accepted");
    assert_eq!(new_canvas.width, CANVAS_MIN_DIM);
    assert_eq!(new_canvas.height, CANVAS_MIN_DIM);
}

#[test]
fn boundary_dimensions_8192x8192_accepted() {
    let prior = empty_project();
    let args = minimal_args("8192x8192");
    let (_, new_canvas) = compute_patch(&prior, &args).expect("8192x8192 must be accepted");
    assert_eq!(new_canvas.width, CANVAS_MAX_DIM);
    assert_eq!(new_canvas.height, CANVAS_MAX_DIM);
}

// ---------------------------------------------------------------------
// Envelope helper
// ---------------------------------------------------------------------

#[test]
fn data_envelope_returns_post_state_canvas() {
    let post_state = project_with_canvas(Canvas {
        width: 1920,
        height: 1080,
        background: "#deadbeef".to_string(),
        pixel_aspect_num: 1,
        pixel_aspect_den: 1,
    });
    let args = minimal_args("1920x1080");
    let env = data_envelope(&args, &post_state);
    assert_eq!(env.project_id, args.project_id);
    assert_eq!(env.canvas, post_state.canvas);
}

// ---------------------------------------------------------------------
// Reconstructor round-trip — the §0.8 startup-gate exercise
// ---------------------------------------------------------------------

#[test]
fn reconstructor_round_trip() {
    let prior = empty_project();
    let args = ProjectSetCanvasArgs {
        project_id: fixture_project_id(),
        canvas: "1920x1080".to_string(),
        background: Some("#11223344".to_string()),
        pixel_aspect_num: Some(2),
        pixel_aspect_den: Some(1),
    };

    let (patch, new_canvas) = compute_patch(&prior, &args).expect("compute_patch ok");

    let mut post_state = prior.clone();
    post_state.canvas = new_canvas;

    let expected_envelope = data_envelope(&args, &post_state);
    let expected_data = serde_json::to_value(&expected_envelope).expect("envelope → Value");

    let recorded = RecordedEvent {
        verb: "project.set_canvas".to_owned(),
        args: serde_json::to_value(&args).expect("args → Value"),
        patch,
        warnings: vec![],
        post_state,
        expected_data,
    };

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(ProjectSetCanvasVerb))
        .expect("register ok");

    let report = validate_reconstructors(&registry, &[recorded])
        .expect("reconstructor round-trip must pass");
    assert_eq!(report.verbs_checked, vec!["project.set_canvas"]);
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
        "canvas": "1920x1080",
        "background": "#abcdef12",
        "pixel_aspect_num": 4,
        "pixel_aspect_den": 3,
    });

    let outcome = store
        .mutate_via_verb("project.set_canvas", args, None)
        .expect("mutate_via_verb happy path");

    let MutateOutcome::Applied { event_id, data } = outcome else {
        panic!("happy path must return Applied, got {outcome:?}");
    };

    assert_eq!(
        store.last_applied_event_id(),
        Some(event_id),
        "store tracks the just-applied event"
    );

    // In-memory project reflects the new canvas.
    assert_eq!(store.project().canvas.width, 1920);
    assert_eq!(store.project().canvas.height, 1080);
    assert_eq!(store.project().canvas.background, "#abcdef12");
    assert_eq!(store.project().canvas.pixel_aspect_num, 4);
    assert_eq!(store.project().canvas.pixel_aspect_den, 3);

    // Data envelope shape: `{ project_id, canvas: {...} }`.
    let expected = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "canvas": {
            "width": 1920,
            "height": 1080,
            "background": "#abcdef12",
            "pixel_aspect_num": 4,
            "pixel_aspect_den": 3,
        },
    });
    assert_eq!(
        data, expected,
        "data envelope is the verb's typed `{{ project_id, canvas }}` shape"
    );
}
