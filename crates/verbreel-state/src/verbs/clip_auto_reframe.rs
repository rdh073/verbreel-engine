//! `clip.auto_reframe` (issue #481) — recompose a clip for a new aspect
//! ratio by keeping a tracked subject centered.
//!
//! ## Behavior
//!
//! Given a subject trace (a time-ordered list of subject-center samples in
//! canvas pixel coordinates) and a `target_aspect`, the verb emits
//! `transform.scale_x` / `transform.scale_y` / `transform.x` /
//! `transform.y` keyframes that zoom the clip to fill the largest
//! target-aspect rectangle inscribed in the canvas and pan so the subject
//! sits at the canvas center. The result is an ordinary editable keyframe
//! track — the existing renderer already plays back `transform.*`
//! keyframes, so no render/codec change is required.
//!
//! ## Pan model (falsifiable assumption — pin with a render-parity test)
//!
//! The pan translation is derived under an **origin-scale** model: the fit
//! zoom maps a canvas point `p -> fit_scale * p`, so landing the subject at
//! the canvas center needs `transform.{x,y} = center - fit_scale * subject`.
//! This matches the spec's documented placement contract for
//! `tracker.apply` / `W_TRACKER_OUT_OF_BOUNDS` (spec/commands/tracker.md
//! §18.3), where `transform.x` directly positions the clip's transform
//! anchor in canvas space with a constant additive offset and *no*
//! pivot-correction term. It does NOT independently re-derive the renderer's
//! affine composition, which lives in `verbreel-render` and is out of this
//! crate's dependency scope (CLAUDE.md crate-graph rule). If the renderer
//! pivots the fit zoom about the clip-center anchor (`Transform.anchor_*`
//! defaults `0.5`) rather than the origin, the centered offset would instead
//! be `fit_scale * (center - subject)`. When `verbreel-render` lands the
//! transform composition, add a render-side parity test asserting the
//! *composed* transform places the subject at canvas center; that test —
//! not this crate — is the authority on the pivot, and will catch a model
//! mismatch deterministically.
//!
//! ## Why there is no `crop` field
//!
//! [`crate::clip::Clip`] has no `crop` field; recomposition is expressed
//! purely as `transform` (a `scale` that performs the equivalent zoom-crop
//! plus an `x`/`y` pan). The "crop window" is a derivation, not stored
//! state — its center is the (clamped) subject position and its size is the
//! inscribed target-aspect rectangle. This is the only deviation from the
//! plan's "crop + transform" wording: `transform` carries the whole effect.
//!
//! ## Why the trace arrives as args
//!
//! [`crate::reconstructor::Verb::compute_patch`] is pure and must not read
//! a trace cache or touch the `verbreel-ai` sidecar (the crate-graph rule
//! forbids a `verbreel-state -> verbreel-ai` edge). The subject trace is
//! therefore passed inline, mirroring the v1-floor split that
//! `tracker.apply` / `audio.analyze` / `clip.auto_color` already observe:
//! perception runs elsewhere, its reduced output crosses the boundary as
//! data, and this verb's output is byte-deterministic given identical args.
//!
//! ## Determinism
//!
//! Smoothing (a fixed-window moving average) and min-hold hysteresis use
//! a single left-to-right pass with fixed-order `f64` arithmetic, so the
//! emitted keyframe values are byte-stable under JCS across platforms for
//! identical input. Keyframe *ids* are minted with [`KeyframeId::now`] and
//! are intentionally NOT part of the value-determinism contract (the §0.8
//! reconstructor gate replays the envelope, not the ids).
//!
//! ## Selector & warnings
//!
//! `target` MUST be the qualified `clip:<UUIDv7>` form (spec §0.4). A bare
//! or unknown-prefix selector is rejected with `E_BAD_SELECTOR`; a missing
//! clip with `E_NOT_FOUND`; a locked clip or parent track with `E_LOCKED`.
//! When a sample's *smoothed* subject center (the moving average that
//! drives the pan, not the raw `cx`/`cy`) falls outside the canvas
//! `[0, width) x [0, height)` rectangle the center is clamped per-axis and
//! `W_TRACKER_OUT_OF_BOUNDS` is emitted carrying
//! `details.clamped_sample_count` and `details.bound`, reusing the
//! `tracker.apply` clamp semantics (spec/commands/tracker.md §18.3,
//! appendix-b-warnings `W_TRACKER_OUT_OF_BOUNDS`).
//!
//! ## Precondition: clean `transform.*` keyframe track
//!
//! The verb *appends* `transform.scale_x` / `scale_y` / `x` / `y`
//! keyframes (`add /.../keyframes/-`); it does not clear or reconcile
//! pre-existing `transform.*` keyframes on the target clip. The emitted
//! plan is therefore correct only for a clip whose `transform.*` track is
//! empty (the natural state for a freshly-imported clip being reframed).
//! If the clip already carries `transform.*` keyframes, the new plan
//! composes with the old one and the reframe is wrong; a pre-existing
//! keyframe at an identical `(property, time_tk)` is additionally rejected
//! by `apply`. Clearing an existing track first is out of this slice's
//! "compose existing primitives" scope (issue #481 Scope-OUT) — an agent
//! that needs to re-reframe a clip removes the stale `transform.*`
//! keyframes before calling this verb.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::{ClipId, KeyframeId, ProjectId, Tick};

use crate::invariants::timeline_duration_tk;
use crate::keyframe::{Easing, Keyframe, KeyframeProperty};
use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use crate::verbs::color_grade::{LocatedGradeTarget, locate_grade_target};

/// Internal warning code carrying the reframe envelope for replay.
pub const W_AUTO_REFRAME_ENVELOPE_CODE: &str = "W_CLIP_AUTO_REFRAME_ENVELOPE";

/// Spec warning emitted when one or more subject samples are clamped to
/// the canvas bounds (reused from `tracker.apply`, §18.3).
pub const W_TRACKER_OUT_OF_BOUNDS: &str = "W_TRACKER_OUT_OF_BOUNDS";

/// Default moving-average window (in samples) used for damping.
pub const DEFAULT_SMOOTHING_WINDOW: u32 = 5;

/// Default minimum hold (in ticks) before a new keyframe may be emitted.
pub const DEFAULT_MIN_HOLD_TK: i64 = 80_000;

/// Default re-key threshold (in canvas pixels) for the hysteresis gate.
pub const DEFAULT_REKEY_THRESHOLD_PX: f64 = 4.0;

const SELECTOR_HINT: &str = "target must be qualified `clip:<UUIDv7>` (spec §0.4)";

const fn default_smoothing_window() -> u32 {
    DEFAULT_SMOOTHING_WINDOW
}

const fn default_min_hold_tk() -> i64 {
    DEFAULT_MIN_HOLD_TK
}

const fn default_rekey_threshold_px() -> f64 {
    DEFAULT_REKEY_THRESHOLD_PX
}

/// One subject-center sample in canvas pixel coordinates at a clip-relative
/// timeline tick.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubjectSample {
    /// Clip-relative timeline tick (`>= 0`, within the clip window).
    pub time_tk: i64,
    /// Subject center X in canvas pixels.
    pub cx: f64,
    /// Subject center Y in canvas pixels.
    pub cy: f64,
}

/// Target aspect ratio as an integer fraction (e.g. `9:16`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetAspect {
    /// Aspect numerator (`>= 1`).
    pub num: u32,
    /// Aspect denominator (`>= 1`).
    pub den: u32,
}

/// Smoothing / min-hold parameters controlling keyframe density.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReframeSmoothing {
    /// Moving-average window in samples. `1` disables damping.
    #[serde(default = "default_smoothing_window")]
    pub window: u32,
    /// Minimum ticks the crop window holds before a new keyframe may be
    /// emitted (hysteresis floor).
    #[serde(default = "default_min_hold_tk")]
    pub min_hold_tk: i64,
    /// Re-key threshold in canvas pixels: below this the window is held.
    #[serde(default = "default_rekey_threshold_px")]
    pub rekey_threshold_px: f64,
}

impl Default for ReframeSmoothing {
    fn default() -> Self {
        Self {
            window: DEFAULT_SMOOTHING_WINDOW,
            min_hold_tk: DEFAULT_MIN_HOLD_TK,
            rekey_threshold_px: DEFAULT_REKEY_THRESHOLD_PX,
        }
    }
}

/// Arguments for `clip.auto_reframe`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClipAutoReframeArgs {
    /// Target project id.
    pub project_id: ProjectId,
    /// Qualified `clip:<UUIDv7>` selector.
    pub target: String,
    /// Target output aspect ratio.
    pub target_aspect: TargetAspect,
    /// Subject trace samples, time-ordered (clip-relative ticks).
    pub subject_trace: Vec<SubjectSample>,
    /// Smoothing / min-hold parameters.
    #[serde(default)]
    pub smoothing: ReframeSmoothing,
}

/// Envelope `data` returned by `clip.auto_reframe`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipAutoReframeData {
    /// Target clip id.
    pub clip_id: ClipId,
    /// Number of keyframes emitted across all four `transform` properties.
    pub emitted_keyframe_count: i64,
    /// Number of distinct timeline ticks at which a pan keyframe was set.
    pub emitted_sample_count: i64,
    /// Number of subject samples whose center was clamped to canvas bounds.
    pub clamped_sample_count: i64,
    /// Uniform fit scale applied (zoom factor that fills the target
    /// aspect rectangle).
    pub fit_scale: f64,
}

/// Verb-level validation failures for `clip.auto_reframe`.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum ClipAutoReframeError {
    /// `args.target` is empty, unqualified, or uses a non-clip prefix.
    #[error("clip.auto_reframe: E_BAD_SELECTOR — {detail}; hint: {hint}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
        /// Recovery hint.
        hint: &'static str,
    },

    /// No clip exists for `args.target`.
    #[error("clip.auto_reframe: E_NOT_FOUND — clip `{clip_id}` not found")]
    NotFound {
        /// Missing clip id string.
        clip_id: String,
    },

    /// Target clip or parent track is locked.
    #[error("clip.auto_reframe: E_LOCKED — {kind} `{id}` is locked")]
    Locked {
        /// Locked entity kind (`"clip"` or `"track"`).
        kind: &'static str,
        /// Locked entity id.
        id: String,
    },

    /// A supplied argument is malformed (empty trace, zero aspect, bad
    /// sample value, or out-of-window sample tick).
    #[error("clip.auto_reframe: E_BAD_PARAMS — field `{field}`: {hint}")]
    BadParams {
        /// Offending field.
        field: &'static str,
        /// Guidance text.
        hint: String,
    },
}

impl From<ClipAutoReframeError> for VerbError {
    fn from(value: ClipAutoReframeError) -> Self {
        VerbError::BadArgs {
            detail: value.to_string(),
        }
    }
}

fn parse_clip_target(target: &str) -> Result<ClipId, ClipAutoReframeError> {
    let (prefix, body) =
        target
            .split_once(':')
            .ok_or_else(|| ClipAutoReframeError::BadSelector {
                detail: format!("selector `{target}` is unqualified (missing `clip:` prefix)"),
                hint: SELECTOR_HINT,
            })?;
    if prefix != "clip" {
        return Err(ClipAutoReframeError::BadSelector {
            detail: format!("unknown selector prefix `{prefix}`"),
            hint: SELECTOR_HINT,
        });
    }
    body.parse::<ClipId>()
        .map_err(|err| ClipAutoReframeError::BadSelector {
            detail: format!("clip body parse failed: {err}"),
            hint: SELECTOR_HINT,
        })
}

fn validate_args(
    args: &ClipAutoReframeArgs,
    clip_duration_tk: i64,
) -> Result<(), ClipAutoReframeError> {
    if args.target_aspect.num == 0 || args.target_aspect.den == 0 {
        return Err(ClipAutoReframeError::BadParams {
            field: "target_aspect",
            hint: "num and den must both be >= 1".to_string(),
        });
    }
    if args.subject_trace.is_empty() {
        return Err(ClipAutoReframeError::BadParams {
            field: "subject_trace",
            hint: "at least one subject sample is required".to_string(),
        });
    }
    if args.smoothing.window == 0 {
        return Err(ClipAutoReframeError::BadParams {
            field: "smoothing.window",
            hint: "window must be >= 1".to_string(),
        });
    }
    if args.smoothing.min_hold_tk < 0 {
        return Err(ClipAutoReframeError::BadParams {
            field: "smoothing.min_hold_tk",
            hint: "min_hold_tk must be >= 0".to_string(),
        });
    }
    if !args.smoothing.rekey_threshold_px.is_finite() || args.smoothing.rekey_threshold_px < 0.0 {
        return Err(ClipAutoReframeError::BadParams {
            field: "smoothing.rekey_threshold_px",
            hint: "rekey_threshold_px must be finite and >= 0".to_string(),
        });
    }

    let mut prev_tk = i64::MIN;
    for (idx, sample) in args.subject_trace.iter().enumerate() {
        if !sample.cx.is_finite() || !sample.cy.is_finite() {
            return Err(ClipAutoReframeError::BadParams {
                field: "subject_trace",
                hint: format!("sample {idx} has a non-finite cx/cy"),
            });
        }
        if sample.time_tk < 0 || sample.time_tk > clip_duration_tk {
            return Err(ClipAutoReframeError::BadParams {
                field: "subject_trace",
                hint: format!(
                    "sample {idx} time_tk {} is outside clip window 0..={clip_duration_tk}",
                    sample.time_tk
                ),
            });
        }
        if sample.time_tk <= prev_tk {
            return Err(ClipAutoReframeError::BadParams {
                field: "subject_trace",
                hint: format!(
                    "sample {idx} time_tk {} is not strictly after the previous sample",
                    sample.time_tk
                ),
            });
        }
        prev_tk = sample.time_tk;
    }
    Ok(())
}

/// Largest target-aspect rectangle inscribed in `width x height`, and the
/// uniform cover-scale that zooms that rectangle to cover the whole canvas.
///
/// Returns `(crop_w, crop_h, fit_scale)`. The crop is the inscribed
/// target-aspect rectangle (≤ canvas on both axes); `fit_scale` is the
/// "cover" factor (`max(width/crop_w, height/crop_h)`), so the cropped
/// subject region fills the frame. `fit_scale >= 1` always.
fn fit_geometry(width: f64, height: f64, aspect_num: f64, aspect_den: f64) -> (f64, f64, f64) {
    let target_ratio = aspect_num / aspect_den;
    let canvas_ratio = width / height;
    let (crop_w, crop_h) = if target_ratio >= canvas_ratio {
        // Target is wider (relative): crop is canvas-width-bound.
        (width, width / target_ratio)
    } else {
        // Target is taller (relative): crop is canvas-height-bound.
        (height * target_ratio, height)
    };
    let fit_scale = (width / crop_w).max(height / crop_h);
    (crop_w, crop_h, fit_scale)
}

/// Deterministic mean of an iterator of `f64`, accumulating the count as
/// `f64` to avoid a `usize -> f64` cast and summing in iteration order so
/// the result is byte-stable under JCS.
fn mean(values: impl Iterator<Item = f64>) -> f64 {
    let mut sum = 0.0_f64;
    let mut count = 0.0_f64;
    let mut any = false;
    for value in values {
        sum += value;
        count += 1.0;
        any = true;
    }
    if any { sum / count } else { 0.0 }
}

#[derive(Debug, Clone, Copy)]
struct PanSample {
    time_tk: i64,
    x: f64,
    y: f64,
}

/// Smooth the trace (fixed-window moving average) and apply min-hold
/// hysteresis, returning the pan keyframe samples plus a clamp count.
///
/// The pass is deterministic: a single left-to-right walk over the sorted
/// trace with fixed-order `f64` arithmetic.
fn reframe_samples(
    args: &ClipAutoReframeArgs,
    width: f64,
    height: f64,
    fit_scale: f64,
) -> (Vec<PanSample>, i64) {
    let trace = &args.subject_trace;
    let window = args.smoothing.window as usize;
    let center_x = width / 2.0;
    let center_y = height / 2.0;
    // Per spec the bound is the half-open `[0, w) x [0, h)` rectangle; the
    // clamp ceiling is the largest f64 strictly below the upper bound so a
    // sample exactly at `w`/`h` lands inside.
    let max_x = (width).next_down();
    let max_y = (height).next_down();

    let mut out: Vec<PanSample> = Vec::new();
    let mut clamped_count: i64 = 0;
    let mut last_emit_tk: Option<i64> = None;
    let mut last_x = 0.0_f64;
    let mut last_y = 0.0_f64;

    for (idx, sample) in trace.iter().enumerate() {
        // Trailing moving average over the last `window` raw samples.
        let lo = idx.saturating_sub(window - 1);
        let avg_horizontal = mean(trace[lo..=idx].iter().map(|s| s.cx));
        let avg_vertical = mean(trace[lo..=idx].iter().map(|s| s.cy));

        // Per-axis clamp of the smoothed crop-window center. A sample is
        // counted as clamped when its *smoothed* center (`avg_*`, the moving
        // average that actually drives the pan) lies outside the half-open
        // canvas bound on either axis — not its raw `cx`/`cy`. With
        // `window == 1` the average equals the raw sample, so the two are
        // identical; with `window > 1` smoothing can pull an off-canvas raw
        // sample back in-bounds, and such a sample is then neither clamped
        // nor counted, by design — the pan only ever reads the average.
        let clamp_horizontal = avg_horizontal.clamp(0.0, max_x);
        let clamp_vertical = avg_vertical.clamp(0.0, max_y);
        let was_clamped = avg_horizontal < 0.0
            || avg_horizontal > max_x
            || avg_vertical < 0.0
            || avg_vertical > max_y;
        if was_clamped {
            clamped_count += 1;
        }

        // Pan keeps the (clamped) subject center at the canvas center
        // after the uniform fit zoom. Modeling the zoom as a scale about
        // the canvas origin, a canvas point `p` maps to `fit_scale * p`;
        // the translation that lands the subject at the canvas center is
        // `center - fit_scale * subject`.
        let x = center_x - fit_scale * clamp_horizontal;
        let y = center_y - fit_scale * clamp_vertical;

        // Hysteresis: always emit the first and last samples; otherwise
        // emit only once min_hold_tk has elapsed AND the window moved past
        // the re-key threshold.
        let is_first = idx == 0;
        let is_last = idx == trace.len() - 1;
        let emit = if is_first || is_last {
            true
        } else {
            let held_long_enough =
                last_emit_tk.is_none_or(|prev| sample.time_tk - prev >= args.smoothing.min_hold_tk);
            let moved_enough = (x - last_x).abs() >= args.smoothing.rekey_threshold_px * fit_scale
                || (y - last_y).abs() >= args.smoothing.rekey_threshold_px * fit_scale;
            held_long_enough && moved_enough
        };

        if emit {
            // The last sample replaces a held keyframe at the same tick
            // rather than duplicating it.
            if out.last().is_some_and(|p| p.time_tk == sample.time_tk) {
                out.pop();
            }
            out.push(PanSample {
                time_tk: sample.time_tk,
                x,
                y,
            });
            last_emit_tk = Some(sample.time_tk);
            last_x = x;
            last_y = y;
        }
    }

    (out, clamped_count)
}

fn number(value: f64) -> Value {
    Value::Number(
        serde_json::Number::from_f64(value)
            .expect("reframe keyframe values are guaranteed finite by validated args"),
    )
}

fn keyframe_op(
    track_idx: usize,
    clip_idx: usize,
    property: &str,
    time_tk: i64,
    value: f64,
) -> Value {
    let keyframe = Keyframe {
        id: KeyframeId::now(),
        property: KeyframeProperty::new(property.to_string())
            .expect("reframe property literals match the keyframe property schema"),
        time_tk: Tick::new(time_tk),
        value: number(value),
        easing: Easing::Linear,
    };
    json!({
        "op": "add",
        "path": format!("/tracks/{track_idx}/clips/{clip_idx}/keyframes/-"),
        "value": serde_json::to_value(keyframe).expect("keyframe serializes"),
    })
}

/// Build the RFC-6902 patch for `clip.auto_reframe`.
///
/// # Errors
///
/// Returns [`ClipAutoReframeError`] for selector parse failure, missing
/// clip, locked clip/track, or malformed args.
pub fn compute_patch(
    prior: &Project,
    args: &ClipAutoReframeArgs,
) -> Result<(Value, Vec<Value>, ClipAutoReframeData), ClipAutoReframeError> {
    let clip_id = parse_clip_target(&args.target)?;

    let located: LocatedGradeTarget =
        locate_grade_target(prior, clip_id).ok_or_else(|| ClipAutoReframeError::NotFound {
            clip_id: clip_id.to_string(),
        })?;
    if located.clip_locked {
        return Err(ClipAutoReframeError::Locked {
            kind: "clip",
            id: located.clip_id,
        });
    }
    if located.track_locked {
        return Err(ClipAutoReframeError::Locked {
            kind: "track",
            id: located.track_id,
        });
    }

    let clip = &prior.tracks[located.track_idx].clips[located.clip_idx];
    let clip_duration_tk =
        timeline_duration_tk(clip.source_in_tk, clip.source_out_tk, clip.speed).get();
    validate_args(args, clip_duration_tk)?;

    let width = f64::from(prior.canvas.width);
    let height = f64::from(prior.canvas.height);
    let (_crop_w, _crop_h, fit_scale) = fit_geometry(
        width,
        height,
        f64::from(args.target_aspect.num),
        f64::from(args.target_aspect.den),
    );

    let (pan, clamped_sample_count) = reframe_samples(args, width, height, fit_scale);

    let mut ops: Vec<Value> = Vec::new();
    // Uniform fit zoom keyed once at the first pan sample's tick.
    let scale_tk = pan[0].time_tk;
    ops.push(keyframe_op(
        located.track_idx,
        located.clip_idx,
        "transform.scale_x",
        scale_tk,
        fit_scale,
    ));
    ops.push(keyframe_op(
        located.track_idx,
        located.clip_idx,
        "transform.scale_y",
        scale_tk,
        fit_scale,
    ));
    for p in &pan {
        ops.push(keyframe_op(
            located.track_idx,
            located.clip_idx,
            "transform.x",
            p.time_tk,
            p.x,
        ));
        ops.push(keyframe_op(
            located.track_idx,
            located.clip_idx,
            "transform.y",
            p.time_tk,
            p.y,
        ));
    }

    let emitted_sample_count = i64::try_from(pan.len()).unwrap_or(i64::MAX);
    let emitted_keyframe_count = i64::try_from(ops.len()).unwrap_or(i64::MAX);

    let data = ClipAutoReframeData {
        clip_id,
        emitted_keyframe_count,
        emitted_sample_count,
        clamped_sample_count,
        fit_scale,
    };

    let mut warnings = vec![envelope_warning(&data)];
    if clamped_sample_count > 0 {
        warnings.push(out_of_bounds_warning(
            &located.clip_id,
            clamped_sample_count,
            prior.canvas.width,
            prior.canvas.height,
        ));
    }

    Ok((Value::Array(ops), warnings, data))
}

fn envelope_warning(data: &ClipAutoReframeData) -> Value {
    json!({
        "code": W_AUTO_REFRAME_ENVELOPE_CODE,
        "message": "clip.auto_reframe envelope",
        "details": data,
    })
}

fn out_of_bounds_warning(
    to_clip_id: &str,
    clamped_sample_count: i64,
    width: u32,
    height: u32,
) -> Value {
    json!({
        "code": W_TRACKER_OUT_OF_BOUNDS,
        "message": "clip.auto_reframe clamped one or more subject samples to the canvas bounds",
        "details": {
            "to_clip_id": to_clip_id,
            "clamped_sample_count": clamped_sample_count,
            "bound": {
                "x": [0, width],
                "y": [0, height],
            },
        },
    })
}

/// Rebuild [`ClipAutoReframeData`] from the recorded envelope warning.
///
/// # Errors
///
/// Returns [`ReconstructError`] if the internal envelope warning is
/// missing or malformed.
pub fn data_envelope_from_warnings(
    warnings: &[Value],
) -> Result<ClipAutoReframeData, ReconstructError> {
    for warning in warnings {
        if warning.get("code").and_then(Value::as_str) != Some(W_AUTO_REFRAME_ENVELOPE_CODE) {
            continue;
        }
        let details = warning
            .get("details")
            .ok_or(ReconstructError::MissingField {
                name: "warnings[].W_CLIP_AUTO_REFRAME_ENVELOPE.details",
            })?;
        return serde_json::from_value(details.clone()).map_err(|_| {
            ReconstructError::TypeMismatch {
                name: "warnings[].W_CLIP_AUTO_REFRAME_ENVELOPE.details",
                expected: "ClipAutoReframeData",
            }
        });
    }
    Err(ReconstructError::MissingField {
        name: "warnings[].W_CLIP_AUTO_REFRAME_ENVELOPE",
    })
}

/// `clip.auto_reframe` verb registration entry.
#[derive(Debug, Default)]
pub struct ClipAutoReframeVerb;

impl Verb for ClipAutoReframeVerb {
    fn verb(&self) -> &'static str {
        "clip.auto_reframe"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: ClipAutoReframeArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("clip.auto_reframe: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, _data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!(
                    "clip.auto_reframe: patch construction failed: {err}"
                ))
            })?;

        let post_state = prior
            .apply(&patch)
            .map_err(|err| VerbError::InvariantViolation {
                detail: format!("clip.auto_reframe: post-state validation failed: {err}"),
            })?;
        drop(post_state);

        // Intentional round-trip: rebuild `data` from the recorded warning
        // so the write path exercises the exact envelope decode replay
        // relies on. Drift between in-memory data and the serialized
        // warning surfaces here at write time, not silently at replay.
        let envelope = data_envelope_from_warnings(&warnings).map_err(|err| {
            VerbError::Custom(format!(
                "clip.auto_reframe: data envelope reconstruction failed: {err}"
            ))
        })?;
        let data = serde_json::to_value(&envelope).map_err(|err| {
            VerbError::Custom(format!("clip.auto_reframe: data serialize failed: {err}"))
        })?;

        Ok((patch, data, warnings))
    }

    fn reconstruct(
        &self,
        _args: &Value,
        _patch: &Value,
        warnings: &[Value],
        _post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let envelope = data_envelope_from_warnings(warnings)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
