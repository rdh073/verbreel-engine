//! `project.set_canvas` (§2.10) — second production verb in the engine.
//!
//! ## Spec quote (`spec/commands/project.md` §2.10, verbatim)
//!
//! > Resizes the project canvas (`Project.canvas.width` / `height` /
//! > `background` / pixel aspect). Clip transforms (`x`, `y`,
//! > `scale_x`, `scale_y`) are **not** rescaled or repositioned — they
//! > continue to reference their original pixel coordinates, which may
//! > now place clips outside the visible area or leave background gaps.
//! > Agents that want clips to remain centered should follow up with
//! > `clip.set_transform` per clip.
//! >
//! > **Args**: `project_id: string`, `canvas: "<W>x<H>"`,
//! > `background?: string`, `pixel_aspect_num?: integer`,
//! > `pixel_aspect_den?: integer`. `width` and `height` must satisfy
//! > the schema's `Canvas` constraints (16–8192 each). Omitted optional
//! > fields are left unchanged (this is a **partial update**, not a
//! > replace).
//! >
//! > **Returns** (`data`): `{ project_id: string; canvas: { width:
//! > integer; height: integer; background: string; pixel_aspect_num:
//! > integer; pixel_aspect_den: integer } }`.
//! >
//! > **Errors**: `E_SCHEMA_VIOLATION` (out-of-range dimensions,
//! > malformed background hex, non-positive pixel aspect, or malformed
//! > `canvas` string — not matching `^[0-9]+x[0-9]+$`).
//! >
//! > **Warnings**: `W_CANVAS_CLIPS_OUT_OF_FRAME` — emitted when any
//! > clip's transformed bounding box (after the resize) falls fully
//! > outside the new canvas.
//!
//! ## Partial-update semantics
//!
//! Width and height are **always** overwritten from the parsed `canvas`
//! arg — there is no way to resize without supplying both. The three
//! optional fields (`background`, `pixel_aspect_num`, `pixel_aspect_den`)
//! follow partial-update semantics:
//!
//! - **Omitted** (`None` in [`ProjectSetCanvasArgs`]) → the
//!   corresponding [`Canvas`] field is left unchanged from `prior`.
//! - **Supplied** → the field is validated then overwritten.
//!
//! This matches the §2.10 wording "omitted optional fields are left
//! unchanged".
//!
//! ## Order of operations
//!
//! Given `(prior, args)`:
//!
//! 1. Parse `args.canvas` against `^([0-9]+)x([0-9]+)$`. Reject
//!    malformed strings as [`ProjectSetCanvasError::CanvasMalformed`].
//!    A capture group whose numeric value overflows [`u32`] is
//!    surfaced as the corresponding `*OutOfRange` variant with
//!    `value: u32::MAX` — the value is, by definition, above the cap.
//! 2. Validate width and height fall in
//!    `[CANVAS_MIN_DIM, CANVAS_MAX_DIM]`. Reject as
//!    [`ProjectSetCanvasError::WidthOutOfRange`] /
//!    [`ProjectSetCanvasError::HeightOutOfRange`]. Width is checked
//!    before height (declaration order; both checks are independent).
//! 3. Seed `new_canvas` by cloning `prior.canvas`. Overwrite
//!    `width` / `height` with the freshly-parsed values.
//! 4. If `args.background.is_some()`: validate via
//!    [`Color::try_from`]. On success, store the **lowercased** form
//!    ([`Color::as_str`] is canonical per §0.5.2) back as the
//!    `background` string. On failure, return
//!    [`ProjectSetCanvasError::BackgroundInvalid`].
//! 5. If `args.pixel_aspect_num.is_some()`: validate `>= PIXEL_ASPECT_MIN`,
//!    store. Else leave unchanged.
//! 6. If `args.pixel_aspect_den.is_some()`: validate `>= PIXEL_ASPECT_MIN`,
//!    store. Else leave unchanged.
//! 7. Emit the patch as a single `replace` op on `/canvas`. Same
//!    wholesale-replace strategy as `project.set_metadata` — simpler
//!    than per-field micro-ops, and the reconstructor reads
//!    `post_state.canvas` anyway so per-field minimization is a future
//!    optimization not required for §0.8 correctness.
//!
//! ## Background validation without typed promotion
//!
//! [`Canvas::background`] is currently `String`, not the [`Color`]
//! newtype, per the deferred-newtype note in
//! `crates/verbreel-state/src/canvas.rs`. This verb validates the
//! supplied background through [`Color::try_from`] (so callers get the
//! same regex + lowercase-normalization the rest of the engine uses)
//! but stores the validated string back via [`Color::as_str`] — the
//! lower-cased canonical form per §0.5.2. The verb does NOT promote
//! the [`Canvas`] field's static type; that's a separate slice that
//! must touch `asset_meta`, `clip`, and every Canvas serializer
//! simultaneously.
//!
//! The schema's [`Color`] pattern (`^#[0-9a-fA-F]{8}$`) accepts mixed
//! case; the engine normalizes to lowercase on persist. A caller that
//! sends `#FFFFFFFF` will see the post-state carry `#ffffffff`. This is
//! the same behavior every other [`Color`] field uses today.
//!
//! ## Reconstructor purity (§0.8)
//!
//! The envelope `data` field is `{ project_id, canvas }` where:
//!
//! - `project_id` is `args.project_id` (recorded verbatim).
//! - `canvas` is the post-state `Project.canvas` (the value the patch
//!   wrote on `/canvas`, applied by the kernel to produce the
//!   post-state).
//!
//! Both fields are derivable from `(args, post_state)` alone — no
//! `randomUUID()`, no wall-clock, no patch inspection, no
//! `warnings.details` escape hatch needed. The
//! [`ProjectSetCanvasVerb`] impl exercises this directly and is
//! registered against [`crate::validate_reconstructors`] in the
//! kernel's `default_fixtures` set to lock the round-trip.
//!
//! ## Out of scope (this slice)
//!
//! - **`W_CANVAS_CLIPS_OUT_OF_FRAME` warning emission** — requires
//!   walking every clip's transform, computing the post-transform
//!   bounding box, and comparing against the new canvas rectangle.
//!   That's a clip-layout slice; it lands separately.
//! - **`Canvas.background` `String` → [`Color`] typed promotion** —
//!   the typed [`Canvas`] field stays `String`. Background goes
//!   through [`Color::try_from`] for validation but is stored back as
//!   a `String`. Promotion is a separate Canvas-typing slice that
//!   touches every Canvas (de)serializer simultaneously.
//! - **`verbreel-args` schema crate population** — serde derive on
//!   [`ProjectSetCanvasArgs`] is sufficient for this slice; the
//!   JSON-schema declaration belongs to that crate's future work.

use crate::{Asset, Clip, TrackKind};
use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::ProjectId;

use crate::canvas::Canvas;
use crate::newtypes::Color;
use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// Minimum canvas width / height per `$defs/Canvas` in
/// `spec/project-schema.json`. Inclusive.
pub const CANVAS_MIN_DIM: u32 = 16;

/// Maximum canvas width / height per `$defs/Canvas` in
/// `spec/project-schema.json`. Inclusive.
pub const CANVAS_MAX_DIM: u32 = 8192;

/// Minimum pixel-aspect numerator / denominator per `$defs/Canvas`.
/// `0` is rejected — pixel aspects must be positive integers.
pub const PIXEL_ASPECT_MIN: u32 = 1;

/// Warning emitted when one or more clip AABBs become fully visible
/// outside the resized canvas.
pub const W_CANVAS_CLIPS_OUT_OF_FRAME: &str = "W_CANVAS_CLIPS_OUT_OF_FRAME";

/// Args for `project.set_canvas`. Mirrors the §2.10 args list.
///
/// `canvas` is the raw `"<W>x<H>"` string; parsing into typed
/// `(u32, u32)` happens inside [`compute_patch`] so a malformed string
/// surfaces as the typed [`ProjectSetCanvasError::CanvasMalformed`]
/// rather than a serde error (which would have to be downstream-decoded
/// into the verb error taxonomy anyway).
///
/// Each `Option<*>` field uses `#[serde(default,
/// skip_serializing_if = "Option::is_none")]` so omitted optionals
/// round-trip cleanly and never appear as `null` on the wire (matching
/// the schema's "omitted = unchanged" semantics).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectSetCanvasArgs {
    /// Target project id. Strongly-typed [`ProjectId`] so a verb-args
    /// payload carrying a non-`UUIDv7` string fails serde deserialize
    /// rather than reaching [`compute_patch`].
    pub project_id: ProjectId,

    /// Canvas dimensions as `"<W>x<H>"`. Required — the only field
    /// without partial-update semantics. Validated by
    /// [`compute_patch`] against `^[0-9]+x[0-9]+$` and the
    /// `[CANVAS_MIN_DIM, CANVAS_MAX_DIM]` range.
    pub canvas: String,

    /// Optional background color hex (`#rrggbbaa`, mixed-case accepted,
    /// normalized to lowercase per §0.5.2). Omitted → leave the
    /// existing background unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,

    /// Optional pixel-aspect numerator (≥ `PIXEL_ASPECT_MIN`). Omitted
    /// → leave the existing value unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pixel_aspect_num: Option<u32>,

    /// Optional pixel-aspect denominator (≥ `PIXEL_ASPECT_MIN`).
    /// Omitted → leave the existing value unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pixel_aspect_den: Option<u32>,
}

/// Envelope `data` shape returned by `project.set_canvas`. Per §2.10:
/// the FULL post-patch canvas plus the target project id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectSetCanvasData {
    /// The target project id (echoed from `args.project_id`).
    pub project_id: ProjectId,
    /// The full post-patch [`Canvas`] (cloned from `post_state.canvas`).
    pub canvas: Canvas,
}

/// Verb-level errors surfaced by [`compute_patch`]. Every variant maps
/// onto §2.10's `E_SCHEMA_VIOLATION` once wired into the kernel
/// error-translation layer. All routed through
/// [`VerbError::BadArgs`] per the §0.7 / §2.10 arg-shape failure
/// taxonomy ([`VerbError::InvariantViolation`] is reserved for §0.13
/// engine invariants, none of which apply here — these are arg-shape
/// validation failures).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProjectSetCanvasError {
    /// `args.canvas` did not match `^[0-9]+x[0-9]+$`. Catches missing
    /// `x`, uppercase `X`, leading/trailing whitespace, non-digit
    /// characters, and the empty string.
    #[error("project.set_canvas: `canvas` must match `^[0-9]+x[0-9]+$`, got {value:?}")]
    CanvasMalformed {
        /// The offending input string.
        value: String,
    },

    /// Width parsed but fell outside `[min, max]`. A capture group that
    /// overflows [`u32`] surfaces here with `value: u32::MAX` — the
    /// numeric value is, by definition, above the cap.
    #[error("project.set_canvas: width {value} outside [{min}, {max}]")]
    WidthOutOfRange {
        /// The parsed width (or [`u32::MAX`] on parse overflow).
        value: u32,
        /// [`CANVAS_MIN_DIM`].
        min: u32,
        /// [`CANVAS_MAX_DIM`].
        max: u32,
    },

    /// Height parsed but fell outside `[min, max]`. A capture group
    /// that overflows [`u32`] surfaces here with `value: u32::MAX`.
    #[error("project.set_canvas: height {value} outside [{min}, {max}]")]
    HeightOutOfRange {
        /// The parsed height (or [`u32::MAX`] on parse overflow).
        value: u32,
        /// [`CANVAS_MIN_DIM`].
        min: u32,
        /// [`CANVAS_MAX_DIM`].
        max: u32,
    },

    /// `args.background` was supplied but did not satisfy the [`Color`]
    /// newtype's `^#[0-9a-fA-F]{8}$` pattern (mixed-case accepted;
    /// lowercased on persist per §0.5.2).
    #[error("project.set_canvas: `background` invalid: {detail}")]
    BackgroundInvalid {
        /// The underlying [`crate::newtypes::AssetNewtypeError`]
        /// description.
        detail: String,
    },

    /// `args.pixel_aspect_num` was supplied but `< PIXEL_ASPECT_MIN`.
    #[error("project.set_canvas: pixel_aspect_num {value} < {min}")]
    PixelAspectNumOutOfRange {
        /// The offending numerator value.
        value: u32,
        /// [`PIXEL_ASPECT_MIN`].
        min: u32,
    },

    /// `args.pixel_aspect_den` was supplied but `< PIXEL_ASPECT_MIN`.
    #[error("project.set_canvas: pixel_aspect_den {value} < {min}")]
    PixelAspectDenOutOfRange {
        /// The offending denominator value.
        value: u32,
        /// [`PIXEL_ASPECT_MIN`].
        min: u32,
    },
}

/// `^([0-9]+)x([0-9]+)$` — the canvas-dimensions parse regex. Anchored
/// full-string; lowercase `x` only (uppercase `X` would surface as
/// [`ProjectSetCanvasError::CanvasMalformed`] per §2.10's literal
/// pattern). Compiled exactly once per process via [`OnceLock`],
/// matching the existing `newtypes.rs` style.
fn canvas_dims_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^([0-9]+)x([0-9]+)$").expect("compile-time-correct regex"))
}

/// Parse `"<W>x<H>"` into `(width, height)` `u32`s.
///
/// A capture group that overflows [`u32`] is rejected as the
/// corresponding `*OutOfRange` variant with `value: u32::MAX` — the
/// numeric value is unambiguously above the cap (which is 8192). The
/// alternative (a dedicated `*ParseOverflow` variant) is correct too,
/// but `*OutOfRange` keeps the taxonomy at six variants and the error
/// message still tells the caller the input was outside the legal
/// range.
///
/// # Errors
///
/// - [`ProjectSetCanvasError::CanvasMalformed`] if `s` does not match
///   `^[0-9]+x[0-9]+$`.
/// - [`ProjectSetCanvasError::WidthOutOfRange`] /
///   [`ProjectSetCanvasError::HeightOutOfRange`] if a capture group
///   overflows [`u32`].
fn parse_canvas_dims(s: &str) -> Result<(u32, u32), ProjectSetCanvasError> {
    let caps =
        canvas_dims_re()
            .captures(s)
            .ok_or_else(|| ProjectSetCanvasError::CanvasMalformed {
                value: s.to_string(),
            })?;
    // Both capture groups are guaranteed present by the regex shape.
    let w_str = caps.get(1).expect("regex guarantees group 1").as_str();
    let h_str = caps.get(2).expect("regex guarantees group 2").as_str();
    let width = w_str
        .parse::<u32>()
        .map_err(|_| ProjectSetCanvasError::WidthOutOfRange {
            value: u32::MAX,
            min: CANVAS_MIN_DIM,
            max: CANVAS_MAX_DIM,
        })?;
    let height = h_str
        .parse::<u32>()
        .map_err(|_| ProjectSetCanvasError::HeightOutOfRange {
            value: u32::MAX,
            min: CANVAS_MIN_DIM,
            max: CANVAS_MAX_DIM,
        })?;
    Ok((width, height))
}

/// Axis-aligned bounding box of a clip's source rectangle after
/// transform, in canvas coordinates.
///
/// Transform order:
///
/// 1. Subtract anchor point (`src_w * anchor_x`, `src_h * anchor_y`).
/// 2. Scale by (`scale_x`, `scale_y`).
/// 3. Rotate by `rotation_deg` (positive means clockwise, in
///    screen-space with Y down).
/// 4. Translate by (`x`, `y`).
///
/// The output is `(min_x, min_y, max_x, max_y)` across the four
/// source corners.
fn clip_bounding_box_aabb(clip: &Clip, src_w: u32, src_h: u32) -> (f64, f64, f64, f64) {
    let w = f64::from(src_w);
    let h = f64::from(src_h);
    let transform = &clip.transform;

    let ax = transform.anchor_x * w;
    let ay = transform.anchor_y * h;

    let theta = transform.rotation_deg.to_radians();
    let (sin_t, cos_t) = theta.sin_cos();

    let corners = [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)];

    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;

    for (cx, cy) in corners {
        let dx = cx - ax;
        let dy = cy - ay;
        let scaled_x = dx * transform.scale_x;
        let scaled_y = dy * transform.scale_y;
        let rx = scaled_x * cos_t - scaled_y * sin_t;
        let ry = scaled_x * sin_t + scaled_y * cos_t;
        let final_x = rx + transform.x;
        let final_y = ry + transform.y;

        min_x = min_x.min(final_x);
        min_y = min_y.min(final_y);
        max_x = max_x.max(final_x);
        max_y = max_y.max(final_y);
    }

    (min_x, min_y, max_x, max_y)
}

/// Returns `true` if an AABB has no overlap with the canvas
/// rectangle.
///
/// Full-off-frame means all points are strictly outside one side of
/// the canvas:
/// - right of left edge (`max_x < 0`) or
/// - left of right edge (`min_x > canvas_w`) or
/// - below top (`max_y < 0`) or
/// - above bottom (`min_y > canvas_h`).
fn is_fully_outside_canvas(aabb: (f64, f64, f64, f64), canvas_w: u32, canvas_h: u32) -> bool {
    let (min_x, min_y, max_x, max_y) = aabb;
    let canvas_w = f64::from(canvas_w);
    let canvas_h = f64::from(canvas_h);
    max_x < 0.0 || min_x > canvas_w || max_y < 0.0 || min_y > canvas_h
}

/// Resolve a clip's source dimensions from prior assets.
///
/// Returns `Some((width, height))` for display-capable assets
/// (`Video` / `Image`) and `None` for everything else.
fn resolve_clip_source_dims(prior: &Project, clip: &Clip) -> Option<(u32, u32)> {
    let asset_id = clip.asset_id.id()?;
    let asset = prior.assets.iter().find(|a| a.id() == asset_id)?;
    match asset {
        Asset::Video(video) => Some((video.metadata.width, video.metadata.height)),
        Asset::Image(image) => Some((image.metadata.width, image.metadata.height)),
        _ => None,
    }
}

/// Walk all video tracks and emit the IDs of clips whose transformed
/// source rects are fully outside the new canvas.
///
/// Output is lexicographically sorted for deterministic serialisation.
fn compute_affected_clips(prior: &Project, canvas_w: u32, canvas_h: u32) -> Vec<String> {
    let mut ids = Vec::new();

    for track in &prior.tracks {
        if track.kind != TrackKind::Video {
            continue;
        }

        for clip in &track.clips {
            let Some((src_w, src_h)) = resolve_clip_source_dims(prior, clip) else {
                continue;
            };
            let aabb = clip_bounding_box_aabb(clip, src_w, src_h);
            if is_fully_outside_canvas(aabb, canvas_w, canvas_h) {
                ids.push(clip.id.to_string());
            }
        }
    }

    ids.sort_unstable();
    ids
}

/// Compute the RFC 6902 patch and post-patch [`Canvas`] for a
/// `project.set_canvas` call.
///
/// Pure function — no I/O, no clock, no RNG. Builds the post-patch
/// canvas per the order-of-operations at the module level, validates
/// every field against the §2.10 / `$defs/Canvas` constraints, and
/// returns:
///
/// - the patch as a [`serde_json::Value`] (an RFC 6902 op array with a
///   single `replace` op on `/canvas`), and
/// - the post-patch [`Canvas`] (the value of the patch's `replace` op,
///   returned separately so the caller doesn't have to walk the patch
///   to recover it — `data_envelope` consumes it directly).
///
/// # Errors
///
/// Returns [`ProjectSetCanvasError`]:
///
/// - [`ProjectSetCanvasError::CanvasMalformed`] when `args.canvas`
///   does not match `^[0-9]+x[0-9]+$`.
/// - [`ProjectSetCanvasError::WidthOutOfRange`] /
///   [`ProjectSetCanvasError::HeightOutOfRange`] when a parsed
///   dimension falls outside `[CANVAS_MIN_DIM, CANVAS_MAX_DIM]` (or
///   when the capture-group parse overflows [`u32`]).
/// - [`ProjectSetCanvasError::BackgroundInvalid`] when `args.background`
///   was supplied but rejected by [`Color::try_from`].
/// - [`ProjectSetCanvasError::PixelAspectNumOutOfRange`] /
///   [`ProjectSetCanvasError::PixelAspectDenOutOfRange`] when a
///   supplied pixel-aspect value is `< PIXEL_ASPECT_MIN`.
///
/// # Panics
///
/// Panics only if [`serde_json::to_value`] on a [`Canvas`] fails —
/// which it can't: [`Canvas`] is a `derive(Serialize)` struct of
/// finite `u32` / `String` fields, every shape serializes cleanly. The
/// `.expect()` is a verb-author-bug surface that would only fire if
/// [`Canvas`] grew a custom `Serialize` impl that could error.
pub fn compute_patch(
    prior: &Project,
    args: &ProjectSetCanvasArgs,
) -> Result<(Value, Canvas, Vec<Value>), ProjectSetCanvasError> {
    // Parse the canvas dims first — every other check assumes we have
    // valid u32 width/height to compare against.
    let (width, height) = parse_canvas_dims(&args.canvas)?;

    if !(CANVAS_MIN_DIM..=CANVAS_MAX_DIM).contains(&width) {
        return Err(ProjectSetCanvasError::WidthOutOfRange {
            value: width,
            min: CANVAS_MIN_DIM,
            max: CANVAS_MAX_DIM,
        });
    }
    if !(CANVAS_MIN_DIM..=CANVAS_MAX_DIM).contains(&height) {
        return Err(ProjectSetCanvasError::HeightOutOfRange {
            value: height,
            min: CANVAS_MIN_DIM,
            max: CANVAS_MAX_DIM,
        });
    }

    // Seed from prior so the partial-update keeps untouched fields
    // intact. Width/height always overwrite.
    let mut new_canvas = prior.canvas.clone();
    new_canvas.width = width;
    new_canvas.height = height;

    if let Some(raw) = args.background.as_ref() {
        match Color::try_from(raw.clone()) {
            Ok(color) => {
                // Canvas.background is a typed Color newtype, so the
                // validated/normalized value can be assigned directly.
                new_canvas.background = color;
            }
            Err(e) => {
                return Err(ProjectSetCanvasError::BackgroundInvalid {
                    detail: e.to_string(),
                });
            }
        }
    }

    if let Some(num) = args.pixel_aspect_num {
        if num < PIXEL_ASPECT_MIN {
            return Err(ProjectSetCanvasError::PixelAspectNumOutOfRange {
                value: num,
                min: PIXEL_ASPECT_MIN,
            });
        }
        new_canvas.pixel_aspect_num = num;
    }

    if let Some(den) = args.pixel_aspect_den {
        if den < PIXEL_ASPECT_MIN {
            return Err(ProjectSetCanvasError::PixelAspectDenOutOfRange {
                value: den,
                min: PIXEL_ASPECT_MIN,
            });
        }
        new_canvas.pixel_aspect_den = den;
    }

    let canvas_value =
        serde_json::to_value(&new_canvas).expect("Canvas is a derive(Serialize) struct");
    let patch = json!([
        { "op": "replace", "path": "/canvas", "value": canvas_value }
    ]);

    let affected_clip_ids = compute_affected_clips(prior, new_canvas.width, new_canvas.height);
    let mut warnings = Vec::new();

    if !affected_clip_ids.is_empty() {
        warnings.push(json!({
            "code": W_CANVAS_CLIPS_OUT_OF_FRAME,
            "message": format!(
                "{} clip(s) now outside the new canvas",
                affected_clip_ids.len()
            ),
            "details": {
                "affected_clip_ids": affected_clip_ids
            }
        }));
    }

    Ok((patch, new_canvas, warnings))
}

/// Build the verb's envelope `data` from `(args, post_state)`. Pure —
/// this is the function the reconstructor exercises during replay.
///
/// Per §0.8 reconstructor purity, every field on
/// [`ProjectSetCanvasData`] is derivable from the recorded inputs:
///
/// - `project_id`: cloned from `args.project_id`.
/// - `canvas`: cloned from `post_state.canvas` (the value the patch
///   wrote during the original execution; the kernel replays the
///   patch to produce the same `post_state` before calling here).
#[must_use]
pub fn data_envelope(args: &ProjectSetCanvasArgs, post_state: &Project) -> ProjectSetCanvasData {
    ProjectSetCanvasData {
        project_id: args.project_id,
        canvas: post_state.canvas.clone(),
    }
}

/// Funnel [`ProjectSetCanvasError`] into the verb-layer [`VerbError`]
/// taxonomy. Every variant is an argument-shape failure
/// (`E_SCHEMA_VIOLATION` per §0.7 / §2.10), so every variant maps to
/// [`VerbError::BadArgs`]. None of these are §0.13 engine invariants
/// — the §0.13 invariants describe relations between graph objects
/// (e.g. clip-track biconditional, no-overlap, asset-hash uniqueness);
/// canvas-dimensions and color-format are arg-shape rules.
impl From<ProjectSetCanvasError> for VerbError {
    fn from(value: ProjectSetCanvasError) -> Self {
        VerbError::BadArgs {
            detail: value.to_string(),
        }
    }
}

/// The §0.8 verb for `project.set_canvas`. Registered in a
/// [`crate::VerbRegistry`] so the §0.8 startup gate
/// ([`crate::validate_reconstructors`]) can exercise its `reconstruct`
/// path against a recorded fixture, and so
/// [`crate::lifecycle::ProjectStore::mutate_via_verb`] can route
/// forward calls through its `compute_patch` path.
///
/// Pure on both legs of the trait — no I/O, no clock, no RNG, no
/// patch / warnings inspection during reconstruct. The forward leg
/// (`compute_patch`) deserialises `args` into [`ProjectSetCanvasArgs`],
/// calls the freestanding [`compute_patch`] helper, and converts the
/// resulting `Value` patch into a typed [`json_patch::Patch`].
#[derive(Debug, Default)]
pub struct ProjectSetCanvasVerb;

impl Verb for ProjectSetCanvasVerb {
    fn verb(&self) -> &'static str {
        "project.set_canvas"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        // Deserialize the raw JSON args into the typed struct. A serde
        // failure here is a [`VerbError::BadArgs`] — the args payload
        // is malformed (wrong shape, missing required fields, wrong
        // types).
        let typed: ProjectSetCanvasArgs =
            serde_json::from_value(args.clone()).map_err(|e| VerbError::BadArgs {
                detail: format!("project.set_canvas: args deserialize failed: {e}"),
            })?;

        // Run the freestanding compute_patch helper. Its typed error
        // funnels through the From impl above.
        let (patch_value, new_canvas, warnings) = compute_patch(prior, &typed)?;

        // Convert the RFC 6902 patch from `serde_json::Value` to the
        // typed `json_patch::Patch`. A failure here is a verb-author
        // bug — `compute_patch` produces a well-formed op array by
        // construction — so it surfaces as [`VerbError::Custom`].
        let patch: json_patch::Patch = serde_json::from_value(patch_value).map_err(|e| {
            VerbError::Custom(format!(
                "project.set_canvas: patch construction failed: {e}"
            ))
        })?;

        // Build the data envelope. `data_envelope` reads only
        // `(args.project_id, post_state.canvas)`; we synthesise the
        // post-state by cloning `prior` and overwriting `canvas` with
        // the value the patch will install. This matches what
        // `Project::apply(&patch)` would produce, but does not run the
        // §0.13 post-apply checks — that's the kernel's job in
        // `apply_write_ordering`.
        let mut post_state = prior.clone();
        post_state.canvas = new_canvas;
        let envelope = data_envelope(&typed, &post_state);
        let data = serde_json::to_value(&envelope).map_err(|e| {
            VerbError::Custom(format!(
                "project.set_canvas: data envelope serialize failed: {e}"
            ))
        })?;

        Ok((patch, data, warnings))
    }

    fn reconstruct(
        &self,
        args: &Value,
        _patch: &Value,
        _warnings: &[Value],
        post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let typed: ProjectSetCanvasArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "ProjectSetCanvasArgs",
            })?;
        let envelope = data_envelope(&typed, post_state);
        serde_json::to_value(&envelope).map_err(|e| ReconstructError::Custom(e.to_string()))
    }
}
