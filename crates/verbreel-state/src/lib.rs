//! verbreel-state — the engine kernel. Project graph types, §0.13
//! invariants, `apply()`/reconstructor.
//!
//! This is the Phase 2 **first-slice** landing — see issue #19. It
//! defines just enough of the [`Project`] type (and its handful of
//! supporting `$defs`) to round-trip an empty
//! `project.create`'d project through serde + RFC 8785 canonicalization
//! + `verbreel-canon::project_hash`.
//!
//! ## What's here
//!
//! - [`Project`] — root of the project graph (16 fields from the
//!   canonical `spec/project-schema.json`).
//! - [`Canvas`] — width/height + background + pixel-aspect.
//! - [`Track`] + [`TrackKind`] — tracks and their kind enum
//!   (`video`/`audio`/`text`/`effect`).
//! - [`Marker`] — project-level time markers.
//! - [`Tracker`] — placeholder shape until §18 work lands.
//! - [`Asset`] tagged-union enum + the 4 variant structs
//!   ([`VideoAsset`], [`AudioAsset`], [`ImageAsset`],
//!   [`SubtitleAsset`]) + per-variant metadata.
//! - [`Sha256`], [`AssetPath`], [`AssetRef`] regex-validated
//!   newtypes (Phase 2 second slice — `AssetRef` is the Clip
//!   blocker).
//! - [`RotationDeg`] enum + [`FileFingerprint`] struct.
//!
//! ## What's deferred (follow-up slices)
//!
//! - `Clip` typing (currently `Vec<serde_json::Value>` placeholder on
//!   [`Track::clips`]). Next slice — uses [`crate::AssetRef`] from
//!   this slice.
//! - `Effect`, `Keyframe`, `TextElement`, `Transform`, `Shadow`,
//!   `Color` newtype.
//! - `apply(patch) -> Result<Project>` — the json-patch consumer.
//! - Event-log integration (§0.8 write-ordering).
//! - §0.13 invariant enforcement (track contiguity, no-overlap,
//!   fade clamp, asset hash uniqueness, `AssetPath` prefix-of-hash,
//!   fingerprint clamp).
//! - `project.open` reconciliation passes.
//! - `project.create` / `project.save` verb implementations.
//!
//! ## Spec references
//!
//! - `spec/project-schema.json` (the canonical source of truth — the
//!   shape this crate's structs encode).
//! - `spec/commands/conventions.md` §0.13 (invariants list, deferred).
//! - `spec/commands/project.md` §2.1 (`project.create` seeded tracks).
//! - `spec/research/05-storage-state.md` (storage rationale).

#![deny(missing_docs)]
#![deny(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]
#![warn(clippy::pedantic)]
// Names match spec §0.13 / $defs identifiers intentionally; the
// "Project" type lives in the `project` module and that's fine.
#![allow(clippy::module_name_repetitions)]

pub mod asset;
pub mod asset_meta;
pub mod canvas;
pub mod marker;
pub mod newtypes;
pub mod project;
pub mod track;
pub mod tracker;

pub use asset::{Asset, AudioAsset, ImageAsset, SubtitleAsset, VideoAsset};
pub use asset_meta::{
    AudioAssetMetadata, FileFingerprint, ImageAssetMetadata, RotationDeg, SubtitleAssetMetadata,
    VideoAssetMetadata,
};
pub use canvas::Canvas;
pub use marker::Marker;
pub use newtypes::{AssetNewtypeError, AssetPath, AssetRef, Sha256};
pub use project::{Project, SCHEMA_VERSION};
pub use track::{Track, TrackKind};
pub use tracker::Tracker;

// Re-export the tick rate constant from verbreel-types so downstream
// crates (verbreel-args, the verb implementations) can refer to it via
// `verbreel_state::TICK_RATE_HZ` without needing a separate import path.
pub use verbreel_types::TICK_RATE_HZ;
