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
pub mod idempotency;
pub mod invariants;
pub mod keyframe;
#[cfg(feature = "native")]
pub mod lifecycle;
pub mod marker;
pub mod newtypes;
pub mod project;
pub mod reconstructor;
pub mod shadow;
pub mod text_element;
pub mod track;
pub mod tracker;
pub mod transform;
pub mod verbs;

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
pub use idempotency::{
    AlreadyExists, Clock, DEFAULT_STALE_IN_PROGRESS, DEFAULT_TTL, Entry, EntryState,
    IdempotencyIndex, LookupOutcome, MockClock, SystemClock,
};
pub use invariants::{
    AssetIdState, EFFECT_PARAMS_MAX_BYTES, EFFECT_PARAMS_MAX_KEYS, InvariantViolation,
    METADATA_MAX_BYTES, METADATA_MAX_KEYS, MaskParamsError, SourceInTkKind, check_asset_existence,
    check_asset_id_biconditional, check_asset_id_uniqueness, check_dangling_keyframes,
    check_duration_tk, check_effect_params_caps, check_effect_track_empty,
    check_effect_window_within_parent, check_fade_clamp, check_mask_params, check_metadata_caps,
    check_no_overlap, check_source_in_tk, check_speed_curve_on_image_text,
    check_speed_on_image_text, check_text_clip_text_field, check_track_contiguity,
    extract_effect_id_from_property, timeline_duration_tk,
};
pub use keyframe::{Easing, Keyframe, KeyframeNewtypeError, KeyframeProperty};
#[cfg(feature = "native")]
pub use lifecycle::{LifecycleError, MutateOutcome, ProjectStore, SaveInfo};
pub use marker::Marker;
pub use newtypes::{AssetNewtypeError, AssetPath, AssetRef, Color, Sha256};
pub use project::{Project, SCHEMA_VERSION};
pub use reconstructor::{
    ReconstructError, RecordedEvent, RegistryError, ValidationError, ValidationReport, Verb,
    VerbError, VerbRegistry, validate_reconstructors,
};
// Deprecated alias kept for one slice cycle (Slice B3 rename). Down-
// stream crates pinned to `VerbReconstructor` keep compiling; new code
// MUST use `Verb`.
#[allow(deprecated)]
pub use reconstructor::VerbReconstructor;
pub use shadow::Shadow;
pub use text_element::{TextAlign, TextElement};
pub use track::{Track, TrackKind};
pub use tracker::Tracker;
pub use transform::Transform;
pub use verbs::project_set_canvas::{
    CANVAS_MAX_DIM, CANVAS_MIN_DIM, PIXEL_ASPECT_MIN, ProjectSetCanvasArgs, ProjectSetCanvasData,
    ProjectSetCanvasError, ProjectSetCanvasVerb,
};
pub use verbs::project_set_fps::{
    FPS_MIN, OffFrameCount, OffFrameEntities, ProjectSetFpsArgs, ProjectSetFpsData,
    ProjectSetFpsError, ProjectSetFpsVerb,
};
pub use verbs::project_set_metadata::{
    ProjectSetMetadataArgs, ProjectSetMetadataData, ProjectSetMetadataError, ProjectSetMetadataVerb,
};
// Deprecated alias kept for one slice cycle (Slice B3 rename).
#[allow(deprecated)]
pub use verbs::project_set_metadata::ProjectSetMetadataReconstructor;
pub use verbs::{default_fixtures, default_registry};

// Re-export the tick rate constant from verbreel-types so downstream
// crates (verbreel-args, the verb implementations) can refer to it via
// `verbreel_state::TICK_RATE_HZ` without needing a separate import path.
pub use verbreel_types::TICK_RATE_HZ;
