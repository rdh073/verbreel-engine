//! Per-verb modules. Each verb declares:
//!
//! - its args / data / error types,
//! - a `compute_patch()` freestanding helper (pure — no I/O, no clock,
//!   no RNG),
//! - a `*Verb` impl of [`crate::reconstructor::Verb`] (also pure per
//!   §0.8) — owns both the forward path (`compute_patch`) and the
//!   replay path (`reconstruct`).
//!
//! Verbs land one at a time. `project.set_metadata` (§2.12) is the
//! first — proves the reconstructor-purity contract end-to-end and,
//! since Slice B3, is also the first verb routed through
//! `ProjectStore::mutate_via_verb`.
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
//! - `spec/commands/project.md` §2.12 (`project.set_metadata`).
//! - `spec/commands/conventions.md` §0.13 (metadata size caps).
//! - `spec/commands/conventions.md` §0.8 (reconstructor purity).

use std::sync::Arc;

use serde_json::{Map, Value, json};

use crate::project::Project;
use crate::reconstructor::{RecordedEvent, VerbRegistry};

pub mod project_set_canvas;
pub mod project_set_fps;
pub mod project_set_metadata;

/// Synthetic `UUIDv7` used as the `project_id` in [`default_fixtures`].
/// Hard-coded so the fixture is deterministic — `ProjectId::now()` would
/// pull from the wall clock and the gate is a startup-time, not
/// runtime, validation. The string is a valid v7 (version nibble `7`,
/// variant nibble in `8..=b`) but otherwise carries no production
/// meaning.
const DEFAULT_FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-0000deadbeef";

/// The canonical set of verbs shipped by this engine build.
///
/// Currently registers exactly one verb:
/// [`ProjectSetMetadataVerb`] (§2.12). Subsequent verbs land here so
/// every consumer that wants "the stock kernel verb set"
/// ([`crate::lifecycle::ProjectStore::create_with_registry`] /
/// [`crate::lifecycle::ProjectStore::open_with_registry`] /
/// [`crate::lifecycle::ProjectStore::mutate_via_verb`], the future
/// args-validator, etc.) picks them up automatically.
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
    ]
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
                "project.set_canvas",
                "project.set_fps",
                "project.set_metadata"
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
