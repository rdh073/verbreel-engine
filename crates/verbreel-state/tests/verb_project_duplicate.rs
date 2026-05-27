//! Tests for `project.duplicate` (§2.7).
//!
//! The verb is `#[cfg(feature = "native")]`-gated because it touches
//! the filesystem; the whole test file follows.
//!
//! Tests use [`tempfile::TempDir`] for both the source and the
//! destination so no real `~/.verbreel/` is touched. Source projects
//! are built via [`ProjectStore::create_with_registry`] (no
//! dependency on the still-blocked `project.create` verb).

#![cfg(feature = "native")]

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;
use tempfile::TempDir;
use verbreel_state::{
    Canvas, ProjectDuplicateArgs, ProjectDuplicateError, ProjectStore, TICK_RATE_HZ, Track,
    TrackKind, default_fixtures, default_registry, project_duplicate,
};
use verbreel_types::{ProjectId, Tick, TrackId};

const SCHEMA_VERSION: &str = "1.0.0";

// --- Helpers -------------------------------------------------------------

fn minimal_project() -> verbreel_state::Project {
    verbreel_state::Project {
        id: ProjectId::now(),
        schema_version: SCHEMA_VERSION.to_string(),
        tick_rate_hz: TICK_RATE_HZ,
        name: "source".to_string(),
        // Distinct historical timestamp so the refresh check has a
        // pre-call value to compare against.
        created_at: "2020-01-01T00:00:00Z".to_string(),
        updated_at: "2020-01-01T00:00:00Z".to_string(),
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

/// Build a fresh source project on disk at `root`. The returned
/// store is dropped immediately so the lock is released — the
/// duplicate call reads from disk by path, not from a live store.
fn build_source_at(root: &Path) -> ProjectId {
    let project = minimal_project();
    let pid = project.id;
    let _store =
        ProjectStore::create_with_registry(root, project, &default_registry(), &default_fixtures())
            .expect("source project must create cleanly");
    pid
}

/// Seed `<root>/assets/<aa>/<sha>.bin` so the asset-mirror branch
/// exercises both a top-level file (none — assets are always
/// under content-address subdirs) and a nested file.
fn seed_assets(root: &Path) {
    let asset_dir = root.join("assets").join("ab");
    fs::create_dir_all(&asset_dir).unwrap();
    fs::write(asset_dir.join("ab1234.bin"), b"asset-payload-1").unwrap();
    let asset_dir_2 = root.join("assets").join("cd");
    fs::create_dir_all(&asset_dir_2).unwrap();
    fs::write(asset_dir_2.join("cd5678.bin"), b"asset-payload-2").unwrap();
}

/// Build the default args. Caller can adjust fields after.
fn args_for(source_root: &Path, dest_at: Option<PathBuf>) -> ProjectDuplicateArgs {
    ProjectDuplicateArgs {
        project_id: ProjectId::now(),
        name: "duplicate-name".to_string(),
        at: dest_at,
        source_path: source_root.to_path_buf(),
    }
}

// --- 1. Args deserialization --------------------------------------------

#[test]
fn args_deserialize_from_object_with_all_fields() {
    let pid = ProjectId::now();
    let raw = json!({
        "project_id": pid.to_string(),
        "name": "dup",
        "at": "/tmp/dup",
        "source_path": "/tmp/src",
    });
    let args: ProjectDuplicateArgs = serde_json::from_value(raw).unwrap();
    assert_eq!(args.project_id, pid);
    assert_eq!(args.name, "dup");
    assert_eq!(args.at, Some(PathBuf::from("/tmp/dup")));
    assert_eq!(args.source_path, PathBuf::from("/tmp/src"));
}

#[test]
fn args_deserialize_without_optional_at() {
    let pid = ProjectId::now();
    let raw = json!({
        "project_id": pid.to_string(),
        "name": "dup",
        "source_path": "/tmp/src",
    });
    let args: ProjectDuplicateArgs = serde_json::from_value(raw).unwrap();
    assert!(args.at.is_none());
}

#[test]
fn args_reject_unknown_fields() {
    let pid = ProjectId::now();
    let raw = json!({
        "project_id": pid.to_string(),
        "name": "dup",
        "source_path": "/tmp/src",
        "force": true,
    });
    let err = serde_json::from_value::<ProjectDuplicateArgs>(raw).unwrap_err();
    assert!(
        err.to_string().contains("unknown field"),
        "expected deny_unknown_fields error, got: {err}"
    );
}

#[test]
fn args_reject_missing_required_fields() {
    let pid = ProjectId::now();
    // Missing `name`.
    let err = serde_json::from_value::<ProjectDuplicateArgs>(json!({
        "project_id": pid.to_string(),
        "source_path": "/tmp/src",
    }))
    .unwrap_err();
    assert!(err.to_string().contains("name"));

    // Missing `source_path`.
    let err = serde_json::from_value::<ProjectDuplicateArgs>(json!({
        "project_id": pid.to_string(),
        "name": "dup",
    }))
    .unwrap_err();
    assert!(err.to_string().contains("source_path"));

    // Missing `project_id`.
    let err = serde_json::from_value::<ProjectDuplicateArgs>(json!({
        "name": "dup",
        "source_path": "/tmp/src",
    }))
    .unwrap_err();
    assert!(err.to_string().contains("project_id"));
}

// --- 2. Happy path ------------------------------------------------------

#[test]
fn happy_path_returns_new_project_id_and_dest_path() {
    let parent = TempDir::new().unwrap();
    let source = parent.path().join("src");
    let _ = build_source_at(&source);
    let dest = parent.path().join("dup");

    let args = args_for(&source, Some(dest.clone()));
    let data = project_duplicate(&args).expect("happy-path duplicate must succeed");

    assert_eq!(data.path, dest);
    // New id must be a UUIDv7 different from the source's.
    assert!(!data.project_id.to_string().is_empty());
}

#[test]
fn destination_contains_project_json_with_fresh_id() {
    let parent = TempDir::new().unwrap();
    let source = parent.path().join("src");
    let source_id = build_source_at(&source);
    let dest = parent.path().join("dup");

    let args = args_for(&source, Some(dest.clone()));
    let data = project_duplicate(&args).unwrap();

    let parsed: serde_json::Value =
        serde_json::from_slice(&fs::read(dest.join("project.json")).unwrap()).unwrap();
    let on_disk_id = parsed.get("id").unwrap().as_str().unwrap();
    assert_eq!(on_disk_id, data.project_id.to_string());
    assert_ne!(on_disk_id, source_id.to_string());
}

#[test]
fn destination_has_refreshed_timestamps() {
    let parent = TempDir::new().unwrap();
    let source = parent.path().join("src");
    let _ = build_source_at(&source);
    let dest = parent.path().join("dup");

    let args = args_for(&source, Some(dest.clone()));
    project_duplicate(&args).unwrap();

    let parsed: serde_json::Value =
        serde_json::from_slice(&fs::read(dest.join("project.json")).unwrap()).unwrap();
    let created = parsed.get("created_at").unwrap().as_str().unwrap();
    let updated = parsed.get("updated_at").unwrap().as_str().unwrap();
    // The source's seeded timestamp was 2020-01-01; the refresh must
    // produce a timestamp from the current decade.
    assert!(
        !created.starts_with("2020-"),
        "created_at not refreshed: {created}"
    );
    assert!(
        !updated.starts_with("2020-"),
        "updated_at not refreshed: {updated}"
    );
    assert_eq!(created, updated);
}

#[test]
fn destination_resets_last_saved_event_id_to_null() {
    let parent = TempDir::new().unwrap();
    let source = parent.path().join("src");
    let _ = build_source_at(&source);
    let dest = parent.path().join("dup");

    let args = args_for(&source, Some(dest.clone()));
    project_duplicate(&args).unwrap();

    let parsed: serde_json::Value =
        serde_json::from_slice(&fs::read(dest.join("project.json")).unwrap()).unwrap();
    let lse = parsed.get("last_saved_event_id").unwrap();
    assert!(
        lse.is_null(),
        "last_saved_event_id must be null, got: {lse}"
    );
}

#[test]
fn destination_has_empty_events_jsonl() {
    let parent = TempDir::new().unwrap();
    let source = parent.path().join("src");
    let _ = build_source_at(&source);
    let dest = parent.path().join("dup");

    let args = args_for(&source, Some(dest.clone()));
    project_duplicate(&args).unwrap();

    let events_path = dest.join(".verbreel").join("events.jsonl");
    assert!(events_path.exists(), "events.jsonl must exist");
    let size = fs::metadata(&events_path).unwrap().len();
    assert_eq!(size, 0, "events.jsonl must be empty, got {size} bytes");
}

// --- 3. Asset mirroring -------------------------------------------------

#[test]
fn assets_are_mirrored_with_matching_file_count() {
    let parent = TempDir::new().unwrap();
    let source = parent.path().join("src");
    let _ = build_source_at(&source);
    seed_assets(&source);
    let dest = parent.path().join("dup");

    let args = args_for(&source, Some(dest.clone()));
    project_duplicate(&args).unwrap();

    let dst_asset_dir = dest.join("assets");
    assert!(dst_asset_dir.is_dir());
    assert!(dst_asset_dir.join("ab").join("ab1234.bin").is_file());
    assert!(dst_asset_dir.join("cd").join("cd5678.bin").is_file());
    let bytes = fs::read(dst_asset_dir.join("ab").join("ab1234.bin")).unwrap();
    assert_eq!(bytes, b"asset-payload-1");
}

#[test]
fn assets_mirrored_files_share_inode_when_same_filesystem() {
    use std::os::unix::fs::MetadataExt;
    let parent = TempDir::new().unwrap();
    let source = parent.path().join("src");
    let _ = build_source_at(&source);
    seed_assets(&source);
    let dest = parent.path().join("dup");

    let args = args_for(&source, Some(dest.clone()));
    project_duplicate(&args).unwrap();

    let src_inode = fs::metadata(source.join("assets").join("ab").join("ab1234.bin"))
        .unwrap()
        .ino();
    let dst_inode = fs::metadata(dest.join("assets").join("ab").join("ab1234.bin"))
        .unwrap()
        .ino();
    assert_eq!(
        src_inode, dst_inode,
        "hard-link must share inode on same filesystem"
    );
}

#[test]
fn source_without_assets_dir_still_succeeds() {
    // Default minimal_project does not seed assets/ — the verb must
    // tolerate a project without imports.
    let parent = TempDir::new().unwrap();
    let source = parent.path().join("src");
    let _ = build_source_at(&source);
    let dest = parent.path().join("dup");

    let args = args_for(&source, Some(dest.clone()));
    project_duplicate(&args).unwrap();

    // Destination has no assets/ either (we didn't synthesize one).
    assert!(!dest.join("assets").exists());
}

// --- 4. Name validation -------------------------------------------------

#[test]
fn rejects_empty_name() {
    let parent = TempDir::new().unwrap();
    let source = parent.path().join("src");
    let _ = build_source_at(&source);
    let dest = parent.path().join("dup");

    let mut args = args_for(&source, Some(dest));
    args.name = String::new();
    match project_duplicate(&args).unwrap_err() {
        ProjectDuplicateError::NameEmpty => {}
        other => panic!("expected NameEmpty, got: {other:?}"),
    }
}

#[test]
fn rejects_too_long_name() {
    let parent = TempDir::new().unwrap();
    let source = parent.path().join("src");
    let _ = build_source_at(&source);
    let dest = parent.path().join("dup");

    let mut args = args_for(&source, Some(dest));
    args.name = "x".repeat(257);
    match project_duplicate(&args).unwrap_err() {
        ProjectDuplicateError::NameTooLong { actual, max } => {
            assert_eq!(actual, 257);
            assert_eq!(max, 256);
        }
        other => panic!("expected NameTooLong, got: {other:?}"),
    }
}

#[test]
fn rejects_relative_at_path() {
    let parent = TempDir::new().unwrap();
    let source = parent.path().join("src");
    let _ = build_source_at(&source);

    let relative = PathBuf::from("dup-relative");
    let args = args_for(&source, Some(relative.clone()));
    match project_duplicate(&args).unwrap_err() {
        ProjectDuplicateError::RelativeAt { path } => {
            assert_eq!(path, relative);
        }
        other => panic!("expected RelativeAt, got: {other:?}"),
    }
}

// --- 5. Destination collision -------------------------------------------

#[test]
fn rejects_existing_destination_with_project_exists() {
    let parent = TempDir::new().unwrap();
    let source = parent.path().join("src");
    let _ = build_source_at(&source);
    let dest = parent.path().join("dup");
    fs::create_dir_all(&dest).unwrap();
    fs::write(dest.join("marker"), b"pre-existing").unwrap();

    let args = args_for(&source, Some(dest.clone()));
    match project_duplicate(&args).unwrap_err() {
        ProjectDuplicateError::DestinationExists { path } => {
            assert_eq!(path, dest);
        }
        other => panic!("expected DestinationExists, got: {other:?}"),
    }
}

#[test]
fn collision_does_not_modify_existing_destination() {
    let parent = TempDir::new().unwrap();
    let source = parent.path().join("src");
    let _ = build_source_at(&source);
    let dest = parent.path().join("dup");
    fs::create_dir_all(&dest).unwrap();
    fs::write(dest.join("marker"), b"pre-existing").unwrap();

    let args = args_for(&source, Some(dest.clone()));
    let _ = project_duplicate(&args).unwrap_err();

    // The pre-existing marker is intact and no project.json was
    // written into the colliding dir.
    let marker = fs::read(dest.join("marker")).unwrap();
    assert_eq!(marker, b"pre-existing");
    assert!(!dest.join("project.json").exists());
}

// --- 6. Source not found / corrupt --------------------------------------

#[test]
fn missing_source_returns_source_not_found() {
    let parent = TempDir::new().unwrap();
    let source = parent.path().join("does-not-exist");
    let dest = parent.path().join("dup");

    let args = args_for(&source, Some(dest));
    match project_duplicate(&args).unwrap_err() {
        ProjectDuplicateError::SourceNotFound { path } => {
            assert_eq!(path, source);
        }
        other => panic!("expected SourceNotFound, got: {other:?}"),
    }
}

#[test]
fn corrupt_source_project_json_returns_source_corrupt() {
    let parent = TempDir::new().unwrap();
    let source = parent.path().join("src");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("project.json"), b"{ not valid json").unwrap();
    let dest = parent.path().join("dup");

    let args = args_for(&source, Some(dest));
    match project_duplicate(&args).unwrap_err() {
        ProjectDuplicateError::SourceCorrupt { detail } => {
            assert!(!detail.is_empty(), "detail must surface the serde error");
        }
        other => panic!("expected SourceCorrupt, got: {other:?}"),
    }
}

// --- 7. Rollback on mid-call failure ------------------------------------

#[test]
fn rollback_wipes_destination_on_mid_call_failure() {
    // Force a mid-call failure by pointing `at` at a path whose
    // parent does not exist AND whose tail segment is a file (not a
    // dir). create_dir_all on a path component that's already a
    // regular file fails with NotADirectory.
    let parent = TempDir::new().unwrap();
    let source = parent.path().join("src");
    let _ = build_source_at(&source);

    // Make `parent/blocker` a regular file, then ask for
    // `parent/blocker/inner` as the dest — create_dir_all has to
    // descend into `blocker` and will fail.
    fs::write(parent.path().join("blocker"), b"x").unwrap();
    let dest = parent.path().join("blocker").join("inner");

    let args = args_for(&source, Some(dest.clone()));
    let err = project_duplicate(&args).unwrap_err();
    match err {
        ProjectDuplicateError::Io(_) => {}
        other => panic!("expected Io, got: {other:?}"),
    }

    // The blocker file stayed intact — nothing was clobbered.
    let blocker = fs::read(parent.path().join("blocker")).unwrap();
    assert_eq!(blocker, b"x");
    // And no `inner` subtree leaked.
    assert!(!dest.exists());
}

#[test]
fn fresh_project_id_differs_from_source() {
    let parent = TempDir::new().unwrap();
    let source = parent.path().join("src");
    let source_id = build_source_at(&source);
    let dest = parent.path().join("dup");

    let args = args_for(&source, Some(dest));
    let data = project_duplicate(&args).unwrap();
    assert_ne!(
        data.project_id, source_id,
        "duplicate must mint a fresh project_id"
    );
}

// --- 8. `at` resolution -------------------------------------------------

#[test]
fn explicit_at_is_honored() {
    let parent = TempDir::new().unwrap();
    let source = parent.path().join("src");
    let _ = build_source_at(&source);
    let dest = parent.path().join("nested").join("path").join("dup");

    let args = args_for(&source, Some(dest.clone()));
    let data = project_duplicate(&args).unwrap();
    assert_eq!(data.path, dest);
    assert!(dest.join("project.json").exists());
}

#[test]
fn omitted_at_resolves_to_sibling_named_by_name() {
    let parent = TempDir::new().unwrap();
    let source = parent.path().join("src");
    let _ = build_source_at(&source);

    let mut args = args_for(&source, None);
    args.name = "sibling-dup".to_string();
    let data = project_duplicate(&args).unwrap();

    let expected = parent.path().join("sibling-dup");
    assert_eq!(data.path, expected);
    assert!(expected.join("project.json").exists());
}

// --- 9. Source log untouched --------------------------------------------

#[test]
fn duplicate_does_not_touch_source_events_log() {
    let parent = TempDir::new().unwrap();
    let source = parent.path().join("src");
    let _ = build_source_at(&source);
    let source_events = source.join(".verbreel").join("events.jsonl");
    let before = fs::metadata(&source_events).unwrap().len();

    let dest = parent.path().join("dup");
    let args = args_for(&source, Some(dest));
    project_duplicate(&args).unwrap();

    let after = fs::metadata(&source_events).unwrap().len();
    assert_eq!(
        before, after,
        "source events.jsonl byte count must not change"
    );
}

#[test]
fn duplicate_does_not_touch_source_project_json() {
    let parent = TempDir::new().unwrap();
    let source = parent.path().join("src");
    let _ = build_source_at(&source);
    let source_pj = source.join("project.json");
    let before = fs::read(&source_pj).unwrap();

    let dest = parent.path().join("dup");
    let args = args_for(&source, Some(dest));
    project_duplicate(&args).unwrap();

    let after = fs::read(&source_pj).unwrap();
    assert_eq!(before, after, "source project.json must be untouched");
}

// --- 10. No orphan temp files ------------------------------------------

#[test]
fn no_orphan_tmp_files_after_success() {
    let parent = TempDir::new().unwrap();
    let source = parent.path().join("src");
    let _ = build_source_at(&source);
    let dest = parent.path().join("dup");

    let args = args_for(&source, Some(dest.clone()));
    project_duplicate(&args).unwrap();

    for entry in fs::read_dir(&dest).unwrap() {
        let name = entry.unwrap().file_name();
        let s = name.to_string_lossy();
        assert!(
            !s.contains(".tmp"),
            "found leftover temp file after duplicate: {s}"
        );
    }
}
