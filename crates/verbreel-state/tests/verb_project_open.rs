//! Tests for `project.open` (§2.2).
//!
//! The verb is `#[cfg(feature = "native")]`-gated because it touches
//! the filesystem and returns a [`ProjectStore`] that owns a live
//! `fs4::flock`; the whole test file follows.
//!
//! Tests use [`tempfile::TempDir`] for the project root so no real
//! `~/.verbreel/` is touched. Fixture projects are built directly via
//! [`ProjectStore::create_with_registry`] so each test is independent
//! of `project.create`.

#![cfg(feature = "native")]

use std::fs;
use std::path::Path;

use serde_json::json;
use tempfile::TempDir;
use verbreel_events::{EventBackend, EventBuilder, NativeBackend, Timestamp};
use verbreel_state::{
    Canvas, ProjectOpenArgs, ProjectOpenError, ProjectStore, TICK_RATE_HZ, Track, TrackKind,
    default_fixtures, default_registry, project_open,
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
        name: "test-open".to_string(),
        created_at: Timestamp::parse("2026-05-28T00:00:00Z").unwrap(),
        updated_at: Timestamp::parse("2026-05-28T00:00:00Z").unwrap(),
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

/// Create a fresh project rooted at `root`, then drop the store so the
/// flock is released — ready for `project_open` to re-acquire it.
fn seed_project_at(root: &Path) -> ProjectId {
    let project = minimal_project();
    let id = project.id;
    let store =
        ProjectStore::create_with_registry(root, project, &default_registry(), &default_fixtures())
            .expect("seed project must create cleanly");
    drop(store);
    id
}

// --- 1. Args deserialization --------------------------------------------

#[test]
fn args_deserialize_with_path_only_defaults_strict_to_false() {
    let raw = json!({ "path": "/tmp/some-project" });
    let args: ProjectOpenArgs = serde_json::from_value(raw).unwrap();
    assert_eq!(args.path, std::path::PathBuf::from("/tmp/some-project"));
    assert!(!args.strict, "strict default must be false");
}

#[test]
fn args_deserialize_with_strict_true() {
    let raw = json!({ "path": "/tmp/foo", "strict": true });
    let args: ProjectOpenArgs = serde_json::from_value(raw).unwrap();
    assert!(args.strict);
}

#[test]
fn args_reject_unknown_fields() {
    let raw = json!({ "path": "/tmp/foo", "force": true });
    let err = serde_json::from_value::<ProjectOpenArgs>(raw).unwrap_err();
    assert!(
        err.to_string().contains("unknown field"),
        "expected deny_unknown_fields error, got: {err}"
    );
}

#[test]
fn args_reject_missing_path() {
    let raw = json!({});
    let err = serde_json::from_value::<ProjectOpenArgs>(raw).unwrap_err();
    assert!(
        err.to_string().contains("path"),
        "expected missing-field error naming path, got: {err}"
    );
}

#[test]
fn args_reject_wrong_type_for_strict() {
    let raw = json!({ "path": "/tmp/foo", "strict": "yes" });
    let err = serde_json::from_value::<ProjectOpenArgs>(raw).unwrap_err();
    assert!(
        !err.to_string().is_empty(),
        "non-bool strict must surface a serde error"
    );
}

#[test]
fn args_reject_wrong_type_for_path() {
    let raw = json!({ "path": 42 });
    let err = serde_json::from_value::<ProjectOpenArgs>(raw).unwrap_err();
    assert!(
        !err.to_string().is_empty(),
        "non-string path must surface a serde error"
    );
}

// --- 2. Happy path -------------------------------------------------------

#[test]
fn open_returns_store_carrying_loaded_project_id() {
    let root = TempDir::new().unwrap();
    let seeded_id = seed_project_at(root.path());

    let args = ProjectOpenArgs {
        path: root.path().to_path_buf(),
        strict: false,
    };
    let (store, data) = project_open(&args).expect("open must succeed");

    assert_eq!(data.project_id, seeded_id);
    assert_eq!(store.project().id, seeded_id);
}

#[test]
fn open_envelope_carries_full_project_graph() {
    let root = TempDir::new().unwrap();
    seed_project_at(root.path());

    let args = ProjectOpenArgs {
        path: root.path().to_path_buf(),
        strict: false,
    };
    let (_store, data) = project_open(&args).unwrap();

    assert_eq!(data.project.name, "test-open");
    assert_eq!(data.project.canvas.width, 1080);
    assert_eq!(data.project.canvas.height, 1920);
    assert_eq!(data.project.tracks.len(), 2);
    assert!(matches!(data.project.tracks[0].kind, TrackKind::Video));
    assert!(matches!(data.project.tracks[1].kind, TrackKind::Audio));
}

#[test]
fn open_unverified_asset_ids_empty_when_project_has_no_assets() {
    let root = TempDir::new().unwrap();
    seed_project_at(root.path());

    let args = ProjectOpenArgs {
        path: root.path().to_path_buf(),
        strict: false,
    };
    let (_store, data) = project_open(&args).unwrap();

    assert!(
        data.unverified_asset_ids.is_empty(),
        "fresh project with no assets must have empty unverified list"
    );
}

#[test]
fn open_round_trips_after_create_then_close() {
    let root = TempDir::new().unwrap();
    let seeded_id = seed_project_at(root.path());

    // Re-open the same path; the store's flock should be acquirable
    // (close-via-drop must have released it).
    let args = ProjectOpenArgs {
        path: root.path().to_path_buf(),
        strict: false,
    };
    let (store, _data) = project_open(&args).expect("re-open must succeed after seed");

    assert_eq!(store.project().id, seeded_id);
    assert_eq!(store.project().name, "test-open");
}

#[test]
fn open_strict_true_is_accepted_and_runs_fast_check() {
    // v1 floor: `strict: true` is accepted at the surface but the
    // verb still runs the fast fingerprint check. The contract under
    // test is "strict does not error", not "strict re-hashes" (which
    // is deferred per module docs).
    let root = TempDir::new().unwrap();
    seed_project_at(root.path());

    let args = ProjectOpenArgs {
        path: root.path().to_path_buf(),
        strict: true,
    };
    let (_store, data) = project_open(&args).expect("strict mode accepted in v1");
    assert!(data.unverified_asset_ids.is_empty());
}

// --- 3. Replay path ------------------------------------------------------

#[test]
fn open_replays_event_appended_past_last_saved() {
    // Seed the project, append one valid event to events.jsonl that
    // mutates `name`, then re-open. Replay should apply the patch.
    let root = TempDir::new().unwrap();
    seed_project_at(root.path());

    // Append the event line via the backend directly, bypassing the
    // mutate() write-ordering — we WANT a post-snapshot event.
    let events_path = root.path().join(".verbreel").join("events.jsonl");
    let backend = NativeBackend::open(&events_path).expect("backend must open");
    let patch_value = json!([
        { "op": "replace", "path": "/name", "value": "replayed-name" }
    ]);
    let patch: json_patch::Patch = serde_json::from_value(patch_value).unwrap();
    let event = EventBuilder::new()
        .verb("test.rename")
        .args(json!({}))
        .patch(patch)
        .build();
    let line = serde_json::to_string(&event).unwrap();
    backend
        .append(line.as_bytes())
        .expect("append must succeed");
    drop(backend);

    let args = ProjectOpenArgs {
        path: root.path().to_path_buf(),
        strict: false,
    };
    let (_store, data) = project_open(&args).expect("open with replay must succeed");

    assert_eq!(
        data.project.name, "replayed-name",
        "replay must have applied the post-snapshot event"
    );
}

// --- 4. E_PROJECT_NOT_FOUND ---------------------------------------------

#[test]
fn open_returns_not_found_on_nonexistent_path() {
    let temp = TempDir::new().unwrap();
    let bogus = temp.path().join("does-not-exist");

    let args = ProjectOpenArgs {
        path: bogus.clone(),
        strict: false,
    };
    let err = project_open(&args).expect_err("open must fail");
    match err {
        ProjectOpenError::ProjectNotFound { path } => assert_eq!(path, bogus),
        other => panic!("expected ProjectNotFound, got: {other:?}"),
    }
}

#[test]
fn open_returns_not_found_when_path_is_a_regular_file() {
    let temp = TempDir::new().unwrap();
    let file_path = temp.path().join("just-a-file");
    fs::write(&file_path, "not a project").unwrap();

    let args = ProjectOpenArgs {
        path: file_path.clone(),
        strict: false,
    };
    let err = project_open(&args).expect_err("open must fail");
    assert!(
        matches!(err, ProjectOpenError::ProjectNotFound { ref path } if path == &file_path),
        "expected ProjectNotFound for non-directory path, got: {err:?}"
    );
}

#[test]
fn open_returns_not_found_when_directory_has_no_project_json() {
    let temp = TempDir::new().unwrap();
    let empty_dir = temp.path().join("empty");
    fs::create_dir(&empty_dir).unwrap();

    let args = ProjectOpenArgs {
        path: empty_dir.clone(),
        strict: false,
    };
    let err = project_open(&args).expect_err("open must fail");
    assert!(
        matches!(err, ProjectOpenError::ProjectNotFound { ref path } if path == &empty_dir),
        "expected ProjectNotFound for dir without project.json, got: {err:?}"
    );
}

// --- 5. E_IO distinguished from E_PROJECT_NOT_FOUND ---------------------
//
// This is the regression test the brief calls out — the prior iter-2
// review caught `Path::is_file` swallowing non-NotFound IO errors and
// misreporting them as `E_PROJECT_NOT_FOUND`. The current
// implementation matches on `io::ErrorKind::NotFound` explicitly, so a
// permission-denied stat must surface as `E_IO`, not `E_PROJECT_NOT_FOUND`.

#[cfg(unix)]
#[test]
fn open_returns_io_when_stat_fails_with_permission_denied() {
    use std::os::unix::fs::PermissionsExt;

    let temp = TempDir::new().unwrap();
    // Build the target path under a parent dir that we will chmod to
    // 0 — stat on the child then surfaces EACCES (permission denied),
    // NOT ENOENT. This is the exact path that `Path::is_dir` would
    // silently turn into a false negative.
    let parent = temp.path().join("locked-parent");
    fs::create_dir(&parent).unwrap();
    let target = parent.join("project-root");
    fs::create_dir(&target).unwrap();

    let mut perms = fs::metadata(&parent).unwrap().permissions();
    let original = perms.mode();
    perms.set_mode(0o000);
    fs::set_permissions(&parent, perms).unwrap();

    let args = ProjectOpenArgs {
        path: target.clone(),
        strict: false,
    };
    let result = project_open(&args);

    // Restore perms unconditionally so TempDir cleanup works.
    let mut restore = fs::metadata(&parent)
        .unwrap_or_else(|_| {
            // If we can't even stat the parent (because we own it but
            // chmod 0 also blocks us), make the dir traversable again
            // first.
            let _ = fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o700));
            fs::metadata(&parent).unwrap()
        })
        .permissions();
    restore.set_mode(original);
    fs::set_permissions(&parent, restore).unwrap();

    let err = result.expect_err("stat on unreadable parent must fail");
    match err {
        ProjectOpenError::Io(io) => {
            assert_ne!(
                io.kind(),
                std::io::ErrorKind::NotFound,
                "non-NotFound IO must surface as Io, not ProjectNotFound — \
                 see prior iter-2 review on the lost branch"
            );
        }
        ProjectOpenError::ProjectNotFound { .. } => {
            panic!(
                "permission-denied stat must surface as E_IO, not \
                 E_PROJECT_NOT_FOUND — the verb must distinguish ENOENT \
                 from EACCES per the prior iter-2 review"
            );
        }
        other => panic!("expected Io variant, got: {other:?}"),
    }
}

// --- 6. E_PROJECT_LOCKED ------------------------------------------------

#[test]
fn open_returns_locked_when_another_store_holds_the_flock() {
    let root = TempDir::new().unwrap();
    let project = minimal_project();
    // First handle holds the flock.
    let first = ProjectStore::create_with_registry(
        root.path(),
        project,
        &default_registry(),
        &default_fixtures(),
    )
    .expect("first store must open");

    // Second open() against the same root contends for the flock and
    // must fail with E_PROJECT_LOCKED.
    let args = ProjectOpenArgs {
        path: root.path().to_path_buf(),
        strict: false,
    };
    let err = project_open(&args).expect_err("second open must fail");
    match err {
        ProjectOpenError::ProjectLocked { path } => assert_eq!(path, root.path()),
        other => panic!("expected ProjectLocked, got: {other:?}"),
    }

    // Dropping the first store releases the flock; a subsequent open
    // succeeds.
    drop(first);
    let (_store, _data) = project_open(&args).expect("re-open after first drops must succeed");
}

// --- 7. E_SCHEMA_VIOLATION ----------------------------------------------

#[test]
fn open_returns_schema_violation_when_project_json_is_empty_object() {
    let root = TempDir::new().unwrap();
    seed_project_at(root.path());
    // Overwrite project.json with `{}` — missing every required field.
    fs::write(root.path().join("project.json"), b"{}").unwrap();

    let args = ProjectOpenArgs {
        path: root.path().to_path_buf(),
        strict: false,
    };
    let err = project_open(&args).expect_err("open must fail");
    assert!(
        matches!(err, ProjectOpenError::SchemaViolation { .. }),
        "tampered project.json must surface as SchemaViolation, got: {err:?}"
    );
}

#[test]
fn open_returns_schema_violation_when_project_json_is_not_json() {
    let root = TempDir::new().unwrap();
    seed_project_at(root.path());
    fs::write(root.path().join("project.json"), b"not json at all").unwrap();

    let args = ProjectOpenArgs {
        path: root.path().to_path_buf(),
        strict: false,
    };
    let err = project_open(&args).expect_err("open must fail");
    assert!(
        matches!(err, ProjectOpenError::SchemaViolation { .. }),
        "non-JSON project.json must surface as SchemaViolation, got: {err:?}"
    );
}

#[test]
fn schema_violation_error_carries_detail_string() {
    let root = TempDir::new().unwrap();
    seed_project_at(root.path());
    fs::write(root.path().join("project.json"), b"{").unwrap();

    let args = ProjectOpenArgs {
        path: root.path().to_path_buf(),
        strict: false,
    };
    let err = project_open(&args).err().unwrap();
    match err {
        ProjectOpenError::SchemaViolation { detail } => {
            assert!(
                !detail.is_empty(),
                "SchemaViolation must carry a non-empty detail string"
            );
        }
        other => panic!("expected SchemaViolation, got: {other:?}"),
    }
}

// --- 8. Path resolution --------------------------------------------------

#[test]
fn open_resolves_relative_path_against_cwd() {
    // Seed inside a tempdir, switch cwd to its parent, then call open
    // with a path relative to that cwd. Cwd changes are process-wide;
    // we restore at end to keep test isolation.
    let root = TempDir::new().unwrap();
    let seeded_id = seed_project_at(root.path());
    let parent = root.path().parent().unwrap().to_path_buf();
    let leaf = root.path().file_name().unwrap().to_owned();

    let saved_cwd = std::env::current_dir().unwrap();
    std::env::set_current_dir(&parent).unwrap();

    let args = ProjectOpenArgs {
        path: std::path::PathBuf::from(&leaf),
        strict: false,
    };
    let result = project_open(&args);

    // Restore cwd before any assertion can panic.
    std::env::set_current_dir(saved_cwd).unwrap();

    let (store, data) = result.expect("relative-path open must succeed");
    assert_eq!(store.project().id, seeded_id);
    assert_eq!(data.project_id, seeded_id);
}

#[test]
fn open_accepts_absolute_path_as_is() {
    let root = TempDir::new().unwrap();
    let seeded_id = seed_project_at(root.path());
    assert!(root.path().is_absolute());

    let args = ProjectOpenArgs {
        path: root.path().to_path_buf(),
        strict: false,
    };
    let (_store, data) = project_open(&args).expect("absolute-path open must succeed");
    assert_eq!(data.project_id, seeded_id);
}

// --- 9. Flock acquisition ------------------------------------------------

#[test]
fn open_acquires_flock_so_concurrent_open_fails() {
    let root = TempDir::new().unwrap();
    seed_project_at(root.path());

    let args = ProjectOpenArgs {
        path: root.path().to_path_buf(),
        strict: false,
    };
    let (first_store, _data) = project_open(&args).expect("first open must succeed");

    // While first_store is alive, a second open must hit the lock.
    let err = project_open(&args).expect_err("second open must fail");
    assert!(
        matches!(err, ProjectOpenError::ProjectLocked { .. }),
        "concurrent open while flock held must be ProjectLocked, got: {err:?}"
    );

    drop(first_store);
    let (_third_store, _third_data) = project_open(&args).expect("third open after drop succeeds");
}

// --- 10. Data envelope serialization shape ------------------------------

#[test]
fn data_envelope_serializes_with_spec_field_names() {
    let root = TempDir::new().unwrap();
    seed_project_at(root.path());

    let args = ProjectOpenArgs {
        path: root.path().to_path_buf(),
        strict: false,
    };
    let (_store, data) = project_open(&args).unwrap();
    let v = serde_json::to_value(&data).unwrap();

    assert!(
        v.get("project_id")
            .is_some_and(serde_json::Value::is_string),
        "data.project_id must serialize as string"
    );
    assert!(
        v.get("project").is_some_and(serde_json::Value::is_object),
        "data.project must serialize as object"
    );
    assert!(
        v.get("unverified_asset_ids")
            .is_some_and(serde_json::Value::is_array),
        "data.unverified_asset_ids must serialize as array"
    );
}
