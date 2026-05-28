//! Tests for `project.save` (§2.3).
//!
//! The verb is `#[cfg(feature = "native")]`-gated because it touches
//! the filesystem; the whole test file follows.
//!
//! Tests use [`tempfile::TempDir`] for the project root so no real
//! `~/.verbreel/` is touched. Fixture projects are built directly via
//! [`ProjectStore::create_with_registry`] (no dependency on the
//! still-blocked `project.create` verb).

#![cfg(feature = "native")]

use std::fs;

use serde_json::json;
use tempfile::TempDir;
use verbreel_events::Timestamp;
use verbreel_state::{
    Canvas, ProjectSaveArgs, ProjectSaveError, ProjectStore, TICK_RATE_HZ, Track, TrackKind,
    default_fixtures, default_registry, project_save,
};
use verbreel_types::{ProjectId, Tick, TrackId};

const SCHEMA_VERSION: &str = "1.0.0";

// --- Helpers -------------------------------------------------------------

/// Build a minimal valid `Project` value with two seeded tracks and the
/// schema_version this engine accepts. Tests that need richer content
/// (assets, mutations) wrap this and mutate the returned store.
fn minimal_project() -> verbreel_state::Project {
    verbreel_state::Project {
        id: ProjectId::now(),
        schema_version: SCHEMA_VERSION.to_string(),
        tick_rate_hz: TICK_RATE_HZ,
        name: "test".to_string(),
        created_at: Timestamp::parse("2026-05-27T00:00:00Z").unwrap(),
        updated_at: Timestamp::parse("2026-05-27T00:00:00Z").unwrap(),
        canvas: Canvas {
            width: 1080,
            height: 1920,
            background: verbreel_state::Color::new("#000000ff".to_string())
                .expect("valid color literal"),
            pixel_aspect_num: 1,
            pixel_aspect_den: 1,
        },
        fps_num: 30,
        fps_den: 1,
        duration_tk: Tick::new(0),
        tracks: vec![
            Track {
                id: TrackId::now(),
                kind: TrackKind::Video,
                name: "Video 1".to_string(),
                clips: Vec::new(),
                muted: false,
                solo: false,
                locked: false,
                hidden: false,
                volume: 1.0,
                pan: 0.0,
                effects: Vec::new(),
            },
            Track {
                id: TrackId::now(),
                kind: TrackKind::Audio,
                name: "Audio 1".to_string(),
                clips: Vec::new(),
                muted: false,
                solo: false,
                locked: false,
                hidden: false,
                volume: 1.0,
                pan: 0.0,
                effects: Vec::new(),
            },
        ],
        assets: Vec::new(),
        markers: Vec::new(),
        metadata: serde_json::Map::new(),
        last_saved_event_id: None,
        trackers: Vec::new(),
    }
}

/// Build a fresh project on disk at `root` via the lifecycle facade.
/// Returns the live store (caller drops it to release the lock).
fn fresh_project_at(root: &std::path::Path) -> ProjectStore {
    let project = minimal_project();
    ProjectStore::create_with_registry(root, project, &default_registry(), &default_fixtures())
        .expect("fresh project must create cleanly")
}

/// Apply one synthetic mutation through the raw `mutate()` API so the
/// store's `last_applied_event_id` advances. Returns nothing because
/// tests inspect the store-side state directly.
fn apply_one_mutation(store: &mut ProjectStore, new_name: &str) {
    let patch_json = serde_json::json!([
        { "op": "replace", "path": "/name", "value": new_name }
    ]);
    let patch: json_patch::Patch = serde_json::from_value(patch_json).unwrap();
    store
        .mutate("test.rename", json!({}), &patch, None)
        .expect("mutate must succeed");
}

// --- 1. Args deserialization --------------------------------------------

#[test]
fn args_deserialize_from_object_with_project_id() {
    let pid = ProjectId::now();
    let raw = json!({ "project_id": pid.to_string() });
    let args: ProjectSaveArgs = serde_json::from_value(raw).unwrap();
    assert_eq!(args.project_id, pid);
}

#[test]
fn args_reject_unknown_fields() {
    let pid = ProjectId::now();
    let raw = json!({
        "project_id": pid.to_string(),
        "force": true,
    });
    let err = serde_json::from_value::<ProjectSaveArgs>(raw).unwrap_err();
    assert!(
        err.to_string().contains("unknown field"),
        "expected deny_unknown_fields error, got: {err}"
    );
}

#[test]
fn args_reject_missing_project_id() {
    let raw = json!({});
    let err = serde_json::from_value::<ProjectSaveArgs>(raw).unwrap_err();
    assert!(
        err.to_string().contains("project_id"),
        "expected missing-field error naming project_id, got: {err}"
    );
}

#[test]
fn args_reject_malformed_project_id() {
    let raw = json!({ "project_id": "not-a-uuid" });
    let err = serde_json::from_value::<ProjectSaveArgs>(raw).unwrap_err();
    assert!(
        !err.to_string().is_empty(),
        "malformed UUID must surface a serde error"
    );
}

// --- 2. Happy path ------------------------------------------------------

#[test]
fn save_returns_path_pointing_at_project_json() {
    let root_dir = TempDir::new().unwrap();
    let mut store = fresh_project_at(root_dir.path());
    let args = ProjectSaveArgs {
        project_id: store.project().id,
    };

    let data = project_save(&mut store, &args).expect("happy-path save must succeed");

    assert_eq!(data.path, root_dir.path().join("project.json"));
}

#[test]
fn save_returns_nonzero_bytes_written() {
    let root_dir = TempDir::new().unwrap();
    let mut store = fresh_project_at(root_dir.path());
    let args = ProjectSaveArgs {
        project_id: store.project().id,
    };

    let data = project_save(&mut store, &args).unwrap();
    assert!(data.bytes_written > 0, "snapshot bytes must be nonzero");
}

#[test]
fn save_bytes_written_matches_actual_file_size() {
    let root_dir = TempDir::new().unwrap();
    let mut store = fresh_project_at(root_dir.path());
    let args = ProjectSaveArgs {
        project_id: store.project().id,
    };

    let data = project_save(&mut store, &args).unwrap();
    let actual_size = fs::metadata(&data.path).unwrap().len();
    assert_eq!(data.bytes_written, actual_size);
}

// --- 3. last_saved_event_id semantics -----------------------------------

#[test]
fn save_against_freshly_created_project_leaves_last_saved_event_id_null() {
    let root_dir = TempDir::new().unwrap();
    let mut store = fresh_project_at(root_dir.path());
    let args = ProjectSaveArgs {
        project_id: store.project().id,
    };

    project_save(&mut store, &args).unwrap();

    // No mutations have run, so the store's last_applied_event_id is
    // None and the snapshot's last_saved_event_id must stay None.
    assert!(store.project().last_saved_event_id.is_none());
    assert!(store.last_applied_event_id().is_none());
}

#[test]
fn save_after_one_mutation_bumps_last_saved_event_id_to_that_event() {
    let root_dir = TempDir::new().unwrap();
    let mut store = fresh_project_at(root_dir.path());
    apply_one_mutation(&mut store, "after-mutate");

    let applied = store
        .last_applied_event_id()
        .expect("one mutation must register a last_applied_event_id");

    let args = ProjectSaveArgs {
        project_id: store.project().id,
    };
    project_save(&mut store, &args).unwrap();

    assert_eq!(store.project().last_saved_event_id, Some(applied));
}

#[test]
fn save_after_multiple_mutations_records_most_recent_event_id() {
    let root_dir = TempDir::new().unwrap();
    let mut store = fresh_project_at(root_dir.path());

    apply_one_mutation(&mut store, "mutate-1");
    apply_one_mutation(&mut store, "mutate-2");
    apply_one_mutation(&mut store, "mutate-3");

    let applied = store.last_applied_event_id().unwrap();

    let args = ProjectSaveArgs {
        project_id: store.project().id,
    };
    project_save(&mut store, &args).unwrap();

    assert_eq!(store.project().last_saved_event_id, Some(applied));
}

#[test]
fn second_save_with_no_new_events_round_trips_without_changing_last_saved_event_id() {
    let root_dir = TempDir::new().unwrap();
    let mut store = fresh_project_at(root_dir.path());
    apply_one_mutation(&mut store, "after-mutate");

    let args = ProjectSaveArgs {
        project_id: store.project().id,
    };
    project_save(&mut store, &args).unwrap();
    let first_id = store.project().last_saved_event_id;
    assert!(first_id.is_some());

    // Second save with no intervening mutations must leave the field
    // unchanged. The store's last_applied_event_id is the same id, so
    // the lifecycle layer re-assigns the same value.
    project_save(&mut store, &args).unwrap();
    assert_eq!(store.project().last_saved_event_id, first_id);
}

// --- 4. On-disk side effect ---------------------------------------------

#[test]
fn save_writes_a_well_formed_project_json_file() {
    let root_dir = TempDir::new().unwrap();
    let mut store = fresh_project_at(root_dir.path());
    let args = ProjectSaveArgs {
        project_id: store.project().id,
    };

    let data = project_save(&mut store, &args).unwrap();
    let bytes = fs::read(&data.path).unwrap();
    let parsed: serde_json::Value =
        serde_json::from_slice(&bytes).expect("written file must be valid JSON");
    assert!(parsed.get("id").is_some());
    assert!(parsed.get("schema_version").is_some());
}

#[test]
fn save_persists_last_saved_event_id_to_disk() {
    let root_dir = TempDir::new().unwrap();
    let mut store = fresh_project_at(root_dir.path());
    apply_one_mutation(&mut store, "persisted");

    let args = ProjectSaveArgs {
        project_id: store.project().id,
    };
    let data = project_save(&mut store, &args).unwrap();

    let bytes = fs::read(&data.path).unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let on_disk = parsed
        .get("last_saved_event_id")
        .expect("project.json must include last_saved_event_id field");
    assert!(
        on_disk.is_string(),
        "last_saved_event_id after a mutation must be a string, got: {on_disk:?}"
    );
}

#[test]
fn save_does_not_leak_orphan_tmp_files_after_success() {
    let root_dir = TempDir::new().unwrap();
    let mut store = fresh_project_at(root_dir.path());
    let args = ProjectSaveArgs {
        project_id: store.project().id,
    };

    project_save(&mut store, &args).unwrap();

    // tempfile::NamedTempFile::persist() consumes the temp file via
    // POSIX rename. Walk the root and ensure no `*.tmp*` siblings remain.
    let entries = fs::read_dir(root_dir.path()).unwrap();
    for entry in entries {
        let name = entry.unwrap().file_name();
        let s = name.to_string_lossy();
        assert!(
            !s.contains(".tmp"),
            "found leftover temp file after save: {s}"
        );
    }
}

// --- 5. project_id mismatch (PROJECT_NOT_FOUND) -------------------------

#[test]
fn save_rejects_mismatched_project_id_with_project_not_found() {
    let root_dir = TempDir::new().unwrap();
    let mut store = fresh_project_at(root_dir.path());
    let loaded_id = store.project().id;
    let bogus_id = ProjectId::now();
    assert_ne!(loaded_id, bogus_id);

    let args = ProjectSaveArgs {
        project_id: bogus_id,
    };
    let err = project_save(&mut store, &args).unwrap_err();

    match err {
        ProjectSaveError::ProjectNotFound { requested, loaded } => {
            assert_eq!(requested, bogus_id);
            assert_eq!(loaded, loaded_id);
        }
        other => panic!("expected ProjectNotFound, got: {other:?}"),
    }
}

#[test]
fn save_with_mismatched_id_does_not_touch_disk() {
    let root_dir = TempDir::new().unwrap();
    let mut store = fresh_project_at(root_dir.path());

    // Capture the initial project.json mtime + size so we can verify
    // the failed save did not rewrite it.
    let target = root_dir.path().join("project.json");
    let before_meta = fs::metadata(&target).unwrap();
    let before_size = before_meta.len();

    let args = ProjectSaveArgs {
        project_id: ProjectId::now(),
    };
    let _ = project_save(&mut store, &args).unwrap_err();

    let after_meta = fs::metadata(&target).unwrap();
    assert_eq!(
        after_meta.len(),
        before_size,
        "size changed on rejected save"
    );
}

// --- 6. IO error path ---------------------------------------------------

#[test]
fn save_surfaces_io_error_when_project_root_disappears() {
    // Build a store at a temp root, then yank the root before save.
    // The atomic-rename path will fail to create the tempfile inside
    // the now-missing directory, surfacing as ProjectSaveError::Io.
    let root_dir = TempDir::new().unwrap();
    let mut store = fresh_project_at(root_dir.path());
    let args = ProjectSaveArgs {
        project_id: store.project().id,
    };

    // Drop the dir contents + the dir itself. tempfile::TempDir's
    // close() would also remove .verbreel/events.jsonl (which the
    // store still has an open file handle on — POSIX deletion is by
    // name, not by fd, so the open lock keeps working). The save's
    // tempfile creation against the missing path is what we want to
    // exercise.
    let path = root_dir.path().to_path_buf();
    fs::remove_dir_all(&path).expect("test setup: remove root");

    let err = project_save(&mut store, &args).unwrap_err();
    match err {
        ProjectSaveError::Io(_) => {}
        other => panic!("expected Io error, got: {other:?}"),
    }
}

// --- 7. ProjectId arg is preserved verbatim in the error -------------

#[test]
fn project_not_found_error_carries_both_ids() {
    let root_dir = TempDir::new().unwrap();
    let mut store = fresh_project_at(root_dir.path());
    let loaded = store.project().id;
    let requested = ProjectId::now();

    let args = ProjectSaveArgs {
        project_id: requested,
    };
    let err = project_save(&mut store, &args).unwrap_err();

    let msg = err.to_string();
    assert!(
        msg.contains(&requested.to_string()),
        "error must echo the caller's project_id, got: {msg}"
    );
    assert!(
        msg.contains(&loaded.to_string()),
        "error must echo the store's loaded project_id, got: {msg}"
    );
}

// --- 8. Repeated save round-trip ----------------------------------------

#[test]
fn three_consecutive_saves_against_a_clean_project_all_succeed() {
    let root_dir = TempDir::new().unwrap();
    let mut store = fresh_project_at(root_dir.path());
    let args = ProjectSaveArgs {
        project_id: store.project().id,
    };

    for _ in 0..3 {
        project_save(&mut store, &args).expect("idempotent save must succeed");
    }

    // last_saved_event_id stayed None throughout because no mutations
    // happened between any of the three saves.
    assert!(store.project().last_saved_event_id.is_none());
}

#[test]
fn save_mutate_save_pattern_records_intermediate_event_id() {
    let root_dir = TempDir::new().unwrap();
    let mut store = fresh_project_at(root_dir.path());
    let args = ProjectSaveArgs {
        project_id: store.project().id,
    };

    // 1st save against fresh project: no events, last_saved stays None.
    project_save(&mut store, &args).unwrap();
    assert!(store.project().last_saved_event_id.is_none());

    // Apply one mutation.
    apply_one_mutation(&mut store, "between-saves");
    let applied = store.last_applied_event_id().unwrap();

    // 2nd save: last_saved must catch up to the just-applied event.
    project_save(&mut store, &args).unwrap();
    assert_eq!(store.project().last_saved_event_id, Some(applied));
}
