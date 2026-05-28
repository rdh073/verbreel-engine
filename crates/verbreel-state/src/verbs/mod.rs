//! Per-verb modules. Each verb declares:
//!
//! - its args / data / error types,
//! - a `compute_patch()` freestanding helper (pure — no I/O, no clock,
//!   no RNG),
//! - a `*Verb` impl of [`crate::reconstructor::Verb`] (also pure per
//!   §0.8) — owns both the forward path (`compute_patch`) and the
//!   replay path (`reconstruct`).
//!
//! Verbs land one at a time. `project.set_metadata` (§2.12) was the
//! first and `project.rename` (§2.9) is the fourth production verb.
//! The fifth is `marker.add` (§13.1), the sixth is `marker.set` (§13.2),
//! the seventh is `marker.remove` (§13.3), and the eighth is
//! `marker.list` (§13.4).
//! The set grows on each slice so every consumer that wants "the stock
//! kernel verb set" (`ProjectStore::create_with_registry` /
//! `ProjectStore::open_with_registry` / `ProjectStore::mutate_via_verb`)
//! picks them up automatically.
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
//!   startup gate ([`crate::lifecycle::ProjectStore::create_with_registry`] /
//!   [`crate::lifecycle::ProjectStore::open_with_registry`]).
//!
//! ## Kernel-verb set
//!
//! [`default_registry`] returns the canonical set of verb
//! reconstructors that ship with this engine build.
//! [`default_fixtures`] returns one matching fixture per registered
//! verb so callers can pass `(default_registry(), default_fixtures())`
//! into the startup gate and clear it by construction. The two are a
//! pair — when the next verb lands, register it in `default_registry`
//! AND add its matching fixture to `default_fixtures`. Custom
//! registries built by tests or downstream tooling must supply their
//! own fixtures.
//!
//! ## Spec references
//!
//! - `spec/commands/asset.md` §3.2 (`asset.list`).
//! - `spec/commands/clip.md` §5.5 (`clip.delete`).
//! - `spec/commands/marker.md` §13.1 (`marker.add`), §13.2
//!   (`marker.set`), §13.3 (`marker.remove`), and §13.4 (`marker.list`).
//! - `spec/commands/project.md` §2.9 (`project.rename`) and §2.12
//!   (`project.set_metadata`).
//! - `spec/commands/text.md` §7.2 (`text.edit`), §7.3
//!   (`text.style`), and §7.4 (`text.animate`).
//! - `spec/commands/keyframe.md` §8.1 (`keyframe.add`), §8.2
//!   (`keyframe.remove`), §8.3 (`keyframe.set`), and §8.4
//!   (`keyframe.list`).
//! - `spec/commands/track.md` §4.1 (`track.add`) and §4.2
//!   (`track.remove`).
//! - `spec/commands/conventions.md` §0.13 (metadata size caps).
//! - `spec/commands/conventions.md` §0.8 (reconstructor purity).

use std::sync::Arc;

use serde_json::{Map, Value, json};

use crate::project::Project;
use crate::reconstructor::{RecordedEvent, VerbRegistry};
use verbreel_types::Tick;

pub mod asset_gc;
pub mod asset_import;
pub mod asset_list;
pub mod asset_probe;
pub mod asset_relink;
pub mod asset_remove;
pub mod asset_verify;
pub mod audio_analyze;
pub mod audio_denoise;
pub mod audio_detect_beats;
pub mod audio_detect_silence;
pub mod audio_extract;
pub mod audio_fade;
pub mod audio_volume;
pub mod caption_auto_generate;
pub mod caption_burn_in;
pub mod caption_burn_off;
pub mod caption_edit;
pub mod caption_export;
pub mod caption_translate;
pub use caption_burn_off::*;
pub mod clip_add;
pub mod clip_delete;
pub mod clip_duplicate;
pub mod clip_list;
pub mod clip_lock;
pub mod clip_move;
pub mod clip_rename;
pub mod clip_reverse;
pub mod clip_set_blend_mode;
pub mod clip_set_fade;
pub mod clip_set_mask;
pub mod clip_set_opacity;
pub mod clip_set_speed;
pub mod clip_set_speed_curve;
pub mod clip_set_transform;
pub mod clip_set_volume;
pub mod clip_split;
pub mod clip_trim;
pub mod clip_unlink;
pub mod compound_create;
pub mod compound_edit_in_place;
pub mod compound_expand;
pub mod compound_flatten;
pub mod describe;
pub mod effect_add;
pub mod effect_list_available;
pub mod effect_remove;
pub mod effect_reorder;
pub mod effect_set_param;
pub mod effect_toggle;
pub mod font_list;
pub mod help;
pub mod keyframe_add;
pub mod keyframe_list;
pub mod keyframe_remove;
pub mod keyframe_set;
pub mod list_capabilities;
pub mod marker_add;
pub mod marker_list;
pub mod marker_remove;
pub mod marker_set;
pub mod preview_frame;
pub mod preview_session_close;
pub mod preview_session_create;
pub mod preview_session_frame_at;
pub mod preview_session_pause;
pub mod preview_session_play;
pub mod preview_session_seek;
pub mod preview_thumbnail;
pub mod preview_waveform;
#[cfg(feature = "native")]
pub mod project_close;
#[cfg(feature = "native")]
pub mod project_create;
#[cfg(feature = "native")]
pub mod project_duplicate;
pub mod project_forget;
pub mod project_info;
pub mod project_list;
#[cfg(feature = "native")]
pub mod project_open;
pub mod project_rename;
#[cfg(feature = "native")]
pub mod project_save;
pub mod project_set_canvas;
pub mod project_set_fps;
pub mod project_set_metadata;
pub mod render_cancel;
pub mod render_list_presets;
pub mod render_queue_add;
pub mod render_queue_cancel;
pub mod render_queue_clear;
pub mod render_queue_list;
pub mod render_queue_status;
pub mod render_start;
pub mod render_status;
pub mod schema;
pub mod stock_describe;
pub mod stock_import;
pub mod stock_list_providers;
pub mod stock_search;
pub mod template_apply;
pub mod template_describe;
pub mod template_from_project;
pub mod template_install;
pub mod template_list;
pub mod template_uninstall;
pub mod text_add;
pub mod text_animate;
pub mod text_edit;
pub mod text_style;
pub mod timeline_diff;
pub mod timeline_history;
pub mod timeline_redo;
pub mod timeline_snapshot;
pub mod timeline_undo;
pub mod track_add;
pub mod track_hide;
pub mod track_lock;
pub mod track_mute;
pub mod track_remove;
pub mod track_rename;
pub mod track_reorder;
pub mod track_set_pan;
pub mod track_set_volume;
pub mod track_solo;
pub mod tracker_apply;
pub mod tracker_create;
pub mod tracker_list;
pub mod tracker_remove;
pub mod tracker_run;
pub mod validate_command;

/// Synthetic `UUIDv7` used as the `project_id` in [`default_fixtures`].
/// Hard-coded so the fixture is deterministic — `ProjectId::now()` would
/// pull from the wall clock and the gate is a startup-time, not
/// runtime, validation. The string is a valid v7 (version nibble `7`,
/// variant nibble in `8..=b`) but otherwise carries no production
/// meaning.
const DEFAULT_FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-0000deadbeef";

/// The canonical set of verbs shipped by this engine build.
///
/// Canonical kernel verbs currently shipped:
/// - `asset.list` (§3.2)
/// - `asset.probe` (§3.3)
/// - `asset.relink` (§3.5)
/// - `asset.remove` (§3.4)
/// - `clip.delete` (§5.5)
/// - `clip.list` (§5.14)
/// - `clip.lock` (§5.13)
/// - `clip.rename` (§5.17)
/// - `clip.set_blend_mode` (§5.18)
/// - `clip.set_fade` (§5.12)
/// - `clip.set_mask` (§5.19)
/// - `clip.set_opacity` (§5.10)
/// - `clip.set_transform` (§5.9)
/// - `clip.set_volume` (§5.11)
/// - `clip.unlink` (§5.16)
/// - `effect.list_available` (§6.5)
/// - `effect.reorder` (§6.6)
/// - `effect.set_param` (§6.3)
/// - `effect.toggle` (§6.4)
/// - `font.list` (§7.5)
/// - `text.animate` (§7.4)
/// - `text.edit` (§7.2)
/// - `text.style` (§7.3)
/// - `caption.auto_generate` (§10.1)
/// - `caption.edit` (§10.2, §7.2 alias)
/// - `caption.export` (§10.6)
/// - `caption.translate` (§10.3)
/// - `keyframe.add` (§8.1)
/// - `keyframe.list` (§8.4)
/// - `keyframe.remove` (§8.2)
/// - `keyframe.set` (§8.3)
/// - `project.info` (§2.4)
/// - `project.set_metadata` (§2.12)
/// - `project.set_canvas` (§2.10)
/// - `project.set_fps` (§2.11)
/// - `project.rename` (§2.9)
/// - `marker.add` (§13.1)
/// - `marker.set` (§13.2)
/// - `marker.remove` (§13.3)
/// - `marker.list` (§13.4)
/// - `track.add` (§4.1)
/// - `track.hide` (§4.10)
/// - `track.lock` (§4.6)
/// - `track.mute` (§4.4)
/// - `track.remove` (§4.2)
/// - `track.solo` (§4.5)
/// - `track.rename` (§4.7)
/// - `track.reorder` (§4.3)
/// - `track.set_pan` (§4.9)
/// - `track.set_volume` (§4.8)
///
/// Paired with [`default_fixtures`]: the two together clear the §0.8
/// reconstructor-purity startup gate by construction.
///
/// # Panics
///
/// Panics if registration of a built-in verb collides — only reachable
/// if the function is edited to register the same verb id twice (a
/// programmer bug, surfaced loudly at the first call site rather than
/// hidden behind a `Result` callers would unwrap anyway).
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn default_registry() -> VerbRegistry {
    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(project_set_metadata::ProjectSetMetadataVerb))
        .expect(
            "ProjectSetMetadataVerb is the first registration in \
             default_registry(); cannot collide",
        );
    registry
        .register(Arc::new(project_set_canvas::ProjectSetCanvasVerb))
        .expect(
            "ProjectSetCanvasVerb is the second registration in \
             default_registry(); cannot collide with project.set_metadata",
        );
    registry
        .register(Arc::new(project_set_fps::ProjectSetFpsVerb))
        .expect(
            "ProjectSetFpsVerb is the third registration in \
             default_registry(); cannot collide with project.set_metadata \
             / project.set_canvas",
        );
    registry
        .register(Arc::new(project_rename::ProjectRenameVerb))
        .expect(
            "ProjectRenameVerb is the fourth registration in \
             default_registry(); cannot collide with project.set_metadata \
             / project.set_canvas / project.set_fps",
        );
    registry
        .register(Arc::new(marker_add::MarkerAddVerb))
        .expect(
            "MarkerAddVerb is the fifth registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(marker_set::MarkerSetVerb))
        .expect(
            "MarkerSetVerb is the sixth registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(marker_remove::MarkerRemoveVerb))
        .expect(
            "MarkerRemoveVerb is the seventh registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(marker_list::MarkerListVerb))
        .expect(
            "MarkerListVerb is the eighth registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(track_add::TrackAddVerb))
        .expect("TrackAddVerb registration in default_registry() cannot collide with prior verbs");
    registry
        .register(Arc::new(track_rename::TrackRenameVerb))
        .expect(
            "TrackRenameVerb is the tenth registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(track_lock::TrackLockVerb))
        .expect(
            "TrackLockVerb is the eleventh registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(track_mute::TrackMuteVerb))
        .expect(
            "TrackMuteVerb is the twelfth registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(track_solo::TrackSoloVerb))
        .expect(
            "TrackSoloVerb is the thirteenth registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(track_hide::TrackHideVerb))
        .expect(
            "TrackHideVerb is the fourteenth registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(track_set_volume::TrackSetVolumeVerb))
        .expect(
            "TrackSetVolumeVerb is the fifteenth registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(track_set_pan::TrackSetPanVerb))
        .expect(
            "TrackSetPanVerb is the sixteenth registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(track_reorder::TrackReorderVerb))
        .expect(
            "TrackReorderVerb is the seventeenth registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry.register(Arc::new(clip_lock::ClipLockVerb)).expect(
        "ClipLockVerb is the eighteenth registration in \
             default_registry(); cannot collide with prior verbs",
    );
    registry.register(Arc::new(clip_move::ClipMoveVerb)).expect(
        "ClipMoveVerb is the forty-sixth registration in \
             default_registry(); cannot collide with prior verbs",
    );
    registry
        .register(Arc::new(clip_rename::ClipRenameVerb))
        .expect(
            "ClipRenameVerb is the nineteenth registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(clip_reverse::ClipReverseVerb))
        .expect(
            "ClipReverseVerb is the forty-fifth registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(clip_set_opacity::ClipSetOpacityVerb))
        .expect(
            "ClipSetOpacityVerb is the twentieth registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(clip_set_speed::ClipSetSpeedVerb))
        .expect(
            "ClipSetSpeedVerb is the fifty-third registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(clip_set_speed_curve::ClipSetSpeedCurveVerb))
        .expect(
            "ClipSetSpeedCurveVerb is the fifty-second registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(clip_set_volume::ClipSetVolumeVerb))
        .expect(
            "ClipSetVolumeVerb is the twenty-first registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(clip_split::ClipSplitVerb))
        .expect(
            "ClipSplitVerb is the forty-eighth registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry.register(Arc::new(clip_trim::ClipTrimVerb)).expect(
        "ClipTrimVerb is the forty-seventh registration in \
             default_registry(); cannot collide with prior verbs",
    );
    registry
        .register(Arc::new(clip_set_blend_mode::ClipSetBlendModeVerb))
        .expect(
            "ClipSetBlendModeVerb is the twenty-second registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(clip_set_fade::ClipSetFadeVerb))
        .expect(
            "ClipSetFadeVerb is the fortieth registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(clip_set_mask::ClipSetMaskVerb))
        .expect(
            "ClipSetMaskVerb is the forty-first registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(clip_set_transform::ClipSetTransformVerb))
        .expect(
            "ClipSetTransformVerb is the twenty-third registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry.register(Arc::new(clip_add::ClipAddVerb)).expect(
        "ClipAddVerb is the fifty-sixth registration in \
             default_registry(); cannot collide with prior verbs",
    );
    registry
        .register(Arc::new(clip_delete::ClipDeleteVerb))
        .expect(
            "ClipDeleteVerb is the thirty-ninth registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(clip_duplicate::ClipDuplicateVerb))
        .expect(
            "ClipDuplicateVerb is the forty-ninth registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry.register(Arc::new(clip_list::ClipListVerb)).expect(
        "ClipListVerb is the twenty-fourth registration in \
             default_registry(); cannot collide with prior verbs",
    );
    registry
        .register(Arc::new(clip_unlink::ClipUnlinkVerb))
        .expect(
            "ClipUnlinkVerb is the twenty-fifth registration in \
         default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(compound_create::CompoundCreateVerb))
        .expect(
            "CompoundCreateVerb registration in default_registry() cannot collide with prior \
             verbs",
        );
    registry
        .register(Arc::new(compound_edit_in_place::CompoundEditInPlaceVerb))
        .expect(
            "CompoundEditInPlaceVerb registration in default_registry() cannot collide with \
             prior verbs",
        );
    registry
        .register(Arc::new(compound_expand::CompoundExpandVerb))
        .expect(
            "CompoundExpandVerb registration in default_registry() cannot collide with prior \
             verbs",
        );
    registry
        .register(Arc::new(compound_flatten::CompoundFlattenVerb))
        .expect(
            "CompoundFlattenVerb registration in default_registry() cannot collide with prior \
             verbs",
        );
    registry.register(Arc::new(describe::DescribeVerb)).expect(
        "DescribeVerb is the sixty-second registration in \
             default_registry(); cannot collide with prior verbs",
    );
    registry
        .register(Arc::new(effect_add::EffectAddVerb))
        .expect(
            "EffectAddVerb is the fiftieth registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(effect_list_available::EffectListAvailableVerb))
        .expect(
            "EffectListAvailableVerb is the twenty-seventh registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(effect_remove::EffectRemoveVerb))
        .expect(
            "EffectRemoveVerb is the forty-fourth registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(effect_reorder::EffectReorderVerb))
        .expect(
            "EffectReorderVerb is the forty-second registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(effect_set_param::EffectSetParamVerb))
        .expect(
            "EffectSetParamVerb is the forty-third registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(effect_toggle::EffectToggleVerb))
        .expect(
            "EffectToggleVerb is the twenty-sixth registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(asset_import::AssetImportVerb))
        .expect(
            "AssetImportVerb is the eighty-fourth registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(asset_list::AssetListVerb))
        .expect(
            "AssetListVerb is the twenty-eighth registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(asset_probe::AssetProbeVerb))
        .expect(
            "AssetProbeVerb is the eighty-fifth registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(asset_relink::AssetRelinkVerb))
        .expect(
            "AssetRelinkVerb is the eighty-sixth registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(asset_remove::AssetRemoveVerb))
        .expect(
            "AssetRemoveVerb is the fifty-eighth registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry.register(Arc::new(asset_gc::AssetGcVerb)).expect(
        "AssetGcVerb is the eighty-seventh registration in \
             default_registry(); cannot collide with prior verbs",
    );
    registry
        .register(Arc::new(asset_verify::AssetVerifyVerb))
        .expect(
            "AssetVerifyVerb is the eighty-eighth registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(audio_analyze::AudioAnalyzeVerb))
        .expect(
            "AudioAnalyzeVerb registration in default_registry() cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(audio_detect_beats::AudioDetectBeatsVerb))
        .expect(
            "AudioDetectBeatsVerb is the ninety-fifth registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(audio_detect_silence::AudioDetectSilenceVerb))
        .expect(
            "AudioDetectSilenceVerb registration in default_registry() cannot collide with prior \
             verbs",
        );
    registry
        .register(Arc::new(audio_extract::AudioExtractVerb))
        .expect(
            "AudioExtractVerb is the eighty-ninth registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(audio_denoise::AudioDenoiseVerb))
        .expect(
            "AudioDenoiseVerb is the ninetieth registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(audio_fade::AudioFadeVerb))
        .expect(
            "AudioFadeVerb is the fifty-ninth registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(audio_volume::AudioVolumeVerb))
        .expect(
            "AudioVolumeVerb is the sixtieth registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(keyframe_list::KeyframeListVerb))
        .expect(
            "KeyframeListVerb is the twenty-ninth registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry.register(Arc::new(text_add::TextAddVerb)).expect(
        "TextAddVerb is the fifty-fourth registration in \
             default_registry(); cannot collide with prior verbs",
    );
    registry
        .register(Arc::new(text_animate::TextAnimateVerb))
        .expect(
            "TextAnimateVerb is the thirty-sixth registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry.register(Arc::new(text_edit::TextEditVerb)).expect(
        "TextEditVerb is the thirtieth registration in \
             default_registry(); cannot collide with prior verbs",
    );
    registry
        .register(Arc::new(caption_auto_generate::CaptionAutoGenerateVerb))
        .expect(
            "CaptionAutoGenerateVerb is the ninety-first registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(caption_edit::CaptionEditVerb))
        .expect(
            "CaptionEditVerb is the thirty-first registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(caption_translate::CaptionTranslateVerb))
        .expect(
            "CaptionTranslateVerb is the ninety-second registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(caption_export::CaptionExportVerb))
        .expect(
            "CaptionExportVerb is the ninety-third registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(caption_burn_in::CaptionBurnInVerb))
        .expect(
            "CaptionBurnInVerb is the fifty-fifth registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(caption_burn_off::CaptionBurnOffVerb))
        .expect(
            "CaptionBurnOffVerb is the fifty-first registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(text_style::TextStyleVerb))
        .expect(
            "TextStyleVerb is the thirty-second registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(keyframe_add::KeyframeAddVerb))
        .expect(
            "KeyframeAddVerb is the thirty-third registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(keyframe_set::KeyframeSetVerb))
        .expect(
            "KeyframeSetVerb is the thirty-fourth registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(keyframe_remove::KeyframeRemoveVerb))
        .expect(
            "KeyframeRemoveVerb is the thirty-fifth registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(track_remove::TrackRemoveVerb))
        .expect(
            "TrackRemoveVerb is the thirty-seventh registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(project_info::ProjectInfoVerb))
        .expect(
            "ProjectInfoVerb is the fifty-seventh registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(timeline_snapshot::TimelineSnapshotVerb))
        .expect(
            "TimelineSnapshotVerb is the sixty-first registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(timeline_diff::TimelineDiffVerb))
        .expect(
            "TimelineDiffVerb registration in default_registry() cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(timeline_undo::TimelineUndoVerb))
        .expect(
            "TimelineUndoVerb registration in default_registry() cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(timeline_redo::TimelineRedoVerb))
        .expect(
            "TimelineRedoVerb registration in default_registry() cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(timeline_history::TimelineHistoryVerb))
        .expect(
            "TimelineHistoryVerb registration in default_registry() cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(preview_frame::PreviewFrameVerb))
        .expect(
            "PreviewFrameVerb is the seventy-ninth registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(preview_waveform::PreviewWaveformVerb))
        .expect(
            "PreviewWaveformVerb registration in default_registry() cannot collide with prior \
             verbs",
        );
    registry
        .register(Arc::new(preview_thumbnail::PreviewThumbnailVerb))
        .expect(
            "PreviewThumbnailVerb registration in default_registry() cannot collide with prior \
             verbs",
        );
    registry
        .register(Arc::new(list_capabilities::ListCapabilitiesVerb))
        .expect(
            "ListCapabilitiesVerb is the sixty-third registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry.register(Arc::new(help::HelpVerb)).expect(
        "HelpVerb is the sixty-fourth registration in \
             default_registry(); cannot collide with prior verbs",
    );
    registry
        .register(Arc::new(validate_command::ValidateCommandVerb))
        .expect(
            "ValidateCommandVerb is the sixty-fifth registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry.register(Arc::new(schema::SchemaVerb)).expect(
        "SchemaVerb is the sixty-sixth registration in \
             default_registry(); cannot collide with prior verbs",
    );
    registry
        .register(Arc::new(stock_list_providers::StockListProvidersVerb))
        .expect(
            "StockListProvidersVerb is the sixty-seventh registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(stock_search::StockSearchVerb))
        .expect(
            "StockSearchVerb is the sixty-eighth registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(stock_import::StockImportVerb))
        .expect(
            "StockImportVerb registration in default_registry() cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(stock_describe::StockDescribeVerb))
        .expect(
            "StockDescribeVerb registration in default_registry() cannot collide with prior verbs",
        );
    registry.register(Arc::new(font_list::FontListVerb)).expect(
        "FontListVerb is the sixty-ninth registration in \
             default_registry(); cannot collide with prior verbs",
    );
    registry
        .register(Arc::new(tracker_apply::TrackerApplyVerb))
        .expect(
            "TrackerApplyVerb registration in default_registry() cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(tracker_list::TrackerListVerb))
        .expect(
            "TrackerListVerb is the seventieth registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(tracker_remove::TrackerRemoveVerb))
        .expect(
            "TrackerRemoveVerb is the seventy-first registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(tracker_create::TrackerCreateVerb))
        .expect(
            "TrackerCreateVerb is the seventy-second registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(tracker_run::TrackerRunVerb))
        .expect(
            "TrackerRunVerb is the ninety-fourth registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(render_queue_add::RenderQueueAddVerb))
        .expect(
            "RenderQueueAddVerb registration in default_registry() cannot collide with prior \
             verbs",
        );
    registry
        .register(Arc::new(render_queue_list::RenderQueueListVerb))
        .expect(
            "RenderQueueListVerb is the seventy-third registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(render_queue_clear::RenderQueueClearVerb))
        .expect(
            "RenderQueueClearVerb is the seventy-fourth registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(render_queue_status::RenderQueueStatusVerb))
        .expect(
            "RenderQueueStatusVerb is the seventy-fifth registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(render_queue_cancel::RenderQueueCancelVerb))
        .expect(
            "RenderQueueCancelVerb is the seventy-sixth registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(render_list_presets::RenderListPresetsVerb))
        .expect(
            "RenderListPresetsVerb is the seventy-seventh registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(render_start::RenderStartVerb))
        .expect(
            "RenderStartVerb registration in default_registry() cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(render_cancel::RenderCancelVerb))
        .expect(
            "RenderCancelVerb is the seventy-eighth registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(render_status::RenderStatusVerb))
        .expect(
            "RenderStatusVerb is the seventy-ninth registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(preview_session_create::PreviewSessionCreateVerb))
        .expect(
            "PreviewSessionCreateVerb registration in default_registry() cannot collide with \
             prior verbs",
        );
    registry
        .register(Arc::new(preview_session_seek::PreviewSessionSeekVerb))
        .expect(
            "PreviewSessionSeekVerb registration in default_registry() cannot collide with \
             prior verbs",
        );
    registry
        .register(Arc::new(preview_session_play::PreviewSessionPlayVerb))
        .expect(
            "PreviewSessionPlayVerb registration in default_registry() cannot collide with prior \
             verbs",
        );
    registry
        .register(Arc::new(preview_session_pause::PreviewSessionPauseVerb))
        .expect(
            "PreviewSessionPauseVerb registration in default_registry() cannot collide with \
             prior verbs",
        );
    registry
        .register(Arc::new(preview_session_close::PreviewSessionCloseVerb))
        .expect(
            "PreviewSessionCloseVerb is the eighty-second registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(
            preview_session_frame_at::PreviewSessionFrameAtVerb,
        ))
        .expect(
            "PreviewSessionFrameAtVerb is the eighty-third registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(template_list::TemplateListVerb))
        .expect(
            "TemplateListVerb registration in default_registry() cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(template_describe::TemplateDescribeVerb))
        .expect(
            "TemplateDescribeVerb registration in default_registry() cannot collide with prior \
             verbs",
        );
    registry
        .register(Arc::new(template_apply::TemplateApplyVerb))
        .expect(
            "TemplateApplyVerb registration in default_registry() cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(template_from_project::TemplateFromProjectVerb))
        .expect(
            "TemplateFromProjectVerb registration in default_registry() cannot collide with \
             prior verbs",
        );
    registry
        .register(Arc::new(template_install::TemplateInstallVerb))
        .expect(
            "TemplateInstallVerb registration in default_registry() cannot collide with prior \
             verbs",
        );
    registry
        .register(Arc::new(template_uninstall::TemplateUninstallVerb))
        .expect(
            "TemplateUninstallVerb registration in default_registry() cannot collide with prior \
             verbs",
        );
    registry
        .register(Arc::new(project_list::ProjectListVerb))
        .expect(
            "ProjectListVerb is the eighty-fifth registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
}

/// One canonical fixture per verb registered in [`default_registry`].
///
/// Each fixture exercises a non-trivial code path through its verb's
/// reconstructor and pairs with the recorded `expected_data` the
/// reconstructor must reproduce under canonical SHA-256.
///
/// Callers using [`default_registry`] should pair it with this function.
/// The two are validated together at every Verbreel test run and pass
/// the §0.8 startup gate by construction. Callers building custom
/// registries must build their own fixtures.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn default_fixtures() -> Vec<RecordedEvent> {
    vec![
        project_set_metadata_fixture(),
        project_set_canvas_fixture(),
        project_set_fps_fixture(),
        project_rename_fixture(),
        text_add_fixture(),
        text_animate_fixture(),
        text_edit_fixture(),
        caption_edit_fixture(),
        caption_auto_generate_fixture(),
        caption_translate_fixture(),
        caption_export_fixture(),
        caption_burn_in_fixture(),
        caption_burn_off_fixture(),
        text_style_fixture(),
        marker_add_fixture(),
        marker_set_fixture(),
        marker_remove_fixture(),
        marker_list_fixture(),
        track_add_fixture(),
        track_rename_fixture(),
        track_lock_fixture(),
        track_mute_fixture(),
        track_solo_fixture(),
        track_hide_fixture(),
        track_set_volume_fixture(),
        track_set_pan_fixture(),
        track_reorder_fixture(),
        clip_lock_fixture(),
        clip_move_fixture(),
        clip_rename_fixture(),
        clip_reverse_fixture(),
        clip_set_blend_mode_fixture(),
        clip_set_fade_fixture(),
        clip_set_mask_fixture(),
        clip_set_speed_fixture(),
        clip_set_speed_curve_fixture(),
        clip_set_transform_fixture(),
        clip_set_opacity_fixture(),
        clip_set_volume_fixture(),
        clip_split_fixture(),
        clip_trim_fixture(),
        clip_add_fixture(),
        clip_delete_fixture(),
        clip_duplicate_fixture(),
        clip_list_fixture(),
        clip_unlink_fixture(),
        compound_create_fixture(),
        compound_edit_in_place_fixture(),
        compound_expand_fixture(),
        compound_flatten_fixture(),
        describe_fixture(),
        effect_add_fixture(),
        effect_list_available_fixture(),
        effect_remove_fixture(),
        effect_reorder_fixture(),
        effect_set_param_fixture(),
        effect_toggle_fixture(),
        asset_gc_fixture(),
        asset_import_fixture(),
        asset_list_fixture(),
        asset_probe_fixture(),
        asset_relink_fixture(),
        asset_remove_fixture(),
        asset_verify_fixture(),
        audio_analyze_fixture(),
        audio_detect_beats_fixture(),
        audio_detect_silence_fixture(),
        audio_extract_fixture(),
        audio_denoise_fixture(),
        audio_fade_fixture(),
        audio_volume_fixture(),
        keyframe_add_fixture(),
        keyframe_list_fixture(),
        keyframe_remove_fixture(),
        keyframe_set_fixture(),
        track_remove_fixture(),
        project_info_fixture(),
        timeline_snapshot_fixture(),
        timeline_diff_fixture(),
        timeline_undo_fixture(),
        timeline_redo_fixture(),
        timeline_history_fixture(),
        preview_frame_fixture(),
        preview_waveform_fixture(),
        preview_thumbnail_fixture(),
        preview_session_create_fixture(),
        preview_session_seek_fixture(),
        preview_session_play_fixture(),
        preview_session_pause_fixture(),
        preview_session_frame_at_fixture(),
        list_capabilities_fixture(),
        help_fixture(),
        validate_command_fixture(),
        schema_fixture(),
        stock_list_providers_fixture(),
        stock_search_fixture(),
        stock_import_fixture(),
        stock_describe_fixture(),
        font_list_fixture(),
        tracker_apply_fixture(),
        tracker_create_fixture(),
        tracker_list_fixture(),
        tracker_remove_fixture(),
        tracker_run_fixture(),
        render_queue_add_fixture(),
        render_queue_list_fixture(),
        render_queue_clear_fixture(),
        render_queue_status_fixture(),
        render_queue_cancel_fixture(),
        render_list_presets_fixture(),
        render_start_fixture(),
        render_cancel_fixture(),
        render_status_fixture(),
        preview_session_close_fixture(),
        template_list_fixture(),
        template_describe_fixture(),
        template_apply_fixture(),
        template_from_project_fixture(),
        template_install_fixture(),
        template_uninstall_fixture(),
        project_list_fixture(),
    ]
}

/// Build the canonical `text.add` fixture used by [`default_fixtures`].
///
/// Starts from a synthetic project with no text track, then adds a
/// one-second text clip and lets the verb auto-create `Text 1`.
fn text_add_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = text_add::TextAddArgs {
        project_id,
        content: "Hello world".to_string(),
        track_position_tk: 0,
        duration_tk: 240_000,
        track: None,
        style: None,
        name: None,
    };

    let (patch_value, warnings, _data) = text_add::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid text.add patch");
    let patch: json_patch::Patch = serde_json::from_value(patch_value.clone())
        .expect("text.add fixture patch is valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("text.add fixture patch must apply cleanly");

    let expected_data = serde_json::to_value(
        text_add::data_envelope_from_warnings(&warnings).expect("text.add fixture expected_data"),
    )
    .expect("text.add fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "text.add".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings,
        post_state,
        expected_data,
    }
}

/// Build the canonical `text.edit` fixture used by [`default_fixtures`].
///
/// Starts from a synthetic project with a single text track and a single
/// text clip, then updates that clip's text content.
fn text_edit_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let mut prior = synthetic_empty_project(project_id);

    let clip_id = "01900000-0000-7000-8000-0000000bb101";

    let track_raw = json!({
        "id": "01900000-0000-7000-8000-0000000aa101",
        "kind": "text",
        "name": "Captions",
        "clips": [{
            "id": clip_id,
            "name": "Caption 1",
            "asset_id": "00000000-0000-0000-0000-000000000000",
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 1000,
            "locked": false,
            "text": {
                "content": "Hello",
                "font_family": "Arial",
                "font_size_px": 24
            },
        }],
    });

    let track: crate::track::Track =
        serde_json::from_value(track_raw).expect("manual track fixture parses");
    prior.tracks.push(track);
    prior.duration_tk = Tick::new(1000);

    let args = text_edit::TextEditArgs {
        project_id,
        clip: clip_id.to_string(),
        content: "World".to_string(),
    };

    let (patch_value, _warnings, _data) = text_edit::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid text.edit patch");
    let patch: json_patch::Patch = serde_json::from_value(patch_value.clone())
        .expect("text.edit fixture patch is valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("text.edit fixture patch must apply cleanly");

    let expected_data = serde_json::to_value(
        text_edit::data_envelope_from_post_state(&args, &post_state)
            .expect("text.edit fixture expected_data"),
    )
    .expect("text.edit fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "text.edit".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings: vec![],
        post_state,
        expected_data,
    }
}

/// Alias fixture for `caption.edit` (`caption.edit` and `text.edit` share
/// the same logical patch shape and envelope).
fn caption_edit_fixture() -> RecordedEvent {
    let mut fixture = text_edit_fixture();
    fixture.verb = "caption.edit".to_string();
    fixture
}

/// Build the canonical `caption.auto_generate` fixture used by
/// [`default_fixtures`].
///
/// `caption.auto_generate` always errors with `E_BUSY` in the v1 floor
/// (writer-class streaming runtime is intentionally deferred). No
/// successful event can ever be recorded, so the reconstructor's input
/// tuple carries no semantically-meaningful payload — the verb's
/// `reconstruct()` exercises args-deserialization and returns
/// `Value::Null` to satisfy the §0.8 gate. The fixture records
/// `expected_data: null` so the gate's canonical-SHA equality holds.
fn caption_auto_generate_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = caption_auto_generate::CaptionAutoGenerateArgs {
        project_id,
        from_selector: "track:audio[0]".to_string(),
        language: None,
        style: None,
        max_line_chars: None,
        to_track: None,
        model: None,
    };

    RecordedEvent {
        verb: "caption.auto_generate".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: json!([]),
        warnings: vec![],
        post_state: prior,
        expected_data: Value::Null,
    }
}

/// Build the canonical `caption.translate` fixture used by
/// [`default_fixtures`].
///
/// `caption.translate` always errors with `E_BUSY` in the v1 floor
/// (writer-class streaming runtime is intentionally deferred). No
/// successful event can ever be recorded, so the reconstructor's input
/// tuple carries no semantically-meaningful payload — the verb's
/// `reconstruct()` exercises args-deserialization and returns
/// `Value::Null` to satisfy the §0.8 gate. The fixture records
/// `expected_data: null` so the gate's canonical-SHA equality holds.
fn caption_translate_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = caption_translate::CaptionTranslateArgs {
        project_id,
        from_selector: "track:audio[0]".to_string(),
        style: None,
    };

    RecordedEvent {
        verb: "caption.translate".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: json!([]),
        warnings: vec![],
        post_state: prior,
        expected_data: Value::Null,
    }
}

/// Build the canonical `caption.export` fixture used by
/// [`default_fixtures`].
///
/// `caption.export` always errors with `E_IO` in the v1 floor because
/// sidecar subtitle writing is intentionally deferred. No successful
/// event can be recorded, so the reconstructor only checks args
/// deserialization and returns `Value::Null`. The fixture records
/// `expected_data: null` so the startup gate's canonical equality
/// holds.
fn caption_export_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = json!({
        "project_id": project_id.to_string(),
        "text_track": "track:text[0]",
        "out_path": "captions.srt",
    });

    RecordedEvent {
        verb: "caption.export".to_string(),
        args,
        patch: json!([]),
        warnings: vec![],
        post_state: prior,
        expected_data: Value::Null,
    }
}

/// Build the canonical `caption.burn_in` fixture used by
/// [`default_fixtures`].
///
/// Starts from a text track with one text clip and a video track with
/// one video clip fully overlapping the text-clip range, then exercises
/// the create-new path emitting exactly one `burned_caption` effect.
#[allow(clippy::too_many_lines)]
fn caption_burn_in_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let mut prior = synthetic_empty_project(project_id);
    let text_track_id = "01900000-0000-7000-8000-0000000aa161";
    let video_track_id = "01900000-0000-7000-8000-0000000aa162";
    let text_clip_id = "01900000-0000-7000-8000-0000000bb161";
    let video_clip_id = "01900000-0000-7000-8000-0000000bb162";
    let video_asset_id = "01900000-0000-7000-8000-0000000cc161";

    prior
        .assets
        .push(serde_json::from_value(json!({
            "id": video_asset_id,
            "kind": "video",
            "hash": "53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658",
            "path": "assets/53/53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.mp4",
            "original_filename": "caption-burn-in.mp4",
            "imported_at": "2026-05-24T00:00:00Z",
            "metadata": {
                "duration_tk": 240_000,
                "width": 1920,
                "height": 1080,
                "fps_num": 30,
                "fps_den": 1,
                "video_codec": "h264",
                "container": "mp4",
                "fingerprint": {
                    "mtime_ms": 1_700_000_000_000_i64,
                    "size_bytes": 1024,
                }
            }
        }))
        .expect("caption.burn_in fixture asset parses"));

    let text_track = json!({
        "id": text_track_id,
        "kind": "text",
        "name": "Captions",
        "locked": false,
        "clips": [{
            "id": text_clip_id,
            "name": "Caption Clip",
            "asset_id": "00000000-0000-0000-0000-000000000000",
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 240_000,
            "locked": false,
            "text": {
                "content": "Caption",
                "font_family": "Arial",
                "font_size_px": 24,
            },
        }],
    });
    let video_track = json!({
        "id": video_track_id,
        "kind": "video",
        "name": "Video",
        "locked": false,
        "clips": [{
            "id": video_clip_id,
            "name": "Shot",
            "asset_id": video_asset_id,
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 240_000,
            "locked": false,
            "effects": [],
        }],
    });
    prior
        .tracks
        .push(serde_json::from_value(text_track).expect("text track fixture parses"));
    prior
        .tracks
        .push(serde_json::from_value(video_track).expect("video track fixture parses"));
    prior.duration_tk = Tick::new(240_000);

    let args = caption_burn_in::CaptionBurnInArgs {
        project_id,
        text_track: text_track_id.to_string(),
        style: None,
    };

    let (patch_value, warnings, _data) = caption_burn_in::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid caption.burn_in patch");
    let patch: json_patch::Patch = serde_json::from_value(patch_value.clone())
        .expect("caption.burn_in fixture patch is valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("caption.burn_in fixture patch must apply cleanly");

    let expected_data = serde_json::to_value(
        caption_burn_in::data_envelope_from_warnings(&warnings)
            .expect("caption.burn_in fixture expected_data"),
    )
    .expect("caption.burn_in fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "caption.burn_in".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings,
        post_state,
        expected_data,
    }
}

/// Build the canonical `caption.burn_off` fixture used by
/// [`default_fixtures`].
///
/// Starts from a text track and a video clip carrying one
/// `burned_caption` effect sourced from that text track, then removes
/// it via the text-track-only path.
#[allow(clippy::too_many_lines)]
fn caption_burn_off_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let mut prior = synthetic_empty_project(project_id);
    let text_track_id = "01900000-0000-7000-8000-0000000aa151";
    let video_track_id = "01900000-0000-7000-8000-0000000aa152";
    let text_clip_id = "01900000-0000-7000-8000-0000000bb151";
    let video_clip_id = "01900000-0000-7000-8000-0000000bb152";
    let video_asset_id = "01900000-0000-7000-8000-0000000cc151";
    let effect_id = "01900000-0000-7000-8000-0000000dd151";

    prior
        .assets
        .push(serde_json::from_value(json!({
            "id": video_asset_id,
            "kind": "video",
            "hash": "53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658",
            "path": "assets/53/53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.mp4",
            "original_filename": "caption-burn-off.mp4",
            "imported_at": "2026-05-24T00:00:00Z",
            "metadata": {
                "duration_tk": 480_000,
                "width": 1920,
                "height": 1080,
                "fps_num": 30,
                "fps_den": 1,
                "video_codec": "h264",
                "container": "mp4",
                "fingerprint": {
                    "mtime_ms": 1_700_000_000_000_i64,
                    "size_bytes": 1024,
                }
            }
        }))
        .expect("caption.burn_off fixture asset parses"));

    let text_track = json!({
        "id": text_track_id,
        "kind": "text",
        "name": "Captions",
        "locked": false,
        "clips": [{
            "id": text_clip_id,
            "name": "Caption Clip",
            "asset_id": "00000000-0000-0000-0000-000000000000",
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 480_000,
            "locked": false,
            "text": {
                "content": "Caption",
                "font_family": "Arial",
                "font_size_px": 24,
            },
        }],
    });
    let video_track = json!({
        "id": video_track_id,
        "kind": "video",
        "name": "Video",
        "locked": false,
        "clips": [{
            "id": video_clip_id,
            "name": "Shot",
            "asset_id": video_asset_id,
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 480_000,
            "locked": false,
            "effects": [{
                "id": effect_id,
                "kind": "burned_caption",
                "enabled": true,
                "params": {
                    "source_text_track_id": text_track_id,
                },
            }],
        }],
    });
    prior
        .tracks
        .push(serde_json::from_value(text_track).expect("text track fixture parses"));
    prior
        .tracks
        .push(serde_json::from_value(video_track).expect("video track fixture parses"));
    prior.duration_tk = Tick::new(480_000);

    let args = caption_burn_off::CaptionBurnOffArgs {
        project_id,
        text_track: Some(text_track_id.to_string()),
        clip: None,
    };

    let (patch_value, warnings, _data) = caption_burn_off::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid caption.burn_off patch");
    let patch: json_patch::Patch = serde_json::from_value(patch_value.clone())
        .expect("caption.burn_off fixture patch is valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("caption.burn_off fixture patch must apply cleanly");

    let expected_data = serde_json::to_value(
        caption_burn_off::data_envelope_from_args_warnings(&args, &warnings)
            .expect("caption.burn_off fixture expected_data"),
    )
    .expect("caption.burn_off fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "caption.burn_off".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings,
        post_state,
        expected_data,
    }
}

/// Build the canonical `text.style` fixture used by [`default_fixtures`].
///
/// Starts from a synthetic project with a single text track and a single
/// text clip, then updates two safe style leaves.
fn text_style_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let mut prior = synthetic_empty_project(project_id);

    let clip_id = "01900000-0000-7000-8000-0000000bb102";

    let track_raw = json!({
        "id": "01900000-0000-7000-8000-0000000aa102",
        "kind": "text",
        "name": "Captions",
        "clips": [{
            "id": clip_id,
            "name": "Caption 2",
            "asset_id": "00000000-0000-0000-0000-000000000000",
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 1000,
            "locked": false,
            "text": {
                "content": "Styled",
                "font_family": "Arial",
                "font_size_px": 24,
                "color": "#ffffffff"
            },
        }],
    });

    let track: crate::track::Track =
        serde_json::from_value(track_raw).expect("manual track fixture parses");
    prior.tracks.push(track);
    prior.duration_tk = Tick::new(1000);

    let args = text_style::TextStyleArgs {
        project_id,
        clip: clip_id.to_string(),
        style: json!({
            "color": "#ff0000ff",
            "font_size_px": 96.0
        }),
    };

    let (patch_value, _warnings, _data) = text_style::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid text.style patch");
    let patch: json_patch::Patch = serde_json::from_value(patch_value.clone())
        .expect("text.style fixture patch is valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("text.style fixture patch must apply cleanly");

    let expected_data = serde_json::to_value(
        text_style::data_envelope_from_post_state(&args, &post_state)
            .expect("text.style fixture expected_data"),
    )
    .expect("text.style fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "text.style".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings: vec![],
        post_state,
        expected_data,
    }
}

/// Build the canonical `text.animate` fixture used by [`default_fixtures`].
///
/// Starts from a synthetic project with a single 100-tick text clip,
/// then applies the `fade_in` preset so fractions resolve cleanly.
fn text_animate_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let mut prior = synthetic_empty_project(project_id);

    let clip_id = "01900000-0000-7000-8000-0000000bb103";

    let track_raw = json!({
        "id": "01900000-0000-7000-8000-0000000aa103",
        "kind": "text",
        "name": "Captions",
        "clips": [{
            "id": clip_id,
            "name": "Animated",
            "asset_id": "00000000-0000-0000-0000-000000000000",
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 100,
            "locked": false,
            "text": {
                "content": "Animated",
                "font_family": "Arial",
                "font_size_px": 24
            },
        }],
    });

    let track: crate::track::Track =
        serde_json::from_value(track_raw).expect("manual track fixture parses");
    prior.tracks.push(track);
    prior.duration_tk = Tick::new(100);

    let args = text_animate::TextAnimateArgs {
        project_id,
        clip: clip_id.to_string(),
        preset: "fade_in".to_string(),
        in_tk: Some(0),
        out_tk: Some(99),
    };

    let (patch_value, warnings, _data) = text_animate::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid text.animate patch");
    let patch: json_patch::Patch = serde_json::from_value(patch_value.clone())
        .expect("text.animate fixture patch is valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("text.animate fixture patch must apply cleanly");

    let expected_data = serde_json::to_value(
        text_animate::data_envelope_from_args_patch_warnings(&args, &patch_value, &warnings)
            .expect("text.animate fixture expected_data"),
    )
    .expect("text.animate fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "text.animate".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings,
        post_state,
        expected_data,
    }
}

/// Build the canonical `clip.reverse` fixture used by
/// [`default_fixtures`].
///
/// Starts from a synthetic project with a linked video/audio pair, then
/// sets `reversed` across the whole sync set.
#[allow(clippy::too_many_lines)]
fn clip_reverse_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let mut prior = synthetic_empty_project(project_id);

    let group_id = "01900000-0000-7000-8000-0000000000ad";
    let video_clip_id = "01900000-0000-7000-8000-0000000bb501";
    let audio_clip_id = "01900000-0000-7000-8000-0000000bb502";
    let video_track_id = "01900000-0000-7000-8000-0000000aa501";
    let audio_track_id = "01900000-0000-7000-8000-0000000aa502";
    let video_asset_id = "01900000-0000-7000-8000-0000000dd501";
    let audio_asset_id = "01900000-0000-7000-8000-0000000dd502";

    let video_track = json!({
        "id": video_track_id,
        "kind": "video",
        "name": "Video Reverse",
        "locked": false,
        "clips": [{
            "id": video_clip_id,
            "name": "Linked Video",
            "asset_id": video_asset_id,
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 240_000,
            "locked": false,
            "reversed": false,
            "link_group": group_id,
        }],
    });
    let audio_track = json!({
        "id": audio_track_id,
        "kind": "audio",
        "name": "Audio Reverse",
        "locked": false,
        "clips": [{
            "id": audio_clip_id,
            "name": "Linked Audio",
            "asset_id": audio_asset_id,
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 240_000,
            "volume": 1.0,
            "locked": false,
            "reversed": false,
            "link_group": group_id,
        }],
    });
    prior
        .tracks
        .push(serde_json::from_value(video_track).expect("manual video track parses"));
    prior
        .tracks
        .push(serde_json::from_value(audio_track).expect("manual audio track parses"));
    prior.duration_tk = Tick::new(240_000);

    prior
        .assets
        .push(serde_json::from_value(json!({
            "id": video_asset_id,
            "kind": "video",
            "hash": "36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da",
            "path": "assets/36/36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da.mp4",
            "original_filename": "video-clip-reverse.mp4",
            "imported_at": "2026-05-24T00:00:00Z",
            "metadata": {
                "duration_tk": 240_000,
                "width": 1920,
                "height": 1080,
                "fps_num": 30,
                "fps_den": 1,
                "video_codec": "h264",
                "container": "mp4",
                "fingerprint": {
                    "mtime_ms": 1_700_000_000_000_i64,
                    "size_bytes": 1024,
                }
            }
        }))
        .expect("video clip fixture asset parses"));
    prior
        .assets
        .push(serde_json::from_value(json!({
            "id": audio_asset_id,
            "kind": "audio",
            "hash": "53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658",
            "path": "assets/53/53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.m4a",
            "original_filename": "audio-clip-reverse.m4a",
            "imported_at": "2026-05-24T00:00:00Z",
            "metadata": {
                "duration_tk": 240_000,
                "audio_codec": "aac",
                "audio_channels": 2,
                "audio_sample_rate_hz": 48000,
                "container": "m4a",
                "fingerprint": {
                    "mtime_ms": 1_700_000_000_000_i64,
                    "size_bytes": 1024,
                }
            }
        }))
        .expect("audio clip fixture asset parses"));

    let args = clip_reverse::ClipReverseArgs {
        project_id,
        clip: video_clip_id.to_string(),
        reversed: None,
    };

    let (patch_value, warnings, _data) = clip_reverse::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid clip.reverse patch");
    let patch: json_patch::Patch = serde_json::from_value(patch_value.clone())
        .expect("clip.reverse fixture patch is valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("clip.reverse fixture patch must apply cleanly");

    let expected_data = serde_json::to_value(
        clip_reverse::data_envelope_from_post_state(&args, &post_state)
            .expect("clip.reverse fixture expected_data"),
    )
    .expect("clip.reverse fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "clip.reverse".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings,
        post_state,
        expected_data,
    }
}

/// Build the canonical `clip.set_blend_mode` fixture used by [`default_fixtures`].
///
/// Starts from a synthetic project with a single video track and a single
/// clip, then sets that clip's blend mode.
fn clip_set_blend_mode_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let mut prior = synthetic_empty_project(project_id);

    let track_raw = json!({
        "id": "01900000-0000-7000-8000-0000000aa103",
        "kind": "video",
        "name": "Video 3",
        "locked": false,
        "clips": [{
            "id": "01900000-0000-7000-8000-0000000bb303",
            "name": "Clip 3",
            "asset_id": "01900000-0000-7000-8000-0000000cc303",
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 480_000,
            "locked": false,
        }],
    });

    let track: crate::track::Track =
        serde_json::from_value(track_raw).expect("manual track fixture parses");
    prior.tracks.push(track);
    prior.duration_tk = Tick::new(480_000);
    prior.assets.push(
        serde_json::from_value(json!({
            "id": "01900000-0000-7000-8000-0000000cc303",
            "kind": "video",
            "hash": "53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658",
            "path": "assets/53/53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.mp4",
            "original_filename": "video-clip-set-blend-mode.mp4",
            "imported_at": "2026-05-24T00:00:00Z",
            "metadata": {
                "duration_tk": 480_000,
                "width": 1920,
                "height": 1080,
                "fps_num": 30,
                "fps_den": 1,
                "video_codec": "h264",
                "container": "mp4",
                "fingerprint": {
                    "mtime_ms": 1_700_000_000_000_i64,
                    "size_bytes": 1024,
                }
            }
        }))
        .expect("clip.set_blend_mode fixture asset parses"),
    );

    let args = clip_set_blend_mode::ClipSetBlendModeArgs {
        project_id,
        clip: "01900000-0000-7000-8000-0000000bb303".to_string(),
        blend_mode: crate::clip::BlendMode::Multiply,
    };

    let (patch_value, _warnings, _data) = clip_set_blend_mode::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid clip.set_blend_mode patch");
    let patch: json_patch::Patch = serde_json::from_value(patch_value.clone())
        .expect("clip.set_blend_mode fixture patch must be valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("clip.set_blend_mode fixture patch must apply cleanly");

    let expected_data = serde_json::to_value(
        clip_set_blend_mode::data_envelope_from_post_state(&args, &post_state)
            .expect("clip.set_blend_mode fixture expected_data"),
    )
    .expect("clip.set_blend_mode fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "clip.set_blend_mode".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings: vec![],
        post_state,
        expected_data,
    }
}

/// Build the canonical `clip.set_fade` fixture used by [`default_fixtures`].
///
/// Starts from a synthetic project with a single text track and a single
/// text clip, then sets both fade durations and curves.
fn clip_set_fade_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let mut prior = synthetic_empty_project(project_id);

    let track_raw = json!({
        "id": "01900000-0000-7000-8000-0000000aa304",
        "kind": "text",
        "name": "Text 4",
        "locked": false,
        "clips": [{
            "id": "01900000-0000-7000-8000-0000000bb304",
            "name": "Fade Clip",
            "asset_id": "00000000-0000-0000-0000-000000000000",
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 480_000,
            "locked": false,
            "text": {
                "content": "Fade",
                "font_family": "Arial",
                "font_size_px": 24
            },
        }],
    });

    let track: crate::track::Track =
        serde_json::from_value(track_raw).expect("manual track fixture parses");
    prior.tracks.push(track);
    prior.duration_tk = Tick::new(480_000);

    let args = clip_set_fade::ClipSetFadeArgs {
        project_id,
        clip: "01900000-0000-7000-8000-0000000bb304".to_string(),
        fade_in_tk: Some(8_000),
        fade_out_tk: Some(16_000),
        fade_in_curve: Some(crate::clip::FadeCurve::Exp),
        fade_out_curve: Some(crate::clip::FadeCurve::Log),
    };

    let (patch_value, warnings, _data) = clip_set_fade::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid clip.set_fade patch");
    let patch: json_patch::Patch = serde_json::from_value(patch_value.clone())
        .expect("clip.set_fade fixture patch must be valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("clip.set_fade fixture patch must apply cleanly");

    let expected_data = serde_json::to_value(
        clip_set_fade::data_envelope_from_post_state(&args, &post_state)
            .expect("clip.set_fade fixture expected_data"),
    )
    .expect("clip.set_fade fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "clip.set_fade".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings,
        post_state,
        expected_data,
    }
}

/// Build the canonical `clip.set_mask` fixture used by [`default_fixtures`].
///
/// Starts from a synthetic project with a single video track and clip, then
/// assigns a simple rectangular mask.
fn clip_set_mask_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let mut prior = synthetic_empty_project(project_id);

    let track_raw = json!({
        "id": "01900000-0000-7000-8000-0000000aa305",
        "kind": "video",
        "name": "Video 5",
        "locked": false,
        "clips": [{
            "id": "01900000-0000-7000-8000-0000000bb305",
            "name": "Masked Clip",
            "asset_id": "01900000-0000-7000-8000-0000000cc305",
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 480_000,
            "locked": false,
        }],
    });

    let track: crate::track::Track =
        serde_json::from_value(track_raw).expect("manual track fixture parses");
    prior.tracks.push(track);
    prior.duration_tk = Tick::new(480_000);
    prior.assets.push(
        serde_json::from_value(json!({
            "id": "01900000-0000-7000-8000-0000000cc305",
            "kind": "video",
            "hash": "53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658",
            "path": "assets/53/53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.mp4",
            "original_filename": "video-clip-set-mask.mp4",
            "imported_at": "2026-05-24T00:00:00Z",
            "metadata": {
                "duration_tk": 480_000,
                "width": 1920,
                "height": 1080,
                "fps_num": 30,
                "fps_den": 1,
                "video_codec": "h264",
                "container": "mp4",
                "fingerprint": {
                    "mtime_ms": 1_700_000_000_000_i64,
                    "size_bytes": 1024,
                }
            }
        }))
        .expect("clip.set_mask fixture asset parses"),
    );

    let mut params = Map::new();
    params.insert("x".to_string(), json!(0.0));
    params.insert("y".to_string(), json!(0.0));
    params.insert("w".to_string(), json!(640.0));
    params.insert("h".to_string(), json!(360.0));

    let args = clip_set_mask::ClipSetMaskArgs {
        project_id,
        clip: "01900000-0000-7000-8000-0000000bb305".to_string(),
        mask: Some(crate::clip::ClipMask {
            kind: crate::clip::MaskKind::Rect,
            params,
            feather_px: 4.0,
            inverted: false,
        }),
    };

    let (patch_value, warnings, _data) = clip_set_mask::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid clip.set_mask patch");
    let patch: json_patch::Patch = serde_json::from_value(patch_value.clone())
        .expect("clip.set_mask fixture patch must be valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("clip.set_mask fixture patch must apply cleanly");

    let expected_data = serde_json::to_value(
        clip_set_mask::data_envelope_from_post_state_warnings(&args, &warnings, &post_state)
            .expect("clip.set_mask fixture expected_data"),
    )
    .expect("clip.set_mask fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "clip.set_mask".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings,
        post_state,
        expected_data,
    }
}

/// Build the canonical `clip.set_speed_curve` fixture used by [`default_fixtures`].
///
/// Starts from a synthetic project with a single video track and clip, then
/// assigns a valid 2-point speed curve.
fn clip_set_speed_curve_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let mut prior = synthetic_empty_project(project_id);

    let track_raw = json!({
        "id": "01900000-0000-7000-8000-0000000aa306",
        "kind": "video",
        "name": "Video 6",
        "locked": false,
        "clips": [{
            "id": "01900000-0000-7000-8000-0000000bb306",
            "name": "Speed Curve Clip",
            "asset_id": "01900000-0000-7000-8000-0000000cc306",
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 480_000,
            "locked": false,
        }],
    });

    let track: crate::track::Track =
        serde_json::from_value(track_raw).expect("manual track fixture parses");
    prior.tracks.push(track);
    prior.duration_tk = Tick::new(480_000);
    prior.assets.push(
        serde_json::from_value(json!({
            "id": "01900000-0000-7000-8000-0000000cc306",
            "kind": "video",
            "hash": "43ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658",
            "path": "assets/43/43ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.mp4",
            "original_filename": "video-clip-set-speed-curve.mp4",
            "imported_at": "2026-05-24T00:00:00Z",
            "metadata": {
                "duration_tk": 480_000,
                "width": 1920,
                "height": 1080,
                "fps_num": 30,
                "fps_den": 1,
                "video_codec": "h264",
                "container": "mp4",
                "fingerprint": {
                    "mtime_ms": 1_700_000_000_000_i64,
                    "size_bytes": 1024,
                }
            }
        }))
        .expect("clip.set_speed_curve fixture asset parses"),
    );

    let args = clip_set_speed_curve::ClipSetSpeedCurveArgs {
        project_id,
        clip: "01900000-0000-7000-8000-0000000bb306".to_string(),
        points: Some(vec![
            crate::clip::SpeedCurvePoint {
                time_tk: Tick::new(0),
                factor: 1.0,
            },
            crate::clip::SpeedCurvePoint {
                time_tk: Tick::new(480_000),
                factor: 2.0,
            },
        ]),
    };

    let (patch_value, warnings, _data) = clip_set_speed_curve::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid clip.set_speed_curve patch");
    let patch: json_patch::Patch = serde_json::from_value(patch_value.clone())
        .expect("clip.set_speed_curve fixture patch must be valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("clip.set_speed_curve fixture patch must apply cleanly");

    let expected_data = serde_json::to_value(
        clip_set_speed_curve::data_envelope_from_post_state(&args, &post_state)
            .expect("clip.set_speed_curve fixture expected_data"),
    )
    .expect("clip.set_speed_curve fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "clip.set_speed_curve".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings,
        post_state,
        expected_data,
    }
}

/// Build the canonical `clip.set_speed` fixture used by [`default_fixtures`].
///
/// Starts from a synthetic project with a single 10-second video clip
/// at scalar speed 1.0, then sets `factor=2.0`.
fn clip_set_speed_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let mut prior = synthetic_empty_project(project_id);
    let clip_id = "01900000-0000-7000-8000-0000000bb307";
    let asset_id = "01900000-0000-7000-8000-0000000cc307";

    prior.assets.push(
        serde_json::from_value(json!({
            "id": asset_id,
            "kind": "video",
            "hash": "54ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658",
            "path": "assets/54/54ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.mp4",
            "original_filename": "video-clip-set-speed.mp4",
            "imported_at": "2026-05-24T00:00:00Z",
            "metadata": {
                "duration_tk": 2_400_000,
                "width": 1920,
                "height": 1080,
                "fps_num": 30,
                "fps_den": 1,
                "video_codec": "h264",
                "container": "mp4",
                "fingerprint": {
                    "mtime_ms": 1_700_000_000_000_i64,
                    "size_bytes": 1024,
                }
            }
        }))
        .expect("clip.set_speed fixture asset parses"),
    );

    let track_raw = json!({
        "id": "01900000-0000-7000-8000-0000000aa307",
        "kind": "video",
        "name": "Video 7",
        "locked": false,
        "clips": [{
            "id": clip_id,
            "name": "Speed Clip",
            "asset_id": asset_id,
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 2_400_000,
            "speed": 1.0,
            "locked": false,
        }],
    });

    let track: crate::track::Track =
        serde_json::from_value(track_raw).expect("manual track fixture parses");
    prior.tracks.push(track);
    prior.duration_tk = Tick::new(2_400_000);

    let args = clip_set_speed::ClipSetSpeedArgs {
        project_id,
        clip: clip_id.to_string(),
        factor: 2.0,
    };

    let (patch_value, warnings, _data) = clip_set_speed::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid clip.set_speed patch");
    let patch: json_patch::Patch = serde_json::from_value(patch_value.clone())
        .expect("clip.set_speed fixture patch must be valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("clip.set_speed fixture patch must apply cleanly");

    let expected_data = serde_json::to_value(
        clip_set_speed::data_envelope_from_post_state(&args, &post_state)
            .expect("clip.set_speed fixture expected_data"),
    )
    .expect("clip.set_speed fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "clip.set_speed".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings,
        post_state,
        expected_data,
    }
}

/// Build the canonical `clip.set_transform` fixture used by [`default_fixtures`].
///
/// Starts from a synthetic project with a single text track and a single
/// clip, then updates that clip's transform.
fn clip_set_transform_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let mut prior = synthetic_empty_project(project_id);

    let track_raw = json!({
        "id": "01900000-0000-7000-8000-0000000aa303",
        "kind": "text",
        "name": "Text 3",
        "locked": false,
        "clips": [{
            "id": "01900000-0000-7000-8000-0000000bb303",
            "name": "Clip 3",
            "asset_id": "00000000-0000-0000-0000-000000000000",
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 480_000,
            "transform": {
                "x": 0.0,
                "y": 10.0,
                "scale_x": 1.0,
                "scale_y": 1.0,
                "rotation_deg": 0.0,
                "anchor_x": 0.5,
                "anchor_y": 0.5,
                "skew_x_deg": 0.0,
                "skew_y_deg": 0.0,
                "flip_h": false,
                "flip_v": false
            },
            "locked": false,
            "text": {
                "content": "Hello",
                "font_family": "Arial",
                "font_size_px": 24
            },
        }],
    });

    let track: crate::track::Track =
        serde_json::from_value(track_raw).expect("manual track fixture parses");
    prior.tracks.push(track);
    prior.duration_tk = Tick::new(480_000);

    let args = clip_set_transform::ClipSetTransformArgs {
        project_id,
        clip: "01900000-0000-7000-8000-0000000bb303".to_string(),
        transform: clip_set_transform::PartialTransform {
            x: Some(100.0),
            scale_x: Some(2.0),
            ..Default::default()
        },
    };

    let (patch_value, _warnings, _data) = clip_set_transform::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid clip.set_transform patch");
    let patch: json_patch::Patch = serde_json::from_value(patch_value.clone())
        .expect("clip.set_transform fixture patch must be valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("clip.set_transform fixture patch must apply cleanly");

    let expected_data = serde_json::to_value(
        clip_set_transform::data_envelope_from_post_state(&args, &post_state)
            .expect("clip.set_transform fixture expected_data"),
    )
    .expect("clip.set_transform fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "clip.set_transform".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings: vec![],
        post_state,
        expected_data,
    }
}

/// Build the canonical `clip.lock` fixture used by [`default_fixtures`].
///
/// Starts from a synthetic project with a single text track and a single
/// clip, then locks that clip.
fn clip_lock_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let mut prior = synthetic_empty_project(project_id);

    let track_raw = json!({
        "id": "01900000-0000-7000-8000-0000000aa101",
        "kind": "text",
        "name": "Text 1",
        "locked": false,
        "clips": [{
            "id": "01900000-0000-7000-8000-0000000bb201",
            "name": "Clip 1",
            "asset_id": "00000000-0000-0000-0000-000000000000",
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 480_000,
            "locked": false,
            "text": {
                "content": "Hello",
                "font_family": "Arial",
                "font_size_px": 24
            },
        }],
    });

    let track: crate::track::Track =
        serde_json::from_value(track_raw).expect("manual track fixture parses");
    prior.tracks.push(track);
    prior.duration_tk = Tick::new(480_000);

    let args = clip_lock::ClipLockArgs {
        project_id,
        clip: "01900000-0000-7000-8000-0000000bb201".to_string(),
        locked: Some(true),
    };

    let (patch_value, _warnings, _data) = clip_lock::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid clip.lock patch");
    let patch: json_patch::Patch = serde_json::from_value(patch_value.clone())
        .expect("clip.lock fixture patch is valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("clip.lock fixture patch must apply cleanly");

    let expected_data = serde_json::to_value(
        clip_lock::data_envelope_from_post_state(&args, &post_state)
            .expect("clip.lock fixture expected_data"),
    )
    .expect("clip.lock fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "clip.lock".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings: vec![],
        post_state,
        expected_data,
    }
}

/// Build the canonical `clip.move` fixture used by [`default_fixtures`].
///
/// Starts from a synthetic project with one video clip, then repositions
/// that clip on the same track.
fn clip_move_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let mut prior = synthetic_empty_project(project_id);
    let track_id = "01900000-0000-7000-8000-0000000aa506";
    let clip_id = "01900000-0000-7000-8000-0000000bb506";
    let asset_id = "01900000-0000-7000-8000-0000000dd506";

    let track_raw = json!({
        "id": track_id,
        "kind": "video",
        "name": "Video Move",
        "locked": false,
        "clips": [{
            "id": clip_id,
            "name": "Move Clip",
            "asset_id": asset_id,
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 240_000,
            "locked": false,
        }],
    });
    prior
        .tracks
        .push(serde_json::from_value(track_raw).expect("manual video track parses"));
    prior.duration_tk = Tick::new(240_000);
    prior.assets.push(
        serde_json::from_value(json!({
            "id": asset_id,
            "kind": "video",
            "hash": "36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da",
            "path": "assets/36/36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da.mp4",
            "original_filename": "video-clip-move.mp4",
            "imported_at": "2026-05-24T00:00:00Z",
            "metadata": {
                "duration_tk": 240_000,
                "width": 1920,
                "height": 1080,
                "fps_num": 30,
                "fps_den": 1,
                "video_codec": "h264",
                "container": "mp4",
                "fingerprint": {
                    "mtime_ms": 1_700_000_000_000_i64,
                    "size_bytes": 1024,
                }
            }
        }))
        .expect("video clip fixture asset parses"),
    );

    let args = clip_move::ClipMoveArgs {
        project_id,
        clip: clip_id.to_string(),
        track_position_tk: Some(240_000),
        to_track: None,
    };

    let (patch_value, warnings, _data) = clip_move::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid clip.move patch");
    let patch: json_patch::Patch = serde_json::from_value(patch_value.clone())
        .expect("clip.move fixture patch is valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("clip.move fixture patch must apply cleanly");

    let expected_data = serde_json::to_value(
        clip_move::data_envelope_from_post_state(&args, &post_state)
            .expect("clip.move fixture expected_data"),
    )
    .expect("clip.move fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "clip.move".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings,
        post_state,
        expected_data,
    }
}

/// Build the canonical `clip.rename` fixture used by [`default_fixtures`].
///
/// Starts from a synthetic project with a single text track and a single
/// clip, then renames that clip.
fn clip_rename_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let mut prior = synthetic_empty_project(project_id);

    let track_raw = json!({
        "id": "01900000-0000-7000-8000-0000000aa102",
        "kind": "text",
        "name": "Text 2",
        "locked": false,
        "clips": [{
            "id": "01900000-0000-7000-8000-0000000bb202",
            "name": "Clip 2",
            "asset_id": "00000000-0000-0000-0000-000000000000",
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 480_000,
            "locked": false,
            "text": {
                "content": "Hello",
                "font_family": "Arial",
                "font_size_px": 24
            },
        }],
    });

    let track: crate::track::Track =
        serde_json::from_value(track_raw).expect("manual track fixture parses");
    prior.tracks.push(track);
    prior.duration_tk = Tick::new(480_000);

    let args = clip_rename::ClipRenameArgs {
        project_id,
        clip: "01900000-0000-7000-8000-0000000bb202".to_string(),
        name: "Renamed Clip".to_string(),
    };

    let (patch_value, _warnings, _data) = clip_rename::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid clip.rename patch");
    let patch: json_patch::Patch = serde_json::from_value(patch_value.clone())
        .expect("clip.rename fixture patch must be valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("clip.rename fixture patch must apply cleanly");

    let expected_data = serde_json::to_value(
        clip_rename::data_envelope_from_post_state(&args, &post_state)
            .expect("clip.rename fixture expected_data"),
    )
    .expect("clip.rename fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "clip.rename".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings: vec![],
        post_state,
        expected_data,
    }
}

/// Build the canonical `clip.set_opacity` fixture used by [`default_fixtures`].
///
/// Starts from a synthetic project with a single text track and a single
/// clip, then sets that clip's opacity.
fn clip_set_opacity_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let mut prior = synthetic_empty_project(project_id);

    let track_raw = json!({
        "id": "01900000-0000-7000-8000-0000000aa103",
        "kind": "text",
        "name": "Text 3",
        "locked": false,
        "clips": [{
            "id": "01900000-0000-7000-8000-0000000bb203",
            "name": "Clip 3",
            "asset_id": "00000000-0000-0000-0000-000000000000",
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 480_000,
            "locked": false,
            "text": {
                "content": "Hello",
                "font_family": "Arial",
                "font_size_px": 24
            },
        }],
    });

    let track: crate::track::Track =
        serde_json::from_value(track_raw).expect("manual track fixture parses");
    prior.tracks.push(track);
    prior.duration_tk = Tick::new(480_000);

    let args = clip_set_opacity::ClipSetOpacityArgs {
        project_id,
        clip: "01900000-0000-7000-8000-0000000bb203".to_string(),
        opacity: 0.5,
    };

    let (patch_value, _warnings, _data) = clip_set_opacity::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid clip.set_opacity patch");
    let patch: json_patch::Patch = serde_json::from_value(patch_value.clone())
        .expect("clip.set_opacity fixture patch must be valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("clip.set_opacity fixture patch must apply cleanly");

    let expected_data = serde_json::to_value(
        clip_set_opacity::data_envelope_from_post_state(&args, &post_state)
            .expect("clip.set_opacity fixture expected_data"),
    )
    .expect("clip.set_opacity fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "clip.set_opacity".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings: vec![],
        post_state,
        expected_data,
    }
}

/// Build the canonical `clip.set_volume` fixture used by [`default_fixtures`].
///
/// Starts from a synthetic project with a single audio track and a single
/// audio clip referencing a real audio asset, then sets that clip's volume.
fn clip_set_volume_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let mut prior = synthetic_empty_project(project_id);

    prior.assets.push(
        serde_json::from_value(json!({
            "id": "01900000-0000-7000-8000-0000000ccaa1",
            "kind": "audio",
            "hash": "36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da",
            "path": "assets/36/36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da.m4a",
            "original_filename": "audio-clip-set-volume.m4a",
            "imported_at": "2026-05-24T00:00:00Z",
            "metadata": {
                "duration_tk": 240_000,
                "audio_codec": "aac",
                "audio_channels": 2,
                "audio_sample_rate_hz": 48000,
                "container": "m4a",
                "fingerprint": {
                    "mtime_ms": 1_700_000_000_000_i64,
                    "size_bytes": 1024,
                }
            }
        }))
        .expect("audio asset fixture parses"),
    );

    let track_raw = json!({
        "id": "01900000-0000-7000-8000-0000000aa201",
        "kind": "audio",
        "name": "Audio 1",
        "locked": false,
        "clips": [{
            "id": "01900000-0000-7000-8000-0000000bb301",
            "name": "Audio Clip 1",
            "asset_id": "01900000-0000-7000-8000-0000000ccaa1",
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 240_000,
            "volume": 1.0,
            "locked": false,
        }],
    });

    let track: crate::track::Track =
        serde_json::from_value(track_raw).expect("manual track fixture parses");
    prior.tracks.push(track);
    prior.duration_tk = Tick::new(240_000);

    let args = clip_set_volume::ClipSetVolumeArgs {
        project_id,
        clip: "01900000-0000-7000-8000-0000000bb301".to_string(),
        volume: 0.5,
    };

    let (patch_value, _warnings, _data) = clip_set_volume::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid clip.set_volume patch");
    let patch: json_patch::Patch = serde_json::from_value(patch_value.clone())
        .expect("clip.set_volume fixture patch is valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("clip.set_volume fixture patch must apply cleanly");

    let expected_data = serde_json::to_value(
        clip_set_volume::data_envelope_from_post_state(&args, &post_state)
            .expect("clip.set_volume fixture expected_data"),
    )
    .expect("clip.set_volume fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "clip.set_volume".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings: vec![],
        post_state,
        expected_data,
    }
}

/// Build the canonical `clip.split` fixture used by [`default_fixtures`].
///
/// Starts from a synthetic project with one video clip, then splits
/// that clip at its midpoint.
fn clip_split_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let mut prior = synthetic_empty_project(project_id);
    let track_id = "01900000-0000-7000-8000-0000000aa508";
    let clip_id = "01900000-0000-7000-8000-0000000bb508";
    let asset_id = "01900000-0000-7000-8000-0000000dd508";

    let track_raw = json!({
        "id": track_id,
        "kind": "video",
        "name": "Video Split",
        "locked": false,
        "clips": [{
            "id": clip_id,
            "name": "Split Clip",
            "asset_id": asset_id,
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 240_000,
            "locked": false,
        }],
    });
    prior
        .tracks
        .push(serde_json::from_value(track_raw).expect("manual video track parses"));
    prior.duration_tk = Tick::new(240_000);
    prior.assets.push(
        serde_json::from_value(json!({
            "id": asset_id,
            "kind": "video",
            "hash": "36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da",
            "path": "assets/36/36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da.mp4",
            "original_filename": "video-clip-split.mp4",
            "imported_at": "2026-05-24T00:00:00Z",
            "metadata": {
                "duration_tk": 240_000,
                "width": 1920,
                "height": 1080,
                "fps_num": 30,
                "fps_den": 1,
                "video_codec": "h264",
                "container": "mp4",
                "fingerprint": {
                    "mtime_ms": 1_700_000_000_000_i64,
                    "size_bytes": 1024,
                }
            }
        }))
        .expect("video clip fixture asset parses"),
    );

    let args = clip_split::ClipSplitArgs {
        project_id,
        clip: clip_id.to_string(),
        at_tk: 120_000,
    };

    let (patch_value, warnings, _data) = clip_split::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid clip.split patch");
    let patch: json_patch::Patch = serde_json::from_value(patch_value.clone())
        .expect("clip.split fixture patch is valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("clip.split fixture patch must apply cleanly");

    let expected_data = serde_json::to_value(
        clip_split::data_envelope_from_args_warnings(&args, &warnings)
            .expect("clip.split fixture expected_data"),
    )
    .expect("clip.split fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "clip.split".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings,
        post_state,
        expected_data,
    }
}

/// Build the canonical `clip.trim` fixture used by [`default_fixtures`].
///
/// Starts from a synthetic project with one video clip, then shortens
/// that clip's source out-point by one 30 fps frame.
fn clip_trim_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let mut prior = synthetic_empty_project(project_id);
    let track_id = "01900000-0000-7000-8000-0000000aa507";
    let clip_id = "01900000-0000-7000-8000-0000000bb507";
    let asset_id = "01900000-0000-7000-8000-0000000dd507";

    let track_raw = json!({
        "id": track_id,
        "kind": "video",
        "name": "Video Trim",
        "locked": false,
        "clips": [{
            "id": clip_id,
            "name": "Trim Clip",
            "asset_id": asset_id,
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 240_000,
            "locked": false,
        }],
    });
    prior
        .tracks
        .push(serde_json::from_value(track_raw).expect("manual video track parses"));
    prior.duration_tk = Tick::new(240_000);
    prior.assets.push(
        serde_json::from_value(json!({
            "id": asset_id,
            "kind": "video",
            "hash": "36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da",
            "path": "assets/36/36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da.mp4",
            "original_filename": "video-clip-trim.mp4",
            "imported_at": "2026-05-24T00:00:00Z",
            "metadata": {
                "duration_tk": 240_000,
                "width": 1920,
                "height": 1080,
                "fps_num": 30,
                "fps_den": 1,
                "video_codec": "h264",
                "container": "mp4",
                "fingerprint": {
                    "mtime_ms": 1_700_000_000_000_i64,
                    "size_bytes": 1024,
                }
            }
        }))
        .expect("video clip fixture asset parses"),
    );

    let args = clip_trim::ClipTrimArgs {
        project_id,
        clip: clip_id.to_string(),
        source_in_tk: None,
        source_out_tk: Some(232_000),
        keep_end: None,
    };

    let (patch_value, warnings, _data) = clip_trim::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid clip.trim patch");
    let patch: json_patch::Patch = serde_json::from_value(patch_value.clone())
        .expect("clip.trim fixture patch is valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("clip.trim fixture patch must apply cleanly");

    let expected_data = serde_json::to_value(
        clip_trim::data_envelope_from_post_state(&args, &post_state)
            .expect("clip.trim fixture expected_data"),
    )
    .expect("clip.trim fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "clip.trim".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings,
        post_state,
        expected_data,
    }
}

/// Build the canonical `clip.add` fixture used by [`default_fixtures`].
///
/// Starts from a synthetic project with one empty video track and one
/// video asset, then places the asset at position 0 with default source
/// bounds.
fn clip_add_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let mut prior = synthetic_empty_project(project_id);
    let track_id = "01900000-0000-7000-8000-0000000aa5a0";
    let asset_id = "01900000-0000-7000-8000-0000000dd5a0";

    let video_track = json!({
        "id": track_id,
        "kind": "video",
        "name": "Video Add",
        "locked": false,
        "clips": [],
    });
    prior
        .tracks
        .push(serde_json::from_value(video_track).expect("manual video track parses"));
    prior.assets.push(
        serde_json::from_value(json!({
            "id": asset_id,
            "kind": "video",
            "hash": "36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da",
            "path": "assets/36/36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da.mp4",
            "original_filename": "video-clip-add.mp4",
            "imported_at": "2026-05-24T00:00:00Z",
            "metadata": {
                "duration_tk": 240_000,
                "width": 1920,
                "height": 1080,
                "fps_num": 30,
                "fps_den": 1,
                "video_codec": "h264",
                "container": "mp4",
                "fingerprint": {
                    "mtime_ms": 1_700_000_000_000_i64,
                    "size_bytes": 1024,
                }
            }
        }))
        .expect("clip.add fixture asset parses"),
    );

    let args = clip_add::ClipAddArgs {
        project_id,
        asset_id: asset_id.to_string(),
        track: track_id.to_string(),
        track_position_tk: 0,
        source_in_tk: None,
        source_out_tk: None,
        name: None,
    };

    let (patch_value, warnings, _data) = clip_add::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid clip.add patch");
    let patch: json_patch::Patch = serde_json::from_value(patch_value.clone())
        .expect("clip.add fixture patch is valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("clip.add fixture patch must apply cleanly");

    let expected_data = serde_json::to_value(
        clip_add::data_envelope_from_warnings(&warnings).expect("clip.add fixture expected_data"),
    )
    .expect("clip.add fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "clip.add".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings,
        post_state,
        expected_data,
    }
}

/// Build the canonical `clip.delete` fixture used by [`default_fixtures`].
///
/// Starts from a synthetic project with one linked video clip and one
/// linked audio clip, then deletes the video clip so the audio
/// survivor's singleton `link_group` is cleared.
#[allow(clippy::too_many_lines)]
fn clip_delete_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let mut prior = synthetic_empty_project(project_id);

    let group_id = "01900000-0000-7000-8000-0000000000ac";
    let video_clip_id = "01900000-0000-7000-8000-0000000bb401";
    let audio_clip_id = "01900000-0000-7000-8000-0000000bb402";
    let video_track_id = "01900000-0000-7000-8000-0000000aa401";
    let audio_track_id = "01900000-0000-7000-8000-0000000aa402";
    let video_asset_id = "01900000-0000-7000-8000-0000000dd401";
    let audio_asset_id = "01900000-0000-7000-8000-0000000dd402";

    let video_track = json!({
        "id": video_track_id,
        "kind": "video",
        "name": "Video Delete",
        "locked": false,
        "clips": [{
            "id": video_clip_id,
            "name": "Linked Video",
            "asset_id": video_asset_id,
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 240_000,
            "locked": false,
            "link_group": group_id,
        }],
    });
    let audio_track = json!({
        "id": audio_track_id,
        "kind": "audio",
        "name": "Audio Delete",
        "locked": false,
        "clips": [{
            "id": audio_clip_id,
            "name": "Linked Audio",
            "asset_id": audio_asset_id,
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 240_000,
            "volume": 1.0,
            "locked": false,
            "link_group": group_id,
        }],
    });
    prior
        .tracks
        .push(serde_json::from_value(video_track).expect("manual video track parses"));
    prior
        .tracks
        .push(serde_json::from_value(audio_track).expect("manual audio track parses"));
    prior.duration_tk = Tick::new(240_000);

    prior
        .assets
        .push(serde_json::from_value(json!({
            "id": video_asset_id,
            "kind": "video",
            "hash": "36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da",
            "path": "assets/36/36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da.mp4",
            "original_filename": "video-clip-delete.mp4",
            "imported_at": "2026-05-24T00:00:00Z",
            "metadata": {
                "duration_tk": 240_000,
                "width": 1920,
                "height": 1080,
                "fps_num": 30,
                "fps_den": 1,
                "video_codec": "h264",
                "container": "mp4",
                "fingerprint": {
                    "mtime_ms": 1_700_000_000_000_i64,
                    "size_bytes": 1024,
                }
            }
        }))
        .expect("video clip fixture asset parses"));
    prior
        .assets
        .push(serde_json::from_value(json!({
            "id": audio_asset_id,
            "kind": "audio",
            "hash": "53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658",
            "path": "assets/53/53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.m4a",
            "original_filename": "audio-clip-delete.m4a",
            "imported_at": "2026-05-24T00:00:00Z",
            "metadata": {
                "duration_tk": 240_000,
                "audio_codec": "aac",
                "audio_channels": 2,
                "audio_sample_rate_hz": 48000,
                "container": "m4a",
                "fingerprint": {
                    "mtime_ms": 1_700_000_000_000_i64,
                    "size_bytes": 1024,
                }
            }
        }))
        .expect("audio clip fixture asset parses"));

    let args = clip_delete::ClipDeleteArgs {
        project_id,
        clips: vec![video_clip_id.to_string()],
        soft: None,
        ripple: None,
        ripple_scope: None,
    };

    let (patch_value, warnings, _data) = clip_delete::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid clip.delete patch");
    let patch: json_patch::Patch = serde_json::from_value(patch_value.clone())
        .expect("clip.delete fixture patch is valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("clip.delete fixture patch must apply cleanly");

    let expected_data = serde_json::to_value(
        clip_delete::data_envelope_from_args_warnings(&args, &warnings)
            .expect("clip.delete fixture expected_data"),
    )
    .expect("clip.delete fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "clip.delete".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings,
        post_state,
        expected_data,
    }
}

/// Build the canonical `clip.duplicate` fixture used by
/// [`default_fixtures`].
///
/// Starts from a synthetic project with one singleton video clip, then
/// duplicates it immediately after its source end tick.
fn clip_duplicate_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let mut prior = synthetic_empty_project(project_id);
    let track_id = "01900000-0000-7000-8000-0000000aa509";
    let clip_id = "01900000-0000-7000-8000-0000000bb509";
    let asset_id = "01900000-0000-7000-8000-0000000dd509";

    let track_raw = json!({
        "id": track_id,
        "kind": "video",
        "name": "Video Duplicate",
        "locked": false,
        "clips": [{
            "id": clip_id,
            "name": "Duplicate Clip",
            "asset_id": asset_id,
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 240_000,
            "locked": false,
        }],
    });
    prior
        .tracks
        .push(serde_json::from_value(track_raw).expect("manual video track parses"));
    prior.duration_tk = Tick::new(240_000);
    prior.assets.push(
        serde_json::from_value(json!({
            "id": asset_id,
            "kind": "video",
            "hash": "36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da",
            "path": "assets/36/36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da.mp4",
            "original_filename": "video-clip-duplicate.mp4",
            "imported_at": "2026-05-24T00:00:00Z",
            "metadata": {
                "duration_tk": 240_000,
                "width": 1920,
                "height": 1080,
                "fps_num": 30,
                "fps_den": 1,
                "video_codec": "h264",
                "container": "mp4",
                "fingerprint": {
                    "mtime_ms": 1_700_000_000_000_i64,
                    "size_bytes": 1024,
                }
            }
        }))
        .expect("video clip fixture asset parses"),
    );

    let args = clip_duplicate::ClipDuplicateArgs {
        project_id,
        clip: clip_id.to_string(),
        gap_tk: Some(0),
        auto_gap: None,
    };

    let (patch_value, warnings, _data) = clip_duplicate::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid clip.duplicate patch");
    let patch: json_patch::Patch = serde_json::from_value(patch_value.clone())
        .expect("clip.duplicate fixture patch is valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("clip.duplicate fixture patch must apply cleanly");

    let expected_data = serde_json::to_value(
        clip_duplicate::data_envelope_from_args_warnings(&args, &warnings, &post_state)
            .expect("clip.duplicate fixture expected_data"),
    )
    .expect("clip.duplicate fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "clip.duplicate".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings,
        post_state,
        expected_data,
    }
}

/// Build the canonical `clip.unlink` fixture used by [`default_fixtures`].
///
/// Starts from a synthetic project with one video clip and one audio clip
/// in the same link group, then unlinks the group.
#[allow(clippy::too_many_lines)]
fn clip_unlink_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let mut prior = synthetic_empty_project(project_id);

    let group_id = "01900000-0000-7000-8000-0000000000ab";
    let video_clip_id = "01900000-0000-7000-8000-0000000bb301";
    let audio_clip_id = "01900000-0000-7000-8000-0000000bb302";
    let video_track_id = "01900000-0000-7000-8000-0000000aa301";
    let audio_track_id = "01900000-0000-7000-8000-0000000aa302";
    let video_asset_id = "01900000-0000-7000-8000-0000000dd001";
    let audio_asset_id = "01900000-0000-7000-8000-0000000dd002";

    let video_track = json!({
        "id": video_track_id,
        "kind": "video",
        "name": "Video Link",
        "locked": false,
        "clips": [{
            "id": video_clip_id,
            "name": "Linked Video",
            "asset_id": video_asset_id,
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 240_000,
            "locked": false,
            "link_group": group_id,
        }],
    });
    let audio_track = json!({
        "id": audio_track_id,
        "kind": "audio",
        "name": "Audio Link",
        "locked": false,
        "clips": [{
            "id": audio_clip_id,
            "name": "Linked Audio",
            "asset_id": audio_asset_id,
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 240_000,
            "volume": 1.0,
            "locked": false,
            "link_group": group_id,
        }],
    });
    prior
        .tracks
        .push(serde_json::from_value(video_track).expect("manual video track parses"));
    prior
        .tracks
        .push(serde_json::from_value(audio_track).expect("manual audio track parses"));
    prior.duration_tk = Tick::new(240_000);

    prior
        .assets
        .push(serde_json::from_value(json!({
            "id": video_asset_id,
            "kind": "video",
            "hash": "36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da",
            "path": "assets/36/36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da.mp4",
            "original_filename": "video-clip-unlink.mp4",
            "imported_at": "2026-05-24T00:00:00Z",
            "metadata": {
                "duration_tk": 240_000,
                "width": 1920,
                "height": 1080,
                "fps_num": 30,
                "fps_den": 1,
                "video_codec": "h264",
                "container": "mp4",
                "fingerprint": {
                    "mtime_ms": 1_700_000_000_000_i64,
                    "size_bytes": 1024,
                }
            }
        }))
        .expect("video clip fixture asset parses"));
    prior
        .assets
        .push(serde_json::from_value(json!({
            "id": audio_asset_id,
            "kind": "audio",
            "hash": "53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658",
            "path": "assets/53/53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.m4a",
            "original_filename": "audio-clip-unlink.m4a",
            "imported_at": "2026-05-24T00:00:00Z",
            "metadata": {
                "duration_tk": 240_000,
                "audio_codec": "aac",
                "audio_channels": 2,
                "audio_sample_rate_hz": 48000,
                "container": "m4a",
                "fingerprint": {
                    "mtime_ms": 1_700_000_000_000_i64,
                    "size_bytes": 1024,
                }
            }
        }))
        .expect("audio clip fixture asset parses"));

    let args = clip_unlink::ClipUnlinkArgs {
        project_id,
        clip: video_clip_id.to_string(),
    };

    let (patch_value, _warnings, _data) = clip_unlink::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid clip.unlink patch");
    let patch: json_patch::Patch = serde_json::from_value(patch_value.clone())
        .expect("clip.unlink fixture patch is valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("clip.unlink fixture patch must apply cleanly");

    let expected_data = serde_json::to_value(
        clip_unlink::data_envelope_from_patch_and_post_state(&patch_value, &post_state)
            .expect("clip.unlink fixture expected_data"),
    )
    .expect("clip.unlink fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "clip.unlink".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings: vec![],
        post_state,
        expected_data,
    }
}

/// Build the canonical `compound.create` fixture used by
/// [`default_fixtures`].
///
/// `compound.create` always errors with `E_SCHEMA_VIOLATION` for
/// accepted non-empty requests in the v1 floor because compound asset
/// schema/storage context is intentionally deferred. No successful
/// event can be recorded, so the reconstructor only checks args
/// deserialization and returns `Value::Null`. The fixture records
/// `expected_data: null` so the startup gate's canonical equality
/// holds.
fn compound_create_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = compound_create::CompoundCreateArgs {
        project_id,
        clips: vec![
            "01900000-0000-7000-8000-0000000bb910"
                .parse()
                .expect("fixture clip id parse"),
        ],
        name: Some("Compound 1".to_string()),
        allow_gaps: None,
    };

    RecordedEvent {
        verb: "compound.create".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: json!([]),
        warnings: vec![],
        post_state: prior,
        expected_data: Value::Null,
    }
}

/// Build the canonical `compound.edit_in_place` fixture used by
/// [`default_fixtures`].
///
/// `compound.edit_in_place` always errors with
/// `E_COMPOUND_NOT_A_COMPOUND` for accepted selectors in the v1 floor
/// because compound schema/session runtime context is intentionally
/// deferred. No successful event can be recorded, so the reconstructor
/// only checks args deserialization and returns `Value::Null`. The
/// fixture records `expected_data: null` so the startup gate's
/// canonical equality holds.
fn compound_edit_in_place_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = compound_edit_in_place::CompoundEditInPlaceArgs {
        project_id,
        clip: "clip:01900000-0000-7000-8000-0000000bb910".to_string(),
    };

    RecordedEvent {
        verb: "compound.edit_in_place".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: json!([]),
        warnings: vec![],
        post_state: prior,
        expected_data: Value::Null,
    }
}

/// Build the canonical `compound.expand` fixture used by
/// [`default_fixtures`].
///
/// `compound.expand` always errors with `E_COMPOUND_NOT_A_COMPOUND`
/// for accepted selectors in the v1 floor because compound
/// schema/runtime context is intentionally deferred. No successful
/// event can be recorded, so the reconstructor only checks args
/// deserialization and returns `Value::Null`. The fixture records
/// `expected_data: null` so the startup gate's canonical equality
/// holds.
fn compound_expand_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = compound_expand::CompoundExpandArgs {
        project_id,
        clip: "clip:01900000-0000-7000-8000-0000000bb910".to_string(),
    };

    RecordedEvent {
        verb: "compound.expand".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: json!([]),
        warnings: vec![],
        post_state: prior,
        expected_data: Value::Null,
    }
}

/// Build the canonical `compound.flatten` fixture used by
/// [`default_fixtures`].
///
/// `compound.flatten` always errors with `E_COMPOUND_NOT_A_COMPOUND`
/// for accepted selectors in the v1 floor because compound
/// schema/runtime context is intentionally deferred. No successful
/// event can be recorded, so the reconstructor only checks args
/// deserialization and returns `Value::Null`. The fixture records
/// `expected_data: null` so the startup gate's canonical equality
/// holds.
fn compound_flatten_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = compound_flatten::CompoundFlattenArgs {
        project_id,
        clip: "clip:01900000-0000-7000-8000-0000000bb910".to_string(),
    };

    RecordedEvent {
        verb: "compound.flatten".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: json!([]),
        warnings: vec![],
        post_state: prior,
        expected_data: Value::Null,
    }
}

/// Build the canonical `effect.toggle` fixture used by [`default_fixtures`].
///
/// Starts from a synthetic project with a single text clip and a single
/// disabled effect, then enables the effect.
fn effect_toggle_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let mut prior = synthetic_empty_project(project_id);

    let track_raw = json!({
        "id": "01900000-0000-7000-8000-0000000aa201",
        "kind": "text",
        "name": "Text 1",
        "locked": false,
        "clips": [{
            "id": "01900000-0000-7000-8000-0000000bb201",
            "name": "Clip 1",
            "asset_id": "00000000-0000-0000-0000-000000000000",
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 480_000,
            "locked": false,
            "text": {
                "content": "Hello",
                "font_family": "Arial",
                "font_size_px": 24,
            },
            "effects": [{
                "id": "01900000-0000-7000-8000-0000000cc201",
                "kind": "blur",
                "enabled": false,
                "params": { "radius": 5 },
            }],
        }],
    });

    let track: crate::track::Track =
        serde_json::from_value(track_raw).expect("manual track fixture parses");
    prior.tracks.push(track);
    prior.duration_tk = Tick::new(480_000);

    let args = effect_toggle::EffectToggleArgs {
        project_id,
        effect: "01900000-0000-7000-8000-0000000cc201".to_string(),
        enabled: Some(true),
    };

    let (patch_value, _warnings, _data) = effect_toggle::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid effect.toggle patch");
    let patch: json_patch::Patch = serde_json::from_value(patch_value.clone())
        .expect("effect.toggle fixture patch is valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("effect.toggle fixture patch must apply cleanly");

    let expected_data = serde_json::to_value(
        effect_toggle::data_envelope_from_post_state(&args, &post_state)
            .expect("effect.toggle fixture expected_data"),
    )
    .expect("effect.toggle fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "effect.toggle".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings: vec![],
        post_state,
        expected_data,
    }
}

/// Build the canonical `effect.add` fixture used by
/// [`default_fixtures`].
fn effect_add_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let mut prior = synthetic_empty_project(project_id);

    let track_raw = json!({
        "id": "01900000-0000-7000-8000-0000000aa251",
        "kind": "text",
        "name": "Text 1",
        "locked": false,
        "clips": [{
            "id": "01900000-0000-7000-8000-0000000bb251",
            "name": "Clip 1",
            "asset_id": "00000000-0000-0000-0000-000000000000",
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 480_000,
            "locked": false,
            "text": {
                "content": "Hello",
                "font_family": "Arial",
                "font_size_px": 24,
            },
            "effects": [],
        }],
    });

    let track: crate::track::Track =
        serde_json::from_value(track_raw).expect("manual track fixture parses");
    prior.tracks.push(track);
    prior.duration_tk = Tick::new(480_000);

    let args = effect_add::EffectAddArgs {
        project_id,
        target: "clip:01900000-0000-7000-8000-0000000bb251".to_string(),
        kind: "blur".to_string(),
        params: None,
        in_tk: None,
        out_tk: None,
    };

    let (patch_value, warnings, _data) = effect_add::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid effect.add patch");
    let patch: json_patch::Patch = serde_json::from_value(patch_value.clone())
        .expect("effect.add fixture patch is valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("effect.add fixture patch must apply cleanly");

    let expected_data = serde_json::to_value(
        effect_add::data_envelope_from_args_warnings(&args, &warnings)
            .expect("effect.add fixture expected_data"),
    )
    .expect("effect.add fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "effect.add".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings,
        post_state,
        expected_data,
    }
}

/// Build the canonical `effect.list_available` fixture used by
/// [`default_fixtures`].
fn effect_list_available_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = effect_list_available::EffectListAvailableArgs {
        project_id,
        category: None,
    };

    let (patch_value, _warnings, data) = effect_list_available::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid effect.list_available data");
    let expected_data = serde_json::to_value(&data)
        .expect("effect.list_available fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "effect.list_available".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings: vec![],
        post_state: prior,
        expected_data,
    }
}

/// Build the canonical `effect.remove` fixture used by [`default_fixtures`].
///
/// Starts from a synthetic project with one clip carrying one unmanaged
/// effect and one keyframe targeting that effect, then removes both via the
/// §6.2 cascade.
fn effect_remove_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let mut prior = synthetic_empty_project(project_id);

    let track_raw = json!({
        "id": "01900000-0000-7000-8000-0000000aa231",
        "kind": "text",
        "name": "Text 1",
        "locked": false,
        "clips": [{
            "id": "01900000-0000-7000-8000-0000000bb231",
            "name": "Clip 1",
            "asset_id": "00000000-0000-0000-0000-000000000000",
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 480_000,
            "locked": false,
            "text": {
                "content": "Hello",
                "font_family": "Arial",
                "font_size_px": 24,
            },
            "effects": [{
                "id": "01900000-0000-7000-8000-0000000cc231",
                "kind": "color_correct",
                "enabled": true,
                "params": { "brightness": 0.1 },
            }],
            "keyframes": [{
                "id": "01900000-0000-7000-8000-0000000dd231",
                "property": "effects[01900000-0000-7000-8000-0000000cc231].params.brightness",
                "time_tk": 120_000,
                "value": 0.25,
                "easing": "linear",
            }],
        }],
    });

    let track: crate::track::Track =
        serde_json::from_value(track_raw).expect("manual track fixture parses");
    prior.tracks.push(track);
    prior.duration_tk = Tick::new(480_000);

    let args = effect_remove::EffectRemoveArgs {
        project_id,
        effect: "01900000-0000-7000-8000-0000000cc231".to_string(),
    };

    let (patch_value, warnings, _data) = effect_remove::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid effect.remove patch");
    let patch: json_patch::Patch = serde_json::from_value(patch_value.clone())
        .expect("effect.remove fixture patch is valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("effect.remove fixture patch must apply cleanly");

    let expected_data = serde_json::to_value(
        effect_remove::data_envelope_from_args_warnings(&args, &warnings)
            .expect("effect.remove fixture expected_data"),
    )
    .expect("effect.remove fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "effect.remove".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings,
        post_state,
        expected_data,
    }
}

/// Build the canonical `effect.reorder` fixture used by [`default_fixtures`].
///
/// Starts from a synthetic project with one clip carrying two effects, then
/// moves the first effect to the tail.
fn effect_reorder_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let mut prior = synthetic_empty_project(project_id);

    let track_raw = json!({
        "id": "01900000-0000-7000-8000-0000000aa211",
        "kind": "text",
        "name": "Text 1",
        "locked": false,
        "clips": [{
            "id": "01900000-0000-7000-8000-0000000bb211",
            "name": "Clip 1",
            "asset_id": "00000000-0000-0000-0000-000000000000",
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 480_000,
            "locked": false,
            "text": {
                "content": "Hello",
                "font_family": "Arial",
                "font_size_px": 24,
            },
            "effects": [
                {
                    "id": "01900000-0000-7000-8000-0000000cc211",
                    "kind": "blur",
                    "enabled": true,
                    "params": { "radius": 5 },
                },
                {
                    "id": "01900000-0000-7000-8000-0000000cc212",
                    "kind": "sharpen",
                    "enabled": true,
                    "params": { "amount": 2 },
                }
            ],
        }],
    });

    let track: crate::track::Track =
        serde_json::from_value(track_raw).expect("manual track fixture parses");
    prior.tracks.push(track);
    prior.duration_tk = Tick::new(480_000);

    let args = effect_reorder::EffectReorderArgs {
        project_id,
        effect: "01900000-0000-7000-8000-0000000cc211".to_string(),
        to_index: effect_reorder::ToIndex::Integer(1),
    };

    let (patch_value, warnings, _data) = effect_reorder::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid effect.reorder patch");
    let patch: json_patch::Patch = serde_json::from_value(patch_value.clone())
        .expect("effect.reorder fixture patch is valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("effect.reorder fixture patch must apply cleanly");

    let expected_data = serde_json::to_value(
        effect_reorder::data_envelope_from_patch_warnings_post_state(
            &args,
            &patch_value,
            &warnings,
            &post_state,
        )
        .expect("effect.reorder fixture expected_data"),
    )
    .expect("effect.reorder fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "effect.reorder".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings,
        post_state,
        expected_data,
    }
}

/// Build the canonical `effect.set_param` fixture used by [`default_fixtures`].
///
/// Starts from a synthetic project with one clip carrying one unmanaged effect,
/// then merges a second params key.
fn effect_set_param_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let mut prior = synthetic_empty_project(project_id);

    let track_raw = json!({
        "id": "01900000-0000-7000-8000-0000000aa221",
        "kind": "text",
        "name": "Text 1",
        "locked": false,
        "clips": [{
            "id": "01900000-0000-7000-8000-0000000bb221",
            "name": "Clip 1",
            "asset_id": "00000000-0000-0000-0000-000000000000",
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 480_000,
            "locked": false,
            "text": {
                "content": "Hello",
                "font_family": "Arial",
                "font_size_px": 24,
            },
            "effects": [{
                "id": "01900000-0000-7000-8000-0000000cc221",
                "kind": "color_correct",
                "enabled": true,
                "params": { "brightness": 0.1 },
            }],
        }],
    });

    let track: crate::track::Track =
        serde_json::from_value(track_raw).expect("manual track fixture parses");
    prior.tracks.push(track);
    prior.duration_tk = Tick::new(480_000);

    let mut params = Map::new();
    params.insert("contrast".to_string(), json!(1.25));

    let args = effect_set_param::EffectSetParamArgs {
        project_id,
        effect: "01900000-0000-7000-8000-0000000cc221".to_string(),
        params,
    };

    let (patch_value, warnings, _data) = effect_set_param::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid effect.set_param patch");
    let patch: json_patch::Patch = serde_json::from_value(patch_value.clone())
        .expect("effect.set_param fixture patch is valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("effect.set_param fixture patch must apply cleanly");

    let expected_data = serde_json::to_value(
        effect_set_param::data_envelope_from_post_state(&args, &post_state)
            .expect("effect.set_param fixture expected_data"),
    )
    .expect("effect.set_param fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "effect.set_param".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings,
        post_state,
        expected_data,
    }
}

/// Build the canonical `asset.list` fixture used by [`default_fixtures`].
///
/// Starts from an empty synthetic project and injects two assets (audio
/// and video), then computes the deterministic read-only envelope.
/// Build the canonical `asset.import` fixture used by
/// [`default_fixtures`].
///
/// The only success path at v1 is the empty-batch no-op; the fixture
/// records that case so the §0.8 reconstructor gate has a meaningful
/// `(args, post_state) → expected_data` pair to verify.
fn asset_import_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = asset_import::AssetImportArgs {
        project_id,
        paths: Vec::new(),
        mode: None,
        soft: None,
    };

    let (patch_value, _warnings, data) = asset_import::compute_patch(&prior, &args)
        .expect("default fixture must produce valid asset.import data (empty-paths no-op)");
    let expected_data = serde_json::to_value(&data)
        .expect("asset.import fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "asset.import".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings: vec![],
        post_state: prior,
        expected_data,
    }
}

fn asset_list_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let mut prior = synthetic_empty_project(project_id);

    prior.assets.push(
        serde_json::from_value(json!({
                "id": "01900000-0000-7000-8000-00000000aaaa",
                "kind": "video",
                "hash": "53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658",
                "path": "assets/53/53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.mp4",
                "original_filename": "video.mp4",
                "imported_at": "2026-05-01T00:00:00Z",
                "metadata": {
                    "duration_tk": 480_000,
                "width": 1920,
                "height": 1080,
                "fps_num": 30,
                "fps_den": 1,
                "video_codec": "h264",
                "container": "mp4",
                "fingerprint": { "mtime_ms": 1_700_000_000_000_i64, "size_bytes": 1024 }
            }
        }))
        .expect("video fixture asset parses"),
    );
    prior.assets.push(
        serde_json::from_value(json!({
                "id": "01900000-0000-7000-8000-00000000bbbb",
                "kind": "audio",
                "hash": "36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da",
                "path": "assets/36/36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da.mp3",
            "original_filename": "audio.mp3",
            "imported_at": "2026-04-01T00:00:00Z",
            "metadata": {
                "duration_tk": 240_000,
                "audio_codec": "mp3",
                "audio_channels": 2,
                "audio_sample_rate_hz": 48000,
                "container": "mp3",
                "fingerprint": { "mtime_ms": 1_700_000_000_001_i64, "size_bytes": 2048 }
            }
        }))
        .expect("audio fixture asset parses"),
    );

    let args = asset_list::AssetListArgs {
        project_id,
        kind: None,
    };

    let (patch_value, _warnings, data) = asset_list::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid asset.list data");
    let expected_data =
        serde_json::to_value(&data).expect("asset.list fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "asset.list".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings: vec![],
        post_state: prior,
        expected_data,
    }
}

/// Build the canonical `asset.remove` fixture used by [`default_fixtures`].
///
/// Starts from a synthetic project with one orphan video asset (no clip
/// references it) and exercises the non-cascade happy path. The asset
/// record is removed; the bytes are deliberately left on disk per spec
/// §3.4. The reconstructor reads back the `W_ASSET_REMOVE_ENVELOPE`
/// warning to rebuild the data envelope, mirroring `track.remove` and
/// `clip.delete`.
fn asset_remove_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let mut prior = synthetic_empty_project(project_id);
    let asset_id = "01900000-0000-7000-8000-0000000cc801";

    prior.assets.push(
        serde_json::from_value(json!({
            "id": asset_id,
            "kind": "video",
            "hash": "53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658",
            "path": "assets/53/53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.mp4",
            "original_filename": "asset-remove.mp4",
            "imported_at": "2026-05-24T00:00:00Z",
            "metadata": {
                "duration_tk": 240_000,
                "width": 1920,
                "height": 1080,
                "fps_num": 30,
                "fps_den": 1,
                "video_codec": "h264",
                "container": "mp4",
                "fingerprint": {
                    "mtime_ms": 1_700_000_000_000_i64,
                    "size_bytes": 1024,
                }
            }
        }))
        .expect("asset.remove fixture asset parses"),
    );

    let args = asset_remove::AssetRemoveArgs {
        project_id,
        asset_id: asset_id.to_string(),
        cascade: None,
    };

    let (patch_value, warnings, _data) = asset_remove::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid asset.remove patch");
    let patch: json_patch::Patch = serde_json::from_value(patch_value.clone())
        .expect("asset.remove fixture patch is valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("asset.remove fixture patch must apply cleanly");

    let expected_data = serde_json::to_value(
        asset_remove::data_envelope_from_args_warnings(&args, &warnings)
            .expect("asset.remove fixture expected_data"),
    )
    .expect("asset.remove fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "asset.remove".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings,
        post_state,
        expected_data,
    }
}

/// Build the canonical `audio.extract` fixture used by
/// [`default_fixtures`].
///
/// `audio.extract` always errors with `E_IO` in the v1 floor because
/// extraction needs storage/codec context outside pure verb execution.
/// No successful event can be recorded, so the reconstructor only checks
/// args deserialization and returns `Value::Null`. The fixture records
/// `expected_data: null` so the startup gate's canonical equality holds.
fn audio_extract_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = audio_extract::AudioExtractArgs {
        project_id,
        clip: "01900000-0000-7000-8000-0000000bb900".to_string(),
        to_track: Some("track:01900000-0000-7000-8000-0000000aa900".to_string()),
        codec: Some(audio_extract::AudioExtractCodec::Aac),
    };

    RecordedEvent {
        verb: "audio.extract".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: json!([]),
        warnings: vec![],
        post_state: prior,
        expected_data: Value::Null,
    }
}

/// Build the canonical `audio.detect_beats` fixture used by
/// [`default_fixtures`].
///
/// `audio.analyze` always errors with `E_ANALYSIS_FAILED` for accepted
/// targets in the v1 floor because analysis runtime/cache context is
/// intentionally deferred. No successful event can be recorded, so the
/// reconstructor only checks args deserialization and returns
/// `Value::Null`. The fixture records `expected_data: null` so the
/// startup gate's canonical equality holds.
fn audio_analyze_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = audio_analyze::AudioAnalyzeArgs {
        project_id,
        target: "clip:01900000-0000-7000-8000-0000000bb910".to_string(),
        features: Some(vec![
            "tempo".to_string(),
            "sections".to_string(),
            "tempo".to_string(),
        ]),
        from_tk: None,
        to_tk: None,
    };

    RecordedEvent {
        verb: "audio.analyze".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: json!([]),
        warnings: vec![],
        post_state: prior,
        expected_data: Value::Null,
    }
}

/// Build the canonical `audio.detect_beats` fixture used by
/// [`default_fixtures`].
///
/// `audio.detect_beats` always errors with `E_ANALYSIS_FAILED` for
/// accepted targets in the v1 floor because analysis runtime/cache
/// context is intentionally deferred. No successful event can be
/// recorded, so the reconstructor only checks args deserialization and
/// returns `Value::Null`. The fixture records `expected_data: null` so
/// the startup gate's canonical equality holds.
fn audio_detect_beats_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = audio_detect_beats::AudioDetectBeatsArgs {
        project_id,
        target: "clip:01900000-0000-7000-8000-0000000bb910".to_string(),
        algorithm: Some("onset".to_string()),
        min_confidence: Some(0.5),
        create_markers: Some(true),
        from_tk: None,
        to_tk: None,
    };

    RecordedEvent {
        verb: "audio.detect_beats".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: json!([]),
        warnings: vec![],
        post_state: prior,
        expected_data: Value::Null,
    }
}

/// Build the canonical `audio.detect_silence` fixture used by
/// [`default_fixtures`].
///
/// `audio.detect_silence` always errors with `E_ANALYSIS_FAILED` for
/// accepted targets in the v1 floor because analysis runtime/cache
/// context is intentionally deferred. No successful event can be
/// recorded, so the reconstructor only checks args deserialization and
/// returns `Value::Null`. The fixture records `expected_data: null` so
/// the startup gate's canonical equality holds.
fn audio_detect_silence_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = audio_detect_silence::AudioDetectSilenceArgs {
        project_id,
        target: "clip:01900000-0000-7000-8000-0000000bb910".to_string(),
        min_silence_tk: Some(120_000),
        threshold_db: Some(-40.0),
        from_tk: None,
        to_tk: None,
    };

    RecordedEvent {
        verb: "audio.detect_silence".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: json!([]),
        warnings: vec![],
        post_state: prior,
        expected_data: Value::Null,
    }
}

/// Build the canonical `audio.denoise` fixture used by [`default_fixtures`].
///
/// Starts from a synthetic project with a single audio clip that already
/// carries the managed denoise effect, then updates its strength in place.
fn audio_denoise_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let mut prior = synthetic_empty_project(project_id);
    let asset_id = "01900000-0000-7000-8000-0000000cc904";
    let clip_id = "01900000-0000-7000-8000-0000000bb904";
    let effect_id = "01900000-0000-7000-8000-0000000dd904";

    prior.assets.push(
        serde_json::from_value(json!({
            "id": asset_id,
            "kind": "audio",
            "hash": "53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658",
            "path": "assets/53/53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.m4a",
            "original_filename": "audio-denoise.m4a",
            "imported_at": "2026-05-24T00:00:00Z",
            "metadata": {
                "duration_tk": 480_000,
                "audio_codec": "aac",
                "audio_channels": 2,
                "audio_sample_rate_hz": 48_000,
                "container": "m4a",
                "fingerprint": {
                    "mtime_ms": 1_700_000_000_000_i64,
                    "size_bytes": 1024,
                }
            }
        }))
        .expect("audio.denoise fixture asset parses"),
    );

    let track_raw = json!({
        "id": "01900000-0000-7000-8000-0000000aa904",
        "kind": "audio",
        "name": "Audio 12",
        "locked": false,
        "clips": [{
            "id": clip_id,
            "name": "Denoise Clip",
            "asset_id": asset_id,
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 480_000,
            "locked": false,
            "effects": [{
                "id": effect_id,
                "kind": "denoise",
                "enabled": true,
                "params": {
                    "strength": 0.5,
                }
            }],
        }],
    });

    let track: crate::track::Track =
        serde_json::from_value(track_raw).expect("manual track fixture parses");
    prior.tracks.push(track);
    prior.duration_tk = Tick::new(480_000);

    let args = audio_denoise::AudioDenoiseArgs {
        project_id,
        target: format!("clip:{clip_id}"),
        strength: Some(0.25),
    };

    let (patch_value, warnings, _data) = audio_denoise::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid audio.denoise patch");
    let patch: json_patch::Patch = serde_json::from_value(patch_value.clone())
        .expect("audio.denoise fixture patch must be valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("audio.denoise fixture patch must apply cleanly");

    let expected_data = serde_json::to_value(
        audio_denoise::data_envelope_from_args_warnings_post_state(&args, &warnings, &post_state)
            .expect("audio.denoise fixture expected_data"),
    )
    .expect("audio.denoise fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "audio.denoise".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings,
        post_state,
        expected_data,
    }
}

/// Build the canonical `audio.fade` fixture used by [`default_fixtures`].
///
/// Starts from a synthetic project with a single audio track holding a
/// single audio clip, then sets both fade durations and curves.
fn audio_fade_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let mut prior = synthetic_empty_project(project_id);
    let asset_id = "01900000-0000-7000-8000-0000000cc901";
    let clip_id = "01900000-0000-7000-8000-0000000bb901";

    prior.assets.push(
        serde_json::from_value(json!({
            "id": asset_id,
            "kind": "audio",
            "hash": "53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658",
            "path": "assets/53/53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.m4a",
            "original_filename": "audio-fade.m4a",
            "imported_at": "2026-05-24T00:00:00Z",
            "metadata": {
                "duration_tk": 480_000,
                "audio_codec": "aac",
                "audio_channels": 2,
                "audio_sample_rate_hz": 48_000,
                "container": "m4a",
                "fingerprint": {
                    "mtime_ms": 1_700_000_000_000_i64,
                    "size_bytes": 1024,
                }
            }
        }))
        .expect("audio.fade fixture asset parses"),
    );

    let track_raw = json!({
        "id": "01900000-0000-7000-8000-0000000aa901",
        "kind": "audio",
        "name": "Audio 9",
        "locked": false,
        "clips": [{
            "id": clip_id,
            "name": "Fade Clip",
            "asset_id": asset_id,
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 480_000,
            "locked": false,
        }],
    });

    let track: crate::track::Track =
        serde_json::from_value(track_raw).expect("manual track fixture parses");
    prior.tracks.push(track);
    prior.duration_tk = Tick::new(480_000);

    let args = audio_fade::AudioFadeArgs {
        project_id,
        clip: clip_id.to_string(),
        fade_in_tk: Some(8_001),
        fade_out_tk: Some(16_001),
        curve: None,
        curve_in: Some(crate::clip::FadeCurve::Exp),
        curve_out: Some(crate::clip::FadeCurve::Log),
    };

    let (patch_value, warnings, _data) = audio_fade::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid audio.fade patch");
    let patch: json_patch::Patch = serde_json::from_value(patch_value.clone())
        .expect("audio.fade fixture patch must be valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("audio.fade fixture patch must apply cleanly");

    let expected_data = serde_json::to_value(
        audio_fade::data_envelope_from_post_state(&args, &post_state)
            .expect("audio.fade fixture expected_data"),
    )
    .expect("audio.fade fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "audio.fade".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings,
        post_state,
        expected_data,
    }
}

/// Build the canonical `audio.volume` fixture used by [`default_fixtures`].
///
/// Starts from a synthetic project with one audio track and one audio
/// clip, then exercises the `clip:` dispatch branch with an explicit
/// `gain` value so reconstruction reads the post-state volume.
fn audio_volume_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let mut prior = synthetic_empty_project(project_id);
    let asset_id = "01900000-0000-7000-8000-0000000cc902";
    let clip_id = "01900000-0000-7000-8000-0000000bb902";

    prior.assets.push(
        serde_json::from_value(json!({
            "id": asset_id,
            "kind": "audio",
            "hash": "53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658",
            "path": "assets/53/53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.m4a",
            "original_filename": "audio-volume.m4a",
            "imported_at": "2026-05-24T00:00:00Z",
            "metadata": {
                "duration_tk": 480_000,
                "audio_codec": "aac",
                "audio_channels": 2,
                "audio_sample_rate_hz": 48_000,
                "container": "m4a",
                "fingerprint": {
                    "mtime_ms": 1_700_000_000_000_i64,
                    "size_bytes": 1024,
                }
            }
        }))
        .expect("audio.volume fixture asset parses"),
    );

    let track_raw = json!({
        "id": "01900000-0000-7000-8000-0000000aa902",
        "kind": "audio",
        "name": "Audio 10",
        "locked": false,
        "clips": [{
            "id": clip_id,
            "name": "Volume Clip",
            "asset_id": asset_id,
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 480_000,
            "locked": false,
        }],
    });

    let track: crate::track::Track =
        serde_json::from_value(track_raw).expect("manual track fixture parses");
    prior.tracks.push(track);
    prior.duration_tk = Tick::new(480_000);

    let args = audio_volume::AudioVolumeArgs {
        project_id,
        target: format!("clip:{clip_id}"),
        gain: Some(0.75),
        db: None,
    };

    let (patch_value, warnings, _data) = audio_volume::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid audio.volume patch");
    let patch: json_patch::Patch = serde_json::from_value(patch_value.clone())
        .expect("audio.volume fixture patch must be valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("audio.volume fixture patch must apply cleanly");

    let expected_data = serde_json::to_value(
        audio_volume::data_envelope_from_post_state(&args, &post_state)
            .expect("audio.volume fixture expected_data"),
    )
    .expect("audio.volume fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "audio.volume".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings,
        post_state,
        expected_data,
    }
}

/// Build the canonical `project.info` fixture used by [`default_fixtures`].
///
/// Starts from the synthetic empty project (no tracks, no assets) and
/// exercises the read-only summary envelope. The reconstructor must
/// rebuild the same envelope from `(args, post_state)` alone — both
/// deferred fields (`path = ""`, `event_count = 0`) are emitted as
/// constants so the round-trip is trivially pure.
fn project_info_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = project_info::ProjectInfoArgs { project_id };

    let (patch_value, _warnings, data) = project_info::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid project.info data");
    let expected_data = serde_json::to_value(&data)
        .expect("project.info fixture expected_data serializes to Value");

    // Assert the envelope round-trips through data_envelope_from_post_state
    // — this mirrors the asset_list_fixture shape but for project.info.
    let round_trip = project_info::data_envelope_from_post_state(&args, &prior)
        .expect("project.info round-trip via data_envelope_from_post_state");
    assert_eq!(
        data, round_trip,
        "project.info fixture envelope must round-trip through reconstructor helper"
    );

    RecordedEvent {
        verb: "project.info".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings: vec![],
        post_state: prior,
        expected_data,
    }
}

/// Build the canonical `describe` fixture used by [`default_fixtures`].
///
/// Starts from a synthetic project with one audio asset and exercises
/// the `asset:<UUID>` lookup branch — the simplest happy path that
/// doesn't require constructing a clip + track + asset chain. The
/// reconstructor must rebuild the same `{kind, entity}` envelope from
/// `(args, post_state)` alone; for a read-only verb the patch is `[]`
/// and the post-state equals the pre-state.
fn describe_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let mut prior = synthetic_empty_project(project_id);
    let asset_id = "01900000-0000-7000-8000-0000000cca01";

    prior.assets.push(
        serde_json::from_value(json!({
            "id": asset_id,
            "kind": "audio",
            "hash": "53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658",
            "path": "assets/53/53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.m4a",
            "original_filename": "describe.m4a",
            "imported_at": "2026-05-24T00:00:00Z",
            "metadata": {
                "duration_tk": 240_000,
                "audio_codec": "aac",
                "audio_channels": 2,
                "audio_sample_rate_hz": 48_000,
                "container": "m4a",
                "fingerprint": {
                    "mtime_ms": 1_700_000_000_000_i64,
                    "size_bytes": 512,
                }
            }
        }))
        .expect("describe fixture asset parses"),
    );

    let args = describe::DescribeArgs {
        project_id,
        target: format!("asset:{asset_id}"),
    };

    let (patch_value, _warnings, data) = describe::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid describe data");
    let expected_data =
        serde_json::to_value(&data).expect("describe fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "describe".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings: vec![],
        post_state: prior,
        expected_data,
    }
}

/// Build the canonical `timeline.snapshot` fixture used by
/// [`default_fixtures`].
///
/// Starts from the synthetic empty project with a non-`None`
/// `last_saved_event_id` so the fixture exercises the "saved project"
/// branch — `event_id == last_saved_event_id.to_string()` — rather than
/// the `"empty"` sentinel branch. The reconstructor must rebuild the
/// same envelope from `(args, post_state)` alone; for a read-only verb
/// the patch is `[]` and the post-state equals the pre-state, so the
/// round-trip is trivially pure.
fn timeline_snapshot_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let mut prior = synthetic_empty_project(project_id);
    let saved_event_id: verbreel_types::EventId = "0190b8d3-15e3-7000-bd00-0000e0e0aa01"
        .parse()
        .expect("hard-coded fixture event id is a valid v7");
    prior.last_saved_event_id = Some(saved_event_id);

    let args = timeline_snapshot::TimelineSnapshotArgs { project_id };

    let (patch_value, _warnings, data) = timeline_snapshot::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid timeline.snapshot data");
    let expected_data = serde_json::to_value(&data)
        .expect("timeline.snapshot fixture expected_data serializes to Value");

    // Sanity: the fixture exercises the saved-project branch.
    assert_eq!(
        data.event_id,
        saved_event_id.to_string(),
        "timeline.snapshot fixture must report event_id == last_saved_event_id"
    );

    // Round-trip via the reconstructor helper to mirror project.info's
    // fixture-builder shape.
    let round_trip = timeline_snapshot::data_envelope_from_post_state(&args, &prior)
        .expect("timeline.snapshot round-trip via data_envelope_from_post_state");
    assert_eq!(
        data, round_trip,
        "timeline.snapshot fixture envelope must round-trip through reconstructor helper"
    );

    RecordedEvent {
        verb: "timeline.snapshot".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings: vec![],
        post_state: prior,
        expected_data,
    }
}

/// Build the canonical `timeline.diff` fixture used by
/// [`default_fixtures`].
///
/// `timeline.diff` always errors with `E_EVENT_NOT_FOUND` in this v1
/// floor (event-log range traversal is deferred until read-surface
/// context is threaded beyond `&Project`). No successful event is
/// recorded, so `reconstruct()` deserializes args and returns
/// `Value::Null`; the fixture records `expected_data: null` to satisfy
/// the §0.8 gate.
fn timeline_diff_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = timeline_diff::TimelineDiffArgs {
        project_id,
        since: "empty".to_string(),
        until: None,
    };

    RecordedEvent {
        verb: "timeline.diff".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: json!([]),
        warnings: vec![],
        post_state: prior,
        expected_data: Value::Null,
    }
}

/// Build the canonical `timeline.undo` fixture used by
/// [`default_fixtures`].
///
/// `timeline.undo` always errors with `E_NOTHING_TO_UNDO` for
/// well-formed args in this v1 floor. No successful event is recorded,
/// so `reconstruct()` deserializes args and returns `Value::Null`; the
/// fixture records `expected_data: null` for the §0.8 gate.
fn timeline_undo_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = timeline_undo::TimelineUndoArgs {
        project_id,
        steps: None,
    };

    RecordedEvent {
        verb: "timeline.undo".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: json!([]),
        warnings: vec![],
        post_state: prior,
        expected_data: Value::Null,
    }
}

/// Build the canonical `timeline.redo` fixture used by
/// [`default_fixtures`].
///
/// `timeline.redo` always errors with `E_NOTHING_TO_REDO` for
/// well-formed args in this v1 floor. No successful event is recorded,
/// so `reconstruct()` deserializes args and returns `Value::Null`; the
/// fixture records `expected_data: null` for the §0.8 gate.
fn timeline_redo_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = timeline_redo::TimelineRedoArgs {
        project_id,
        steps: None,
    };

    RecordedEvent {
        verb: "timeline.redo".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: json!([]),
        warnings: vec![],
        post_state: prior,
        expected_data: Value::Null,
    }
}

/// Build the canonical `timeline.history` fixture used by
/// [`default_fixtures`].
///
/// `timeline.history` is a read-only v1 floor and always succeeds for
/// well-formed args with `patch: []`, no warnings, and
/// `data: { events: [] }`.
fn timeline_history_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = timeline_history::TimelineHistoryArgs {
        project_id,
        limit: None,
        since: None,
        include_undone: None,
    };

    let expected_data =
        serde_json::to_value(timeline_history::TimelineHistoryData { events: Vec::new() })
            .expect("timeline.history fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "timeline.history".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: json!([]),
        warnings: vec![],
        post_state: prior,
        expected_data,
    }
}

/// Build the canonical `keyframe.add` fixture used by [`default_fixtures`].
///
/// Starts from a synthetic project with one video clip, then appends one
/// opacity keyframe at the beginning of the clip.
fn keyframe_add_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let mut prior = synthetic_empty_project(project_id);

    let clip_id = "01900000-0000-7000-8000-0000000bb704";
    let track_raw = json!({
        "id": "01900000-0000-7000-8000-0000000aa704",
        "kind": "video",
        "name": "Video 4",
        "locked": false,
        "clips": [{
            "id": clip_id,
            "name": "Clip 4",
            "asset_id": "01900000-0000-7000-8000-0000000cc704",
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 480_000,
            "locked": false,
        }],
    });
    let track: crate::track::Track =
        serde_json::from_value(track_raw).expect("manual track fixture parses");
    prior.tracks.push(track);
    prior.duration_tk = Tick::new(480_000);
    prior.assets.push(
        serde_json::from_value(json!({
            "id": "01900000-0000-7000-8000-0000000cc704",
            "kind": "video",
            "hash": "53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658",
            "path": "assets/53/53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.mp4",
            "original_filename": "video-keyframe-add.mp4",
            "imported_at": "2026-05-24T00:00:00Z",
            "metadata": {
                "duration_tk": 480_000,
                "width": 1920,
                "height": 1080,
                "fps_num": 30,
                "fps_den": 1,
                "video_codec": "h264",
                "container": "mp4",
                "fingerprint": {
                    "mtime_ms": 1_700_000_000_000_i64,
                    "size_bytes": 1024,
                }
            }
        }))
        .expect("keyframe.add fixture asset parses"),
    );

    let args = keyframe_add::KeyframeAddArgs {
        project_id,
        clip: clip_id.to_string(),
        property: "opacity".to_string(),
        time_tk: 0,
        value: json!(0.5),
        easing: Some("linear".to_string()),
        bezier: None,
    };

    let (patch_value, _warnings, _data) = keyframe_add::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid keyframe.add patch");
    let patch: json_patch::Patch = serde_json::from_value(patch_value.clone())
        .expect("keyframe.add fixture patch is valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("keyframe.add fixture patch must apply cleanly");

    let expected_data = serde_json::to_value(
        keyframe_add::data_envelope_from_args_and_patch(&args, &patch_value)
            .expect("keyframe.add fixture expected_data"),
    )
    .expect("keyframe.add fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "keyframe.add".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings: vec![],
        post_state,
        expected_data,
    }
}

/// Build the canonical `keyframe.list` fixture used by [`default_fixtures`].
///
/// Starts from a synthetic project with one text clip and three keyframes
/// across two properties so the sort path is exercised.
fn keyframe_list_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let mut prior = synthetic_empty_project(project_id);

    let track_raw = json!({
        "id": "01900000-0000-7000-8000-0000000aa701",
        "kind": "text",
        "name": "Text 1",
        "clips": [{
            "id": "01900000-0000-7000-8000-0000000bb701",
            "name": "Clip 1",
            "asset_id": "00000000-0000-0000-0000-000000000000",
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 480_000,
            "locked": false,
            "text": {
                "content": "Hello",
                "font_family": "Arial",
                "font_size_px": 24,
            },
            "keyframes": [
                { "id": "01900000-0000-7000-8000-0000000cc701", "property": "opacity", "time_tk": 500, "value": 0.5 },
                { "id": "01900000-0000-7000-8000-0000000cc702", "property": "opacity", "time_tk": 250, "value": 1.0 },
                { "id": "01900000-0000-7000-8000-0000000cc703", "property": "transform.x", "time_tk": 250, "value": 12.0 },
            ],
        }],
    });
    let track: crate::track::Track =
        serde_json::from_value(track_raw).expect("manual track fixture parses");
    prior.tracks.push(track);

    let args = keyframe_list::KeyframeListArgs {
        project_id,
        clip: "01900000-0000-7000-8000-0000000bb701".to_string(),
        property: None,
    };

    let (patch_value, _warnings, _data) = keyframe_list::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid keyframe.list data");
    let expected_data = serde_json::to_value(
        keyframe_list::data_envelope_from_post_state(&args, &prior)
            .expect("keyframe.list fixture expected_data"),
    )
    .expect("keyframe.list fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "keyframe.list".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings: vec![],
        post_state: prior,
        expected_data,
    }
}

/// Build the canonical `keyframe.set` fixture used by [`default_fixtures`].
///
/// Starts from the `keyframe.add` fixture's post-state and updates the
/// opacity keyframe value from `0.5` to `0.8`.
fn keyframe_set_fixture() -> RecordedEvent {
    let add_fixture = keyframe_add_fixture();
    let prior = add_fixture.post_state;
    let keyframe_id = prior.tracks[0].clips[0].keyframes[0].id;
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let args = keyframe_set::KeyframeSetArgs {
        project_id,
        keyframe: keyframe_id.to_string(),
        time_tk: None,
        value: Some(json!(0.8)),
        easing: None,
        bezier: None,
    };

    let (patch_value, warnings, _data) = keyframe_set::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid keyframe.set patch");
    let patch: json_patch::Patch = serde_json::from_value(patch_value.clone())
        .expect("keyframe.set fixture patch is valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("keyframe.set fixture patch must apply cleanly");

    let expected_data = serde_json::to_value(
        keyframe_set::data_envelope_from_post_state(&args, &post_state)
            .expect("keyframe.set fixture expected_data"),
    )
    .expect("keyframe.set fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "keyframe.set".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings,
        post_state,
        expected_data,
    }
}

/// Build the canonical `keyframe.remove` fixture used by
/// [`default_fixtures`].
///
/// Starts from a synthetic project with two opacity keyframes on one
/// clip, then removes one of them.
fn keyframe_remove_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let mut prior = synthetic_empty_project(project_id);

    let track_raw = json!({
        "id": "01900000-0000-7000-8000-0000000aa705",
        "kind": "video",
        "name": "Video 5",
        "locked": false,
        "clips": [{
            "id": "01900000-0000-7000-8000-0000000bb705",
            "name": "Clip 5",
            "asset_id": "01900000-0000-7000-8000-0000000cc705",
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 480_000,
            "locked": false,
            "keyframes": [
                { "id": "01900000-0000-7000-8000-0000000ff705", "property": "opacity", "time_tk": 0, "value": 1.0 },
                { "id": "01900000-0000-7000-8000-0000000ff706", "property": "opacity", "time_tk": 100, "value": 0.5 },
            ],
        }],
    });
    let track: crate::track::Track =
        serde_json::from_value(track_raw).expect("manual track fixture parses");
    prior.tracks.push(track);
    prior.duration_tk = Tick::new(480_000);
    prior.assets.push(
        serde_json::from_value(json!({
            "id": "01900000-0000-7000-8000-0000000cc705",
            "kind": "video",
            "hash": "53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658",
            "path": "assets/53/53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.mp4",
            "original_filename": "video-keyframe-remove.mp4",
            "imported_at": "2026-05-24T00:00:00Z",
            "metadata": {
                "duration_tk": 480_000,
                "width": 1920,
                "height": 1080,
                "fps_num": 30,
                "fps_den": 1,
                "video_codec": "h264",
                "container": "mp4",
                "fingerprint": {
                    "mtime_ms": 1_700_000_000_000_i64,
                    "size_bytes": 1024,
                }
            }
        }))
        .expect("keyframe.remove fixture asset parses"),
    );

    let args = keyframe_remove::KeyframeRemoveArgs {
        project_id,
        keyframes: vec!["01900000-0000-7000-8000-0000000ff705".to_string()],
        soft: None,
    };

    let (patch_value, warnings, data) = keyframe_remove::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid keyframe.remove patch");
    let patch: json_patch::Patch = serde_json::from_value(patch_value.clone())
        .expect("keyframe.remove fixture patch is valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("keyframe.remove fixture patch must apply cleanly");
    let expected_data = serde_json::to_value(data).expect("keyframe.remove fixture expected_data");

    RecordedEvent {
        verb: "keyframe.remove".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings,
        post_state,
        expected_data,
    }
}

/// Build the canonical `track.reorder` fixture used by [`default_fixtures`].
///
/// Starts from an empty synthetic project, then adds two video tracks.
/// Reorders the second track to kind index `0`.
fn track_reorder_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let mut prior = synthetic_empty_project(project_id);

    for _ in 0..2 {
        let add_args = track_add::TrackAddArgs {
            project_id,
            kind: crate::track::TrackKind::Video,
            name: None,
            index: None,
        };

        let (add_patch_val, _warnings) = track_add::compute_patch(&prior, &add_args)
            .expect("track.add fixture should produce a valid patch");
        let add_patch: json_patch::Patch =
            serde_json::from_value(add_patch_val).expect("track.add fixture patch parses");
        prior = prior
            .apply(&add_patch)
            .expect("track.add fixture patch applies cleanly");
    }

    let track_id = prior
        .tracks
        .get(1)
        .expect("track.reorder fixture should have two tracks");

    let args = track_reorder::TrackReorderArgs {
        project_id,
        track: track_id.id.to_string(),
        to_index: 0,
    };

    let (patch_value, _warnings, _data) = track_reorder::compute_patch(&prior, &args)
        .expect("track.reorder fixture should produce a valid patch");
    let patch: json_patch::Patch = serde_json::from_value(patch_value.clone())
        .expect("track.reorder fixture patch is valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("track.reorder fixture patch applies cleanly");

    let expected_data = serde_json::to_value(
        track_reorder::data_envelope_from_patch_and_post_state(&patch_value, &args, &post_state)
            .expect("track.reorder fixture expected_data"),
    )
    .expect("track.reorder fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "track.reorder".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings: vec![],
        post_state,
        expected_data,
    }
}

/// Build the canonical `project.set_metadata` fixture used by
/// [`default_fixtures`].
///
/// Exercises the shallow-merge happy path: prior state with empty
/// metadata, args adding a single `author` key, post-state holds the
/// merged result. The reconstructor only reads `args.project_id` and
/// `post_state.metadata` so this is the minimum-surface-area fixture
/// that still proves the round-trip.
fn project_set_metadata_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let mut metadata = Map::new();
    metadata.insert("author".to_string(), Value::String("alice".to_string()));
    let args = project_set_metadata::ProjectSetMetadataArgs {
        project_id,
        metadata: Some(metadata),
        replace: false,
        unset: None,
    };

    let (patch, new_metadata) = project_set_metadata::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid patch");

    let mut post_state = prior.clone();
    post_state.metadata = new_metadata;

    let expected_data =
        serde_json::to_value(project_set_metadata::data_envelope(&args, &post_state))
            .expect("ProjectSetMetadataData serializes to Value");

    RecordedEvent {
        verb: "project.set_metadata".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch,
        warnings: vec![],
        post_state,
        expected_data,
    }
}

/// Build the canonical `project.set_canvas` fixture used by
/// [`default_fixtures`].
///
/// Exercises the happy path with all four optional fields **omitted**
/// (so partial-update semantics are exercised — width/height update,
/// background and pixel-aspect stay at the prior defaults). The prior
/// project's portrait `1080x1920` canvas becomes the landscape
/// `1920x1080` canvas (background `#000000ff`, pixel aspect `1/1`
/// unchanged from the synthetic empty project). The reconstructor only
/// reads `args.project_id` and `post_state.canvas` so this is the
/// minimum-surface-area fixture that still proves the round-trip.
fn project_set_canvas_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = project_set_canvas::ProjectSetCanvasArgs {
        project_id,
        canvas: "1920x1080".to_string(),
        background: None,
        pixel_aspect_num: None,
        pixel_aspect_den: None,
    };

    let (patch, new_canvas, _warnings) = project_set_canvas::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid patch");

    let mut post_state = prior.clone();
    post_state.canvas = new_canvas;

    let expected_data = serde_json::to_value(project_set_canvas::data_envelope(&args, &post_state))
        .expect("ProjectSetCanvasData serializes to Value");

    RecordedEvent {
        verb: "project.set_canvas".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch,
        warnings: vec![],
        post_state,
        expected_data,
    }
}

/// Build the canonical `project.set_fps` fixture used by
/// [`default_fixtures`].
///
/// Exercises the happy path with `fps_den: Some(1)` (so the
/// two-op patch shape is exercised — the partial-update form has a
/// dedicated test in `tests/verb_project_set_fps.rs`). The prior
/// project is the synthetic empty one (`fps_num=30, fps_den=1`, no
/// tracks/clips/markers); args bump to `60/1`. Every off-frame
/// counter walks the empty graph and yields zero; the
/// `off_frame_entities` block is `None` (counts all zero rule).
fn project_set_fps_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = project_set_fps::ProjectSetFpsArgs {
        project_id,
        fps_num: 60,
        fps_den: Some(1),
        list_off_frame_entities: None,
    };

    let (patch, _counts, _entities) = project_set_fps::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid patch");

    let mut post_state = prior.clone();
    post_state.fps_num = args.fps_num;
    if let Some(d) = args.fps_den {
        post_state.fps_den = d;
    }

    let expected_data = serde_json::to_value(project_set_fps::data_envelope(&args, &post_state))
        .expect("ProjectSetFpsData serializes to Value");

    RecordedEvent {
        verb: "project.set_fps".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch,
        warnings: vec![],
        post_state,
        expected_data,
    }
}

/// Build the canonical `project.rename` fixture used by
/// [`default_fixtures`].
///
/// Exercises the minimum-surface-area happy path: prior state with
/// `name = "default-fixture"`, args set `name = "Renamed"`, post-state
/// holds the new name. The reconstructor only reads `args.project_id`
/// and `post_state.name`, so this is the narrowest fixture needed to
/// prove pure round-trip replay.
fn project_rename_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);
    let args = project_rename::ProjectRenameArgs {
        project_id,
        name: "Renamed".to_string(),
    };

    let (patch, new_name, _warnings) = project_rename::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid patch");

    let mut post_state = prior.clone();
    post_state.name = new_name;

    let expected_data = serde_json::to_value(project_rename::data_envelope(&args, &post_state))
        .expect("ProjectRenameData serializes to Value");

    RecordedEvent {
        verb: "project.rename".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch,
        warnings: vec![],
        post_state,
        expected_data,
    }
}

/// Build the canonical `marker.add` fixture used by
/// [`default_fixtures`].
///
/// This verb mints a fresh `MarkerId::now()` ID when computing the
/// patch. `default_fixtures()` must record exactly that ID once for
/// stable replay validation, so we compute the patch exactly once during
/// fixture construction and then apply it to the prior project to produce
/// the fixture's post-state.
fn marker_add_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);
    let args = marker_add::MarkerAddArgs {
        project_id,
        time_tk: 0,
        label: "Intro".to_string(),
        color: None,
        note: None,
    };

    let (patch_value, _warnings) = marker_add::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid patch");
    let patch: json_patch::Patch = serde_json::from_value(patch_value.clone())
        .expect("marker.add fixture patch must be valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("marker.add fixture patch must apply cleanly");

    let expected_data = serde_json::to_value(
        marker_add::data_envelope_from_patch(&patch_value)
            .expect("marker.add fixture expected_data"),
    )
    .expect("marker.add fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "marker.add".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings: vec![],
        post_state,
        expected_data,
    }
}

/// Build the canonical `marker.set` fixture used by
/// [`default_fixtures`].
///
/// Starts from the `marker.add` fixture's post-state (which has exactly one
/// marker), then applies `marker.set` to rename that marker.
fn marker_set_fixture() -> RecordedEvent {
    let fixture = marker_add_fixture();
    let prior = fixture.post_state;
    let marker_id = prior
        .markers
        .first()
        .expect("marker.add fixture has exactly one marker")
        .id
        .to_string();

    let args = marker_set::MarkerSetArgs {
        project_id: DEFAULT_FIXTURE_PROJECT_ID
            .parse()
            .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7"),
        marker: marker_id,
        time_tk: None,
        label: Some("Renamed Marker".to_string()),
        color: None,
        note: None,
    };

    let (patch_value, _warnings) = marker_set::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid patch");
    let patch: json_patch::Patch = serde_json::from_value(patch_value.clone())
        .expect("marker.set fixture patch must be valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("marker.set fixture patch must apply cleanly");

    let expected_data = serde_json::to_value(
        marker_set::data_envelope_from_post_state(&args, &post_state)
            .expect("marker.set fixture expected_data"),
    )
    .expect("marker.set fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "marker.set".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings: vec![],
        post_state,
        expected_data,
    }
}

/// Build the canonical `marker.remove` fixture used by
/// [`default_fixtures`].
///
/// Starts from the `marker.set` fixture's post-state (which has exactly
/// one marker), then applies `marker.remove` to delete that marker.
fn marker_remove_fixture() -> RecordedEvent {
    let fixture = marker_set_fixture();
    let prior = fixture.post_state;
    let marker_id = prior
        .markers
        .first()
        .expect("marker.set fixture has exactly one marker")
        .id
        .to_string();

    let args = marker_remove::MarkerRemoveArgs {
        project_id: prior.id,
        markers: vec![marker_id],
        soft: false,
    };

    let (patch_value, _warnings, data) = marker_remove::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid patch");
    let patch: json_patch::Patch = serde_json::from_value(patch_value.clone())
        .expect("marker.remove fixture patch must be valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("marker.remove fixture patch must apply cleanly");

    let expected_data = serde_json::to_value(data).expect("marker.remove fixture expected_data");

    RecordedEvent {
        verb: "marker.remove".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings: vec![],
        post_state,
        expected_data,
    }
}

/// Build the canonical `marker.list` fixture used by
/// [`default_fixtures`].
///
/// Starts from a synthetic project with two markers at distinct times so the
/// list sorting path is exercised before the project is used as post-state.
fn marker_list_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let mut prior = synthetic_empty_project(project_id);
    prior.markers.push(
        serde_json::from_value(json!({
            "id": "01900000-0000-7000-8000-000000000002",
            "time_tk": 1_000,
            "label": "Second",
            "color": "#ffaa00ff",
        }))
        .expect("marker fixture parses"),
    );
    prior.markers.push(
        serde_json::from_value(json!({
            "id": "01900000-0000-7000-8000-000000000001",
            "time_tk": 500,
            "label": "First",
            "color": "#ffaa00ff",
        }))
        .expect("marker fixture parses"),
    );

    let args = marker_list::MarkerListArgs { project_id };
    let (patch, _warnings) = marker_list::compute_patch(&prior, &args);
    let post_state = prior.clone();
    let expected_data = serde_json::to_value(marker_list::data_envelope(&post_state))
        .expect("marker.list fixture expected_data");

    RecordedEvent {
        verb: "marker.list".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch,
        warnings: vec![],
        post_state,
        expected_data,
    }
}

/// Build the canonical `clip.list` fixture used by [`default_fixtures`].
///
/// Starts from a synthetic project with two tracks and three clips in
/// intentionally unsorted insertion order, so the sort path is exercised.
fn clip_list_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let mut prior = synthetic_empty_project(project_id);

    let track_audio = serde_json::from_value(json!({
        "id": "01900000-0000-7000-8000-0000000aa101",
        "kind": "audio",
        "name": "Audio 1",
        "locked": false,
        "clips": [{
            "id": "01900000-0000-7000-8000-0000000bb201",
            "name": "Audio Clip A",
            "asset_id": "01900000-0000-7000-8000-0000000dd001",
            "track_position_tk": 2_500,
            "source_in_tk": 0,
            "source_out_tk": 1_000,
        }, {
            "id": "01900000-0000-7000-8000-0000000bb101",
            "name": "Audio Clip B",
            "asset_id": "01900000-0000-7000-8000-0000000dd001",
            "track_position_tk": 500,
            "source_in_tk": 0,
            "source_out_tk": 1_000,
        }],
    }))
    .expect("audio track for clip.list fixture parses");

    let track_video = serde_json::from_value(json!({
        "id": "01900000-0000-7000-8000-0000000aa201",
        "kind": "video",
        "name": "Video 1",
        "locked": false,
        "clips": [{
            "id": "01900000-0000-7000-8000-0000000bb301",
            "name": "Video Clip",
            "asset_id": "01900000-0000-7000-8000-0000000dd002",
            "track_position_tk": 1_500,
            "source_in_tk": 0,
            "source_out_tk": 1_000,
        }],
    }))
    .expect("video track for clip.list fixture parses");

    prior.tracks.push(track_audio);
    prior.tracks.push(track_video);
    prior.duration_tk = Tick::new(3_500);

    let args = clip_list::ClipListArgs {
        project_id,
        track: None,
        at_tk: None,
    };

    let (patch_value, _warnings, _data) = clip_list::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid clip.list patch");
    let post_state = prior;
    let expected_data = serde_json::to_value(
        clip_list::data_envelope_from_post_state(&args, &post_state)
            .expect("clip.list fixture expected_data"),
    )
    .expect("clip.list fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "clip.list".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings: vec![],
        post_state,
        expected_data,
    }
}

/// Build the canonical `track.add` fixture used by [`default_fixtures`].
///
/// Starts from a synthetic project with one video track named `Video 1`,
/// then inserts a second video track with auto-name `Video 2` at the end.
fn track_add_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let mut prior = synthetic_empty_project(project_id);
    let track_raw = json!({
        "id": "01900000-0000-7000-8000-0000000aa001",
        "kind": "video",
        "name": "Video 1",
        "clips": [],
    });
    let track: crate::track::Track =
        serde_json::from_value(track_raw).expect("manual track fixture parses");
    prior.tracks.push(track);

    let args = track_add::TrackAddArgs {
        project_id,
        kind: crate::track::TrackKind::Video,
        name: None,
        index: None,
    };

    let (patch_value, _warnings) = track_add::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid track.add patch");
    let patch: json_patch::Patch = serde_json::from_value(patch_value.clone())
        .expect("track.add fixture patch must be valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("track.add fixture patch must apply cleanly");

    let expected_data = serde_json::to_value(
        track_add::data_envelope_from_post_state(&patch_value, &post_state)
            .expect("track.add fixture expected_data"),
    )
    .expect("track.add fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "track.add".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings: vec![],
        post_state,
        expected_data,
    }
}

/// Build the canonical `track.rename` fixture used by [`default_fixtures`].
///
/// Starts from the `track_add_fixture()` post-state (which has video
/// tracks named `Video 1` and `Video 2`) and renames the first track to
/// `Main Camera`.
fn track_rename_fixture() -> RecordedEvent {
    let prior = track_add_fixture().post_state;
    let track = prior
        .tracks
        .first()
        .expect("track_add fixture has at least one track");

    let args = track_rename::TrackRenameArgs {
        project_id: DEFAULT_FIXTURE_PROJECT_ID
            .parse()
            .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7"),
        track: track.id.to_string(),
        name: "Main Camera".to_string(),
    };

    let (patch_value, _warnings, _data) = track_rename::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid track.rename patch");
    let patch: json_patch::Patch = serde_json::from_value(patch_value.clone())
        .expect("track.rename fixture patch must be valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("track.rename fixture patch must apply cleanly");

    let expected_data = serde_json::to_value(
        track_rename::data_envelope_from_post_state(&args, &post_state)
            .expect("track.rename fixture expected_data"),
    )
    .expect("track.rename fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "track.rename".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings: vec![],
        post_state,
        expected_data,
    }
}

/// Build the canonical `track.lock` fixture used by [`default_fixtures`].
///
/// Starts from the `track_add_fixture()` post-state and locks the first
/// video track.
fn track_lock_fixture() -> RecordedEvent {
    let prior = track_add_fixture().post_state;
    let track = prior
        .tracks
        .first()
        .expect("track_add fixture has at least one track");

    let args = track_lock::TrackLockArgs {
        project_id: DEFAULT_FIXTURE_PROJECT_ID
            .parse()
            .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7"),
        track: track.id.to_string(),
        locked: Some(true),
    };

    let (patch_value, _warnings, _data) = track_lock::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid track.lock patch");
    let patch: json_patch::Patch = serde_json::from_value(patch_value.clone())
        .expect("track.lock fixture patch must be valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("track.lock fixture patch must apply cleanly");

    let expected_data = serde_json::to_value(
        track_lock::data_envelope_from_post_state(&args, &post_state)
            .expect("track.lock fixture expected_data"),
    )
    .expect("track.lock fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "track.lock".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings: vec![],
        post_state,
        expected_data,
    }
}

/// Build the canonical `track.mute` fixture used by [`default_fixtures`].
///
/// Starts from the `track_add_fixture()` post-state and mutes the first
/// video track.
fn track_mute_fixture() -> RecordedEvent {
    let prior = track_add_fixture().post_state;
    let track = prior
        .tracks
        .first()
        .expect("track_add fixture has at least one track");

    let args = track_mute::TrackMuteArgs {
        project_id: DEFAULT_FIXTURE_PROJECT_ID
            .parse()
            .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7"),
        track: track.id.to_string(),
        muted: Some(true),
    };

    let (patch_value, _warnings, _data) = track_mute::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid track.mute patch");
    let patch: json_patch::Patch = serde_json::from_value(patch_value.clone())
        .expect("track.mute fixture patch must be valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("track.mute fixture patch must apply cleanly");

    let expected_data = serde_json::to_value(
        track_mute::data_envelope_from_post_state(&args, &post_state)
            .expect("track.mute fixture expected_data"),
    )
    .expect("track.mute fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "track.mute".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings: vec![],
        post_state,
        expected_data,
    }
}

/// Build the canonical `track.remove` fixture used by
/// [`default_fixtures`].
///
/// Starts from a synthetic project with two video tracks. The target
/// track has one clip with one opacity keyframe; the surviving track
/// has an equal-duration clip so the existing project duration remains
/// valid after removal.
#[allow(clippy::too_many_lines)]
fn track_remove_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let mut prior = synthetic_empty_project(project_id);
    let asset_id = "01900000-0000-7000-8000-0000000cc701";
    prior
        .assets
        .push(serde_json::from_value(json!({
            "id": asset_id,
            "kind": "video",
            "hash": "53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658",
            "path": "assets/53/53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.mp4",
            "original_filename": "track-remove.mp4",
            "imported_at": "2026-05-24T00:00:00Z",
            "metadata": {
                "duration_tk": 240_000,
                "width": 1920,
                "height": 1080,
                "fps_num": 30,
                "fps_den": 1,
                "video_codec": "h264",
                "container": "mp4",
                "fingerprint": {
                    "mtime_ms": 1_700_000_000_000_i64,
                    "size_bytes": 1024,
                }
            }
        }))
        .expect("track.remove fixture asset parses"));

    let target_track_id = "01900000-0000-7000-8000-0000000aa701";
    let survivor_track_id = "01900000-0000-7000-8000-0000000aa702";
    let target_clip_id = "01900000-0000-7000-8000-0000000bb701";
    let survivor_clip_id = "01900000-0000-7000-8000-0000000bb702";
    let keyframe_id = "01900000-0000-7000-8000-0000000ff701";

    let target_track = json!({
        "id": target_track_id,
        "kind": "video",
        "name": "Remove Me",
        "locked": false,
        "clips": [{
            "id": target_clip_id,
            "name": "Target Clip",
            "asset_id": asset_id,
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 240_000,
            "locked": false,
            "keyframes": [{
                "id": keyframe_id,
                "property": "opacity",
                "time_tk": 0,
                "value": 1.0,
                "easing": "linear",
            }],
        }],
    });
    let survivor_track = json!({
        "id": survivor_track_id,
        "kind": "video",
        "name": "Survivor",
        "locked": false,
        "clips": [{
            "id": survivor_clip_id,
            "name": "Survivor Clip",
            "asset_id": asset_id,
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 240_000,
            "locked": false,
        }],
    });
    prior
        .tracks
        .push(serde_json::from_value(target_track).expect("target track parses"));
    prior
        .tracks
        .push(serde_json::from_value(survivor_track).expect("survivor track parses"));
    prior.duration_tk = Tick::new(240_000);

    let args = track_remove::TrackRemoveArgs {
        project_id,
        track: target_track_id.to_string(),
    };

    let (patch_value, warnings, _data) = track_remove::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid track.remove patch");
    let patch: json_patch::Patch = serde_json::from_value(patch_value.clone())
        .expect("track.remove fixture patch is valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("track.remove fixture patch must apply cleanly");

    let expected_data = serde_json::to_value(
        track_remove::data_envelope_from_args_warnings(&args, &warnings)
            .expect("track.remove fixture expected_data"),
    )
    .expect("track.remove fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "track.remove".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings,
        post_state,
        expected_data,
    }
}

/// Build the canonical `track.solo` fixture used by [`default_fixtures`].
///
/// Starts from the `track_add_fixture()` post-state and solos the first
/// video track.
fn track_solo_fixture() -> RecordedEvent {
    let prior = track_add_fixture().post_state;
    let track = prior
        .tracks
        .first()
        .expect("track_add fixture has at least one track");

    let args = track_solo::TrackSoloArgs {
        project_id: DEFAULT_FIXTURE_PROJECT_ID
            .parse()
            .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7"),
        track: track.id.to_string(),
        solo: Some(true),
    };

    let (patch_value, _warnings, _data) = track_solo::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid track.solo patch");
    let patch: json_patch::Patch = serde_json::from_value(patch_value.clone())
        .expect("track.solo fixture patch must be valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("track.solo fixture patch must apply cleanly");

    let expected_data = serde_json::to_value(
        track_solo::data_envelope_from_post_state(&args, &post_state)
            .expect("track.solo fixture expected_data"),
    )
    .expect("track.solo fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "track.solo".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings: vec![],
        post_state,
        expected_data,
    }
}

/// Build the canonical `track.hide` fixture used by [`default_fixtures`].
///
/// Starts from the `track_add_fixture()` post-state and hides the first
/// video track.
fn track_hide_fixture() -> RecordedEvent {
    let prior = track_add_fixture().post_state;
    let track = prior
        .tracks
        .first()
        .expect("track_add fixture has at least one track");

    let args = track_hide::TrackHideArgs {
        project_id: DEFAULT_FIXTURE_PROJECT_ID
            .parse()
            .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7"),
        track: track.id.to_string(),
        hidden: Some(true),
    };

    let (patch_value, _warnings, _data) = track_hide::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid track.hide patch");
    let patch: json_patch::Patch = serde_json::from_value(patch_value.clone())
        .expect("track.hide fixture patch must be valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("track.hide fixture patch must apply cleanly");

    let expected_data = serde_json::to_value(
        track_hide::data_envelope_from_post_state(&args, &post_state)
            .expect("track.hide fixture expected_data"),
    )
    .expect("track.hide fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "track.hide".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings: vec![],
        post_state,
        expected_data,
    }
}

/// Build the canonical `track.set_volume` fixture used by [`default_fixtures`].
///
/// Build an audio track via `track.add` first, then set its `volume` to
/// `0.5`.
fn track_set_volume_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");
    let mut prior = synthetic_empty_project(project_id);
    // Build an audio track via track_add::compute_patch.
    let add_args = track_add::TrackAddArgs {
        project_id,
        kind: crate::track::TrackKind::Audio,
        name: None,
        index: None,
    };
    let (add_patch_val, _warnings) = track_add::compute_patch(&prior, &add_args)
        .expect("track.add audio fixture must produce a valid patch");
    let add_patch: json_patch::Patch =
        serde_json::from_value(add_patch_val).expect("track.add patch is valid RFC 6902");
    prior = prior.apply(&add_patch).expect("apply add patch");

    let track_id = prior.tracks[0].id.to_string();
    let args = track_set_volume::TrackSetVolumeArgs {
        project_id,
        track: track_id,
        volume: 0.5,
    };
    let (patch_value, _warnings, _data) = track_set_volume::compute_patch(&prior, &args)
        .expect("set_volume fixture must produce a valid patch");
    let patch: json_patch::Patch =
        serde_json::from_value(patch_value.clone()).expect("set_volume patch is valid RFC 6902");
    let post_state = prior.apply(&patch).expect("apply set_volume patch");
    let expected_data = serde_json::to_value(
        track_set_volume::data_envelope_from_post_state(&args, &post_state)
            .expect("set_volume envelope"),
    )
    .expect("set_volume expected_data serializes");

    RecordedEvent {
        verb: "track.set_volume".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings: vec![],
        post_state,
        expected_data,
    }
}

/// Build the canonical `track.set_pan` fixture used by [`default_fixtures`].
///
/// Build an audio track via `track.add` first, then set its `pan` to
/// `-0.5`.
fn track_set_pan_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");
    let mut prior = synthetic_empty_project(project_id);
    // Build an audio track via track_add::compute_patch.
    let add_args = track_add::TrackAddArgs {
        project_id,
        kind: crate::track::TrackKind::Audio,
        name: None,
        index: None,
    };
    let (add_patch_val, _warnings) = track_add::compute_patch(&prior, &add_args)
        .expect("track.add audio fixture must produce a valid patch");
    let add_patch: json_patch::Patch =
        serde_json::from_value(add_patch_val).expect("track.add patch is valid RFC 6902");
    prior = prior.apply(&add_patch).expect("apply add patch");

    let track_id = prior.tracks[0].id.to_string();
    let args = track_set_pan::TrackSetPanArgs {
        project_id,
        track: track_id,
        pan: -0.5,
    };
    let (patch_value, _warnings, _data) = track_set_pan::compute_patch(&prior, &args)
        .expect("set_pan fixture must produce a valid patch");
    let patch: json_patch::Patch =
        serde_json::from_value(patch_value.clone()).expect("set_pan patch is valid RFC 6902");
    let post_state = prior.apply(&patch).expect("apply set_pan patch");
    let expected_data = serde_json::to_value(
        track_set_pan::data_envelope_from_post_state(&args, &post_state).expect("set_pan envelope"),
    )
    .expect("set_pan expected_data serializes");

    RecordedEvent {
        verb: "track.set_pan".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings: vec![],
        post_state,
        expected_data,
    }
}

/// Build the canonical `list_capabilities` fixture used by
/// [`default_fixtures`].
///
/// `list_capabilities` ignores project state, so the prior is just the
/// empty synthetic project and the patch is empty. The expected data
/// comes from a forward `compute_patch` against that same prior.
fn list_capabilities_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = list_capabilities::ListCapabilitiesArgs { project_id };

    let (patch_value, _warnings, data) = list_capabilities::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid list_capabilities data");
    let expected_data = serde_json::to_value(&data)
        .expect("list_capabilities fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "list_capabilities".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings: vec![],
        post_state: prior,
        expected_data,
    }
}

/// Build the canonical `help` fixture used by [`default_fixtures`].
///
/// `help` ignores project state, so the prior is just the empty
/// synthetic project and the patch is empty. The fixture exercises the
/// no-topic branch (`topic = None`) which returns the noun list — the
/// non-trivial dispatch case most useful for the §0.8 reconstructor
/// gate.
fn help_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = help::HelpArgs {
        project_id,
        topic: None,
    };

    let (patch_value, _warnings, data) =
        help::compute_patch(&prior, &args).expect("default fixture must produce a valid help data");
    let expected_data =
        serde_json::to_value(&data).expect("help fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "help".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings: vec![],
        post_state: prior,
        expected_data,
    }
}

/// Build the canonical `validate_command` fixture used by
/// [`default_fixtures`].
///
/// `validate_command` ignores project state, so the prior is just the
/// empty synthetic project and the patch is empty. The fixture
/// exercises the Valid path against `marker.add` with a minimum-shape
/// valid args object.
fn validate_command_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = validate_command::ValidateCommandArgs {
        project_id,
        verb: "marker.add".to_string(),
        args: json!({
            "project_id": project_id.to_string(),
            "time_tk": 0_i64,
            "label": "fixture",
        }),
    };

    let (patch_value, _warnings, data) = validate_command::compute_patch(&prior, &args)
        .expect("default fixture must produce valid validate_command data");
    let expected_data = serde_json::to_value(&data)
        .expect("validate_command fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "validate_command".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings: vec![],
        post_state: prior,
        expected_data,
    }
}

/// Build the canonical `schema` fixture used by [`default_fixtures`].
///
/// `schema` ignores project state, so the prior is just the empty
/// synthetic project and the patch is empty. The fixture exercises the
/// `target=Project` branch — the most stable shape across builds
/// (the embedded project schema is checked-in spec content).
fn schema_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = schema::SchemaArgs {
        project_id,
        target: schema::SchemaTarget::Project,
        name: None,
    };

    let (patch_value, _warnings, data) = schema::compute_patch(&prior, &args)
        .expect("default fixture must produce valid schema data");
    let expected_data =
        serde_json::to_value(&data).expect("schema fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "schema".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings: vec![],
        post_state: prior,
        expected_data,
    }
}

/// Build the canonical `stock.list_providers` fixture used by
/// [`default_fixtures`].
///
/// `stock.list_providers` ignores project state, so the prior is just
/// the empty synthetic project and the patch is empty. Expected data
/// comes from a forward `compute_patch` against that same prior.
fn stock_list_providers_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = stock_list_providers::StockListProvidersArgs { project_id };

    let (patch_value, _warnings, data) = stock_list_providers::compute_patch(&prior, &args)
        .expect("default fixture must produce valid stock.list_providers data");
    let expected_data = serde_json::to_value(&data)
        .expect("stock.list_providers fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "stock.list_providers".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings: vec![],
        post_state: prior,
        expected_data,
    }
}

/// Build the canonical `stock.search` fixture used by
/// [`default_fixtures`].
///
/// `stock.search` v1 floor is read-only and uses local provider-only
/// registration, so the canonical fixture is a well-formed local call
/// that returns empty items with empty patch/warnings.
fn stock_search_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = stock_search::StockSearchArgs {
        project_id,
        provider_id: "local".to_string(),
        query: "sunset".to_string(),
        kind: "video".to_string(),
        limit: 25,
        filters: stock_search::StockSearchFilters::default(),
    };

    let (patch_value, _warnings, data) = stock_search::compute_patch(&prior, &args)
        .expect("default fixture must produce valid stock.search data");
    let expected_data = serde_json::to_value(&data)
        .expect("stock.search fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "stock.search".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings: vec![],
        post_state: prior,
        expected_data,
    }
}

/// Build the canonical `stock.import` fixture used by
/// [`default_fixtures`].
///
/// `stock.import` v1 floor always errors for well-formed calls (local
/// provider resolves to `E_STOCK_NOT_FOUND`), so no successful event
/// can be recorded. The reconstructor path only checks arg shape and
/// returns `Value::Null`; this fixture records `expected_data: null`.
fn stock_import_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = stock_import::StockImportArgs {
        project_id,
        provider_id: "local".to_string(),
        stock_id: "local:v1-fixture-stock-id".to_string(),
        mode: stock_import::StockImportMode::Copy,
        accept_license_unknown: false,
    };

    RecordedEvent {
        verb: "stock.import".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: json!([]),
        warnings: vec![],
        post_state: prior,
        expected_data: Value::Null,
    }
}

/// Build the canonical `stock.describe` fixture used by
/// [`default_fixtures`].
///
/// `stock.describe` v1 floor always errors for well-formed calls
/// (`local` provider resolves to `E_STOCK_NOT_FOUND`), so no
/// successful event can be recorded. The reconstructor path only
/// checks arg shape and returns `Value::Null`; this fixture records
/// `expected_data: null`.
fn stock_describe_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = stock_describe::StockDescribeArgs {
        project_id,
        provider_id: "local".to_string(),
        stock_id: "local:v1-fixture-stock-id".to_string(),
    };

    RecordedEvent {
        verb: "stock.describe".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: json!([]),
        warnings: vec![],
        post_state: prior,
        expected_data: Value::Null,
    }
}

/// Build the canonical `font.list` fixture used by [`default_fixtures`].
///
/// `font.list` ignores project state, so the prior is just the empty
/// synthetic project and the patch is empty. Expected data comes from
/// a forward `compute_patch` against that same prior — currently an
/// empty `fonts` list per the v1 floor.
fn font_list_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = font_list::FontListArgs { project_id };

    let (patch_value, _warnings, data) = font_list::compute_patch(&prior, &args)
        .expect("default fixture must produce valid font.list data");
    let expected_data =
        serde_json::to_value(&data).expect("font.list fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "font.list".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings: vec![],
        post_state: prior,
        expected_data,
    }
}

/// Build the canonical `template.list` fixture used by
/// [`default_fixtures`].
///
/// `template.list` ignores project state in the v1 floor, so the prior
/// is just the empty synthetic project and the patch is empty.
/// Expected data comes from a forward `compute_patch` against that
/// same prior — currently an empty `templates` list.
fn template_list_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = template_list::TemplateListArgs {
        project_id,
        source: None,
        kind: None,
    };

    let (patch_value, _warnings, data) = template_list::compute_patch(&prior, &args)
        .expect("default fixture must produce valid template.list data");
    let expected_data =
        serde_json::to_value(&data).expect("template.list fixture expected_data serializes");

    RecordedEvent {
        verb: "template.list".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings: vec![],
        post_state: prior,
        expected_data,
    }
}

/// Build the canonical `template.describe` fixture used by
/// [`default_fixtures`].
///
/// `template.describe` validates local args shape in the v1 floor and
/// then always errors with `E_TEMPLATE_NOT_FOUND` because runtime
/// template-catalog lookup is deferred. No successful event can ever
/// be recorded, so the reconstructor's input tuple carries no
/// semantically-meaningful payload — the verb's `reconstruct()`
/// exercises args-deserialization and returns `Value::Null` to satisfy
/// the §0.8 gate. The fixture records `expected_data: null` so the
/// gate's canonical-SHA equality holds.
fn template_describe_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = template_describe::TemplateDescribeArgs {
        project_id,
        template_id: "0190b8d3-15e3-7000-bd00-0000feedbeef".to_string(),
    };

    RecordedEvent {
        verb: "template.describe".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: json!([]),
        warnings: vec![],
        post_state: prior,
        expected_data: Value::Null,
    }
}

/// Build the canonical `template.apply` fixture used by
/// [`default_fixtures`].
///
/// `template.apply` validates local args shape in the v1 floor, checks
/// only local `at_tk >= 0`, then always errors with
/// `E_TEMPLATE_NOT_FOUND` because template-resource lookup is deferred.
/// No successful event can be recorded at this slice, so this fixture
/// carries a well-formed args payload with `patch: []`, `warnings: []`,
/// and `expected_data: null` to satisfy the §0.8 reconstructor gate.
fn template_apply_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = template_apply::TemplateApplyArgs {
        project_id,
        template_id: "0190b8d3-15e3-7000-bd00-0000feedbabe".to_string(),
        slots: std::collections::BTreeMap::new(),
        at_tk: None,
        track_strategy: template_apply::TemplateTrackStrategy::CreateNew,
    };

    RecordedEvent {
        verb: "template.apply".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: json!([]),
        warnings: vec![],
        post_state: prior,
        expected_data: Value::Null,
    }
}

/// Build the canonical `template.from_project` fixture used by
/// [`default_fixtures`].
///
/// `template.from_project` validates local args shape in the v1 floor
/// and then always errors with `E_IO` because runtime file-writing is
/// deferred. No successful event can be recorded at this slice, so
/// this fixture carries a well-formed args payload with `patch: []`,
/// `warnings: []`, and `expected_data: null` for the §0.8 gate.
fn template_from_project_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = template_from_project::TemplateFromProjectArgs {
        project_id,
        out_path: "/tmp/template-from-project.verbreel-template".to_string(),
        name: "Exported Template".to_string(),
        description: String::new(),
        author: String::new(),
        slot_clips: Vec::new(),
        slot_texts: Vec::new(),
        include_slot_defaults: false,
        from_tk: None,
        to_tk: None,
        preview_png: None,
        tags: Vec::new(),
    };

    RecordedEvent {
        verb: "template.from_project".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: json!([]),
        warnings: vec![],
        post_state: prior,
        expected_data: Value::Null,
    }
}

/// Build the canonical `template.install` fixture used by
/// [`default_fixtures`].
///
/// `template.install` validates local args shape in the v1 floor and
/// then always errors with `E_IO` because runtime file-install support
/// is deferred. No successful event can be recorded at this slice, so
/// this fixture carries a well-formed args payload with `patch: []`,
/// `warnings: []`, and `expected_data: null` for the §0.8 gate.
fn template_install_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = template_install::TemplateInstallArgs {
        project_id,
        path: "/tmp/template-install.verbreel-template".to_string(),
        overwrite: false,
    };

    RecordedEvent {
        verb: "template.install".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: json!([]),
        warnings: vec![],
        post_state: prior,
        expected_data: Value::Null,
    }
}

/// Build the canonical `template.uninstall` fixture used by
/// [`default_fixtures`].
///
/// `template.uninstall` validates local args shape in the v1 floor and
/// then always errors with `E_TEMPLATE_NOT_FOUND` because runtime
/// template-catalog and filesystem uninstall context is deferred. No
/// successful event can be recorded at this slice, so this fixture
/// carries a well-formed args payload with `patch: []`,
/// `warnings: []`, and `expected_data: null` for the §0.8 gate.
fn template_uninstall_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = template_uninstall::TemplateUninstallArgs {
        project_id,
        template_id: "0190b8d3-15e3-7000-bd00-0000deadbead".to_string(),
    };

    RecordedEvent {
        verb: "template.uninstall".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: json!([]),
        warnings: vec![],
        post_state: prior,
        expected_data: Value::Null,
    }
}

/// Build the canonical `asset.verify` fixture used by
/// [`default_fixtures`].
///
/// Synthetic empty project carries zero assets, so the v1 floor's
/// `checked_count` is `0` and `unverified_asset_ids` is `[]`. Mode
/// defaults to `Fast` because `strict` is omitted.
fn asset_verify_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = asset_verify::AssetVerifyArgs {
        project_id,
        strict: None,
    };

    let (patch_value, _warnings, data) = asset_verify::compute_patch(&prior, &args)
        .expect("default fixture must produce valid asset.verify data");
    let expected_data = serde_json::to_value(&data)
        .expect("asset.verify fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "asset.verify".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings: vec![],
        post_state: prior,
        expected_data,
    }
}

/// Build the canonical `tracker.create` fixture used by
/// [`default_fixtures`].
///
/// Starts from a synthetic project carrying a single video track with a
/// single video clip spanning `[0, 240_000)` ticks, then creates an
/// `object`-algorithm tracker with a valid `bbox+at_tk` inside that
/// window. Exercises the envelope-warning round-trip — the
/// reconstructor recovers `tracker_id`, `source_clip_id`, and
/// `algorithm` from the warning rather than from post-state (which
/// could not disambiguate multiple trackers sharing identical
/// placeholder fields).
fn tracker_create_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let mut prior = synthetic_empty_project(project_id);

    let video_track_id = "01900000-0000-7000-8000-0000000aa701";
    let video_clip_id = "01900000-0000-7000-8000-0000000bb701";
    let video_asset_id = "01900000-0000-7000-8000-0000000cc701";

    prior.assets.push(
        serde_json::from_value(json!({
            "id": video_asset_id,
            "kind": "video",
            "hash": "36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da",
            "path": "assets/36/36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da.mp4",
            "original_filename": "tracker-create.mp4",
            "imported_at": "2026-05-24T00:00:00Z",
            "metadata": {
                "duration_tk": 240_000,
                "width": 1920,
                "height": 1080,
                "fps_num": 30,
                "fps_den": 1,
                "video_codec": "h264",
                "container": "mp4",
                "fingerprint": {
                    "mtime_ms": 1_700_000_000_000_i64,
                    "size_bytes": 1024,
                }
            }
        }))
        .expect("tracker.create fixture asset parses"),
    );

    let video_track = json!({
        "id": video_track_id,
        "kind": "video",
        "name": "Video Tracker Source",
        "locked": false,
        "clips": [{
            "id": video_clip_id,
            "name": "Source Clip",
            "asset_id": video_asset_id,
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 240_000,
            "locked": false,
        }],
    });
    prior
        .tracks
        .push(serde_json::from_value(video_track).expect("tracker.create fixture track parses"));
    prior.duration_tk = Tick::new(240_000);

    let args = tracker_create::TrackerCreateArgs {
        project_id,
        clip: video_clip_id.to_string(),
        algorithm: tracker_create::TrackerAlgorithm::Object,
        params: Some(json!({
            "object_bbox_at_tk": {
                "x": 640.0,
                "y": 360.0,
                "w": 120.0,
                "h": 160.0,
                "at_tk": 0,
            }
        })),
    };

    let (patch_value, warnings, _data) = tracker_create::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid tracker.create patch");
    let patch: json_patch::Patch = serde_json::from_value(patch_value.clone())
        .expect("tracker.create fixture patch is valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("tracker.create fixture patch must apply cleanly");

    let expected_data = serde_json::to_value(
        tracker_create::data_envelope_from_args_warnings(&args, &warnings)
            .expect("tracker.create fixture expected_data"),
    )
    .expect("tracker.create fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "tracker.create".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings,
        post_state,
        expected_data,
    }
}

/// Build the canonical `tracker.apply` fixture used by
/// [`default_fixtures`].
///
/// `tracker.apply` always errors in the v1 floor. The fixture still
/// uses well-formed args and an existing unrun tracker id to pin the
/// reconstructor tuple shape: `patch: []`, `warnings: []`,
/// `expected_data: null`.
fn tracker_apply_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let mut prior = synthetic_empty_project(project_id);
    let tracker_id = "01900000-0000-7000-8000-0000000ee111";
    let to_clip_id = "01900000-0000-7000-8000-0000000bb111";

    prior.trackers.push(
        serde_json::from_value(json!({
            "tracker_id": tracker_id,
            "source_clip_id": "",
            "algorithm": "object",
            "applied_to_clip_ids": [],
            "sample_count": -1,
            "cache_hash": "",
            "cache_path": "",
        }))
        .expect("tracker.apply fixture tracker parses"),
    );

    let args = tracker_apply::TrackerApplyArgs {
        project_id,
        tracker_id: tracker_id.to_string(),
        to_clip: to_clip_id.to_string(),
        properties: None,
        offset: None,
        decimate_to_every: None,
    };

    RecordedEvent {
        verb: "tracker.apply".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: json!([]),
        warnings: vec![],
        post_state: prior,
        expected_data: Value::Null,
    }
}

/// Build the canonical `tracker.list` fixture used by
/// [`default_fixtures`].
///
/// Exercises the empty-trackers path — `Project.trackers: []`. The
/// verb iterates the placeholder vec, so the empty path is
/// structurally complete (the loop body never runs and the envelope
/// resolves to `{trackers: []}`). Mirrors the `font.list` v1-floor
/// fixture which also resolves to an empty list.
fn tracker_list_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = tracker_list::TrackerListArgs { project_id };

    let (patch_value, _warnings, data) = tracker_list::compute_patch(&prior, &args)
        .expect("default fixture must produce valid tracker.list data");
    let expected_data = serde_json::to_value(&data)
        .expect("tracker.list fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "tracker.list".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings: vec![],
        post_state: prior,
        expected_data,
    }
}

/// Build the canonical `tracker.remove` fixture used by
/// [`default_fixtures`].
///
/// Starts from a synthetic project carrying a single tracker placeholder
/// record with empty `cache_path`, then removes it. Exercises the
/// envelope-warning path with `cache_path: None` (the v1-floor common
/// case — no `tracker.run` has populated the cache).
fn tracker_remove_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let mut prior = synthetic_empty_project(project_id);
    let tracker_id = "01900000-0000-7000-8000-0000000ee001";

    prior.trackers.push(
        serde_json::from_value(json!({
            "tracker_id": tracker_id,
            "source_clip_id": "",
            "algorithm": "object",
            "applied_to_clip_ids": [],
            "cache_hash": "",
            "cache_path": "",
        }))
        .expect("tracker.remove fixture tracker parses"),
    );

    let args = tracker_remove::TrackerRemoveArgs {
        project_id,
        tracker_id: tracker_id.to_string(),
        purge_cache: Some(true),
    };

    let (patch_value, warnings, _data) = tracker_remove::compute_patch(&prior, &args)
        .expect("default fixture must produce a valid tracker.remove patch");
    let patch: json_patch::Patch = serde_json::from_value(patch_value.clone())
        .expect("tracker.remove fixture patch is valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("tracker.remove fixture patch must apply cleanly");

    let expected_data = serde_json::to_value(
        tracker_remove::data_envelope_from_args_warnings(&args, &warnings)
            .expect("tracker.remove fixture expected_data"),
    )
    .expect("tracker.remove fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "tracker.remove".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings,
        post_state,
        expected_data,
    }
}

/// Build the canonical `tracker.run` fixture used by
/// [`default_fixtures`].
///
/// `tracker.run` always errors in the v1 floor. The fixture still uses
/// well-formed args and an existing tracker id to pin the reconstructor
/// tuple shape: `patch: []`, `warnings: []`, `expected_data: null`.
fn tracker_run_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let mut prior = synthetic_empty_project(project_id);
    let tracker_id = "01900000-0000-7000-8000-0000000ee101";

    prior.trackers.push(
        serde_json::from_value(json!({
            "tracker_id": tracker_id,
            "source_clip_id": "",
            "algorithm": "object",
            "applied_to_clip_ids": [],
            "sample_count": -1,
            "cache_hash": "",
            "cache_path": "",
        }))
        .expect("tracker.run fixture tracker parses"),
    );

    let args = tracker_run::TrackerRunArgs {
        project_id,
        tracker_id: tracker_id.to_string(),
        from_tk: None,
        to_tk: None,
        sample_every_ticks: None,
    };

    RecordedEvent {
        verb: "tracker.run".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: json!([]),
        warnings: vec![],
        post_state: prior,
        expected_data: Value::Null,
    }
}

/// Build the canonical `render.queue.add` fixture used by
/// [`default_fixtures`].
///
/// `render.queue.add` validates static args in v1, then always errors
/// with `E_QUEUE_FULL` because queue persistence/worker context is
/// deferred. No successful event can ever be recorded, so the
/// reconstructor's input tuple carries no semantically-meaningful
/// payload — the verb's `reconstruct()` exercises args-deserialization
/// and returns `Value::Null` to satisfy the §0.8 gate. The fixture
/// records `expected_data: null` so the gate's canonical-SHA equality
/// holds.
fn render_queue_add_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = render_queue_add::RenderQueueAddArgs {
        project_id,
        preset: "youtube-1080p".to_string(),
        out_path: "exports/queue-floor.mp4".to_string(),
        from_tk: None,
        to_tk: None,
        video_codec: None,
        audio_codec: None,
        bitrate_bps: None,
        crf: None,
        deterministic: false,
        keep_temp: false,
        overwrite: false,
        priority: 0,
        wait: false,
    };

    RecordedEvent {
        verb: "render.queue.add".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: json!([]),
        warnings: vec![],
        post_state: prior,
        expected_data: Value::Null,
    }
}

/// Build the canonical `render.queue.list` fixture used by
/// [`default_fixtures`].
///
/// `render.queue.list` ignores project state, so the prior is just the
/// empty synthetic project and the patch is empty. Expected data comes
/// from a forward `compute_patch` against that same prior — currently
/// an empty `items` list per the v1 floor (queue persistence file
/// reading is deferred — see the module-level doc on
/// `verbs::render_queue_list`).
fn render_queue_list_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = render_queue_list::RenderQueueListArgs {
        project_id,
        state_filter: None,
    };

    let (patch_value, _warnings, data) = render_queue_list::compute_patch(&prior, &args)
        .expect("default fixture must produce valid render.queue.list data");
    let expected_data = serde_json::to_value(&data)
        .expect("render.queue.list fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "render.queue.list".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings: vec![],
        post_state: prior,
        expected_data,
    }
}

/// Build the canonical `render.queue.clear` fixture used by
/// [`default_fixtures`].
///
/// `render.queue.clear` ignores project state, so the prior is just the
/// empty synthetic project and the patch is empty. The default-args
/// (None / None) path bypasses the confirm gate (filter defaults to
/// terminal states only) and produces empty `removed` / `canceled`
/// arrays per the v1 floor (queue persistence file mutation is
/// deferred — see the module-level doc on `verbs::render_queue_clear`).
fn render_queue_clear_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = render_queue_clear::RenderQueueClearArgs {
        project_id,
        state_filter: None,
        confirm: None,
    };

    let (patch_value, _warnings, data) = render_queue_clear::compute_patch(&prior, &args)
        .expect("default fixture must produce valid render.queue.clear data");
    let expected_data = serde_json::to_value(&data)
        .expect("render.queue.clear fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "render.queue.clear".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings: vec![],
        post_state: prior,
        expected_data,
    }
}

/// Build the canonical `render.queue.status` fixture used by
/// [`default_fixtures`].
///
/// `render.queue.status` always errors with `E_QUEUE_JOB_NOT_FOUND` in
/// the v1 floor (the queue persistence file is never read; see the
/// module-level doc on `verbs::render_queue_status`). No successful
/// event can ever be recorded, so the reconstructor's input tuple
/// carries no semantically-meaningful payload — the verb's
/// `reconstruct()` exercises args-deserialization and returns
/// `Value::Null` to satisfy the §0.8 gate. The fixture records
/// `expected_data: null` so the gate's canonical-SHA equality holds.
fn render_queue_status_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = render_queue_status::RenderQueueStatusArgs {
        project_id,
        queue_job_id: "0190b8d3-15e3-7000-bd00-0000feedbeef".to_string(),
    };

    RecordedEvent {
        verb: "render.queue.status".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: json!([]),
        warnings: vec![],
        post_state: prior,
        expected_data: Value::Null,
    }
}

/// Build the canonical `render.queue.cancel` fixture used by
/// [`default_fixtures`].
///
/// `render.queue.cancel` always errors with `E_QUEUE_JOB_NOT_FOUND` in
/// the v1 floor (the queue persistence file is never read; see the
/// module-level doc on `verbs::render_queue_cancel`). No successful
/// event can ever be recorded, so the reconstructor's input tuple
/// carries no semantically-meaningful payload — the verb's
/// `reconstruct()` exercises args-deserialization and returns
/// `Value::Null` to satisfy the §0.8 gate. The fixture records
/// `expected_data: null` so the gate's canonical-SHA equality holds.
fn render_queue_cancel_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = render_queue_cancel::RenderQueueCancelArgs {
        project_id,
        queue_job_id: "0190b8d3-15e3-7000-bd00-0000feedbeef".to_string(),
    };

    RecordedEvent {
        verb: "render.queue.cancel".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: json!([]),
        warnings: vec![],
        post_state: prior,
        expected_data: Value::Null,
    }
}

/// Build the canonical `render.list_presets` fixture used by
/// [`default_fixtures`].
fn render_list_presets_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = render_list_presets::RenderListPresetsArgs {
        project_id: Some(project_id),
    };

    let (patch_value, _warnings, data) = render_list_presets::compute_patch(&prior, &args)
        .expect("default fixture must produce valid render.list_presets data");
    let expected_data = serde_json::to_value(&data)
        .expect("render.list_presets fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "render.list_presets".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings: vec![],
        post_state: prior,
        expected_data,
    }
}

/// Build the canonical `render.start` fixture used by
/// [`default_fixtures`].
///
/// `render.start` always errors with `E_RENDER_FAIL` in the v1 floor
/// because runtime worker/renderer orchestration is intentionally
/// deferred (see the module-level doc on `verbs::render_start`). No
/// successful event can ever be recorded, so the reconstructor's input
/// tuple carries no semantically-meaningful payload — the verb's
/// `reconstruct()` exercises args-deserialization and returns
/// `Value::Null` to satisfy the §0.8 gate. The fixture records
/// `expected_data: null` so the gate's canonical-SHA equality holds.
fn render_start_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = render_start::RenderStartArgs {
        project_id,
        preset: "youtube-1080p".to_string(),
        out_path: "exports/out.mp4".to_string(),
        from_tk: None,
        to_tk: None,
        video_codec: None,
        audio_codec: None,
        bitrate_bps: None,
        crf: None,
        deterministic: false,
        keep_temp: false,
        overwrite: false,
    };

    RecordedEvent {
        verb: "render.start".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: json!([]),
        warnings: vec![],
        post_state: prior,
        expected_data: Value::Null,
    }
}

/// Build the canonical `asset.probe` fixture used by
/// [`default_fixtures`].
///
/// `asset.probe` always errors with `E_ASSET_PATH_NOT_FOUND` in the v1
/// floor (file I/O is forbidden in the pure `Verb::compute_patch`; see
/// the module-level doc on `verbs::asset_probe`). No successful event
/// can ever be recorded, so the reconstructor's input tuple carries no
/// semantically-meaningful payload — the verb's `reconstruct()`
/// exercises args-deserialization and returns `Value::Null` to satisfy
/// the §0.8 gate. The fixture records `expected_data: null` so the
/// gate's canonical-SHA equality holds.
fn asset_probe_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = asset_probe::AssetProbeArgs {
        project_id,
        path: "/does/not/exist.mp4".to_string(),
    };

    RecordedEvent {
        verb: "asset.probe".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: json!([]),
        warnings: vec![],
        post_state: prior,
        expected_data: Value::Null,
    }
}

/// Build the canonical `asset.relink` fixture used by
/// [`default_fixtures`].
///
/// `asset.relink` always errors with `E_ASSET_PATH_NOT_FOUND` in the
/// v1 floor (file I/O is forbidden in the pure `Verb::compute_patch`;
/// see the module-level doc on `verbs::asset_relink`). No successful
/// event can ever be recorded, so the reconstructor's input tuple
/// carries no semantically-meaningful payload — the verb's
/// `reconstruct()` exercises args-deserialization and returns
/// `Value::Null` to satisfy the §0.8 gate. The fixture records
/// `expected_data: null` so the gate's canonical-SHA equality holds.
fn asset_relink_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = asset_relink::AssetRelinkArgs {
        project_id,
        asset_id: "01900000-0000-7000-8000-00000000cce1".to_string(),
        source_path: "/does/not/exist.mp4".to_string(),
        mode: None,
    };

    RecordedEvent {
        verb: "asset.relink".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: json!([]),
        warnings: vec![],
        post_state: prior,
        expected_data: Value::Null,
    }
}

/// Build the canonical `render.cancel` fixture used by
/// [`default_fixtures`].
///
/// `render.cancel` always errors with `E_JOB_NOT_FOUND` in the v1 floor
/// (`render.start` is a v1 always-error floor, so no render job is ever
/// in flight; see the module-level doc on `verbs::render_cancel`). No
/// successful event can ever be recorded, so the reconstructor's input
/// tuple carries no semantically-meaningful payload — the verb's
/// `reconstruct()` exercises args-deserialization and returns
/// `Value::Null` to satisfy the §0.8 gate. The fixture records
/// `expected_data: null` so the gate's canonical-SHA equality holds.
fn render_cancel_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = render_cancel::RenderCancelArgs {
        project_id,
        job_id: "0190b8d3-15e3-7000-bd00-0000feedbeef".to_string(),
    };

    RecordedEvent {
        verb: "render.cancel".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: json!([]),
        warnings: vec![],
        post_state: prior,
        expected_data: Value::Null,
    }
}

/// Build the canonical `render.status` fixture used by
/// [`default_fixtures`].
///
/// `render.status` always errors with `E_JOB_NOT_FOUND` in the v1 floor
/// (`render.start` is a v1 always-error floor, so no render job is ever
/// in flight; see the module-level doc on `verbs::render_status`). No
/// successful event can ever be recorded, so the reconstructor's input
/// tuple carries no semantically-meaningful payload — the verb's
/// `reconstruct()` exercises args-deserialization and returns
/// `Value::Null` to satisfy the §0.8 gate. The fixture records
/// `expected_data: null` so the gate's canonical-SHA equality holds.
fn render_status_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = render_status::RenderStatusArgs {
        project_id,
        job_id: "0190b8d3-15e3-7000-bd00-0000feedbeef".to_string(),
    };

    RecordedEvent {
        verb: "render.status".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: json!([]),
        warnings: vec![],
        post_state: prior,
        expected_data: Value::Null,
    }
}

/// Build the canonical `preview.frame` fixture used by
/// [`default_fixtures`].
///
/// `preview.frame` validates only local argument bounds in v1
/// (`at_tk >= 0`, optional `width_px ∈ [1, 8192]`) and then always
/// errors with `E_IO` because renderer/cache runtime is deferred. No
/// successful event can ever be recorded, so the reconstructor's input
/// tuple carries no semantically-meaningful payload — the verb's
/// `reconstruct()` exercises args-deserialization and returns
/// `Value::Null` to satisfy the §0.8 gate. The fixture records
/// `expected_data: null` so the gate's canonical-SHA equality holds.
fn preview_frame_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = preview_frame::PreviewFrameArgs {
        project_id,
        at_tk: 0,
        out_path: None,
        width_px: None,
        deterministic: false,
    };

    RecordedEvent {
        verb: "preview.frame".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: json!([]),
        warnings: vec![],
        post_state: prior,
        expected_data: Value::Null,
    }
}

/// Build the canonical `preview.waveform` fixture used by
/// [`default_fixtures`].
///
/// `preview.waveform` validates only local selector/range constraints
/// in v1 (qualified `target`, optional `samples ∈ [1, 100000]`) and
/// then always errors with `E_IO` because waveform/runtime cache
/// execution is deferred. No successful event can ever be recorded, so
/// the reconstructor's input tuple carries no semantically-meaningful
/// payload — the verb's `reconstruct()` exercises args-deserialization
/// and returns `Value::Null` to satisfy the §0.8 gate. The fixture
/// records `expected_data: null` so the gate's canonical-SHA equality
/// holds.
fn preview_waveform_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = preview_waveform::PreviewWaveformArgs {
        project_id,
        target: "asset:0190b8d3-15e3-7000-bd00-0000feedbeef".to_string(),
        samples: None,
        out_path: None,
    };

    RecordedEvent {
        verb: "preview.waveform".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: json!([]),
        warnings: vec![],
        post_state: prior,
        expected_data: Value::Null,
    }
}

/// Build the canonical `preview.thumbnail` fixture used by
/// [`default_fixtures`].
///
/// `preview.thumbnail` validates only local selector/range constraints
/// in v1 (qualified `target`, `count ∈ [1, 1000]`, optional
/// `width_px ∈ [1, 8192]`) and then always errors with `E_IO` because
/// thumbnail/runtime cache execution is deferred. No successful event
/// can ever be recorded, so the reconstructor's input tuple carries no
/// semantically-meaningful payload — the verb's `reconstruct()`
/// exercises args-deserialization and returns `Value::Null` to satisfy
/// the §0.8 gate. The fixture records `expected_data: null` so the
/// gate's canonical-SHA equality holds.
fn preview_thumbnail_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = preview_thumbnail::PreviewThumbnailArgs {
        project_id,
        target: "asset:0190b8d3-15e3-7000-bd00-0000feedbeef".to_string(),
        count: 1,
        out_dir: None,
        width_px: None,
    };

    RecordedEvent {
        verb: "preview.thumbnail".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: json!([]),
        warnings: vec![],
        post_state: prior,
        expected_data: Value::Null,
    }
}

/// Build the canonical `preview.session.create` fixture used by
/// [`default_fixtures`].
///
/// `preview.session.create` validates only local args/ranges in the v1
/// floor and then always errors with `E_PREVIEW_SESSION_LIMIT` because
/// runtime session-manager allocation is deferred. No successful event
/// can ever be recorded, so the reconstructor's input tuple carries no
/// semantically-meaningful payload — the verb's `reconstruct()`
/// exercises args-deserialization and returns `Value::Null` to satisfy
/// the §0.8 gate. The fixture records `expected_data: null` so the
/// gate's canonical-SHA equality holds.
fn preview_session_create_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = preview_session_create::PreviewSessionCreateArgs {
        project_id,
        playback_rate: None,
        width_px: None,
        audio_enabled: true,
        format: preview_session_create::PreviewSessionChannelKind::Ndjson,
        start_at_tk: None,
    };

    RecordedEvent {
        verb: "preview.session.create".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: json!([]),
        warnings: vec![],
        post_state: prior,
        expected_data: Value::Null,
    }
}

/// Build the canonical `preview.session.seek` fixture used by
/// [`default_fixtures`].
///
/// `preview.session.seek` validates only local `at_tk >= 0` in the v1
/// floor and then always errors with `E_PREVIEW_SESSION_NOT_FOUND`
/// because runtime session lookup is deferred. No successful event can
/// ever be recorded, so the reconstructor's input tuple carries no
/// semantically-meaningful payload — the verb's `reconstruct()`
/// exercises args-deserialization and returns `Value::Null` to satisfy
/// the §0.8 gate. The fixture records `expected_data: null` so the
/// gate's canonical-SHA equality holds.
fn preview_session_seek_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = preview_session_seek::PreviewSessionSeekArgs {
        project_id,
        session_id: "0190b8d3-15e3-7000-bd00-0000feedbeef".to_string(),
        at_tk: 0,
    };

    RecordedEvent {
        verb: "preview.session.seek".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: json!([]),
        warnings: vec![],
        post_state: prior,
        expected_data: Value::Null,
    }
}

/// Build the canonical `preview.session.play` fixture used by
/// [`default_fixtures`].
///
/// `preview.session.play` validates local args shape in the v1 floor and
/// then always errors with `E_PREVIEW_SESSION_NOT_FOUND` because runtime
/// session lookup and channel wiring are deferred. No successful event can
/// ever be recorded, so the reconstructor's input tuple carries no
/// semantically-meaningful payload — the verb's `reconstruct()`
/// exercises args-deserialization and returns `Value::Null` to satisfy
/// the §0.8 gate. The fixture records `expected_data: null` so the
/// gate's canonical-SHA equality holds.
fn preview_session_play_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = preview_session_play::PreviewSessionPlayArgs {
        project_id,
        session_id: "0190b8d3-15e3-7000-bd00-0000feedbeef".to_string(),
    };

    RecordedEvent {
        verb: "preview.session.play".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: json!([]),
        warnings: vec![],
        post_state: prior,
        expected_data: Value::Null,
    }
}

/// Build the canonical `preview.session.pause` fixture used by
/// [`default_fixtures`].
///
/// `preview.session.pause` validates local args shape in the v1 floor and
/// then always errors with `E_PREVIEW_SESSION_NOT_FOUND` because runtime
/// session lookup and cooperative pause behavior are deferred. No successful
/// event can ever be recorded, so the reconstructor's input tuple carries no
/// semantically-meaningful payload — the verb's `reconstruct()`
/// exercises args-deserialization and returns `Value::Null` to satisfy
/// the §0.8 gate. The fixture records `expected_data: null` so the
/// gate's canonical-SHA equality holds.
fn preview_session_pause_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = preview_session_pause::PreviewSessionPauseArgs {
        project_id,
        session_id: "0190b8d3-15e3-7000-bd00-0000feedbeef".to_string(),
    };

    RecordedEvent {
        verb: "preview.session.pause".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: json!([]),
        warnings: vec![],
        post_state: prior,
        expected_data: Value::Null,
    }
}

/// Build the canonical `preview.session.close` fixture used by
/// [`default_fixtures`].
///
/// `preview.session.close` always errors with
/// `E_PREVIEW_SESSION_NOT_FOUND` in the v1 floor (no
/// `preview.session.create` verb exists yet, so no preview session is
/// ever in flight; see the module-level doc on
/// `verbs::preview_session_close`). No successful event can ever be
/// recorded, so the reconstructor's input tuple carries no
/// semantically-meaningful payload — the verb's `reconstruct()`
/// exercises args-deserialization and returns `Value::Null` to satisfy
/// the §0.8 gate. The fixture records `expected_data: null` so the
/// gate's canonical-SHA equality holds.
fn preview_session_close_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = preview_session_close::PreviewSessionCloseArgs {
        project_id,
        session_id: "0190b8d3-15e3-7000-bd00-0000feedbeef".to_string(),
    };

    RecordedEvent {
        verb: "preview.session.close".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: json!([]),
        warnings: vec![],
        post_state: prior,
        expected_data: Value::Null,
    }
}

/// Build the canonical `preview.session.frame_at` fixture used by
/// [`default_fixtures`].
///
/// `preview.session.frame_at` validates only local `at_tk` / `width_px`
/// bounds in the v1 floor and then always errors with
/// `E_PREVIEW_SESSION_NOT_FOUND` because runtime session lookup and
/// frame-at execution are deferred. No successful event can ever be
/// recorded, so the reconstructor's input tuple carries no
/// semantically-meaningful payload — the verb's `reconstruct()`
/// exercises args-deserialization and returns `Value::Null` to satisfy
/// the §0.8 gate. The fixture records `expected_data: null` so the
/// gate's canonical-SHA equality holds.
fn preview_session_frame_at_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = preview_session_frame_at::PreviewSessionFrameAtArgs {
        project_id,
        session_id: "0190b8d3-15e3-7000-bd00-0000feedbeef".to_string(),
        at_tk: 0,
        out_path: None,
        width_px: None,
    };

    RecordedEvent {
        verb: "preview.session.frame_at".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: json!([]),
        warnings: vec![],
        post_state: prior,
        expected_data: Value::Null,
    }
}

/// Build the canonical `project.list` fixture used by
/// [`default_fixtures`].
///
/// v1 floor always returns `{ projects: [] }` regardless of prior
/// state, so the fixture pairs the synthetic empty project with an
/// empty patch and a hard-coded empty `projects` array.
fn project_list_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = project_list::ProjectListArgs { project_id };

    let (patch_value, _warnings, data) = project_list::compute_patch(&prior, &args)
        .expect("default fixture must produce valid project.list data");
    let expected_data = serde_json::to_value(&data)
        .expect("project.list fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "project.list".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings: vec![],
        post_state: prior,
        expected_data,
    }
}

/// Build the canonical `asset.gc` fixture used by [`default_fixtures`].
///
/// `asset.gc` ignores project state, so the prior is just the empty
/// synthetic project and the patch is empty. The single-project scope
/// path is the only one that returns Ok in v1 (the cross-validation
/// matrix sends every other path to an error before reaching data
/// emission); the fixture exercises it with `project_id: Some(_),
/// global: None` and records the empty data envelope.
fn asset_gc_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);

    let args = asset_gc::AssetGcArgs {
        project_id: Some(project_id),
        global: None,
        suppress_orphan_risk: None,
    };

    let (patch_value, _warnings, data) = asset_gc::compute_patch(&prior, &args)
        .expect("default fixture must produce valid asset.gc data");
    let expected_data =
        serde_json::to_value(&data).expect("asset.gc fixture expected_data serializes to Value");

    RecordedEvent {
        verb: "asset.gc".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings: vec![],
        post_state: prior,
        expected_data,
    }
}

/// Construct a minimum-shape [`Project`] suitable as a fixture's prior
/// state. Built via `serde_json::from_value` from a literal so we
/// don't depend on `tests/fixtures/*` (which `src/` cannot
/// `include_str!`) and don't need a `Project::default` impl. Every
/// field matches the schema's required-with-defaults shape used in
/// `tests/fixtures/empty_project_create.json`.
///
/// Publicly exposed so project-agnostic, read-only verbs
/// (e.g. `project.list`) can be invoked through the `Verb` trait from
/// outside this crate (CLI, MCP, HTTP surfaces) without each surface
/// reinventing a literal `Project` JSON.
///
/// # Panics
///
/// Panics if the embedded literal ever drifts from the [`Project`]
/// schema — a `cargo test`-time guarantee, not a runtime concern.
#[must_use]
pub fn synthetic_empty_project(project_id: verbreel_types::ProjectId) -> Project {
    let raw = json!({
        "id": project_id.to_string(),
        "schema_version": crate::project::SCHEMA_VERSION,
        "tick_rate_hz": verbreel_types::TICK_RATE_HZ,
        "name": "default-fixture",
        "created_at": "2026-05-24T00:00:00Z",
        "updated_at": "2026-05-24T00:00:00Z",
        "canvas": {
            "width": 1080,
            "height": 1920,
            "background": "#000000ff",
            "pixel_aspect_num": 1,
            "pixel_aspect_den": 1
        },
        "fps_num": 30,
        "fps_den": 1,
        "duration_tk": 0,
        "tracks": [],
        "assets": [],
        "markers": [],
        "metadata": {},
        "last_saved_event_id": null,
        "trackers": []
    });
    serde_json::from_value(raw).expect("synthetic empty project literal matches the Project schema")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reconstructor::validate_reconstructors;

    #[test]
    #[allow(clippy::too_many_lines)]
    fn default_registry_and_fixtures_pass_the_gate() {
        let registry = default_registry();
        let fixtures = default_fixtures();
        let report = validate_reconstructors(&registry, &fixtures)
            .expect("default_registry + default_fixtures must clear the §0.8 gate");
        assert_eq!(report.fixtures_run, fixtures.len());
        // `verbs_checked` is sort_unstable-then-dedup'd inside the
        // validator; alphabetical order is the published contract.
        assert_eq!(
            report.verbs_checked,
            vec![
                "asset.gc",
                "asset.import",
                "asset.list",
                "asset.probe",
                "asset.relink",
                "asset.remove",
                "asset.verify",
                "audio.analyze",
                "audio.denoise",
                "audio.detect_beats",
                "audio.detect_silence",
                "audio.extract",
                "audio.fade",
                "audio.volume",
                "caption.auto_generate",
                "caption.burn_in",
                "caption.burn_off",
                "caption.edit",
                "caption.export",
                "caption.translate",
                "clip.add",
                "clip.delete",
                "clip.duplicate",
                "clip.list",
                "clip.lock",
                "clip.move",
                "clip.rename",
                "clip.reverse",
                "clip.set_blend_mode",
                "clip.set_fade",
                "clip.set_mask",
                "clip.set_opacity",
                "clip.set_speed",
                "clip.set_speed_curve",
                "clip.set_transform",
                "clip.set_volume",
                "clip.split",
                "clip.trim",
                "clip.unlink",
                "compound.create",
                "compound.edit_in_place",
                "compound.expand",
                "compound.flatten",
                "describe",
                "effect.add",
                "effect.list_available",
                "effect.remove",
                "effect.reorder",
                "effect.set_param",
                "effect.toggle",
                "font.list",
                "help",
                "keyframe.add",
                "keyframe.list",
                "keyframe.remove",
                "keyframe.set",
                "list_capabilities",
                "marker.add",
                "marker.list",
                "marker.remove",
                "marker.set",
                "preview.frame",
                "preview.session.close",
                "preview.session.create",
                "preview.session.frame_at",
                "preview.session.pause",
                "preview.session.play",
                "preview.session.seek",
                "preview.thumbnail",
                "preview.waveform",
                "project.info",
                "project.list",
                "project.rename",
                "project.set_canvas",
                "project.set_fps",
                "project.set_metadata",
                "render.cancel",
                "render.list_presets",
                "render.queue.add",
                "render.queue.cancel",
                "render.queue.clear",
                "render.queue.list",
                "render.queue.status",
                "render.start",
                "render.status",
                "schema",
                "stock.describe",
                "stock.import",
                "stock.list_providers",
                "stock.search",
                "template.apply",
                "template.describe",
                "template.from_project",
                "template.install",
                "template.list",
                "template.uninstall",
                "text.add",
                "text.animate",
                "text.edit",
                "text.style",
                "timeline.diff",
                "timeline.history",
                "timeline.redo",
                "timeline.snapshot",
                "timeline.undo",
                "track.add",
                "track.hide",
                "track.lock",
                "track.mute",
                "track.remove",
                "track.rename",
                "track.reorder",
                "track.set_pan",
                "track.set_volume",
                "track.solo",
                "tracker.apply",
                "tracker.create",
                "tracker.list",
                "tracker.remove",
                "tracker.run",
                "validate_command",
            ]
        );
    }

    #[test]
    fn default_fixtures_count_matches_default_registry_verbs() {
        // Every verb in default_registry() must have at least one
        // fixture in default_fixtures() — that's the construction
        // contract documented at the function level.
        let registry = default_registry();
        let fixtures = default_fixtures();
        let fixture_verbs: std::collections::HashSet<&str> =
            fixtures.iter().map(|f| f.verb.as_str()).collect();
        for verb in registry.verbs() {
            assert!(
                fixture_verbs.contains(verb),
                "verb `{verb}` is in default_registry but has no fixture in default_fixtures"
            );
        }
    }
}
