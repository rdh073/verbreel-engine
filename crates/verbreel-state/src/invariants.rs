//! §0.13 engine invariant checks. Composed at the end of
//! [`crate::project::Project::apply`] so every patch lands in a
//! Project that satisfies every documented post-condition.
//!
//! ## Slice progression
//!
//! Each §0.13 invariant family lands as its own slice with a new
//! [`InvariantViolation`] variant and a corresponding `check_*`
//! function here. As of this slice:
//!
//! - [`check_fade_clamp`] — `clip.fade_in_tk + clip.fade_out_tk ≤
//!   timeline_duration_tk` on every clip.
//!
//! Planned (each its own slice + variant):
//! - `check_track_contiguity` — same-kind tracks grouped, kind-block
//!   order enforced.
//! - `check_no_overlap` — clips on the same track don't overlap.
//! - `check_duration_tk_maintenance` —
//!   `Project.duration_tk == max(end_of_last_clip per track)`.
//! - `check_dangling_keyframe` — every keyframe's `effects[<uuid>]`
//!   path references an actual effect on the parent clip.
//! - `check_source_in_tk_normalized_for_text_image` —
//!   `source_in_tk == 0` on text/image clips.
//! - `check_speed_curve_forbidden_for_text_image`.
//! - `check_mask_per_kind_params` — `mask.params.<leaf>` matches the
//!   `mask.kind`'s expected leaf set.
//! - `check_asset_id_text_track_biconditional` —
//!   `Clip.asset_id == nil-UUID ⇔ Track.kind == "text"`.
//! - `check_metadata_size_caps` — `Project.metadata` ≤ 256 keys / 64 KiB.
//! - `check_effect_params_size_caps` — `Effect.params` ≤ 64 keys / 16 KiB.
//!
//! ## Spec references
//!
//! - §0.13 — full invariants list.

use thiserror::Error;
use verbreel_types::{ClipId, Tick};

use crate::project::Project;

// ---------------------------------------------------------------------
// InvariantViolation
// ---------------------------------------------------------------------

/// A §0.13 invariant violation surfaced at [`crate::project::Project::apply`]
/// time.
///
/// Not marked `#[non_exhaustive]` per the established
/// [`crate::apply::ApplyError`] convention — callers' exhaustive
/// pattern-matches will need updating when each new invariant variant
/// lands, which is the desired feedback signal.
#[derive(Debug, Error)]
pub enum InvariantViolation {
    /// `clip.fade_in_tk + clip.fade_out_tk > clip timeline duration`.
    /// Spec §0.13 — fades must fit within the clip's timeline
    /// duration. Verb-layer `clip.set_speed` / `clip.trim` /
    /// `clip.split` clamp proportionally + emit `W_FADE_CLAMPED`;
    /// `apply()` only enforces the post-condition (this variant is
    /// the rejection path when a patch tries to bypass that
    /// maintenance).
    #[error(
        "§0.13 fade-clamp invariant: fade_in_tk ({}) + fade_out_tk ({}) > \
         timeline_duration_tk ({}) on clip {clip_id}",
        fade_in_tk.get(), fade_out_tk.get(), timeline_duration_tk.get()
    )]
    FadeClamp {
        /// Offending clip id (helps callers locate it without
        /// re-walking the tree).
        clip_id: ClipId,
        /// `Clip.fade_in_tk` as read from the patched project.
        fade_in_tk: Tick,
        /// `Clip.fade_out_tk` as read from the patched project.
        fade_out_tk: Tick,
        /// Computed `timeline_duration_tk = ceil((source_out_tk -
        /// source_in_tk) / speed)`.
        timeline_duration_tk: Tick,
    },
}

// ---------------------------------------------------------------------
// timeline_duration_tk
// ---------------------------------------------------------------------

/// Pure helper — compute a clip's timeline-domain duration in ticks
/// from `(source_in_tk, source_out_tk, speed)`. Spec §0.13.
///
/// Formula: `ceil((source_out_tk - source_in_tk) / speed)`. Correct
/// for both video/audio (where `speed` may be != 1) AND text/image
/// (where the `speed == 1` invariant — enforced in a later slice —
/// reduces it to `source_out_tk - source_in_tk`).
///
/// `speed <= 0.0` is invalid per `spec/project-schema.json` (clip
/// schema bound is `[0.001, 100]`), but hand-edited project.json or
/// in-flight patches could produce a non-positive value before any
/// other invariant catches it. This helper saturates to [`Tick::MAX`]
/// in that case so callers don't divide-by-zero / negate-overflow and
/// the `FadeClamp` check still produces a meaningful rejection later
/// in the chain.
///
/// Returns a [`Tick`] (newtype over i64). For absolute durations
/// computed from non-negative inputs the returned value is always
/// non-negative.
#[must_use]
#[allow(clippy::cast_precision_loss)]
#[allow(clippy::cast_possible_truncation)]
#[allow(clippy::cast_sign_loss)]
pub fn timeline_duration_tk(source_in_tk: Tick, source_out_tk: Tick, speed: f64) -> Tick {
    if speed <= 0.0 || !speed.is_finite() {
        return Tick::new(i64::MAX);
    }
    let diff = source_out_tk.get().saturating_sub(source_in_tk.get());
    if diff <= 0 {
        return Tick::ZERO;
    }
    let scaled = (diff as f64) / speed;
    if !scaled.is_finite() || scaled >= i64::MAX as f64 {
        return Tick::new(i64::MAX);
    }
    Tick::new(scaled.ceil() as i64)
}

// ---------------------------------------------------------------------
// check_fade_clamp
// ---------------------------------------------------------------------

/// Walk every clip on every track; return the first
/// [`InvariantViolation::FadeClamp`] found, or [`Ok`].
///
/// Iteration order is deterministic — `project.tracks` in declared
/// order, then each track's `clips` in declared order. Callers can
/// rely on the order so error reporting is reproducible across runs.
///
/// # Errors
///
/// Returns [`InvariantViolation::FadeClamp`] for the first clip where
/// `fade_in_tk + fade_out_tk > timeline_duration_tk(source_in, source_out, speed)`.
pub fn check_fade_clamp(project: &Project) -> Result<(), InvariantViolation> {
    for track in &project.tracks {
        for clip in &track.clips {
            let dur = timeline_duration_tk(clip.source_in_tk, clip.source_out_tk, clip.speed);
            let fade_sum = clip.fade_in_tk.get().saturating_add(clip.fade_out_tk.get());
            if fade_sum > dur.get() {
                return Err(InvariantViolation::FadeClamp {
                    clip_id: clip.id,
                    fade_in_tk: clip.fade_in_tk,
                    fade_out_tk: clip.fade_out_tk,
                    timeline_duration_tk: dur,
                });
            }
        }
    }
    Ok(())
}
