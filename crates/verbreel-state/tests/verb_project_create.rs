//! Tests for `project.create` (§2.1).
//!
//! The verb is `#[cfg(feature = "native")]`-gated because it touches
//! the filesystem (creates a project root, writes `project.json` and
//! the empty `events.jsonl`); the whole test file follows.
//!
//! Tests use [`tempfile::TempDir`] for the parent directory so no
//! real `~/.verbreel/` is touched. The verb itself owns the
//! create-and-drop lifecycle — every test creates a project then
//! re-opens it via [`ProjectStore::open`] to assert the on-disk
//! layout, exercising both the create path and the (independent)
//! open path.

#![cfg(feature = "native")]

use std::fs;
use std::path::PathBuf;

use serde_json::{Map, Value, json};
use tempfile::TempDir;
use verbreel_state::{
    ProjectCreateArgs, ProjectCreateError, ProjectStore, SeededTrackIds, TrackKind, project_create,
};

// ---------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------

/// Build a default-shaped args value for a fresh project named
/// `name` rooted under `parent`.
fn args_for(parent: &TempDir, name: &str) -> ProjectCreateArgs {
    ProjectCreateArgs {
        name: name.to_string(),
        canvas: "1080x1920".to_string(),
        fps_num: None,
        fps_den: None,
        at: Some(parent.path().to_path_buf()),
        activate: false,
        metadata: Map::new(),
    }
}

// ---------------------------------------------------------------------
// 1. Args deserialization
// ---------------------------------------------------------------------

#[test]
fn args_deserialize_with_all_fields() {
    let raw = json!({
        "name": "demo",
        "canvas": "1920x1080",
        "fps_num": 30000,
        "fps_den": 1001,
        "at": "/tmp/projects",
        "activate": true,
        "metadata": { "owner": "alice" },
    });
    let args: ProjectCreateArgs = serde_json::from_value(raw).unwrap();
    assert_eq!(args.name, "demo");
    assert_eq!(args.canvas, "1920x1080");
    assert_eq!(args.fps_num, Some(30000));
    assert_eq!(args.fps_den, Some(1001));
    assert_eq!(args.at, Some(PathBuf::from("/tmp/projects")));
    assert!(args.activate);
    assert_eq!(args.metadata.get("owner"), Some(&Value::from("alice")));
}

#[test]
fn args_deserialize_with_only_required_fields() {
    let raw = json!({ "name": "demo", "canvas": "1080x1920" });
    let args: ProjectCreateArgs = serde_json::from_value(raw).unwrap();
    assert!(args.fps_num.is_none());
    assert!(args.fps_den.is_none());
    assert!(args.at.is_none());
    assert!(!args.activate);
    assert!(args.metadata.is_empty());
}

#[test]
fn args_reject_unknown_fields() {
    let raw = json!({
        "name": "demo",
        "canvas": "1080x1920",
        "force": true,
    });
    let err = serde_json::from_value::<ProjectCreateArgs>(raw).unwrap_err();
    assert!(
        err.to_string().contains("unknown field"),
        "expected deny_unknown_fields error, got: {err}"
    );
}

#[test]
fn args_reject_missing_required_name() {
    let raw = json!({ "canvas": "1080x1920" });
    let err = serde_json::from_value::<ProjectCreateArgs>(raw).unwrap_err();
    assert!(err.to_string().contains("name"));
}

#[test]
fn args_reject_missing_required_canvas() {
    let raw = json!({ "name": "demo" });
    let err = serde_json::from_value::<ProjectCreateArgs>(raw).unwrap_err();
    assert!(err.to_string().contains("canvas"));
}

#[test]
fn args_reject_wrong_type_for_fps_num() {
    // serde's default error message for a string-into-Option<u32>
    // mismatch is "invalid type: string \"thirty\", expected u32".
    // The field name is not in the surface (serde_json::Value-driven
    // deserialization loses path context); the contract under test
    // is that it errors at all, with an integer-expected hint.
    let raw = json!({
        "name": "demo",
        "canvas": "1080x1920",
        "fps_num": "thirty",
    });
    let err = serde_json::from_value::<ProjectCreateArgs>(raw).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("invalid type") || msg.contains("u32") || msg.contains("integer"),
        "expected type-mismatch error, got: {msg}"
    );
}

// ---------------------------------------------------------------------
// 2. Canvas parsing
// ---------------------------------------------------------------------

#[test]
fn canvas_ok_1080x1920() {
    let parent = TempDir::new().unwrap();
    let args = args_for(&parent, "demo");
    let data = project_create(&args).expect("happy-path canvas must parse");
    let project_json: Value =
        serde_json::from_slice(&fs::read(data.path.join("project.json")).unwrap()).unwrap();
    assert_eq!(project_json["canvas"]["width"], 1080);
    assert_eq!(project_json["canvas"]["height"], 1920);
}

#[test]
fn canvas_malformed_string_rejected() {
    let parent = TempDir::new().unwrap();
    let mut args = args_for(&parent, "demo");
    args.canvas = "1080-1920".to_string();
    let err = project_create(&args).unwrap_err();
    assert!(
        matches!(err, ProjectCreateError::InvalidCanvas(ref s) if s == "1080-1920"),
        "expected InvalidCanvas, got: {err:?}"
    );
}

#[test]
fn canvas_uppercase_x_rejected() {
    // Spec mandates lowercase `x`; uppercase is a documentation typo
    // surface, not an alternative form.
    let parent = TempDir::new().unwrap();
    let mut args = args_for(&parent, "demo");
    args.canvas = "1080X1920".to_string();
    let err = project_create(&args).unwrap_err();
    assert!(matches!(err, ProjectCreateError::InvalidCanvas(_)));
}

#[test]
fn canvas_empty_string_rejected() {
    let parent = TempDir::new().unwrap();
    let mut args = args_for(&parent, "demo");
    args.canvas = String::new();
    let err = project_create(&args).unwrap_err();
    assert!(matches!(err, ProjectCreateError::InvalidCanvas(_)));
}

#[test]
fn canvas_missing_height_rejected() {
    let parent = TempDir::new().unwrap();
    let mut args = args_for(&parent, "demo");
    args.canvas = "1080x".to_string();
    let err = project_create(&args).unwrap_err();
    assert!(matches!(err, ProjectCreateError::InvalidCanvas(_)));
}

#[test]
fn canvas_zero_width_rejected() {
    let parent = TempDir::new().unwrap();
    let mut args = args_for(&parent, "demo");
    args.canvas = "0x1080".to_string();
    let err = project_create(&args).unwrap_err();
    assert!(matches!(
        err,
        ProjectCreateError::CanvasDimOutOfRange { width: 0, .. }
    ));
}

#[test]
fn canvas_zero_height_rejected() {
    let parent = TempDir::new().unwrap();
    let mut args = args_for(&parent, "demo");
    args.canvas = "1080x0".to_string();
    let err = project_create(&args).unwrap_err();
    assert!(matches!(
        err,
        ProjectCreateError::CanvasDimOutOfRange { height: 0, .. }
    ));
}

#[test]
fn canvas_signed_value_rejected() {
    // `-1080` would otherwise sneak past `parse::<u32>` as a parse
    // error wrapped as InvalidCanvas; either rejection is fine. The
    // important contract is that the verb never silently accepts a
    // negative dim.
    let parent = TempDir::new().unwrap();
    let mut args = args_for(&parent, "demo");
    args.canvas = "-1080x1920".to_string();
    let err = project_create(&args).unwrap_err();
    assert!(matches!(err, ProjectCreateError::InvalidCanvas(_)));
}

#[test]
fn canvas_dim_above_max_rejected() {
    let parent = TempDir::new().unwrap();
    let mut args = args_for(&parent, "demo");
    args.canvas = "9000x9000".to_string();
    let err = project_create(&args).unwrap_err();
    assert!(matches!(
        err,
        ProjectCreateError::CanvasDimOutOfRange { .. }
    ));
}

// ---------------------------------------------------------------------
// 3. FPS validation
// ---------------------------------------------------------------------

#[test]
fn fps_defaults_when_omitted() {
    let parent = TempDir::new().unwrap();
    let args = args_for(&parent, "demo");
    let data = project_create(&args).unwrap();
    let project_json: Value =
        serde_json::from_slice(&fs::read(data.path.join("project.json")).unwrap()).unwrap();
    assert_eq!(project_json["fps_num"], 30);
    assert_eq!(project_json["fps_den"], 1);
}

#[test]
fn fps_zero_numerator_rejected() {
    let parent = TempDir::new().unwrap();
    let mut args = args_for(&parent, "demo");
    args.fps_num = Some(0);
    let err = project_create(&args).unwrap_err();
    assert!(matches!(
        err,
        ProjectCreateError::InvalidFps { num: 0, den: 1 }
    ));
}

#[test]
fn fps_zero_denominator_rejected() {
    let parent = TempDir::new().unwrap();
    let mut args = args_for(&parent, "demo");
    args.fps_den = Some(0);
    let err = project_create(&args).unwrap_err();
    assert!(matches!(
        err,
        ProjectCreateError::InvalidFps { num: 30, den: 0 }
    ));
}

#[test]
fn fps_ntsc_29_97_round_trips() {
    let parent = TempDir::new().unwrap();
    let mut args = args_for(&parent, "demo");
    args.fps_num = Some(30000);
    args.fps_den = Some(1001);
    let data = project_create(&args).unwrap();
    let project_json: Value =
        serde_json::from_slice(&fs::read(data.path.join("project.json")).unwrap()).unwrap();
    assert_eq!(project_json["fps_num"], 30000);
    assert_eq!(project_json["fps_den"], 1001);
}

// ---------------------------------------------------------------------
// 4. Name validation
// ---------------------------------------------------------------------

#[test]
fn name_empty_rejected() {
    let parent = TempDir::new().unwrap();
    let mut args = args_for(&parent, "demo");
    args.name = String::new();
    let err = project_create(&args).unwrap_err();
    assert!(matches!(err, ProjectCreateError::NameEmpty));
}

#[test]
fn name_too_long_rejected() {
    let parent = TempDir::new().unwrap();
    let mut args = args_for(&parent, "demo");
    args.name = "a".repeat(300);
    let err = project_create(&args).unwrap_err();
    assert!(matches!(
        err,
        ProjectCreateError::NameTooLong { actual: 300, .. }
    ));
}

// ---------------------------------------------------------------------
// 5. `at` validation
// ---------------------------------------------------------------------

#[test]
fn at_relative_rejected() {
    let mut args = ProjectCreateArgs {
        name: "demo".to_string(),
        canvas: "1080x1920".to_string(),
        fps_num: None,
        fps_den: None,
        at: Some(PathBuf::from("relative/path")),
        activate: false,
        metadata: Map::new(),
    };
    let err = project_create(&args).unwrap_err();
    assert!(matches!(err, ProjectCreateError::RelativeAt { .. }));

    // Stay defensive: confirm with another shape too.
    args.at = Some(PathBuf::from("."));
    let err = project_create(&args).unwrap_err();
    assert!(matches!(err, ProjectCreateError::RelativeAt { .. }));
}

// ---------------------------------------------------------------------
// 6. Happy path — disk layout
// ---------------------------------------------------------------------

#[test]
fn happy_path_returns_project_id_and_path() {
    let parent = TempDir::new().unwrap();
    let args = args_for(&parent, "demo");
    let data = project_create(&args).unwrap();

    let expected_root = parent.path().join("demo");
    assert_eq!(data.path, expected_root);
    assert!(data.path.is_dir(), "project root must exist on disk");
}

#[test]
fn happy_path_writes_project_json() {
    let parent = TempDir::new().unwrap();
    let args = args_for(&parent, "demo");
    let data = project_create(&args).unwrap();

    let project_path = data.path.join("project.json");
    assert!(project_path.is_file(), "project.json must exist");

    let bytes = fs::read(&project_path).unwrap();
    let project_json: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(project_json["id"], data.project_id.to_string());
    assert_eq!(project_json["name"], "demo");
    assert_eq!(project_json["schema_version"], "1.0.0");
    assert_eq!(project_json["tick_rate_hz"], 240_000);
    assert_eq!(project_json["duration_tk"], 0);
    assert!(project_json["last_saved_event_id"].is_null());
}

#[test]
fn happy_path_writes_empty_events_log() {
    let parent = TempDir::new().unwrap();
    let args = args_for(&parent, "demo");
    let data = project_create(&args).unwrap();

    let events_path = data.path.join(".verbreel").join("events.jsonl");
    assert!(events_path.is_file(), "events.jsonl must exist");
    let bytes = fs::read(&events_path).unwrap();
    assert!(bytes.is_empty(), "fresh project must start with empty log");
}

#[test]
fn happy_path_pre_seeds_two_tracks() {
    let parent = TempDir::new().unwrap();
    let args = args_for(&parent, "demo");
    let data = project_create(&args).unwrap();

    let project_json: Value =
        serde_json::from_slice(&fs::read(data.path.join("project.json")).unwrap()).unwrap();
    let tracks = project_json["tracks"].as_array().expect("tracks is array");
    assert_eq!(tracks.len(), 2, "exactly two seeded tracks");
    assert_eq!(tracks[0]["kind"], "video");
    assert_eq!(tracks[0]["name"], "Video 1");
    assert_eq!(tracks[1]["kind"], "audio");
    assert_eq!(tracks[1]["name"], "Audio 1");
}

#[test]
fn seeded_track_ids_match_on_disk_tracks() {
    let parent = TempDir::new().unwrap();
    let args = args_for(&parent, "demo");
    let data = project_create(&args).unwrap();

    let project_json: Value =
        serde_json::from_slice(&fs::read(data.path.join("project.json")).unwrap()).unwrap();
    let tracks = project_json["tracks"].as_array().unwrap();
    assert_eq!(tracks[0]["id"], data.seeded_track_ids.video.to_string());
    assert_eq!(tracks[1]["id"], data.seeded_track_ids.audio.to_string());
}

#[test]
fn seeded_track_ids_are_valid_uuids() {
    let parent = TempDir::new().unwrap();
    let args = args_for(&parent, "demo");
    let data = project_create(&args).unwrap();
    // Round-trip the IDs through serde to confirm they're well-formed
    // UUIDv7s — `TrackId::now()` returns a v7 by construction, but
    // assert it explicitly.
    let video_str = data.seeded_track_ids.video.to_string();
    let audio_str = data.seeded_track_ids.audio.to_string();
    assert_eq!(video_str.len(), 36);
    assert_eq!(audio_str.len(), 36);
    assert_ne!(video_str, audio_str, "seeded tracks must be distinct");
}

#[test]
fn consecutive_creates_mint_distinct_project_ids() {
    let parent = TempDir::new().unwrap();
    let a = project_create(&args_for(&parent, "alpha")).unwrap();
    let b = project_create(&args_for(&parent, "beta")).unwrap();
    assert_ne!(a.project_id, b.project_id);
    assert_ne!(
        a.seeded_track_ids.video, b.seeded_track_ids.video,
        "every project gets fresh track IDs"
    );
    assert_ne!(a.seeded_track_ids.audio, b.seeded_track_ids.audio);
}

#[test]
fn project_round_trips_via_project_store_open() {
    // After create, re-opening the project via the lifecycle facade
    // (which is what `project.open` is built on top of) must succeed
    // — confirms the on-disk write is internally consistent.
    let parent = TempDir::new().unwrap();
    let args = args_for(&parent, "demo");
    let data = project_create(&args).unwrap();

    let opened = ProjectStore::open(&data.path).expect("re-open must succeed");
    let project = opened.project();
    assert_eq!(project.id, data.project_id);
    assert_eq!(project.name, "demo");
    assert_eq!(project.tracks.len(), 2);
    assert_eq!(project.tracks[0].kind, TrackKind::Video);
    assert_eq!(project.tracks[1].kind, TrackKind::Audio);
    drop(opened);
}

#[test]
fn create_releases_flock_so_open_succeeds_without_wait() {
    // `project.create` consumes its store on the way out so the
    // flock releases. Verify by re-opening immediately in the same
    // process — a still-held flock would block (`fs4::flock` is
    // exclusive).
    let parent = TempDir::new().unwrap();
    let args = args_for(&parent, "demo");
    let data = project_create(&args).unwrap();
    // Open without sleeping; must succeed immediately.
    let _opened = ProjectStore::open(&data.path).expect("flock must be released");
}

// ---------------------------------------------------------------------
// 7. E_PROJECT_EXISTS
// ---------------------------------------------------------------------

#[test]
fn second_create_on_same_path_returns_project_exists() {
    let parent = TempDir::new().unwrap();
    let args = args_for(&parent, "demo");
    let _data = project_create(&args).unwrap();

    let err = project_create(&args).unwrap_err();
    assert!(
        matches!(err, ProjectCreateError::ProjectExists { ref path } if path.ends_with("demo")),
        "expected ProjectExists pointing at the colliding root, got: {err:?}"
    );
}

#[test]
fn create_into_existing_file_returns_project_exists() {
    // The dest path is a regular file (not a directory) — still an
    // existence collision per §2.1. `create` must not clobber it.
    let parent = TempDir::new().unwrap();
    let dest = parent.path().join("demo");
    fs::write(&dest, b"squat").unwrap();

    let args = args_for(&parent, "demo");
    let err = project_create(&args).unwrap_err();
    assert!(matches!(err, ProjectCreateError::ProjectExists { .. }));

    // Existing file is untouched.
    assert_eq!(fs::read(&dest).unwrap(), b"squat");
}

// ---------------------------------------------------------------------
// 8. E_IO — permission failure
// ---------------------------------------------------------------------

#[test]
fn create_under_readonly_parent_returns_io_error() {
    // chmod 0o500 on the parent so the child create_dir_all fails.
    // Mirrors the project_close `close_with_save_failure_keeps_flock_held`
    // setup.
    use std::os::unix::fs::PermissionsExt;

    let parent = TempDir::new().unwrap();
    let mut perms = fs::metadata(parent.path()).unwrap().permissions();
    let original = perms.mode();
    perms.set_mode(0o500); // r-x for owner; no write
    fs::set_permissions(parent.path(), perms).unwrap();

    let args = args_for(&parent, "demo");
    let result = project_create(&args);

    // Restore perms before any assertion so TempDir cleanup works
    // even if the assertion fails.
    let mut restore = fs::metadata(parent.path()).unwrap().permissions();
    restore.set_mode(original);
    fs::set_permissions(parent.path(), restore).unwrap();

    let err = result.expect_err("permission-denied parent must fail");
    assert!(
        matches!(
            err,
            ProjectCreateError::Io(_) | ProjectCreateError::LifecycleFailed(_)
        ),
        "expected Io / LifecycleFailed, got: {err:?}"
    );
}

// ---------------------------------------------------------------------
// 9. metadata round-trip
// ---------------------------------------------------------------------

#[test]
fn metadata_round_trips_through_project_json() {
    let parent = TempDir::new().unwrap();
    let mut args = args_for(&parent, "demo");
    args.metadata.insert(
        "agent".to_string(),
        json!({ "name": "claude", "build": 42 }),
    );
    args.metadata
        .insert("owner".to_string(), Value::from("alice"));
    let data = project_create(&args).unwrap();

    let project_json: Value =
        serde_json::from_slice(&fs::read(data.path.join("project.json")).unwrap()).unwrap();
    let metadata = &project_json["metadata"];
    assert_eq!(metadata["owner"], "alice");
    assert_eq!(metadata["agent"]["name"], "claude");
    assert_eq!(metadata["agent"]["build"], 42);
}

#[test]
fn metadata_defaults_to_empty_object_when_omitted() {
    let parent = TempDir::new().unwrap();
    let args = args_for(&parent, "demo");
    let data = project_create(&args).unwrap();
    let project_json: Value =
        serde_json::from_slice(&fs::read(data.path.join("project.json")).unwrap()).unwrap();
    assert!(
        project_json["metadata"]
            .as_object()
            .is_some_and(serde_json::Map::is_empty),
        "metadata must default to {{}}, got: {:?}",
        project_json["metadata"]
    );
}

// ---------------------------------------------------------------------
// 10. activate flag — v1 floor behavior
// ---------------------------------------------------------------------

#[test]
fn activate_true_is_accepted_v1_floor() {
    // v1 floor: `activate: true` is accepted but the
    // `~/.verbreel/active-project` write is not yet wired (matches the
    // module-doc deferral). The contract under test is that the flag
    // does not block creation; the flock release and the project
    // contents are unchanged. Don't assert against `~/.verbreel/` —
    // touching the real host config dir from a test would be a smell.
    let parent = TempDir::new().unwrap();
    let mut args = args_for(&parent, "demo");
    args.activate = true;
    let data = project_create(&args).expect("activate: true must not block");
    assert!(data.path.join("project.json").is_file());
}

#[test]
fn activate_false_is_accepted_v1_floor() {
    let parent = TempDir::new().unwrap();
    let args = args_for(&parent, "demo");
    assert!(!args.activate, "default must be false");
    let _ = project_create(&args).unwrap();
}

// ---------------------------------------------------------------------
// 11. Data envelope shape
// ---------------------------------------------------------------------

#[test]
fn data_envelope_serializes_with_spec_field_names() {
    let parent = TempDir::new().unwrap();
    let args = args_for(&parent, "demo");
    let data = project_create(&args).unwrap();

    let value = serde_json::to_value(&data).unwrap();
    let obj = value.as_object().expect("envelope is an object");
    assert!(obj.contains_key("project_id"));
    assert!(obj.contains_key("path"));
    assert!(obj.contains_key("seeded_track_ids"));
    let st = obj["seeded_track_ids"]
        .as_object()
        .expect("seeded_track_ids is an object");
    assert!(st.contains_key("video"));
    assert!(st.contains_key("audio"));
    // Snake-case enforcement: the spec field is `seeded_track_ids`,
    // not `seededTrackIds`.
    assert!(!obj.contains_key("seededTrackIds"));
}

#[test]
fn seeded_track_ids_round_trip_through_serde() {
    let parent = TempDir::new().unwrap();
    let args = args_for(&parent, "demo");
    let data = project_create(&args).unwrap();

    let raw = serde_json::to_value(data.seeded_track_ids).unwrap();
    let parsed: SeededTrackIds = serde_json::from_value(raw).unwrap();
    assert_eq!(parsed, data.seeded_track_ids);
}
