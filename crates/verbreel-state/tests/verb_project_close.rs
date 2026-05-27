//! Tests for `project.close` (§2.5).
//!
//! The verb is `#[cfg(feature = "native")]`-gated because it touches
//! the filesystem and consumes a [`ProjectStore`] (which owns a live
//! `fs4::flock`); the whole test file follows.
//!
//! Tests use [`tempfile::TempDir`] for the project root so no real
//! `~/.verbreel/` is touched. Fixture projects are built directly via
//! [`ProjectStore::create_with_registry`] (no dependency on the still-
//! blocked `project.create` verb).

#![cfg(feature = "native")]

use std::fs;

use serde_json::json;
use tempfile::TempDir;
use verbreel_state::{
    Canvas, ProjectCloseArgs, ProjectCloseError, ProjectStore, TICK_RATE_HZ, Track, TrackKind,
    default_fixtures, default_registry, project_close,
};
use verbreel_types::{ProjectId, Tick, TrackId};

const SCHEMA_VERSION: &str = "1.0.0";

// --- Helpers -------------------------------------------------------------

/// Build a minimal valid `Project` value with two seeded tracks and the
/// schema_version this engine accepts.
fn minimal_project() -> verbreel_state::Project {
    verbreel_state::Project {
        id: ProjectId::now(),
        schema_version: SCHEMA_VERSION.to_string(),
        tick_rate_hz: TICK_RATE_HZ,
        name: "test".to_string(),
        created_at: "2026-05-27T00:00:00Z".to_string(),
        updated_at: "2026-05-27T00:00:00Z".to_string(),
        canvas: Canvas {
            width: 1080,
            height: 1920,
            background: "#000000ff".to_string(),
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
/// Returns the live store (caller passes it to `project_close` or drops
/// it to release the lock).
fn fresh_project_at(root: &std::path::Path) -> ProjectStore {
    let project = minimal_project();
    ProjectStore::create_with_registry(root, project, &default_registry(), &default_fixtures())
        .expect("fresh project must create cleanly")
}

/// Apply one synthetic mutation through the raw `mutate()` API so the
/// store's `last_applied_event_id` advances.
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
fn args_deserialize_with_project_id_only_defaults_flags_to_false() {
    let pid = ProjectId::now();
    let raw = json!({ "project_id": pid.to_string() });
    let args: ProjectCloseArgs = serde_json::from_value(raw).unwrap();
    assert_eq!(args.project_id, pid);
    assert!(!args.save, "save default must be false");
    assert!(!args.cancel_jobs, "cancel_jobs default must be false");
}

#[test]
fn args_deserialize_with_save_true_and_cancel_jobs_true() {
    let pid = ProjectId::now();
    let raw = json!({
        "project_id": pid.to_string(),
        "save": true,
        "cancel_jobs": true,
    });
    let args: ProjectCloseArgs = serde_json::from_value(raw).unwrap();
    assert!(args.save);
    assert!(args.cancel_jobs);
}

#[test]
fn args_reject_unknown_fields() {
    let pid = ProjectId::now();
    let raw = json!({
        "project_id": pid.to_string(),
        "force": true,
    });
    let err = serde_json::from_value::<ProjectCloseArgs>(raw).unwrap_err();
    assert!(
        err.to_string().contains("unknown field"),
        "expected deny_unknown_fields error, got: {err}"
    );
}

#[test]
fn args_reject_missing_project_id() {
    let raw = json!({});
    let err = serde_json::from_value::<ProjectCloseArgs>(raw).unwrap_err();
    assert!(
        err.to_string().contains("project_id"),
        "expected missing-field error naming project_id, got: {err}"
    );
}

#[test]
fn args_reject_malformed_project_id() {
    let raw = json!({ "project_id": "not-a-uuid" });
    let err = serde_json::from_value::<ProjectCloseArgs>(raw).unwrap_err();
    assert!(
        !err.to_string().is_empty(),
        "malformed UUID must surface a serde error"
    );
}

#[test]
fn args_reject_wrong_type_for_save_flag() {
    let pid = ProjectId::now();
    let raw = json!({ "project_id": pid.to_string(), "save": "yes" });
    let err = serde_json::from_value::<ProjectCloseArgs>(raw).unwrap_err();
    assert!(
        !err.to_string().is_empty(),
        "non-bool save must surface a serde error"
    );
}

// --- 2. Happy path: save=false ------------------------------------------

#[test]
fn close_without_save_returns_closed_true() {
    let root_dir = TempDir::new().unwrap();
    let store = fresh_project_at(root_dir.path());
    let args = ProjectCloseArgs {
        project_id: store.project().id,
        save: false,
        cancel_jobs: false,
    };

    let data = project_close(store, &args).expect("close must succeed");

    assert!(data.closed, "closed must be true on Ok path");
}

#[test]
fn close_without_save_returns_empty_canceled_job_ids() {
    let root_dir = TempDir::new().unwrap();
    let store = fresh_project_at(root_dir.path());
    let args = ProjectCloseArgs {
        project_id: store.project().id,
        save: false,
        cancel_jobs: false,
    };

    let data = project_close(store, &args).unwrap();
    assert!(
        data.canceled_job_ids.is_empty(),
        "v1 floor: canceled_job_ids must be empty"
    );
}

#[test]
fn close_without_save_returns_empty_cancel_failures() {
    let root_dir = TempDir::new().unwrap();
    let store = fresh_project_at(root_dir.path());
    let args = ProjectCloseArgs {
        project_id: store.project().id,
        save: false,
        cancel_jobs: false,
    };

    let data = project_close(store, &args).unwrap();
    assert!(
        data.cancel_failures.is_empty(),
        "v1 floor: cancel_failures must be empty"
    );
}

// --- 3. Flock release ----------------------------------------------------

#[test]
fn close_releases_flock_so_subsequent_open_succeeds() {
    let root_dir = TempDir::new().unwrap();
    let store = fresh_project_at(root_dir.path());
    let project_id = store.project().id;

    let args = ProjectCloseArgs {
        project_id,
        save: false,
        cancel_jobs: false,
    };
    project_close(store, &args).expect("close must succeed");

    // The flock on `<root>/.verbreel/events.jsonl` should be released
    // — a fresh open against the same root must not return
    // `LockHeldByAnotherProcess`.
    let reopened = ProjectStore::open(root_dir.path()).expect("re-open must succeed after close");
    assert_eq!(reopened.project().id, project_id);
}

#[test]
fn close_with_save_true_also_releases_flock() {
    let root_dir = TempDir::new().unwrap();
    let store = fresh_project_at(root_dir.path());
    let project_id = store.project().id;

    let args = ProjectCloseArgs {
        project_id,
        save: true,
        cancel_jobs: false,
    };
    project_close(store, &args).expect("close+save must succeed");

    let reopened =
        ProjectStore::open(root_dir.path()).expect("re-open must succeed after close+save");
    assert_eq!(reopened.project().id, project_id);
}

// --- 4. Happy path: save=true (save fires before release) ----------------

#[test]
fn close_with_save_persists_last_applied_event_id_to_disk() {
    let root_dir = TempDir::new().unwrap();
    let mut store = fresh_project_at(root_dir.path());
    apply_one_mutation(&mut store, "renamed-before-close");
    let applied = store
        .last_applied_event_id()
        .expect("one mutation must register a last_applied_event_id");

    let args = ProjectCloseArgs {
        project_id: store.project().id,
        save: true,
        cancel_jobs: false,
    };
    project_close(store, &args).expect("close+save must succeed");

    // Read project.json directly off disk — must reflect the mutation
    // and have last_saved_event_id pointing at the applied event.
    let bytes = fs::read(root_dir.path().join("project.json")).unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        parsed.get("name").and_then(|v| v.as_str()),
        Some("renamed-before-close"),
        "save must persist the in-memory mutation before close"
    );
    let on_disk = parsed
        .get("last_saved_event_id")
        .and_then(|v| v.as_str())
        .expect("last_saved_event_id must be a string after a mutation");
    assert_eq!(on_disk, applied.to_string());
}

#[test]
fn close_without_save_does_not_persist_post_create_mutations() {
    let root_dir = TempDir::new().unwrap();
    let mut store = fresh_project_at(root_dir.path());

    // create_with_registry already wrote project.json with name="test"
    // (its embedded save step). Apply a mutation but DO NOT save before
    // close.
    apply_one_mutation(&mut store, "renamed-then-discarded");

    let args = ProjectCloseArgs {
        project_id: store.project().id,
        save: false,
        cancel_jobs: false,
    };
    project_close(store, &args).expect("close-without-save must succeed");

    // On-disk project.json still carries the create-time name because
    // the mutation was never saved.
    let bytes = fs::read(root_dir.path().join("project.json")).unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        parsed.get("name").and_then(|v| v.as_str()),
        Some("test"),
        "close without save must not persist in-memory mutations"
    );
}

// --- 5. cancel_jobs accepted-and-ignored in v1 ---------------------------

#[test]
fn cancel_jobs_true_is_accepted_and_does_not_change_outcome() {
    let root_dir = TempDir::new().unwrap();
    let store = fresh_project_at(root_dir.path());
    let args = ProjectCloseArgs {
        project_id: store.project().id,
        save: false,
        cancel_jobs: true,
    };

    let data = project_close(store, &args).expect("cancel_jobs is accepted in v1");
    assert!(data.closed);
    assert!(data.canceled_job_ids.is_empty());
    assert!(data.cancel_failures.is_empty());
}

// --- 6. project_id mismatch (PROJECT_NOT_FOUND) -------------------------

#[test]
fn close_rejects_mismatched_project_id_with_project_not_found() {
    let root_dir = TempDir::new().unwrap();
    let store = fresh_project_at(root_dir.path());
    let loaded_id = store.project().id;
    let bogus_id = ProjectId::now();
    assert_ne!(loaded_id, bogus_id);

    let args = ProjectCloseArgs {
        project_id: bogus_id,
        save: false,
        cancel_jobs: false,
    };
    let (_returned_store, err) = project_close(store, &args).unwrap_err();

    match err {
        ProjectCloseError::ProjectNotFound { requested, loaded } => {
            assert_eq!(requested, bogus_id);
            assert_eq!(loaded, loaded_id);
        }
        other => panic!("expected ProjectNotFound, got: {other:?}"),
    }
}

#[test]
fn project_not_found_error_carries_both_ids() {
    let root_dir = TempDir::new().unwrap();
    let store = fresh_project_at(root_dir.path());
    let loaded = store.project().id;
    let requested = ProjectId::now();

    let args = ProjectCloseArgs {
        project_id: requested,
        save: false,
        cancel_jobs: false,
    };
    let (_returned_store, err) = project_close(store, &args).unwrap_err();

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

#[test]
fn rejected_close_retains_flock_via_returned_store() {
    // When `project.close` rejects (id mismatch or save failure), the
    // store is returned to the caller inside the error tuple, so the
    // flock stays held. A concurrent `ProjectStore::open` against the
    // same root must FAIL to acquire the lock — the §2.5 retain-on-
    // failure contract.
    let root_dir = TempDir::new().unwrap();
    let store = fresh_project_at(root_dir.path());
    let args = ProjectCloseArgs {
        project_id: ProjectId::now(),
        save: false,
        cancel_jobs: false,
    };
    let (returned_store, _err) = project_close(store, &args).unwrap_err();

    // Flock still held by `returned_store` — re-open must fail.
    let second = ProjectStore::open(root_dir.path());
    assert!(
        second.is_err(),
        "flock must remain held while the rejected store is alive; got Ok"
    );

    // After dropping the returned store, the flock releases and re-open succeeds.
    drop(returned_store);
    let _reopened = ProjectStore::open(root_dir.path())
        .expect("re-open must succeed after returned store is dropped");
}

#[test]
fn close_with_save_failure_keeps_flock_held() {
    // §2.5 retain-on-failure: when `save: true` is set and the pre-
    // close save raises an IO error, `close()` must abort, return the
    // store to the caller, and keep the flock held so the caller can
    // retry after addressing the IO issue.
    //
    // Trigger the save failure by chmod 000 on the project root so
    // the atomic write in `project_save::save` fails on temp-file
    // create. The actual error variant maps to SaveIo or SaveLifecycle
    // depending on lifecycle layer behavior; the contract under test
    // is store retention, not the exact variant.
    use std::os::unix::fs::PermissionsExt;

    let root_dir = TempDir::new().unwrap();
    let store = fresh_project_at(root_dir.path());
    let project_id = store.project().id;

    // Drop write permission to force the atomic save to fail.
    let mut perms = std::fs::metadata(root_dir.path()).unwrap().permissions();
    let original = perms.mode();
    perms.set_mode(0o500); // r-x for owner; no write
    std::fs::set_permissions(root_dir.path(), perms).unwrap();

    let args = ProjectCloseArgs {
        project_id,
        save: true,
        cancel_jobs: false,
    };
    let result = project_close(store, &args);

    // Restore perms regardless of test outcome so TempDir cleanup works.
    let mut restore = std::fs::metadata(root_dir.path()).unwrap().permissions();
    restore.set_mode(original);
    std::fs::set_permissions(root_dir.path(), restore).unwrap();

    let (returned_store, err) = result.expect_err("save failure must abort close");

    // Error must be a save-class variant, not project-not-found.
    assert!(
        matches!(
            err,
            ProjectCloseError::SaveIo(_) | ProjectCloseError::SaveLifecycle(_)
        ),
        "save failure must map to SaveIo or SaveLifecycle, got: {err:?}"
    );

    // Caller can still observe the project via the returned store —
    // flock is held, project state is intact, retry is possible.
    assert_eq!(returned_store.project().id, project_id);
}

// --- 7. Data envelope serialization shape -------------------------------

#[test]
fn data_envelope_serializes_with_spec_field_names() {
    let root_dir = TempDir::new().unwrap();
    let store = fresh_project_at(root_dir.path());
    let args = ProjectCloseArgs {
        project_id: store.project().id,
        save: false,
        cancel_jobs: false,
    };
    let data = project_close(store, &args).unwrap();

    let json_val = serde_json::to_value(&data).unwrap();
    assert_eq!(
        json_val.get("closed").and_then(|v| v.as_bool()),
        Some(true),
        "data.closed must serialize as bool true"
    );
    assert!(
        json_val
            .get("canceled_job_ids")
            .is_some_and(serde_json::Value::is_array),
        "data.canceled_job_ids must serialize as an array"
    );
    assert!(
        json_val
            .get("cancel_failures")
            .is_some_and(serde_json::Value::is_array),
        "data.cancel_failures must serialize as an array"
    );
}
