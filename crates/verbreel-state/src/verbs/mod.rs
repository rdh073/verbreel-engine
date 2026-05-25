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
//! - `spec/commands/marker.md` §13.1 (`marker.add`), §13.2
//!   (`marker.set`), §13.3 (`marker.remove`), and §13.4 (`marker.list`).
//! - `spec/commands/project.md` §2.9 (`project.rename`) and §2.12
//!   (`project.set_metadata`).
//! - `spec/commands/conventions.md` §0.13 (metadata size caps).
//! - `spec/commands/conventions.md` §0.8 (reconstructor purity).

use std::sync::Arc;

use serde_json::{Map, Value, json};

use crate::project::Project;
use crate::reconstructor::{RecordedEvent, VerbRegistry};
use verbreel_types::Tick;

pub mod clip_lock;
pub mod clip_rename;
pub mod clip_set_opacity;
pub mod clip_set_volume;
pub mod marker_add;
pub mod marker_list;
pub mod marker_remove;
pub mod marker_set;
pub mod project_rename;
pub mod project_set_canvas;
pub mod project_set_fps;
pub mod project_set_metadata;
pub mod track_add;
pub mod track_hide;
pub mod track_lock;
pub mod track_mute;
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
/// - `clip.lock` (§5.13)
/// - `clip.rename` (§5.17)
/// - `clip.set_opacity` (§5.10)
/// - `clip.set_volume` (§5.11)
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
    registry.register(Arc::new(track_add::TrackAddVerb)).expect(
        "TrackAddVerb is the ninth registration in \
             default_registry(); cannot collide with prior verbs",
    );
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
}

/// One canonical fixture per verb registered in [`default_registry`].
///
/// Each fixture exercises a non-trivial code path through its verb's
/// reconstructor and pairs with the recorded `expected_data` the
/// reconstructor must reproduce under canonical SHA-256.
///
/// Callers using [`default_registry`] should pair it with this function
/// — the two are validated together at every Verbreel test run and
/// pass the §0.8 startup gate by construction. Callers building custom
/// registries must build their own fixtures.
#[must_use]
pub fn default_fixtures() -> Vec<RecordedEvent> {
    vec![
        project_set_metadata_fixture(),
        project_set_canvas_fixture(),
        project_set_fps_fixture(),
        project_rename_fixture(),
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
        clip_set_opacity_fixture(),
        clip_set_volume_fixture(),
    ]
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

/// Build the canonical `track.add` fixture used by [`default_fixtures`].
///
/// Starts from an empty synthetic project, then inserts a first video
/// track with auto-name `Video 1` at global index `0`.
fn track_add_fixture() -> RecordedEvent {
    let project_id = DEFAULT_FIXTURE_PROJECT_ID
        .parse()
        .expect("DEFAULT_FIXTURE_PROJECT_ID is a hard-coded valid v7");

    let prior = synthetic_empty_project(project_id);
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
/// Starts from the `track_add_fixture()` post-state (which has one video
/// track named `Video 1`) and renames that track to `Main Camera`.
fn track_rename_fixture() -> RecordedEvent {
    let prior = track_add_fixture().post_state;
    let track = prior
        .tracks
        .first()
        .expect("track_add fixture has exactly one track");

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
/// Starts from the `track_add_fixture()` post-state (which has one video
/// track named `Video 1` and an unlocked state) and locks that track.
fn track_lock_fixture() -> RecordedEvent {
    let prior = track_add_fixture().post_state;
    let track = prior
        .tracks
        .first()
        .expect("track_add fixture has exactly one track");

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
/// Starts from the `track_add_fixture()` post-state (which has one video
/// track named `Video 1` and an unmuted state) and mutes that track.
fn track_mute_fixture() -> RecordedEvent {
    let prior = track_add_fixture().post_state;
    let track = prior
        .tracks
        .first()
        .expect("track_add fixture has exactly one track");

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

/// Build the canonical `track.solo` fixture used by [`default_fixtures`].
///
/// Starts from the `track_add_fixture()` post-state (which has one video
/// track named `Video 1` and an unsoloed state) and solos that track.
fn track_solo_fixture() -> RecordedEvent {
    let prior = track_add_fixture().post_state;
    let track = prior
        .tracks
        .first()
        .expect("track_add fixture has exactly one track");

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
/// Starts from the `track_add_fixture()` post-state (which has one video
/// track named `Video 1` and an unhidden state) and hides that track.
fn track_hide_fixture() -> RecordedEvent {
    let prior = track_add_fixture().post_state;
    let track = prior
        .tracks
        .first()
        .expect("track_add fixture has exactly one track");

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
                "clip.lock",
                "clip.rename",
                "clip.set_opacity",
                "clip.set_volume",
                "marker.add",
                "marker.list",
                "marker.remove",
                "marker.set",
                "project.rename",
                "project.set_canvas",
                "project.set_fps",
                "project.set_metadata",
                "track.add",
                "track.hide",
                "track.lock",
                "track.mute",
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
