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

pub mod asset_list;
pub mod caption_edit;
pub mod clip_delete;
pub mod clip_list;
pub mod clip_lock;
pub mod clip_rename;
pub mod clip_set_blend_mode;
pub mod clip_set_fade;
pub mod clip_set_mask;
pub mod clip_set_opacity;
pub mod clip_set_transform;
pub mod clip_set_volume;
pub mod clip_unlink;
pub mod effect_list_available;
pub mod effect_reorder;
pub mod effect_set_param;
pub mod effect_toggle;
pub mod keyframe_add;
pub mod keyframe_list;
pub mod keyframe_remove;
pub mod keyframe_set;
pub mod marker_add;
pub mod marker_list;
pub mod marker_remove;
pub mod marker_set;
pub mod project_rename;
pub mod project_set_canvas;
pub mod project_set_fps;
pub mod project_set_metadata;
pub mod text_animate;
pub mod text_edit;
pub mod text_style;
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
/// - `text.animate` (§7.4)
/// - `text.edit` (§7.2)
/// - `text.style` (§7.3)
/// - `caption.edit` (§10.2, §7.2 alias)
/// - `keyframe.add` (§8.1)
/// - `keyframe.list` (§8.4)
/// - `keyframe.remove` (§8.2)
/// - `keyframe.set` (§8.3)
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
    registry
        .register(Arc::new(clip_rename::ClipRenameVerb))
        .expect(
            "ClipRenameVerb is the nineteenth registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(clip_set_opacity::ClipSetOpacityVerb))
        .expect(
            "ClipSetOpacityVerb is the twentieth registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(clip_set_volume::ClipSetVolumeVerb))
        .expect(
            "ClipSetVolumeVerb is the twenty-first registration in \
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
    registry
        .register(Arc::new(clip_delete::ClipDeleteVerb))
        .expect(
            "ClipDeleteVerb is the thirty-ninth registration in \
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
        .register(Arc::new(effect_list_available::EffectListAvailableVerb))
        .expect(
            "EffectListAvailableVerb is the twenty-seventh registration in \
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
        .register(Arc::new(asset_list::AssetListVerb))
        .expect(
            "AssetListVerb is the twenty-eighth registration in \
             default_registry(); cannot collide with prior verbs",
        );
    registry
        .register(Arc::new(keyframe_list::KeyframeListVerb))
        .expect(
            "KeyframeListVerb is the twenty-ninth registration in \
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
        .register(Arc::new(caption_edit::CaptionEditVerb))
        .expect(
            "CaptionEditVerb is the thirty-first registration in \
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
pub fn default_fixtures() -> Vec<RecordedEvent> {
    vec![
        project_set_metadata_fixture(),
        project_set_canvas_fixture(),
        project_set_fps_fixture(),
        project_rename_fixture(),
        text_animate_fixture(),
        text_edit_fixture(),
        caption_edit_fixture(),
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
        clip_rename_fixture(),
        clip_set_blend_mode_fixture(),
        clip_set_fade_fixture(),
        clip_set_mask_fixture(),
        clip_set_transform_fixture(),
        clip_set_opacity_fixture(),
        clip_set_volume_fixture(),
        clip_delete_fixture(),
        clip_list_fixture(),
        clip_unlink_fixture(),
        effect_list_available_fixture(),
        effect_reorder_fixture(),
        effect_set_param_fixture(),
        effect_toggle_fixture(),
        asset_list_fixture(),
        keyframe_add_fixture(),
        keyframe_list_fixture(),
        keyframe_remove_fixture(),
        keyframe_set_fixture(),
        track_remove_fixture(),
    ]
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

/// Construct a minimum-shape [`Project`] suitable as a fixture's prior
/// state. Built via `serde_json::from_value` from a literal so we
/// don't depend on `tests/fixtures/*` (which `src/` cannot
/// `include_str!`) and don't need a `Project::default` impl. Every
/// field matches the schema's required-with-defaults shape used in
/// `tests/fixtures/empty_project_create.json`.
fn synthetic_empty_project(project_id: verbreel_types::ProjectId) -> Project {
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
                "asset.list",
                "caption.edit",
                "clip.delete",
                "clip.list",
                "clip.lock",
                "clip.rename",
                "clip.set_blend_mode",
                "clip.set_fade",
                "clip.set_mask",
                "clip.set_opacity",
                "clip.set_transform",
                "clip.set_volume",
                "clip.unlink",
                "effect.list_available",
                "effect.reorder",
                "effect.set_param",
                "effect.toggle",
                "keyframe.add",
                "keyframe.list",
                "keyframe.remove",
                "keyframe.set",
                "marker.add",
                "marker.list",
                "marker.remove",
                "marker.set",
                "project.rename",
                "project.set_canvas",
                "project.set_fps",
                "project.set_metadata",
                "text.animate",
                "text.edit",
                "text.style",
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
