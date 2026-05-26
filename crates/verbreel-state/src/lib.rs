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
pub use verbs::asset_list::{
    AssetKindFilter, AssetListArgs, AssetListData, AssetListError, AssetListVerb,
};
pub use verbs::asset_remove::{
    AssetRemoveArgs, AssetRemoveData, AssetRemoveError, AssetRemoveVerb,
    W_ASSET_REMOVE_ENVELOPE_CODE,
};
pub use verbs::audio_fade::{AudioFadeArgs, AudioFadeData, AudioFadeError, AudioFadeVerb};
pub use verbs::audio_volume::{
    AudioVolumeArgs, AudioVolumeData, AudioVolumeError, AudioVolumeTargetKind, AudioVolumeVerb,
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
pub use verbs::caption_burn_in::{
    CaptionBurnInArgs, CaptionBurnInData, CaptionBurnInError, CaptionBurnInVerb,
    W_CAPTION_BURN_DEDUP_CODE, W_CAPTION_BURN_IN_ENVELOPE_CODE,
};
pub use verbs::caption_burn_off::{
    CaptionBurnOffArgs, CaptionBurnOffData, CaptionBurnOffError, CaptionBurnOffVerb,
};
pub use verbs::clip_add::{
    ClipAddArgs, ClipAddData, ClipAddError, ClipAddVerb, DEFAULT_IMAGE_DURATION_TK,
    W_CLIP_ADD_ENVELOPE_CODE,
};
pub use verbs::clip_duplicate::{
    ClipDuplicateArgs, ClipDuplicateData, ClipDuplicateError, ClipDuplicateVerb, SiblingDuplicate,
};
pub use verbs::clip_list::{ClipListArgs, ClipListData, ClipListError, ClipListVerb};
pub use verbs::clip_lock::{ClipLockArgs, ClipLockData, ClipLockError, ClipLockVerb};
pub use verbs::clip_move::{ClipMoveArgs, ClipMoveData, ClipMoveError, ClipMoveVerb};
pub use verbs::clip_rename::{ClipRenameArgs, ClipRenameData, ClipRenameError, ClipRenameVerb};
pub use verbs::clip_reverse::{
    ClipReverseArgs, ClipReverseData, ClipReverseError, ClipReverseVerb,
};
pub use verbs::clip_set_blend_mode::{
    ClipSetBlendModeArgs, ClipSetBlendModeData, ClipSetBlendModeError, ClipSetBlendModeVerb,
};
pub use verbs::clip_set_fade::{
    ClipSetFadeArgs, ClipSetFadeData, ClipSetFadeError, ClipSetFadeVerb,
};
pub use verbs::clip_set_mask::{
    ClipSetMaskArgs, ClipSetMaskData, ClipSetMaskError, ClipSetMaskVerb,
};
pub use verbs::clip_set_opacity::{
    ClipSetOpacityArgs, ClipSetOpacityData, ClipSetOpacityError, ClipSetOpacityVerb,
};
pub use verbs::clip_set_speed::{
    ClipSetSpeedArgs, ClipSetSpeedData, ClipSetSpeedError, ClipSetSpeedVerb,
};
pub use verbs::clip_set_speed_curve::{
    ClipSetSpeedCurveArgs, ClipSetSpeedCurveData, ClipSetSpeedCurveError, ClipSetSpeedCurveVerb,
};
pub use verbs::clip_set_transform::{
    ClipSetTransformArgs, ClipSetTransformData, ClipSetTransformError, ClipSetTransformVerb,
    PartialTransform,
};
pub use verbs::clip_set_volume::{
    ClipSetVolumeArgs, ClipSetVolumeData, ClipSetVolumeError, ClipSetVolumeVerb,
};
pub use verbs::clip_split::{
    ClipSplitArgs, ClipSplitData, ClipSplitError, ClipSplitVerb, SiblingSplit,
};
pub use verbs::clip_trim::{ClipTrimArgs, ClipTrimData, ClipTrimError, ClipTrimVerb};
pub use verbs::clip_unlink::{ClipUnlinkArgs, ClipUnlinkData, ClipUnlinkError, ClipUnlinkVerb};
pub use verbs::describe::{DescribeArgs, DescribeData, DescribeError, DescribeKind, DescribeVerb};
pub use verbs::effect_add::{EffectAddArgs, EffectAddData, EffectAddError, EffectAddVerb};
pub use verbs::effect_list_available::{
    AvailableEffect, EffectCategory, EffectListAvailableArgs, EffectListAvailableData,
    EffectListAvailableError, EffectListAvailableVerb,
};
pub use verbs::effect_remove::{
    EffectRemoveArgs, EffectRemoveData, EffectRemoveError, EffectRemoveVerb,
};
pub use verbs::effect_reorder::{
    EffectReorderArgs, EffectReorderData, EffectReorderError, EffectReorderVerb, ToIndex,
};
pub use verbs::effect_set_param::{
    EffectSetParamArgs, EffectSetParamData, EffectSetParamError, EffectSetParamVerb,
};
pub use verbs::effect_toggle::{
    EffectToggleArgs, EffectToggleData, EffectToggleError, EffectToggleVerb,
};
pub use verbs::keyframe_list::{
    KeyframeListArgs, KeyframeListData, KeyframeListError, KeyframeListVerb,
};
pub use verbs::marker_add::{
    DEFAULT_MARKER_COLOR, LABEL_MAX as MARKER_LABEL_MAX, MarkerAddArgs, MarkerAddData,
    MarkerAddError, MarkerAddVerb, NOTE_MAX as MARKER_NOTE_MAX,
};
pub use verbs::marker_list::{MarkerListArgs, MarkerListData, MarkerListVerb};
pub use verbs::marker_remove::{
    MARKERS_MAX_BATCH, MarkerRemoveArgs, MarkerRemoveData, MarkerRemoveError, MarkerRemoveVerb,
};
pub use verbs::marker_set::{MarkerSetArgs, MarkerSetData, MarkerSetError, MarkerSetVerb};
pub use verbs::project_info::{
    ProjectInfoArgs, ProjectInfoCanvas, ProjectInfoData, ProjectInfoError, ProjectInfoTrackCounts,
    ProjectInfoVerb,
};
pub use verbs::project_rename::{
    PROJECT_NAME_MAX, PROJECT_NAME_MIN, ProjectRenameArgs, ProjectRenameData, ProjectRenameError,
    ProjectRenameVerb,
};
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
pub use verbs::text_add::{
    StyleArg, TextAddArgs, TextAddData, TextAddError, TextAddVerb, W_TEXT_ADD_ENVELOPE_CODE,
};
pub use verbs::text_edit::{TextEditArgs, TextEditData, TextEditError, TextEditVerb};
pub use verbs::timeline_snapshot::{
    TimelineSnapshotArgs, TimelineSnapshotData, TimelineSnapshotError, TimelineSnapshotVerb,
};
pub use verbs::track_add::{
    TRACK_NAME_MAX, TrackAddArgs, TrackAddData, TrackAddError, TrackAddVerb,
};
pub use verbs::track_hide::{
    DEFAULT_HIDDEN, TrackHideArgs, TrackHideData, TrackHideError, TrackHideVerb,
};
pub use verbs::track_lock::{
    DEFAULT_LOCKED, TrackLockArgs, TrackLockData, TrackLockError, TrackLockVerb,
};
pub use verbs::track_mute::{
    DEFAULT_MUTED, TrackMuteArgs, TrackMuteData, TrackMuteError, TrackMuteVerb,
};
pub use verbs::track_rename::{
    TrackRenameArgs, TrackRenameData, TrackRenameError, TrackRenameVerb,
};
pub use verbs::track_reorder::{
    TrackReorderArgs, TrackReorderData, TrackReorderError, TrackReorderVerb,
};
pub use verbs::track_set_pan::{
    TrackSetPanArgs, TrackSetPanData, TrackSetPanError, TrackSetPanVerb,
};
pub use verbs::track_set_volume::{
    TrackSetVolumeArgs, TrackSetVolumeData, TrackSetVolumeError, TrackSetVolumeVerb,
};
pub use verbs::track_solo::{
    DEFAULT_SOLO, TrackSoloArgs, TrackSoloData, TrackSoloError, TrackSoloVerb,
};
// Deprecated alias kept for one slice cycle (Slice B3 rename).
#[allow(deprecated)]
pub use verbs::project_set_metadata::ProjectSetMetadataReconstructor;
pub use verbs::{default_fixtures, default_registry};

// Re-export the tick rate constant from verbreel-types so downstream
// crates (verbreel-args, the verb implementations) can refer to it via
// `verbreel_state::TICK_RATE_HZ` without needing a separate import path.
pub use verbreel_types::TICK_RATE_HZ;
