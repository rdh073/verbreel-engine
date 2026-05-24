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
//! - [`check_speed_on_image_text`] — `Clip.speed == 1.0` on text /
//!   image clips. `speed` is a source-slice playback-rate concept
//!   that has no meaning for display-duration kinds.
//! - [`check_speed_curve_on_image_text`] — `Clip.speed_curve == None`
//!   on text / image clips. Same source-time-semantics rationale as
//!   the scalar `speed` invariant (v1.1-additive companion).
//! - [`check_asset_id_biconditional`] — `Clip.asset_id == nil-UUID ⇔
//!   Track.kind == Text`. A non-nil `asset_id` on a text-track clip
//!   or a nil `asset_id` on a non-text-track clip is rejected.
//! - [`check_asset_existence`] — every non-nil `Clip.asset_id`
//!   resolves to an [`Asset`] in `Project.assets[]`.
//! - [`check_effect_track_empty`] — `Track.kind == Effect` implies
//!   `Track.clips.is_empty()`. Effect tracks are container metadata
//!   only; clip-shaped children belong on a video/audio/text track.
//! - [`check_text_clip_text_field`] — text-track clips MUST have
//!   `Clip.text == Some(_)`; non-text-track clips MUST have
//!   `Clip.text == None`. The biconditional shape mirrors the
//!   `asset_id ↔ Track.kind == Text` slice.
//! - [`check_mask_params`] — when `Clip.mask.is_some()`, the
//!   `mask.params` map shape must match the `mask.kind` discriminant
//!   (rect `{w,h} > 0`, ellipse `{rx,ry} > 0`, polygon
//!   `points.len() in 3..=256`, asset `asset_id` resolves to an
//!   image asset with optional `threshold ∈ [0,1]`).
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
//! 7. [`check_speed_on_image_text`] — text/image clip `speed == 1.0`.
//! 8. [`check_speed_curve_on_image_text`] — text/image clip
//!    `speed_curve == None`.
//! 9. [`check_asset_id_biconditional`] — text-track ⇔ nil
//!    `asset_id`.
//! 10. [`check_asset_existence`] — non-nil `asset_id` resolves into
//!     `Project.assets[]`.
//! 11. [`check_effect_track_empty`] — effect tracks carry no clips.
//! 12. [`check_text_clip_text_field`] — text-track ⇔
//!     `Clip.text.is_some()`.
//! 13. [`check_mask_params`] — per-kind `Clip.mask.params` shape.
//! 14. (future slices append here)
//!
//! Biconditional runs before existence intentionally: a nil
//! `asset_id` on a non-text track is a kind-mismatch (structurally
//! clearer error), while running existence first would surface a
//! misleading "asset not found" for a value the spec mandates be
//! nil in the first place. Mask-params runs after asset existence
//! for the same reason — by the time a mask-asset reference is
//! cross-checked, `Project.assets[]` has already been audited and
//! the same `resolve_asset_kind` helper is reused.
//!
//! ## Planned future invariant slices
//!
//! - `check_metadata_size_caps` — `Project.metadata` ≤ 256 keys / 64 KiB.
//! - `check_effect_params_size_caps` — `Effect.params` ≤ 64 keys / 16 KiB.
//! - `check_speed_curve_internal_validity` — `speed_curve` bounds
//!   (`2 ≤ len ≤ 256`, monotonic `time_tk`, factor `[0.001, 100]`).
//! - `check_effect_window_inside_parent` — `0 ≤ in_tk < out_tk ≤
//!   parent_clip.timeline_duration_tk` on every `Effect.window`.
//!
//! ## Spec references
//!
//! - §0.13 — full invariants list.

use std::collections::HashSet;
use std::sync::OnceLock;

use regex::Regex;
use thiserror::Error;
use verbreel_types::{AssetId, ClipId, KeyframeId, Tick};

use crate::asset::Asset;
use crate::clip::{ClipMask, MaskKind};
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

/// Display-clip kind discriminator. Originally carried by
/// [`InvariantViolation::InvalidSourceInTk`]; subsequently reused as
/// the shared "why was this clip flagged" indicator across the
/// text/image display-duration invariant family
/// ([`InvariantViolation::InvalidSpeedOnDisplayClip`],
/// [`InvariantViolation::InvalidSpeedCurveOnDisplayClip`], and any
/// future invariants that target the same predicate). The name was
/// minted before the reuse — kept as-is for type-stability across
/// downstream callers; conceptually it is a display-clip-kind tag.
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

/// `asset_id` shape discriminator carried by
/// [`InvariantViolation::AssetIdBiconditionalViolation`]. Tells the
/// caller which side of the biconditional was broken — `Nil`
/// (text-track-required value found on a non-text track) or `NonNil`
/// (asset-required value found on a text track).
///
/// Lives alongside [`SourceInTkKind`] in the same conceptual slot —
/// a tiny shape-of-the-mismatch hint for diagnostic surfacing. We
/// keep it as a bare enum (not `Option`) because the field's
/// presence on the violation variant *is* the violation; the variant
/// is never constructed when the biconditional holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetIdState {
    /// `Clip.asset_id == nil-UUID`. Valid only on text-track clips.
    Nil,
    /// `Clip.asset_id` is a real `UUIDv7`. Valid only on non-text-
    /// track clips. The referenced id itself lives on the variant
    /// (when the existence check is the one that fired); here it
    /// just discriminates which side of the biconditional broke.
    NonNil,
}

impl std::fmt::Display for AssetIdState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AssetIdState::Nil => f.write_str("nil"),
            AssetIdState::NonNil => f.write_str("non-nil"),
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

/// Centralized "is this clip a display-duration (text or image) clip"
/// predicate. Returns:
///
/// - `Some(SourceInTkKind::Text)` if `track_kind == TrackKind::Text`
///   (text clip — regardless of `asset_id`).
/// - `Some(SourceInTkKind::Image)` if the clip's `asset_id` resolves
///   to `Asset::Image(_)` in `project.assets[]` (image clip on a
///   non-text track).
/// - `None` otherwise: video / audio / subtitle assets, or
///   unresolvable `asset_id` (let the asset-existence invariant —
///   future slice — surface that separately).
///
/// Used by every §0.13 invariant whose semantic boundary is
/// "display-duration clip" — currently [`check_source_in_tk`],
/// [`check_speed_on_image_text`], and
/// [`check_speed_curve_on_image_text`]. Factoring it out keeps the
/// "which clips count as display-kind" answer in one place so future
/// invariants (e.g. the `asset_id` ↔ `Track.kind` biconditional) extend
/// or query the same predicate instead of re-deriving it.
///
/// Note: precedence is `Track.kind == Text` first — the text-track
/// branch wins even if a text-track clip happens to reference an
/// image asset id (which would itself be a separate invariant
/// violation surfaced elsewhere). This matches the existing
/// `check_source_in_tk` behavior verbatim.
fn clip_is_display_kind(
    project: &Project,
    track_kind: TrackKind,
    asset_id: &AssetRef,
) -> Option<SourceInTkKind> {
    if track_kind == TrackKind::Text {
        return Some(SourceInTkKind::Text);
    }
    if matches!(
        resolve_asset_kind(project, asset_id),
        Some(AssetKind::Image)
    ) {
        return Some(SourceInTkKind::Image);
    }
    None
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

    /// `Clip.speed != 1.0` on a clip that the spec requires to be
    /// `1.0` — text clips (parent `Track.kind == Text`) or image
    /// clips (referenced `Asset` is `Image`). Spec §0.13 — *"source-
    /// slice playback rate has no meaning for display-duration kinds;
    /// `clip.set_speed` rejects text/image clips with
    /// `E_CLIP_KIND_MISMATCH`."*
    ///
    /// `project.open` surfaces this with `E_SCHEMA_VIOLATION`;
    /// `apply()` hard-rejects to surface verb-layer bugs loudly.
    /// Equality is exact (`!= 1.0`) on the `f64` — schema default is
    /// integer `1` and verb-layer is forbidden from writing other
    /// values; a hand-edit landing `1.0000000001` should still fail.
    #[error(
        "§0.13 speed invariant: Clip.speed = {speed} on {clip_kind_indicator} clip {clip_id}, \
         must be 1.0"
    )]
    InvalidSpeedOnDisplayClip {
        /// Offending clip id.
        clip_id: ClipId,
        /// Why the clip is required to have `speed == 1.0` —
        /// `Text` (parent Track.kind) or `Image` (referenced Asset).
        clip_kind_indicator: SourceInTkKind,
        /// The actual non-1.0 value found.
        speed: f64,
    },

    /// `Clip.speed_curve` is `Some(_)` on a clip that the spec
    /// requires to be `None` — text clips (parent `Track.kind ==
    /// Text`) or image clips (referenced `Asset` is `Image`). Spec
    /// §0.13 / §0.16 — *"`speed_curve` is forbidden on text and image
    /// clips; `clip.set_speed_curve` rejects with
    /// `E_CLIP_KIND_MISMATCH`."*
    ///
    /// `project.open` surfaces this with `E_SCHEMA_VIOLATION`;
    /// `apply()` hard-rejects. The `point_count` is surfaced for
    /// caller-side debugging (so logs can tell whether the
    /// violating curve is empty / minimal / suspicious-shape without
    /// re-walking the project tree).
    #[error(
        "§0.13 speed_curve invariant: Clip.speed_curve = [{point_count} points] on \
         {clip_kind_indicator} clip {clip_id}, must be None"
    )]
    InvalidSpeedCurveOnDisplayClip {
        /// Offending clip id.
        clip_id: ClipId,
        /// Why the clip is required to have `speed_curve == None` —
        /// `Text` (parent Track.kind) or `Image` (referenced Asset).
        clip_kind_indicator: SourceInTkKind,
        /// Number of points in the offending `speed_curve`.
        /// Diagnostic only — even `0` points is a violation
        /// (the field is `Some(Vec<...>)`, not `None`).
        point_count: usize,
    },

    /// `Clip.asset_id` and the parent `Track.kind` disagree about
    /// the text-track / nil-UUID biconditional. Spec §0.13 —
    /// *"`Clip.asset_id == nil-UUID ⇔ Track.kind == "text"`."*
    ///
    /// Two failure shapes share this variant:
    /// - text-track clip with a non-nil `asset_id` (text clips must
    ///   not reference an asset; their content lives in the
    ///   `text` field), or
    /// - non-text-track clip (video/audio/effect) with `asset_id ==
    ///   nil-UUID` (real clips must reference a real asset).
    ///
    /// `project.open` surfaces this with `E_SCHEMA_VIOLATION`;
    /// `apply()` hard-rejects.
    #[error(
        "§0.13 asset_id biconditional invariant: clip {clip_id} on {track_kind:?} track has \
         {asset_id_state} asset_id, violating the text-track ⇔ nil-UUID biconditional"
    )]
    AssetIdBiconditionalViolation {
        /// Offending clip id.
        clip_id: ClipId,
        /// Parent `Track.kind` — pairs with `asset_id_state` to
        /// disambiguate which side of the biconditional broke.
        track_kind: TrackKind,
        /// Which side of the biconditional the offending value
        /// landed on — `Nil` on a non-text track, or `NonNil` on a
        /// text track.
        asset_id_state: AssetIdState,
    },

    /// `Clip.asset_id` is a non-nil `UUIDv7` that doesn't resolve to
    /// any `Asset.id` in `Project.assets[]`. Spec §0.13 — *"For
    /// every non-text clip, `asset_id` MUST resolve to an existing
    /// `Asset.id`."*
    ///
    /// Layer note: the biconditional check runs first; this variant
    /// fires only when a clip's `asset_id` shape is consistent
    /// (non-nil on a non-text track) but the referenced id is
    /// dangling.
    ///
    /// `project.open` surfaces this with `E_SCHEMA_VIOLATION`;
    /// `apply()` hard-rejects.
    #[error(
        "§0.13 asset-existence invariant: clip {clip_id} references asset_id \
         {referenced_asset_id} which does not exist in Project.assets[]"
    )]
    AssetIdUnresolved {
        /// Offending clip id.
        clip_id: ClipId,
        /// The dangling `AssetId` — surfaced so callers can locate
        /// the missing entry (typically a verb-layer write that
        /// forgot to add the asset, or a hand-edited project.json).
        referenced_asset_id: AssetId,
    },

    /// `Track.kind == Effect` carries one or more clips. Spec §0.13 —
    /// *"Effect tracks are container metadata only; clip-shaped
    /// children live on a video / audio / text track."* A
    /// hand-edited project.json that pushes clips onto an effect
    /// track is rejected (rather than silently dropped) so the
    /// hand-editor sees the structural error.
    ///
    /// `project.open` surfaces this with `E_SCHEMA_VIOLATION`;
    /// `apply()` hard-rejects.
    #[error(
        "§0.13 effect-track-empty invariant: effect track at index \
         {track_index} carries {clip_count} clip(s), must be empty"
    )]
    EffectTrackHasClips {
        /// Index in `tracks[]` of the offending effect track.
        track_index: usize,
        /// Number of clips found on it (helps callers locate the
        /// offending entries without re-walking).
        clip_count: usize,
    },

    /// A text-track clip is missing the `Clip.text` field
    /// (`text == None`). Spec §0.13 — *"Text-track clips carry their
    /// rendered content via `Clip.text`; the field is mandatory on
    /// `Track.kind == Text` and forbidden elsewhere."*
    ///
    /// Companion of [`InvariantViolation::NonTextClipHasTextField`].
    /// The reconciliation pass does NOT auto-normalize — the
    /// explicit-failure path forces the hand-editor to resolve.
    ///
    /// `project.open` surfaces this with `E_SCHEMA_VIOLATION`;
    /// `apply()` hard-rejects.
    #[error(
        "§0.13 text-clip-text-field invariant: clip {clip_id} on text track is missing \
         Clip.text (text-track clips must carry a TextElement)"
    )]
    TextClipMissingTextField {
        /// Offending clip id.
        clip_id: ClipId,
    },

    /// A non-text-track clip (video / audio / effect) has the
    /// `Clip.text` field populated. Spec §0.13 — companion of
    /// [`InvariantViolation::TextClipMissingTextField`] — text
    /// content is forbidden on non-text-kind tracks.
    ///
    /// `project.open` surfaces this with `E_SCHEMA_VIOLATION`;
    /// `apply()` hard-rejects.
    #[error(
        "§0.13 text-clip-text-field invariant: clip {clip_id} on {track_kind:?} track has \
         Clip.text set (only Text-kind tracks may carry a TextElement)"
    )]
    NonTextClipHasTextField {
        /// Offending clip id.
        clip_id: ClipId,
        /// Parent `Track.kind` — explains *why* the text field is
        /// disallowed.
        track_kind: TrackKind,
    },

    /// `Clip.mask.params` does not match the shape required by the
    /// owning `mask.kind` discriminant. Spec §0.13 — *"`Clip.mask`
    /// cross-field invariants per-kind."*
    ///
    /// The schema marks `mask.params` as `additionalProperties:
    /// true` (opaque `Map<String, Value>`) so all per-kind shape
    /// validation lives at the engine layer. This variant is the
    /// single rejection surface; the discriminating
    /// [`MaskParamsError`] sub-enum identifies which per-kind rule
    /// fired.
    ///
    /// `project.open` surfaces this with `E_SCHEMA_VIOLATION` /
    /// `E_MASK_INVALID_PARAMS`; `apply()` hard-rejects.
    #[error(
        "§0.13 mask-params invariant: clip {clip_id} mask (kind={mask_kind:?}) has invalid \
         params: {reason}"
    )]
    InvalidMaskParams {
        /// Offending clip id.
        clip_id: ClipId,
        /// Mask shape kind whose per-kind validation failed.
        mask_kind: MaskKind,
        /// Specific per-kind rule that fired (see [`MaskParamsError`]).
        reason: MaskParamsError,
    },
}

// ---------------------------------------------------------------------
// MaskParamsError
// ---------------------------------------------------------------------

/// Per-kind cross-field reason carried by
/// [`InvariantViolation::InvalidMaskParams::reason`]. Kept as a
/// separate enum (rather than four sibling `InvariantViolation`
/// variants) so the main invariant enum doesn't bloat with
/// mask-specific shapes while still surfacing the precise rule that
/// fired to callers.
///
/// Public so downstream consumers (the upcoming
/// `clip.set_mask_params` verb in particular) can reuse the
/// taxonomy when shaping their own pre-apply rejections.
#[derive(Debug, Clone, Copy, PartialEq, Error)]
pub enum MaskParamsError {
    /// Rect mask: `params.w` and / or `params.h` is missing,
    /// non-numeric, or `≤ 0`. Spec: `mask.kind == "rect"` requires
    /// `w > 0 ∧ h > 0`.
    #[error("rect mask requires params.w > 0 and params.h > 0")]
    RectInvalidWH,

    /// Ellipse mask: `params.rx` and / or `params.ry` is missing,
    /// non-numeric, or `≤ 0`. Spec: `mask.kind == "ellipse"` requires
    /// `rx > 0 ∧ ry > 0`.
    #[error("ellipse mask requires params.rx > 0 and params.ry > 0")]
    EllipseInvalidRxRy,

    /// Polygon mask: `params.points` is missing, not an array, or
    /// has a length outside `3..=256`. Spec: a polygon needs at least
    /// three vertices to enclose an area and is capped at 256 to bound
    /// rasterizer cost.
    #[error("polygon mask requires 3..=256 points, got {count}")]
    PolygonPointsOutOfRange {
        /// Actual vertex count seen (0 when `params.points` is
        /// missing or not an array).
        count: usize,
    },

    /// Asset mask: `params.asset_id` is missing or not a string. Spec:
    /// `mask.kind == "asset"` requires a populated `asset_id` field.
    #[error("asset mask requires params.asset_id")]
    AssetMissingAssetId,

    /// Asset mask: `params.asset_id` parses as a UUID but doesn't
    /// resolve to any entry in `Project.assets[]`. Spec: the mask
    /// asset reference must point at an existing asset.
    #[error(
        "asset mask params.asset_id {referenced_asset_id} does not resolve in Project.assets[]"
    )]
    AssetUnresolvable {
        /// The dangling `AssetId` — surfaced so callers can locate
        /// the missing asset entry.
        referenced_asset_id: AssetId,
    },

    /// Asset mask: `params.asset_id` resolves to an asset that isn't
    /// of kind `image`. Spec: mask sources must be still-image
    /// assets so the alpha channel can be sampled at render time
    /// without temporal interpolation.
    #[error(
        "asset mask params.asset_id {referenced_asset_id} resolves to a non-image asset \
         (actual kind: {actual_kind})"
    )]
    AssetNotImageKind {
        /// The asset id whose kind disqualifies it.
        referenced_asset_id: AssetId,
        /// What the asset actually is.
        actual_kind: &'static str,
    },

    /// Asset mask: `params.threshold` is present but outside `[0, 1]`
    /// (or is non-numeric). Spec: when supplied, the threshold is a
    /// normalized alpha cutoff in `[0, 1]`.
    #[error("asset mask params.threshold = {threshold} is outside [0, 1]")]
    AssetThresholdOutOfRange {
        /// Actual value found — surfaced for debugging.
        threshold: f64,
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
/// Uses [`clip_is_display_kind`] to compute the predicate so every
/// invariant in the display-kind family shares the same "which clips
/// count" answer.
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
            if let Some(kind) = clip_is_display_kind(project, track_kind, &clip.asset_id)
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

// ---------------------------------------------------------------------
// check_speed_on_image_text
// ---------------------------------------------------------------------

/// Verify `Clip.speed == 1.0` on every text clip (parent
/// `Track.kind == Text`) and every image clip (referenced `Asset` is
/// `Image`). Spec §0.13.
///
/// Uses [`clip_is_display_kind`] to compute the predicate so every
/// invariant in the display-kind family shares the same "which clips
/// count" answer.
///
/// Equality is **exact** on the underlying `f64` (`!= 1.0`). The
/// schema default is the integer literal `1`, and the verb-layer
/// (`clip.set_speed`) is forbidden from writing other values on
/// text/image clips. A hand-edit that lands `1.0000000001` would
/// still fail this check — that's intentional; the verb-layer is the
/// only legitimate writer of `speed`, and any drift past exact `1.0`
/// indicates a bypass.
///
/// **Layer note**: `project.open` surfaces this with
/// `E_SCHEMA_VIOLATION`; the verb `clip.set_speed` rejects with
/// `E_CLIP_KIND_MISMATCH`. `apply()` hard-rejects.
///
/// # Errors
///
/// Returns [`InvariantViolation::InvalidSpeedOnDisplayClip`] for the
/// first text or image clip whose `speed != 1.0`.
#[allow(clippy::float_cmp)]
pub fn check_speed_on_image_text(project: &Project) -> Result<(), InvariantViolation> {
    for track in &project.tracks {
        let track_kind = track.kind;
        for clip in &track.clips {
            if let Some(kind) = clip_is_display_kind(project, track_kind, &clip.asset_id)
                && clip.speed != 1.0
            {
                return Err(InvariantViolation::InvalidSpeedOnDisplayClip {
                    clip_id: clip.id,
                    clip_kind_indicator: kind,
                    speed: clip.speed,
                });
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------
// check_speed_curve_on_image_text
// ---------------------------------------------------------------------

/// Verify `Clip.speed_curve == None` on every text clip (parent
/// `Track.kind == Text`) and every image clip (referenced `Asset` is
/// `Image`). Spec §0.13 / §0.16 (v1.1-additive).
///
/// Uses [`clip_is_display_kind`] to compute the predicate so every
/// invariant in the display-kind family shares the same "which clips
/// count" answer.
///
/// The check is on **presence** alone (`Option::is_some`); the
/// curve's internal validity (`2 ≤ len ≤ 256`, monotonic `time_tk`,
/// factor bounds) is a separate slice. Even a zero-point
/// `Some(vec![])` is a violation here — the field's `None`-ness is
/// the spec-mandated state.
///
/// **Layer note**: `project.open` surfaces this with
/// `E_SCHEMA_VIOLATION`; the verb `clip.set_speed_curve` rejects with
/// `E_CLIP_KIND_MISMATCH`. `apply()` hard-rejects.
///
/// # Errors
///
/// Returns [`InvariantViolation::InvalidSpeedCurveOnDisplayClip`]
/// for the first text or image clip whose `speed_curve != None`.
pub fn check_speed_curve_on_image_text(project: &Project) -> Result<(), InvariantViolation> {
    for track in &project.tracks {
        let track_kind = track.kind;
        for clip in &track.clips {
            if let Some(kind) = clip_is_display_kind(project, track_kind, &clip.asset_id)
                && let Some(curve) = clip.speed_curve.as_ref()
            {
                return Err(InvariantViolation::InvalidSpeedCurveOnDisplayClip {
                    clip_id: clip.id,
                    clip_kind_indicator: kind,
                    point_count: curve.len(),
                });
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------
// check_asset_id_biconditional
// ---------------------------------------------------------------------

/// Verify the spec §0.13 `Clip.asset_id ↔ Track.kind` biconditional:
/// `Clip.asset_id == nil-UUID` iff the parent `Track.kind` is
/// [`TrackKind::Text`]. Two failure shapes share the variant:
///
/// - text-track clip with a non-nil `asset_id` — text clips carry
///   their content in `Clip.text`, never via `asset_id`.
/// - non-text-track clip with `asset_id == nil-UUID` — real clips
///   (video / audio / effect track) must reference a real asset.
///
/// **Layer note**: `project.open` surfaces this with
/// `E_SCHEMA_VIOLATION`; mutating verbs that would write either
/// shape fail with `E_SCHEMA_VIOLATION` before patch computation.
/// `apply()` hard-rejects.
///
/// **Check order**: runs *before* [`check_asset_existence`] so that
/// a non-text-track clip with `asset_id == nil-UUID` surfaces as a
/// biconditional violation (structurally clearer) rather than a
/// confusing "nil-UUID not found in `Project.assets[]`" miss.
///
/// # Errors
///
/// Returns [`InvariantViolation::AssetIdBiconditionalViolation`] for
/// the first clip where the biconditional breaks.
pub fn check_asset_id_biconditional(project: &Project) -> Result<(), InvariantViolation> {
    for track in &project.tracks {
        let is_text_track = track.kind == TrackKind::Text;
        for clip in &track.clips {
            let is_nil = clip.asset_id.is_nil();
            // Biconditional: is_text_track ⇔ is_nil.
            // Violation when exactly one side is true.
            if is_text_track != is_nil {
                return Err(InvariantViolation::AssetIdBiconditionalViolation {
                    clip_id: clip.id,
                    track_kind: track.kind,
                    asset_id_state: if is_nil {
                        AssetIdState::Nil
                    } else {
                        AssetIdState::NonNil
                    },
                });
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------
// check_asset_existence
// ---------------------------------------------------------------------

/// Verify every non-nil [`Clip.asset_id`](crate::clip::Clip) resolves
/// to an [`Asset`] in `Project.assets[]`. Spec §0.13.
///
/// Allocates one `HashSet<&AssetId>` over `project.assets[]` for
/// O(1)-amortized lookup, then walks clips in declared order. Nil
/// `asset_id` values are skipped (the biconditional check — which
/// runs first in `apply()` — handles the "nil where non-nil
/// required" case; here we only care about non-nil refs that
/// dangle).
///
/// **Layer note**: `project.open` surfaces this with
/// `E_SCHEMA_VIOLATION`; `apply()` hard-rejects.
///
/// # Errors
///
/// Returns [`InvariantViolation::AssetIdUnresolved`] for the first
/// clip whose non-nil `asset_id` doesn't resolve.
pub fn check_asset_existence(project: &Project) -> Result<(), InvariantViolation> {
    // Index the project's assets once. `Asset::id` returns `&AssetId`
    // (Copy), so the set can borrow into `project.assets`.
    let asset_ids: HashSet<&AssetId> = project.assets.iter().map(Asset::id).collect();
    for track in &project.tracks {
        for clip in &track.clips {
            if let Some(id) = clip.asset_id.id()
                && !asset_ids.contains(id)
            {
                return Err(InvariantViolation::AssetIdUnresolved {
                    clip_id: clip.id,
                    referenced_asset_id: *id,
                });
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------
// check_effect_track_empty
// ---------------------------------------------------------------------

/// Verify every `Track.kind == Effect` track carries zero clips. Spec
/// §0.13.
///
/// Effect tracks are container metadata only — they exist to host
/// track-scoped `Effect` entries (Phase 2 fourth slice will land the
/// typed `Track.effects` field), not clip-shaped children. A hand-
/// edited project.json that pushes clips onto an effect track is
/// rejected rather than silently dropped so the editor surfaces the
/// structural error.
///
/// **Layer note**: `project.open` surfaces this with
/// `E_SCHEMA_VIOLATION`; the verb `clip.add` rejects when the
/// destination track is `Effect` with `E_TRACK_KIND_MISMATCH`.
/// `apply()` hard-rejects.
///
/// # Errors
///
/// Returns [`InvariantViolation::EffectTrackHasClips`] for the first
/// effect track with non-empty `clips`.
pub fn check_effect_track_empty(project: &Project) -> Result<(), InvariantViolation> {
    for (track_index, track) in project.tracks.iter().enumerate() {
        if track.kind == TrackKind::Effect && !track.clips.is_empty() {
            return Err(InvariantViolation::EffectTrackHasClips {
                track_index,
                clip_count: track.clips.len(),
            });
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------
// check_text_clip_text_field
// ---------------------------------------------------------------------

/// Verify the spec §0.13 `Clip.text` ↔ `Track.kind == Text`
/// biconditional. Two failure shapes share the check (each with its
/// own variant for clean caller pattern-matching):
///
/// - text-track clip with `Clip.text == None` →
///   [`InvariantViolation::TextClipMissingTextField`]. Text clips
///   carry their rendered content via the `text` field, never via
///   `asset_id`, so the field is mandatory on text-track clips.
/// - non-text-track clip with `Clip.text == Some(_)` →
///   [`InvariantViolation::NonTextClipHasTextField`]. The `text`
///   field has no rendering pipeline on video / audio / effect
///   tracks and would silently confuse `project.open`'s
///   text-renderer dispatch.
///
/// **Layer note**: `project.open` surfaces these with
/// `E_SCHEMA_VIOLATION`; the verb-layer cannot construct either
/// shape (`clip.add` / `text.set` reject with
/// `E_CLIP_KIND_MISMATCH`). `apply()` hard-rejects.
///
/// # Errors
///
/// Returns one of the two text-field variants on the first offending
/// clip.
pub fn check_text_clip_text_field(project: &Project) -> Result<(), InvariantViolation> {
    for track in &project.tracks {
        let is_text_track = track.kind == TrackKind::Text;
        for clip in &track.clips {
            match (is_text_track, clip.text.is_some()) {
                (true, false) => {
                    return Err(InvariantViolation::TextClipMissingTextField { clip_id: clip.id });
                }
                (false, true) => {
                    return Err(InvariantViolation::NonTextClipHasTextField {
                        clip_id: clip.id,
                        track_kind: track.kind,
                    });
                }
                _ => {}
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------
// check_mask_params
// ---------------------------------------------------------------------

/// Verify every `Clip.mask` (when present) carries `params` matching
/// the shape required by its `mask.kind` discriminant. Spec §0.13.
///
/// The schema declares `mask.params` as
/// `additionalProperties: true` (an opaque `Map<String, Value>`) so
/// all per-kind validation lives at the engine layer. Per-kind rules
/// (spec §0.13):
///
/// - `MaskKind::Rect` — `params.w > 0 ∧ params.h > 0`.
/// - `MaskKind::Ellipse` — `params.rx > 0 ∧ params.ry > 0`.
/// - `MaskKind::Polygon` — `3 ≤ params.points.len() ≤ 256`.
/// - `MaskKind::Asset` — `params.asset_id` parses + resolves to an
///   image-kind `Asset`; optional `params.threshold` ∈ `[0, 1]`.
///
/// Discriminating reasons surface through [`MaskParamsError`] in the
/// [`InvariantViolation::InvalidMaskParams::reason`] field so callers
/// can pattern-match precisely (the upcoming `clip.set_mask_params`
/// verb will reuse the taxonomy for pre-apply rejections).
///
/// **Layer note**: `project.open` surfaces this with
/// `E_SCHEMA_VIOLATION` / `E_MASK_INVALID_PARAMS`; `apply()` hard-
/// rejects.
///
/// **Check order**: runs *after* [`check_asset_existence`] so the
/// asset-mask asset-id resolution can rely on `Project.assets[]`
/// having been audited first (and reuses the same
/// [`resolve_asset_kind`] helper).
///
/// # Errors
///
/// Returns [`InvariantViolation::InvalidMaskParams`] for the first
/// mask whose per-kind shape is invalid.
pub fn check_mask_params(project: &Project) -> Result<(), InvariantViolation> {
    for track in &project.tracks {
        for clip in &track.clips {
            let Some(mask) = clip.mask.as_ref() else {
                continue;
            };
            if let Err(reason) = validate_mask_params(project, mask) {
                return Err(InvariantViolation::InvalidMaskParams {
                    clip_id: clip.id,
                    mask_kind: mask.kind,
                    reason,
                });
            }
        }
    }
    Ok(())
}

/// Per-kind shape validator for [`ClipMask::params`]. Pure helper —
/// no project mutation, no allocation beyond `serde_json` value
/// inspection. Returned `MaskParamsError` is wrapped by
/// [`check_mask_params`] into the user-facing
/// [`InvariantViolation::InvalidMaskParams`] variant.
fn validate_mask_params(project: &Project, mask: &ClipMask) -> Result<(), MaskParamsError> {
    match mask.kind {
        MaskKind::Rect => {
            let w = mask.params.get("w").and_then(serde_json::Value::as_f64);
            let h = mask.params.get("h").and_then(serde_json::Value::as_f64);
            match (w, h) {
                (Some(w), Some(h)) if w > 0.0 && h > 0.0 => Ok(()),
                _ => Err(MaskParamsError::RectInvalidWH),
            }
        }
        MaskKind::Ellipse => {
            let rx = mask.params.get("rx").and_then(serde_json::Value::as_f64);
            let ry = mask.params.get("ry").and_then(serde_json::Value::as_f64);
            match (rx, ry) {
                (Some(rx), Some(ry)) if rx > 0.0 && ry > 0.0 => Ok(()),
                _ => Err(MaskParamsError::EllipseInvalidRxRy),
            }
        }
        MaskKind::Polygon => {
            // Missing or non-array `points` reports count=0 — the
            // PolygonPointsOutOfRange variant covers both "not enough"
            // and "wrong shape" with one error per spec §0.13.
            let count = mask
                .params
                .get("points")
                .and_then(serde_json::Value::as_array)
                .map_or(0, Vec::len);
            if (3..=256).contains(&count) {
                Ok(())
            } else {
                Err(MaskParamsError::PolygonPointsOutOfRange { count })
            }
        }
        MaskKind::Asset => {
            let asset_id_str = mask
                .params
                .get("asset_id")
                .and_then(serde_json::Value::as_str)
                .ok_or(MaskParamsError::AssetMissingAssetId)?;
            // `AssetId` parses via `UuidV7::from_str` underneath — a
            // non-UUIDv7 string fails the parse and is funneled into
            // AssetMissingAssetId so callers don't have to handle a
            // separate "malformed" variant. The hand-editor case where
            // params.asset_id is literally absent and the case where
            // it's "lolnotauuid" both indicate "no resolvable asset
            // reference"; one error variant is enough.
            let asset_id: AssetId = asset_id_str
                .parse()
                .map_err(|_| MaskParamsError::AssetMissingAssetId)?;
            let asset_ref = AssetRef::from_id(asset_id);
            match resolve_asset_kind(project, &asset_ref) {
                Some(AssetKind::Image) => {}
                Some(other) => {
                    return Err(MaskParamsError::AssetNotImageKind {
                        referenced_asset_id: asset_id,
                        actual_kind: asset_kind_name(other),
                    });
                }
                None => {
                    return Err(MaskParamsError::AssetUnresolvable {
                        referenced_asset_id: asset_id,
                    });
                }
            }
            // Threshold is optional — spec says "when present" — so
            // absent is OK. When present, must be a finite f64 in
            // [0, 1].
            if let Some(threshold_value) = mask.params.get("threshold") {
                let threshold =
                    threshold_value
                        .as_f64()
                        .ok_or(MaskParamsError::AssetThresholdOutOfRange {
                            threshold: f64::NAN,
                        })?;
                if !(0.0..=1.0).contains(&threshold) {
                    return Err(MaskParamsError::AssetThresholdOutOfRange { threshold });
                }
            }
            Ok(())
        }
    }
}

/// Pretty-name for an [`AssetKind`] used in
/// [`MaskParamsError::AssetNotImageKind::actual_kind`]. Mirrors the
/// schema's `Asset.kind` enum literals so the error message reads as
/// the user's spec-level vocabulary.
fn asset_kind_name(kind: AssetKind) -> &'static str {
    match kind {
        AssetKind::Video => "video",
        AssetKind::Audio => "audio",
        AssetKind::Image => "image",
        AssetKind::Subtitle => "subtitle",
    }
}
