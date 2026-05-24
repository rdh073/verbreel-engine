//! Per-verb modules. Each verb declares:
//!
//! - its args / data / error types,
//! - a `compute_patch()` freestanding helper (pure — no I/O, no clock,
//!   no RNG),
//! - a `*Reconstructor` impl of
//!   [`crate::reconstructor::VerbReconstructor`] (also pure per §0.8).
//!
//! Verbs land one at a time. `project.set_metadata` (§2.12) is the
//! first — proves the reconstructor-purity contract end-to-end without
//! touching `ProjectStore::mutate()` or the startup gate.
//!
//! ## What lives here vs. elsewhere
//!
//! - The freestanding `compute_patch()` returns the RFC 6902 patch
//!   value and the post-state shape it implies. It does NOT write to
//!   the event log, does NOT apply the patch in place, does NOT touch
//!   `ProjectStore`. Those are kernel-integration concerns landing in a
//!   subsequent slice (B2 / B3) — this module is the pure verb logic.
//! - The `*Reconstructor` impl rebuilds the envelope `data` field from
//!   the recorded `(args, patch, warnings, post-state)` 5-tuple per
//!   §0.8 reconstructor purity. It is the validation surface
//!   exercised by [`crate::validate_reconstructors`] at the §0.8
//!   startup gate (wiring is Slice B2).
//!
//! ## Spec references
//!
//! - `spec/commands/project.md` §2.12 (`project.set_metadata`).
//! - `spec/commands/conventions.md` §0.13 (metadata size caps).
//! - `spec/commands/conventions.md` §0.8 (reconstructor purity).

pub mod project_set_metadata;
