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
//! - [`check_track_contiguity`] — tracks of the same `TrackKind` are
//!   grouped together in `Project.tracks[]` (no interleaving).
//!   Specifically does NOT enforce a block ORDER across kinds —
//!   that's `project.open` reconciliation territory.
//! - [`check_no_overlap`] — clips on the same track don't overlap.
//!   Half-open intervals; adjacent clips sharing an endpoint are
//!   not considered overlapping.
//! - [`check_duration_tk`] — persisted `Project.duration_tk` equals
//!   `max(track_position_tk + timeline_duration_tk)` across all
//!   clips. Post-condition only — verbs are responsible for the
//!   `/duration_tk` replace op in their patches.
//! - [`check_dangling_keyframes`] — every `Keyframe.property` of the
//!   form `effects[<uuid>].params.<…>` references an effect that
//!   exists on the parent clip. Non-effect-targeting properties
//!   (`transform.*`, `opacity`, `volume`, `mask.*`) skip the check.
//! - [`check_source_in_tk`] — `source_in_tk == 0` on text clips
//!   (parent `Track.kind == Text`) and image clips (referenced
//!   `Asset` is `Image`). Hard-rejects; the `project.open` silent-
//!   normalization behavior with `W_CLIP_SOURCE_IN_NORMALIZED`
//!   warning is a separate reconciliation slice.
//!
//! ## `apply()` check order
//!
//! The chain in [`crate::project::Project::apply`] runs invariant
//! checks in a deterministic order so agents debugging which
//! invariant fires can rely on the same answer across runs:
//!
//! 1. [`check_fade_clamp`] — per-clip.
//! 2. [`check_track_contiguity`] — per-project structure.
//! 3. [`check_no_overlap`] — per-track clip intervals.
//! 4. [`check_duration_tk`] — project-level extent.
//! 5. [`check_dangling_keyframes`] — per-clip keyframe → effect-id
//!    referential integrity.
//! 6. [`check_source_in_tk`] — text/image clip `source_in_tk == 0`.
//! 7. (future slices append here)
//!
//! ## Planned future invariant slices
//!
//! - `check_source_in_tk_normalized_for_text_image` —
//!   `source_in_tk == 0` on text/image clips.
//! - `check_speed_curve_forbidden_for_text_image`.
//! - `check_mask_per_kind_params` — `mask.params.<leaf>` matches the
//!   `mask.kind`'s expected leaf set.
//! - `check_asset_id_text_track_biconditional` —
//!   `Clip.asset_id == nil-UUID ⇔ Track.kind == "text"`.
//! - `check_metadata_size_caps` — `Project.metadata` ≤ 256 keys / 64 KiB.
//! - `check_effect_params_size_caps` — `Effect.params` ≤ 64 keys / 16 KiB.
//! - `check_effect_track_empty_clips` — `Track.kind == "effect"`
//!   ⇒ `Track.clips.is_empty()`.
//!
//! ## Spec references
//!
//! - §0.13 — full invariants list.

use std::collections::HashSet;
use std::sync::OnceLock;

use regex::Regex;
use thiserror::Error;
use verbreel_types::{ClipId, KeyframeId, Tick};

use crate::asset::Asset;
use crate::newtypes::AssetRef;
use crate::project::Project;
use crate::track::TrackKind;

// ---------------------------------------------------------------------
// AssetKind / SourceInTkKind helpers
// ---------------------------------------------------------------------

/// Discriminator surfaced by [`resolve_asset_kind`]. Mirrors the
/// `Asset` tagged-union variants from PR #22. Lives in `invariants.rs`
/// because it's currently only used here; promote to `asset.rs` if
/// other consumers emerge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AssetKind {
    Video,
    Audio,
    Image,
    Subtitle,
}

/// Indicator carried by [`InvariantViolation::InvalidSourceInTk`] so
/// callers can tell whether the offending clip was flagged via its
/// parent `Track.kind` (text) or via its referenced
/// `Asset.kind` (image).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceInTkKind {
    /// Parent `Track.kind == Text`.
    Text,
    /// `Clip.asset_id` resolves to `Asset::Image(_)`.
    Image,
}

impl std::fmt::Display for SourceInTkKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SourceInTkKind::Text => f.write_str("text"),
            SourceInTkKind::Image => f.write_str("image"),
        }
    }
}

/// Resolve a [`AssetRef`] to its [`AssetKind`] by walking
/// `project.assets[]`. Returns:
///
/// - `None` if the `AssetRef` is nil (text clip case — there is no
///   asset to resolve).
/// - `None` if the `AssetRef` is `Id(...)` but no matching asset is
///   found in `project.assets[]`. The asset-existence invariant
///   (separate slice) will catch this; the `source_in_tk` check just
///   skips it.
/// - `Some(kind)` on successful resolution.
fn resolve_asset_kind(project: &Project, asset_id: &AssetRef) -> Option<AssetKind> {
    let id = asset_id.id()?;
    project.assets.iter().find_map(|a| {
        if a.id() == id {
            Some(match a {
                Asset::Video(_) => AssetKind::Video,
                Asset::Audio(_) => AssetKind::Audio,
                Asset::Image(_) => AssetKind::Image,
                Asset::Subtitle(_) => AssetKind::Subtitle,
            })
        } else {
            None
        }
    })
}

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

    /// Tracks of the same [`TrackKind`] are not contiguous in
    /// `Project.tracks[]`. Spec §0.13 — interleaved tracks (e.g. a
    /// video track between two audio tracks) are forbidden by the
    /// invariant; `project.open` reconciliation stable-sorts them
    /// back into contiguous blocks and emits `W_TRACKS_REORDERED`,
    /// but mutating verbs (`track.add`, `track.move`) must never
    /// write that state — this variant is the rejection path when
    /// a patch tries to bypass that maintenance.
    ///
    /// Specifically does NOT enforce the canonical block ORDER
    /// (`video → audio → text → effect`); only same-kind contiguity.
    /// That stronger check is `project.open` reconciliation
    /// territory.
    #[error(
        "§0.13 track contiguity invariant: track at index {first_violation_index} has kind \
         {actual_kind:?}, breaking the contiguity of an earlier {prior_kind_block:?} block \
         (expected continuation of {expected_kind_block:?} block)"
    )]
    InterleavedTracks {
        /// Index in `tracks[]` where the violation appears.
        first_violation_index: usize,
        /// The kind of the previously-completed block whose contiguity
        /// is broken (same value as `actual_kind` — surfaced for caller
        /// convenience).
        prior_kind_block: TrackKind,
        /// The kind found at `first_violation_index`.
        actual_kind: TrackKind,
        /// The kind the in-progress block was extending before this
        /// violation.
        expected_kind_block: TrackKind,
    },

    /// Two clips on the same track have overlapping intervals. Spec
    /// §0.13 — *"Engine enforces that clip intervals on the same
    /// track do not overlap."* Half-open intervals
    /// `[track_position_tk, track_position_tk + timeline_duration_tk)`
    /// — adjacent clips sharing an endpoint are NOT considered
    /// overlapping. Sort-by-position then pairwise scan makes the
    /// reported `earlier` / `later` deterministic.
    #[error(
        "§0.13 no-overlap invariant: clip {later_clip_id} starts at {} on track #{track_index}, \
         before earlier clip {earlier_clip_id} ends at {} (overlap detected)",
        later_start_tk.get(), earlier_end_tk.get()
    )]
    ClipOverlap {
        /// Index in `tracks[]` of the affected track.
        track_index: usize,
        /// `id` of the clip whose interval started first (lower
        /// `track_position_tk`; ties broken by stable sort).
        earlier_clip_id: ClipId,
        /// Computed end of the earlier clip's interval (exclusive
        /// upper bound).
        earlier_end_tk: Tick,
        /// `id` of the clip whose interval started second.
        later_clip_id: ClipId,
        /// The later clip's `track_position_tk`.
        later_start_tk: Tick,
    },

    /// `Project.duration_tk` doesn't equal the computed maximum
    /// across all clips. Spec §0.13 — *"`Project.duration_tk` is
    /// maintained on every mutation, not lazily."* Mutating verbs
    /// that change a clip extent must include a `/duration_tk`
    /// replace op in their patch; this variant is the rejection
    /// path when a patch tries to bypass that maintenance.
    ///
    /// Empty projects: computed = 0; `Project.duration_tk` must
    /// be `Tick(0)`.
    #[error(
        "§0.13 duration_tk invariant: Project.duration_tk = {} but \
         max(track_position_tk + timeline_duration_tk) across all clips = {}",
        stored_duration_tk.get(), computed_duration_tk.get()
    )]
    ProjectDurationStale {
        /// Value persisted on the patched `Project`.
        stored_duration_tk: Tick,
        /// Value computed from a fresh walk over every clip.
        computed_duration_tk: Tick,
    },

    /// `Clip.source_in_tk` is non-zero on a clip that the spec
    /// requires to be zero — text clips (parent `Track.kind ==
    /// Text`) or image clips (referenced `Asset` is `Image`). Spec
    /// §0.13 — *"For image and text clips, the engine pins this to
    /// 0 on every write."*
    ///
    /// `project.open` reconciliation silently normalizes hand-
    /// edited non-zero values with a `W_CLIP_SOURCE_IN_NORMALIZED`
    /// warning; that lives in a separate slice. `apply()` hard-
    /// rejects to surface verb-layer bugs loudly.
    #[error(
        "§0.13 source_in_tk invariant: source_in_tk = {} on {clip_kind_indicator} clip \
         {clip_id}, must be 0",
        source_in_tk.get()
    )]
    InvalidSourceInTk {
        /// Offending clip id.
        clip_id: ClipId,
        /// Why the clip is required to have `source_in_tk == 0` —
        /// `Text` (parent Track.kind) or `Image` (referenced Asset).
        clip_kind_indicator: SourceInTkKind,
        /// The actual non-zero value found.
        source_in_tk: Tick,
    },

    /// A keyframe targets `effects[<uuid>].params.<…>` but the
    /// referenced `<uuid>` is not present on the parent clip's
    /// `Effect` list. Spec §0.13 — *"No dangling keyframe property
    /// references."*
    ///
    /// Cascade-delete behavior on `effect.remove` and the
    /// `W_KEYFRAMES_ORPHANED` reconciliation warning are verb-layer /
    /// `project.open` territory; this variant is the rejection path
    /// when a patch tries to bypass them.
    #[error(
        "§0.13 dangling-keyframe invariant: keyframe {keyframe_id} on clip {clip_id} targets \
         effect {referenced_effect_id} which does not exist on this clip (property: {property:?})"
    )]
    DanglingKeyframe {
        /// Parent clip carrying the dangling keyframe.
        clip_id: ClipId,
        /// The keyframe with the unresolved effect reference.
        keyframe_id: KeyframeId,
        /// Raw effect-id string extracted from the property path.
        /// Kept as `String` (not `EffectId`) because the value came
        /// from a regex-validated property string, not a typed lookup.
        referenced_effect_id: String,
        /// Full property string, surfaced for caller-side debugging.
        property: String,
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

// ---------------------------------------------------------------------
// check_no_overlap
// ---------------------------------------------------------------------

/// Walk every track; for each track sort its clips by
/// `track_position_tk` and verify that no two adjacent intervals
/// overlap. Returns the first
/// [`InvariantViolation::ClipOverlap`] found, or [`Ok`].
///
/// Half-open intervals: a clip starting exactly where the previous
/// ended (`earlier_end == later_start`) is **adjacent**, not
/// overlapping. The predicate is `earlier_end > later_start`.
///
/// Empty / single-clip tracks are trivially overlap-free.
///
/// Sort is stable (`slice::sort_by_key`) so when two clips share the
/// same `track_position_tk` (genuine overlap), the one appearing
/// first in the input array is reported as `earlier_clip_id`.
///
/// # Errors
///
/// Returns [`InvariantViolation::ClipOverlap`] for the first overlap
/// detected. Walks tracks in `Project.tracks[]` order; within each
/// track, walks adjacent pairs in sorted-position order.
pub fn check_no_overlap(project: &Project) -> Result<(), InvariantViolation> {
    for (track_index, track) in project.tracks.iter().enumerate() {
        if track.clips.len() < 2 {
            continue;
        }
        // Build (clip_ref, start, end) tuples. `start` is the clip's
        // `track_position_tk`; `end = start + timeline_duration_tk`
        // (exclusive upper bound).
        let mut intervals: Vec<(&crate::clip::Clip, Tick, Tick)> = track
            .clips
            .iter()
            .map(|c| {
                let dur = timeline_duration_tk(c.source_in_tk, c.source_out_tk, c.speed);
                let start = c.track_position_tk;
                let end = Tick::new(start.get().saturating_add(dur.get()));
                (c, start, end)
            })
            .collect();
        intervals.sort_by_key(|(_, start, _)| start.get());

        for window in intervals.windows(2) {
            let (a, _a_start, a_end) = &window[0];
            let (b, b_start, _b_end) = &window[1];
            if a_end.get() > b_start.get() {
                return Err(InvariantViolation::ClipOverlap {
                    track_index,
                    earlier_clip_id: a.id,
                    earlier_end_tk: *a_end,
                    later_clip_id: b.id,
                    later_start_tk: *b_start,
                });
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------
// check_track_contiguity
// ---------------------------------------------------------------------

/// Walk `project.tracks` linearly; return the first
/// [`InvariantViolation::InterleavedTracks`] found, or [`Ok`].
///
/// A "kind boundary" is a position `i` where `tracks[i].kind !=
/// tracks[i-1].kind`. At every such boundary, the new kind must not
/// have been seen earlier — if it has, the kind block was previously
/// completed and is being broken by this re-appearance.
///
/// Backing storage is a `Vec<TrackKind>` of length ≤ 4 (one per
/// enum variant); `Vec::contains` is O(N) but N ≤ 4 = effectively
/// constant. No `HashSet` / `Hash` derive needed on [`TrackKind`].
///
/// Does NOT enforce the canonical block ORDER
/// (`video → audio → text → effect`) — that's `project.open`
/// reconciliation territory. Only same-kind contiguity.
///
/// # Errors
///
/// Returns [`InvariantViolation::InterleavedTracks`] for the first
/// position where contiguity breaks.
pub fn check_track_contiguity(project: &Project) -> Result<(), InvariantViolation> {
    let mut seen_kinds: Vec<TrackKind> = Vec::with_capacity(4);
    let mut last_kind: Option<TrackKind> = None;

    for (i, track) in project.tracks.iter().enumerate() {
        let kind = track.kind;
        // Same kind as previous → still inside the in-progress block; skip.
        if last_kind == Some(kind) {
            continue;
        }
        // Crossing a kind boundary. If we've seen this kind in a
        // previously-closed block, the contiguity is broken.
        if seen_kinds.contains(&kind) {
            let prev = last_kind.unwrap_or(kind); // `seen_kinds` non-empty ⇒ last_kind Some
            return Err(InvariantViolation::InterleavedTracks {
                first_violation_index: i,
                prior_kind_block: kind,
                actual_kind: kind,
                expected_kind_block: prev,
            });
        }
        // Brand-new kind block starts here.
        seen_kinds.push(kind);
        last_kind = Some(kind);
    }
    Ok(())
}

// ---------------------------------------------------------------------
// check_duration_tk
// ---------------------------------------------------------------------

/// Verify `Project.duration_tk` equals
/// `max(track_position_tk + timeline_duration_tk)` across every clip
/// in every track. Empty projects compute max = 0 → `duration_tk`
/// must be `Tick(0)`.
///
/// Post-condition only — this check does NOT synthesize a patch op
/// to fix `duration_tk`. The verb-layer side of §0.13 requires
/// mutating verbs to include the `/duration_tk` replace op
/// themselves; this function rejects the result when they don't.
///
/// Arithmetic uses [`i64::saturating_add`] defensively. Spec bounds
/// (~1.19 trillion years of `Tick` range) make overflow impossible
/// in practice but the type-level helper stays honest.
///
/// # Errors
///
/// Returns [`InvariantViolation::ProjectDurationStale`] when
/// `project.duration_tk != computed`.
pub fn check_duration_tk(project: &Project) -> Result<(), InvariantViolation> {
    let mut computed: i64 = 0;
    for track in &project.tracks {
        for clip in &track.clips {
            let dur = timeline_duration_tk(clip.source_in_tk, clip.source_out_tk, clip.speed);
            let end = clip.track_position_tk.get().saturating_add(dur.get());
            if end > computed {
                computed = end;
            }
        }
    }
    let computed = Tick::new(computed);
    if project.duration_tk != computed {
        return Err(InvariantViolation::ProjectDurationStale {
            stored_duration_tk: project.duration_tk,
            computed_duration_tk: computed,
        });
    }
    Ok(())
}

// ---------------------------------------------------------------------
// check_dangling_keyframes
// ---------------------------------------------------------------------

/// Schema-derived regex matching `effects[<uuidv7>].params.<…>`
/// shape. The `Keyframe.property` newtype already validates the full
/// property pattern at deserialize (PR #28), so this regex assumes
/// well-formed input — it only needs to extract the effect-id
/// capture group on properties of the third category.
const EFFECT_PROPERTY_PATTERN: &str =
    r"^effects\[([0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12})\]\.params\.";

fn effect_property_regex() -> &'static Regex {
    static EFFECT_RE: OnceLock<Regex> = OnceLock::new();
    EFFECT_RE.get_or_init(|| {
        Regex::new(EFFECT_PROPERTY_PATTERN).expect("EFFECT_PROPERTY_PATTERN compiles")
    })
}

/// Extract the effect-id from a `Keyframe.property` string. Returns
/// `Some(uuid_str)` if `property` starts with
/// `effects[<uuidv7>].params.<…>`; `None` for non-effect-targeting
/// properties (`transform.*`, `opacity`, `volume`, `mask.*`).
///
/// The returned `&str` borrows from the input — the caller can use
/// it for `HashSet<String>::contains(&str)` lookups without
/// allocating.
#[must_use]
pub fn extract_effect_id_from_property(property: &str) -> Option<&str> {
    effect_property_regex()
        .captures(property)
        .and_then(|c| c.get(1).map(|m| m.as_str()))
}

/// Verify every effect-targeting keyframe references an effect that
/// exists on the parent clip.
///
/// Walks every clip on every track in declared order. For each clip,
/// builds a `HashSet<String>` of effect-id strings (allocated once
/// per clip, dropped before moving on), then iterates the clip's
/// keyframes. Non-effect-targeting keyframes are skipped trivially
/// (by `extract_effect_id_from_property` returning `None`).
///
/// # Errors
///
/// Returns [`InvariantViolation::DanglingKeyframe`] for the first
/// effect-targeting keyframe whose effect-id is not in its parent
/// clip's effect list.
pub fn check_dangling_keyframes(project: &Project) -> Result<(), InvariantViolation> {
    for track in &project.tracks {
        for clip in &track.clips {
            // Per-clip effect-id index. Allocated only once per clip.
            let effect_ids: HashSet<String> =
                clip.effects.iter().map(|e| e.id.to_string()).collect();
            for kf in &clip.keyframes {
                let property = kf.property.as_str();
                if let Some(target) = extract_effect_id_from_property(property)
                    && !effect_ids.contains(target)
                {
                    return Err(InvariantViolation::DanglingKeyframe {
                        clip_id: clip.id,
                        keyframe_id: kf.id,
                        referenced_effect_id: target.to_string(),
                        property: property.to_string(),
                    });
                }
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------
// check_source_in_tk
// ---------------------------------------------------------------------

/// Verify `Clip.source_in_tk == 0` on every text clip (parent
/// `Track.kind == Text`) and every image clip (referenced `Asset` is
/// `Image`). Spec §0.13.
///
/// Two-tier check per clip:
/// - If `Track.kind == Text` → must be `Tick::ZERO`.
/// - Else if [`resolve_asset_kind`] returns `Some(AssetKind::Image)` →
///   must be `Tick::ZERO`.
/// - Else (video/audio/subtitle assets, or unresolvable `asset_id`) →
///   no constraint here. Unresolvable `asset_id` is a separate
///   invariant (asset-existence — future slice); skip silently to
///   avoid double-reporting.
///
/// **Layer note**: `project.open` reconciliation silently normalizes
/// non-zero values with a `W_CLIP_SOURCE_IN_NORMALIZED` warning;
/// that's verb-layer / reconciliation territory. `apply()` hard-
/// rejects because mutating verbs are spec-bound never to write the
/// state and a patch that does is bug-shaped.
///
/// # Errors
///
/// Returns [`InvariantViolation::InvalidSourceInTk`] for the first
/// text or image clip with non-zero `source_in_tk`.
pub fn check_source_in_tk(project: &Project) -> Result<(), InvariantViolation> {
    for track in &project.tracks {
        let track_kind = track.kind;
        for clip in &track.clips {
            let must_be_zero_because = if track_kind == TrackKind::Text {
                Some(SourceInTkKind::Text)
            } else if matches!(
                resolve_asset_kind(project, &clip.asset_id),
                Some(AssetKind::Image)
            ) {
                Some(SourceInTkKind::Image)
            } else {
                None
            };
            if let Some(kind) = must_be_zero_because
                && clip.source_in_tk != Tick::ZERO
            {
                return Err(InvariantViolation::InvalidSourceInTk {
                    clip_id: clip.id,
                    clip_kind_indicator: kind,
                    source_in_tk: clip.source_in_tk,
                });
            }
        }
    }
    Ok(())
}
