//! [`Project::apply`] — RFC 6902 JSON Patch application against the
//! typed [`Project`] graph. Phase 2 sixth (and FINAL state-typing)
//! slice.
//!
//! ## What this module does
//!
//! `apply()` is the pure functional building block on top of which
//! the event-flow code (§0.8 step 3 of the write-ordering protocol)
//! will compose. Given an immutable `&Project` and an RFC 6902
//! patch:
//!
//! 1. Serialize `self` to `serde_json::Value`.
//! 2. Apply the patch atomically (rollback on failure) via
//!    [`json_patch::patch`].
//! 3. Deserialize the patched `Value` back into a typed [`Project`].
//!
//! Any failure surfaces as a typed [`ApplyError`] variant. The input
//! is never mutated — callers receive a new `Project`.
//!
//! ## §0.13 invariant enforcement
//!
//! After the deserialize step succeeds, `apply()` composes the
//! [`crate::invariants`] `check_*` family in a deterministic order:
//!
//! 1. `check_fade_clamp` — every clip satisfies
//!    `fade_in_tk + fade_out_tk ≤ timeline_duration_tk`.
//! 2. `check_track_contiguity` — tracks of the same kind are grouped
//!    in `Project.tracks[]`.
//! 3. `check_no_overlap` — clips on the same track don't overlap.
//! 4. `check_duration_tk` — `Project.duration_tk` equals the
//!    max of `track_position_tk + timeline_duration_tk` across all
//!    clips.
//! 5. `check_dangling_keyframes` — every effect-targeting keyframe
//!    references an effect that exists on the parent clip.
//! 6. `check_source_in_tk` — text/image clips have
//!    `source_in_tk == 0`.
//! 7. `check_speed_on_image_text` — text/image clips have
//!    `speed == 1.0`.
//! 8. `check_speed_curve_on_image_text` — text/image clips have
//!    `speed_curve == None`.
//! 9. `check_asset_id_biconditional` — `Clip.asset_id == nil-UUID`
//!    iff `Track.kind == Text`.
//! 10. `check_asset_existence` — every non-nil `Clip.asset_id`
//!     resolves to an `Asset.id` in `Project.assets[]`.
//!
//! Subsequent slices add more checks here. Each violation surfaces
//! as [`ApplyError::InvariantViolation`] wrapping the typed
//! [`crate::invariants::InvariantViolation`] enum so callers can
//! pattern-match on the specific failure.
//!
//! ## Spec references
//!
//! - §0.6 — JSON Patch contract (RFC 6902).
//! - §0.8 — write-ordering protocol; `apply()` is step 3
//!   (events.jsonl is written BEFORE this is called).
//! - §0.13 — invariants list. Explicit out-of-scope reference for
//!   the follow-up slices.

use thiserror::Error;

use crate::invariants::{
    InvariantViolation, check_asset_existence, check_asset_id_biconditional,
    check_dangling_keyframes, check_duration_tk, check_fade_clamp, check_no_overlap,
    check_source_in_tk, check_speed_curve_on_image_text, check_speed_on_image_text,
    check_track_contiguity,
};
use crate::project::Project;

// ---------------------------------------------------------------------
// ApplyError
// ---------------------------------------------------------------------

/// Errors surfaced by [`Project::apply`] / [`Project::apply_ops`].
///
/// Not marked `#[non_exhaustive]` deliberately — that would force
/// downstream callers into a catch-all `_` arm even after the §0.13
/// invariant variants are added in follow-up slices (anti-ergonomic
/// since the typical caller wants to pattern-match each known
/// failure mode and treat unknown ones as a hard bug). Future
/// variants will be additive (new arms callers may want to handle);
/// breaking caller pattern-matches is acceptable when a new
/// invariant variant has different semantics from `TypeViolation`.
#[derive(Debug, Error)]
pub enum ApplyError {
    /// RFC 6902 patch failed. Wraps the upstream [`json_patch::PatchError`]
    /// — which carries `operation` index, `path`, and `kind` (test-
    /// failed, invalid-pointer, etc.).
    #[error("RFC 6902 patch failed: {source}")]
    PatchFailed {
        /// The underlying patch error from `json-patch`.
        #[from]
        source: json_patch::PatchError,
    },

    /// Internal serialization of `&Project` to `serde_json::Value`
    /// failed. Should not normally happen — `Project` has a stable
    /// serde impl — but kept honest at the type level.
    #[error("Project → serde_json::Value serialization failed: {0}")]
    SerializationFailed(serde_json::Error),

    /// The patched `Value` did not deserialize back into a typed
    /// `Project`. Typical cause: the patch wrote a value with the
    /// wrong shape (e.g. a string into a `Tick` integer field, or a
    /// non-UUIDv7 into an ID slot, or an out-of-pattern asset path).
    #[error(
        "patch applied but result violates the typed Project schema (a patch op wrote a value \
         that doesn't match its destination's type): {0}"
    )]
    TypeViolation(serde_json::Error),

    /// The patch produced a Project that violates a §0.13 engine
    /// invariant. See [`InvariantViolation`] for the per-invariant
    /// variants currently checked.
    ///
    /// Each subsequent invariant slice adds a new variant to
    /// [`InvariantViolation`]; callers' exhaustive matches are
    /// expected to re-prompt for handling at that point (the design
    /// intent — see the type-level doc comment for the
    /// `#[non_exhaustive]`-NOT rationale).
    #[error("§0.13 invariant violation: {0}")]
    InvariantViolation(#[from] InvariantViolation),
}

// ---------------------------------------------------------------------
// Project::apply / Project::apply_ops
// ---------------------------------------------------------------------

impl Project {
    /// Apply an RFC 6902 JSON Patch to this project, returning a new
    /// [`Project`]. Pure functional — `&self` is unchanged.
    ///
    /// The patch is applied atomically: if any operation in the
    /// patch fails, the intermediate `Value` is rolled back to its
    /// pre-patch state (then the rolled-back `Value` is discarded —
    /// the caller's `&self` is never touched). This is the upstream
    /// `json-patch::patch` behavior (as opposed to `patch_unsafe`,
    /// which leaves the document in a partially-applied state on
    /// failure).
    ///
    /// Enforces **type-level** validity (the patched `Value` must
    /// deserialize back into `Project`) AND every §0.13 invariant
    /// landed so far (fade clamp as of this slice; future slices
    /// add track contiguity, no-overlap, `duration_tk` maintenance,
    /// dangling-keyframe cascade, etc.).
    ///
    /// # Errors
    ///
    /// - [`ApplyError::PatchFailed`] — RFC 6902 op failed (bad path,
    ///   test-op mismatch, etc.). Carries the upstream
    ///   [`json_patch::PatchError`] with the failing op index +
    ///   path.
    /// - [`ApplyError::SerializationFailed`] — internal `&Project →
    ///   Value` step failed. Should not normally happen.
    /// - [`ApplyError::TypeViolation`] — the patch applied
    ///   successfully but the resulting `Value` is no longer a
    ///   well-typed `Project` (e.g. a string where a `Tick` is
    ///   expected). Carries the underlying serde error.
    /// - [`ApplyError::InvariantViolation`] — the patched Project is
    ///   well-typed but violates a §0.13 invariant (currently
    ///   `fade_in_tk + fade_out_tk ≤ timeline_duration_tk`).
    pub fn apply(&self, patch: &json_patch::Patch) -> Result<Self, ApplyError> {
        let mut value = serde_json::to_value(self).map_err(ApplyError::SerializationFailed)?;
        json_patch::patch(&mut value, patch)?;
        let project: Self = serde_json::from_value(value).map_err(ApplyError::TypeViolation)?;
        // §0.13 invariant chain — order matters for deterministic
        // error reporting. See `crate::invariants` module docs.
        check_fade_clamp(&project)?;
        check_track_contiguity(&project)?;
        check_no_overlap(&project)?;
        check_duration_tk(&project)?;
        check_dangling_keyframes(&project)?;
        check_source_in_tk(&project)?;
        check_speed_on_image_text(&project)?;
        check_speed_curve_on_image_text(&project)?;
        check_asset_id_biconditional(&project)?;
        check_asset_existence(&project)?;
        Ok(project)
    }

    /// Batch helper — wrap `ops` in a [`json_patch::Patch`] and
    /// delegate to [`Project::apply`]. Convenience for callers that
    /// have a slice of [`json_patch::PatchOperation`] but don't want
    /// to construct the wrapping `Patch` themselves.
    ///
    /// # Errors
    ///
    /// Same as [`Project::apply`].
    pub fn apply_ops(&self, ops: &[json_patch::PatchOperation]) -> Result<Self, ApplyError> {
        self.apply(&json_patch::Patch(ops.to_vec()))
    }
}
