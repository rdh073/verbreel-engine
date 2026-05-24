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
//! - [`Sha256`], [`AssetPath`], [`AssetRef`], [`Color`]
//!   regex-validated newtypes.
//! - [`RotationDeg`] enum + [`FileFingerprint`] struct.
//! - [`Clip`] + [`Transform`] + [`Shadow`] + [`TextElement`] +
//!   [`FadeCurve`] / [`BlendMode`] / [`MaskKind`] enums +
//!   [`ClipMask`] / [`SpeedCurvePoint`] (Phase 2 third slice).
//! - [`Effect`] + [`EffectKind`] + [`EffectWindow`] (Phase 2 fourth
//!   slice). [`Clip::effects`] is now `Vec<Effect>`.
//! - [`Keyframe`] + [`KeyframeProperty`] + [`Easing`] (Phase 2 fifth
//!   slice). [`Clip::keyframes`] is now `Vec<Keyframe>`. Every
//!   nested `$def` in `spec/project-schema.json` is now typed.
//! - [`Project::apply`] + [`ApplyError`] (Phase 2 sixth and FINAL
//!   state-typing slice). MVP — pure RFC 6902 patch application;
//!   §0.13 invariant enforcement is deferred to follow-up slices.
//!
//! ## What's deferred (follow-up slices)
//!
//! - `Effect` typing on tracks (currently `Vec<serde_json::Value>`
//!   placeholder on [`Track::effects`] — replacement coupled with
//!   track-level effects work in a future slice; the Effect slice
//!   only replaced [`Clip::effects`] per task scope).
//! - Event-log integration (§0.8 write-ordering).
//! - §0.13 invariant enforcement (track contiguity, no-overlap,
//!   fade clamp, asset hash uniqueness, `AssetPath` prefix-of-hash,
//!   fingerprint clamp, `source_in_tk == 0` for image/text clips,
//!   `speed_curve` forbidden on image/text clips, biconditional
//!   `Clip.asset_id == nil-UUID ⇔ Track.kind == "text"`).
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

pub mod apply;
pub mod asset;
pub mod asset_meta;
pub mod canvas;
pub mod clip;
pub mod effect;
pub mod invariants;
pub mod keyframe;
#[cfg(feature = "native")]
pub mod lifecycle;
pub mod marker;
pub mod newtypes;
pub mod project;
pub mod shadow;
pub mod text_element;
pub mod track;
pub mod tracker;
pub mod transform;

pub use apply::ApplyError;
pub use asset::{Asset, AudioAsset, ImageAsset, SubtitleAsset, VideoAsset};
pub use asset_meta::{
    AudioAssetMetadata, FileFingerprint, ImageAssetMetadata, RotationDeg, SubtitleAssetMetadata,
    VideoAssetMetadata,
};
pub use canvas::Canvas;
pub use clip::{BlendMode, Clip, ClipMask, FadeCurve, MaskKind, SpeedCurvePoint};
pub use effect::{
    Effect, EffectKind, EffectNewtypeError, EffectWindow, EffectWindowDependencyError,
};
pub use invariants::{
    InvariantViolation, check_duration_tk, check_fade_clamp, check_no_overlap,
    check_track_contiguity, timeline_duration_tk,
};
pub use keyframe::{Easing, Keyframe, KeyframeNewtypeError, KeyframeProperty};
#[cfg(feature = "native")]
pub use lifecycle::{LifecycleError, ProjectStore, SaveInfo};
pub use marker::Marker;
pub use newtypes::{AssetNewtypeError, AssetPath, AssetRef, Color, Sha256};
pub use project::{Project, SCHEMA_VERSION};
pub use shadow::Shadow;
pub use text_element::{TextAlign, TextElement};
pub use track::{Track, TrackKind};
pub use tracker::Tracker;
pub use transform::Transform;

// Re-export the tick rate constant from verbreel-types so downstream
// crates (verbreel-args, the verb implementations) can refer to it via
// `verbreel_state::TICK_RATE_HZ` without needing a separate import path.
pub use verbreel_types::TICK_RATE_HZ;
