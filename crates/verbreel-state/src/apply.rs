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
//! ## What this module does NOT do
//!
//! `apply()` enforces only **type-level** validity (the patched
//! `Value` must deserialize back into `Project`). §0.13 engine
//! invariants — fade clamp, track contiguity, no-overlap, the
//! `Project.duration_tk` maintenance contract, dangling-keyframe
//! cascade, etc. — are deliberately NOT enforced at this MVP level.
//! They land in follow-up slices, one invariant family per slice,
//! each gated by a new [`ApplyError`] variant under
//! `InvariantViolation::<Kind>`.
//!
//! The MVP boundary is locked by an explicit test
//! (`apply_does_not_enforce_invariants` in `tests/apply.rs`) which
//! the follow-up fade-clamp slice will flip from a success
//! assertion to a failure assertion.
//!
//! ## Spec references
//!
//! - §0.6 — JSON Patch contract (RFC 6902).
//! - §0.8 — write-ordering protocol; `apply()` is step 3
//!   (events.jsonl is written BEFORE this is called).
//! - §0.13 — invariants list. Explicit out-of-scope reference for
//!   the follow-up slices.

use thiserror::Error;

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
    // TODO(follow-up slices §0.13): add `InvariantViolation::<Kind>`
    // variants here, one per invariant family. Examples planned:
    //   InvariantViolation::FadeClamp { clip_id, .. }
    //   InvariantViolation::TrackContiguity { .. }
    //   InvariantViolation::NoOverlap { track_id, clip_a, clip_b }
    //   InvariantViolation::DanglingKeyframe { keyframe_id, .. }
    //   InvariantViolation::DurationOutOfSync { .. }
    // The MVP boundary test `apply_does_not_enforce_invariants` will
    // be flipped to assert failure when the corresponding variant
    // lands.
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
    /// MVP scope: this method enforces only **type-level** validity
    /// (the patched `Value` must deserialize back into `Project`).
    /// §0.13 engine invariants (fade clamp, track contiguity,
    /// no-overlap, `duration_tk` maintenance, dangling-keyframe
    /// cascade, etc.) are deliberately NOT enforced here — they land
    /// in follow-up slices, each gated by their own [`ApplyError`]
    /// variants.
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
    pub fn apply(&self, patch: &json_patch::Patch) -> Result<Self, ApplyError> {
        let mut value = serde_json::to_value(self).map_err(ApplyError::SerializationFailed)?;
        json_patch::patch(&mut value, patch)?;
        let project: Self = serde_json::from_value(value).map_err(ApplyError::TypeViolation)?;
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
