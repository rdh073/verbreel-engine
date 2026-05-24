//! Tests for [`ProjectStore`] — the §0.8 / §2.1 / §2.2 / §2.3 lifecycle.
//!
//! These tests run native-only (the `lifecycle` module is gated behind
//! `cfg(feature = "native")`). The events.jsonl file lock is held by
//! the `ProjectStore` instance — tests deliberately `drop()` between
//! phases so the lock releases and the next phase can reopen.

#![cfg(feature = "native")]

use std::fs;
use std::io::Write as _;
use std::thread::sleep;
use std::time::Duration;

use tempfile::TempDir;
use verbreel_events::{Event, EventBackend, NativeBackend};
use verbreel_state::{LifecycleError, Project, ProjectStore};

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");

fn load_empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture → Project")
}

fn replace_name_patch(new: &str) -> json_patch::Patch {
    let body = format!(r#"[{{"op":"replace","path":"/name","value":"{new}"}}]"#);
    serde_json::from_str(&body).unwrap_or_else(|e| panic!("patch literal {body:?}: {e}"))
}

#[test]
fn lifecycle_create_writes_project_json() {
    let dir = TempDir::new().unwrap();
    let project = load_empty_project();

    let store = ProjectStore::create(dir.path(), project.clone())
        .expect("create must succeed on empty dir");

    let project_json = dir.path().join("project.json");
    assert!(
        project_json.is_file(),
        "project.json must exist after create"
    );

    // The on-disk project.json deserializes back to the same Project.
    let bytes = fs::read(&project_json).unwrap();
    let parsed: Project = serde_json::from_slice(&bytes).expect("project.json round-trips");
    assert_eq!(parsed, project, "on-disk project equals input");

    // .verbreel/events.jsonl exists (zero-length).
    let events = dir.path().join(".verbreel").join("events.jsonl");
    assert!(events.is_file(), "events.jsonl created");

    // Sanity on the store's project reference.
    assert_eq!(store.project(), &project);
}

#[test]
fn lifecycle_create_fails_if_path_already_has_project() {
    let dir = TempDir::new().unwrap();
    let project = load_empty_project();

    {
        let _store = ProjectStore::create(dir.path(), project.clone()).unwrap();
    }
    // Lock dropped — try to create again. Must refuse (project.json exists).
    let err = ProjectStore::create(dir.path(), project)
        .expect_err("create on existing project.json must fail");
    assert!(
        matches!(err, LifecycleError::ProjectAlreadyExists),
        "expected ProjectAlreadyExists, got {err:?}"
    );
}

#[test]
fn lifecycle_mutate_writes_event_line() {
    let dir = TempDir::new().unwrap();
    let project = load_empty_project();
    let mut store = ProjectStore::create(dir.path(), project).unwrap();

    let patch = replace_name_patch("after-mutate");
    let p = store
        .mutate(
            "project.set_name",
            serde_json::json!({"name":"after-mutate"}),
            &patch,
        )
        .expect("mutate must succeed");
    assert_eq!(p.name, "after-mutate");

    // events.jsonl now has exactly one line — read the file directly.
    drop(store); // release the lock so we can read normally
    let events_path = dir.path().join(".verbreel").join("events.jsonl");
    let bytes = fs::read(&events_path).unwrap();
    let lines: Vec<&[u8]> = bytes
        .split(|&b| b == b'\n')
        .filter(|l| !l.is_empty())
        .collect();
    assert_eq!(
        lines.len(),
        1,
        "exactly one event line after a single mutate"
    );

    let ev: Event = serde_json::from_slice(lines[0]).expect("event line parses");
    assert_eq!(ev.verb, "project.set_name");
    assert_eq!(ev.patch, patch);
}

#[test]
fn lifecycle_mutate_then_save_bumps_last_saved_event_id() {
    let dir = TempDir::new().unwrap();
    let project = load_empty_project();
    let mut store = ProjectStore::create(dir.path(), project).unwrap();

    // Before any mutation, last_saved_event_id remains whatever the
    // input project had (None for the empty fixture).
    assert!(store.project().last_saved_event_id.is_none());

    store
        .mutate(
            "project.set_name",
            serde_json::Value::Null,
            &replace_name_patch("after"),
        )
        .unwrap();
    let applied = store.last_applied_event_id().expect("event applied");

    let info = store.save().expect("save must succeed");
    assert_eq!(store.project().last_saved_event_id, Some(applied));
    assert_eq!(info.path, dir.path().join("project.json"));

    // Read the on-disk project.json and confirm the bump persisted.
    drop(store);
    let bytes = fs::read(dir.path().join("project.json")).unwrap();
    let on_disk: Project = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(on_disk.last_saved_event_id, Some(applied));
}

#[test]
fn lifecycle_open_replays_post_save_events() {
    let dir = TempDir::new().unwrap();
    let project = load_empty_project();

    // Phase 1: create + mutate + save.
    {
        let mut store = ProjectStore::create(dir.path(), project).unwrap();
        store
            .mutate(
                "project.set_name",
                serde_json::Value::Null,
                &replace_name_patch("first"),
            )
            .unwrap();
        store.save().unwrap();
    }

    // Phase 2: hand-append a post-save event. Need an event id minted
    // *after* the snapshot's last_saved_event_id so the replay filter
    // keeps it. Sleep 2ms to guarantee millisecond separation.
    sleep(Duration::from_millis(2));
    let post_save_patch = replace_name_patch("post-save");
    let post_save_event = Event::new(
        "project.set_name",
        serde_json::Value::Null,
        post_save_patch.clone(),
    );
    {
        let backend =
            NativeBackend::open(dir.path().join(".verbreel").join("events.jsonl")).unwrap();
        let line = serde_json::to_string(&post_save_event).unwrap();
        backend.append(line.as_bytes()).unwrap();
    }

    // Phase 3: reopen. Replay must apply the post-save event.
    let reopened = ProjectStore::open(dir.path()).expect("reopen must succeed");
    assert_eq!(
        reopened.project().name,
        "post-save",
        "reopen replayed the hand-appended event"
    );
    assert_eq!(
        reopened.last_applied_event_id(),
        Some(post_save_event.id),
        "last_applied_event_id reflects the replayed event"
    );
}

#[test]
fn lifecycle_open_recovers_torn_last_line() {
    let dir = TempDir::new().unwrap();
    let project = load_empty_project();

    {
        let mut store = ProjectStore::create(dir.path(), project).unwrap();
        store
            .mutate(
                "project.set_name",
                serde_json::Value::Null,
                &replace_name_patch("ok"),
            )
            .unwrap();
        store.save().unwrap();
    }

    // Hand-append a torn last line — malformed JSON, no trailing \n.
    let events_path = dir.path().join(".verbreel").join("events.jsonl");
    let pre_torn_len = fs::metadata(&events_path).unwrap().len();
    {
        let mut f = fs::OpenOptions::new()
            .append(true)
            .open(&events_path)
            .unwrap();
        f.write_all(b"{this is not valid json").unwrap();
    }
    let with_torn_len = fs::metadata(&events_path).unwrap().len();
    assert!(with_torn_len > pre_torn_len, "torn bytes appended");

    // Reopen — must recover by truncating to last valid offset.
    let reopened = ProjectStore::open(dir.path()).expect("reopen recovers from torn line");
    let post_open_len = fs::metadata(&events_path).unwrap().len();
    assert_eq!(
        post_open_len, pre_torn_len,
        "events.jsonl truncated back to last valid offset"
    );
    // The valid event was already in the saved snapshot
    // (last_saved_event_id matches), so no replay happened.
    assert_eq!(reopened.project().name, "ok");
}

#[test]
fn lifecycle_save_is_atomic() {
    // Minimum bar per task spec: orphan tmp file in path is ignored
    // on next open(). NamedTempFile uses random suffixes, so any
    // leftover from a half-completed save() should not break a
    // subsequent open().
    let dir = TempDir::new().unwrap();
    let project = load_empty_project();
    {
        let _store = ProjectStore::create(dir.path(), project).unwrap();
    }
    // Plant an orphan tmp file.
    let orphan = dir.path().join(".tmp-orphan-xyz");
    fs::write(&orphan, b"leftover from a hypothetical half-completed save").unwrap();
    // Reopen — must succeed despite the orphan.
    let reopened = ProjectStore::open(dir.path()).expect("orphan tmp ignored");
    assert!(
        orphan.exists(),
        "orphan still present (we don't clean it up here)"
    );
    // Confirm we have a working store back.
    assert_eq!(reopened.project().tracks.len(), 2);
}

#[test]
fn lifecycle_apply_step_3_failure_does_not_lose_event() {
    // The §0.8 contract says: even if step 3 (apply()) fails after
    // step 2 (event written + fsynced), the event is on disk and the
    // next open() will replay it cleanly. In current impl, step 1
    // (validate clone) and step 3 (real apply) use the same
    // deterministic apply(), so step 3 never fails when step 1
    // passed. This test exercises the durability property *as if*
    // step 3 had failed: write an event directly to disk (skipping
    // the in-memory apply), drop, reopen — the event replays.
    let dir = TempDir::new().unwrap();
    let project = load_empty_project();
    {
        let _store = ProjectStore::create(dir.path(), project).unwrap();
    }
    // Simulate a "wrote event but didn't apply" by hand-appending.
    sleep(Duration::from_millis(2));
    let patch = replace_name_patch("survived-step-3-failure");
    let event = Event::new("project.set_name", serde_json::Value::Null, patch);
    {
        let backend =
            NativeBackend::open(dir.path().join(".verbreel").join("events.jsonl")).unwrap();
        let line = serde_json::to_string(&event).unwrap();
        backend.append(line.as_bytes()).unwrap();
    }

    let reopened = ProjectStore::open(dir.path()).expect("reopen replays the durable event");
    assert_eq!(
        reopened.project().name,
        "survived-step-3-failure",
        "event durability survives a hypothetical step-3 failure"
    );
}

#[test]
fn event_round_trip_serde() {
    // Extended Event shape round-trips through serde without loss.
    let patch: json_patch::Patch =
        serde_json::from_str(r#"[{"op":"replace","path":"/name","value":"x"}]"#).unwrap();
    let mut ev = Event::new("project.set_name", serde_json::json!({"name":"x"}), patch);
    ev.warnings.push(serde_json::json!({"code":"W_DUMMY"}));
    ev.idempotency_key = Some("idem-key-abc".to_string());

    let s = serde_json::to_string(&ev).unwrap();
    let back: Event = serde_json::from_str(&s).unwrap();
    assert_eq!(ev, back, "Event round-trips through serde without loss");
}

#[test]
fn event_line_is_one_line_no_embedded_newlines() {
    // The single-line invariant is the basis of events.jsonl: each
    // serialized event must contain no embedded \n; the backend adds
    // the terminator. Use a non-trivial patch with multi-byte content.
    let patch: json_patch::Patch = serde_json::from_str(
        r#"[
            {"op":"replace","path":"/name","value":"with\nembedded\nnewlines"},
            {"op":"add","path":"/markers/-","value":{
                "id":"01890000-0000-7000-8000-0000000000aa",
                "time_tk":100,
                "label":"multi\nline\nlabel"
            }}
        ]"#,
    )
    .unwrap();
    let ev = Event::new(
        "project.set_metadata",
        serde_json::json!({"weird":"v\nv"}),
        patch,
    );
    let s = serde_json::to_string(&ev).unwrap();
    // Embedded newlines in *string content* must be escaped to `\n`
    // (a backslash + 'n', NOT a literal newline byte).
    assert!(
        !s.as_bytes().contains(&b'\n'),
        "serialized event must contain no raw newline bytes (got: {s:?})"
    );
    // But the round-trip must preserve the string content's literal
    // newlines.
    let back: Event = serde_json::from_str(&s).unwrap();
    assert_eq!(back, ev);
}

#[test]
fn lifecycle_save_returns_bytes_written() {
    let dir = TempDir::new().unwrap();
    let project = load_empty_project();
    let mut store = ProjectStore::create(dir.path(), project).unwrap();

    let info = store.save().expect("save must succeed");
    let on_disk_size = fs::metadata(&info.path).unwrap().len();
    assert_eq!(
        info.bytes_written, on_disk_size,
        "SaveInfo.bytes_written matches actual file size"
    );
    assert!(info.bytes_written > 0, "non-empty project.json");
}
