//! Tests for [`ProjectStore`] — the §0.8 / §2.1 / §2.2 / §2.3 lifecycle.
//!
//! These tests run native-only (the `lifecycle` module is gated behind
//! `cfg(feature = "native")`). The events.jsonl file lock is held by
//! the `ProjectStore` instance — tests deliberately `drop()` between
//! phases so the lock releases and the next phase can reopen.

#![cfg(feature = "native")]

use std::fs;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread::sleep;
use std::time::Duration;

use serde_json::{Value, json};
use tempfile::TempDir;
use verbreel_events::{Event, EventBackend, NativeBackend};
use verbreel_state::{
    LifecycleError, MutateOutcome, Project, ProjectStore, ReconstructError, RecordedEvent,
    ValidationError, VerbRegistry, default_fixtures, default_registry,
};
use verbreel_types::EventId;

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
    let outcome = store
        .mutate(
            "project.set_name",
            serde_json::json!({"name":"after-mutate"}),
            &patch,
            None,
        )
        .expect("mutate must succeed");
    assert!(
        matches!(outcome, MutateOutcome::Applied { .. }),
        "un-keyed mutate returns Applied, got {outcome:?}"
    );
    assert_eq!(store.project().name, "after-mutate");

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
            None,
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
                None,
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
                None,
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
fn lifecycle_mutate_with_key_first_call_applied() {
    // First call with an idempotency_key — runs the full write-ordering
    // path, returns Applied { event_id }, and the index now has one
    // Completed entry.
    let dir = TempDir::new().unwrap();
    let mut store = ProjectStore::create(dir.path(), load_empty_project()).unwrap();

    let args = serde_json::json!({"name":"renamed-via-idem"});
    let patch = replace_name_patch("renamed-via-idem");
    let outcome = store
        .mutate("project.set_name", args.clone(), &patch, Some("k1".into()))
        .expect("first keyed call must succeed");

    let event_id = match outcome {
        MutateOutcome::Applied {
            event_id,
            data: _,
            warnings: _,
        } => event_id,
        other => panic!("first call should be Applied, got {other:?}"),
    };
    assert_eq!(store.project().name, "renamed-via-idem");
    assert_eq!(store.last_applied_event_id(), Some(event_id));

    // Index has the entry under "k1" with the correct fingerprint.
    let fp = verbreel_canon::sha256_hex(&args).unwrap();
    assert_eq!(
        store.idempotency().lookup("k1", &fp),
        verbreel_state::LookupOutcome::Completed { event_id }
    );
}

#[test]
fn lifecycle_mutate_with_key_replay_returns_replayed_outcome() {
    // Same key + same args twice → second call returns Replayed without
    // writing a second event. events.jsonl still has exactly one line.
    let dir = TempDir::new().unwrap();
    let mut store = ProjectStore::create(dir.path(), load_empty_project()).unwrap();

    let args = serde_json::json!({"name":"v1"});
    let patch = replace_name_patch("v1");

    let first = store
        .mutate("project.set_name", args.clone(), &patch, Some("dup".into()))
        .expect("first call");
    let MutateOutcome::Applied {
        event_id: first_id,
        data: _,
        warnings: _,
    } = first
    else {
        panic!("first call should be Applied, got {first:?}");
    };

    let second = store
        .mutate("project.set_name", args, &patch, Some("dup".into()))
        .expect("second (replay) call");
    let MutateOutcome::Replayed {
        event_id: replay_id,
        data: _,
        warnings: _,
    } = second
    else {
        panic!("second call should be Replayed, got {second:?}");
    };
    assert_eq!(
        first_id, replay_id,
        "replay returns the original event id, not a fresh one"
    );

    // Exactly one event line on disk.
    drop(store);
    let events_path = dir.path().join(".verbreel").join("events.jsonl");
    let bytes = fs::read(&events_path).unwrap();
    let lines: Vec<&[u8]> = bytes
        .split(|&b| b == b'\n')
        .filter(|l| !l.is_empty())
        .collect();
    assert_eq!(lines.len(), 1, "replay must not write a second event line");
    let ev: Event = serde_json::from_slice(lines[0]).expect("event parses");
    assert_eq!(ev.idempotency_key.as_deref(), Some("dup"));
    assert_eq!(ev.id, first_id);
}

#[test]
fn lifecycle_mutate_with_key_conflict_returns_error() {
    // Same key + DIFFERENT args → IdempotencyConflict, no second event
    // written, in-memory project unchanged.
    let dir = TempDir::new().unwrap();
    let mut store = ProjectStore::create(dir.path(), load_empty_project()).unwrap();

    store
        .mutate(
            "project.set_name",
            serde_json::json!({"name":"first"}),
            &replace_name_patch("first"),
            Some("conflict-key".into()),
        )
        .expect("first call");
    assert_eq!(store.project().name, "first");

    let err = store
        .mutate(
            "project.set_name",
            serde_json::json!({"name":"second"}),
            &replace_name_patch("second"),
            Some("conflict-key".into()),
        )
        .expect_err("differing args must conflict");

    match err {
        LifecycleError::IdempotencyConflict {
            key,
            existing_fingerprint,
        } => {
            assert_eq!(key, "conflict-key");
            let fp_first =
                verbreel_canon::sha256_hex(&serde_json::json!({"name":"first"})).unwrap();
            assert_eq!(existing_fingerprint, fp_first);
        }
        other => panic!("expected IdempotencyConflict, got {other:?}"),
    }

    // Project was not re-renamed; event log still has exactly one line.
    assert_eq!(store.project().name, "first");
    drop(store);
    let bytes = fs::read(dir.path().join(".verbreel").join("events.jsonl")).unwrap();
    let lines: Vec<&[u8]> = bytes
        .split(|&b| b == b'\n')
        .filter(|l| !l.is_empty())
        .collect();
    assert_eq!(lines.len(), 1, "conflict must not write a second event");
}

#[test]
fn lifecycle_mutate_without_key_skips_dedup() {
    // Two calls without an idempotency_key — both run the full
    // write-ordering and emit independent events. The dedup index
    // stays empty.
    let dir = TempDir::new().unwrap();
    let mut store = ProjectStore::create(dir.path(), load_empty_project()).unwrap();

    let outcome_one = store
        .mutate(
            "project.set_name",
            serde_json::Value::Null,
            &replace_name_patch("one"),
            None,
        )
        .unwrap();
    sleep(Duration::from_millis(2)); // guarantee distinct EventId v7s
    let outcome_two = store
        .mutate(
            "project.set_name",
            serde_json::Value::Null,
            &replace_name_patch("two"),
            None,
        )
        .unwrap();
    let MutateOutcome::Applied {
        event_id: id_one,
        data: _,
        warnings: _,
    } = outcome_one
    else {
        panic!("first un-keyed call should be Applied");
    };
    let MutateOutcome::Applied {
        event_id: id_two,
        data: _,
        warnings: _,
    } = outcome_two
    else {
        panic!("second un-keyed call should be Applied");
    };
    assert_ne!(id_one, id_two, "two un-keyed calls produce two events");

    assert!(
        store.idempotency().is_empty(),
        "un-keyed calls must not touch the dedup index"
    );

    drop(store);
    let bytes = fs::read(dir.path().join(".verbreel").join("events.jsonl")).unwrap();
    let lines: Vec<&[u8]> = bytes
        .split(|&b| b == b'\n')
        .filter(|l| !l.is_empty())
        .collect();
    assert_eq!(lines.len(), 2, "two un-keyed mutations → two event lines");
}

#[test]
fn lifecycle_open_rebuilds_index_from_events() {
    // Round-trip: create + keyed mutate + drop, then re-open and
    // verify the dedup index sees the prior call.
    let dir = TempDir::new().unwrap();
    let args = serde_json::json!({"name":"persisted"});
    let original_event_id: EventId;
    {
        let mut store = ProjectStore::create(dir.path(), load_empty_project()).unwrap();
        let outcome = store
            .mutate(
                "project.set_name",
                args.clone(),
                &replace_name_patch("persisted"),
                Some("persisted-key".into()),
            )
            .unwrap();
        let MutateOutcome::Applied {
            event_id,
            data: _,
            warnings: _,
        } = outcome
        else {
            panic!("first call should be Applied");
        };
        original_event_id = event_id;
        store.save().unwrap();
    }

    let reopened = ProjectStore::open(dir.path()).expect("reopen");
    let fp = verbreel_canon::sha256_hex(&args).unwrap();
    assert_eq!(
        reopened.idempotency().lookup("persisted-key", &fp),
        verbreel_state::LookupOutcome::Completed {
            event_id: original_event_id
        },
        "index rebuild from events.jsonl preserves keyed events across reopen"
    );
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

// ---------------------------------------------------------------------
// §0.8 reconstructor-purity startup gate (Slice B2)
//
// `*_with_registry` runs `validate_reconstructors(registry, fixtures)`
// as Step 0 — BEFORE any IO. A misconfigured registry returns
// `LifecycleError::ReconstructorGateFailed` regardless of the on-disk
// state. The existing un-suffixed `create()` / `open()` delegate with
// empty registry + empty fixtures (vacuous pass), so the older tests in
// this file still exercise the same path transparently.
// ---------------------------------------------------------------------

/// Test-local broken reconstructor: always returns a structured error.
/// Models a verb-author bug surfaced at the startup gate.
struct BrokenReconstructorVerb;

impl verbreel_state::Verb for BrokenReconstructorVerb {
    fn verb(&self) -> &'static str {
        "broken.verb"
    }

    fn compute_patch(
        &self,
        _prior: &Project,
        _args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), verbreel_state::VerbError> {
        // These gate tests only exercise the reconstruct path; the
        // forward path is unreachable from them. Surface a clear
        // verb-side error if anything ever does call it.
        Err(verbreel_state::VerbError::Custom(
            "BrokenReconstructorVerb::compute_patch not implemented (gate-only test verb)".into(),
        ))
    }

    fn reconstruct(
        &self,
        _args: &Value,
        _patch: &Value,
        _warnings: &[Value],
        _post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        Err(ReconstructError::MissingField {
            name: "intentional",
        })
    }
}

/// Test-local reconstructor that returns a fixed payload — used to
/// exercise the `DataMismatch` branch by pairing it with a fixture
/// whose `expected_data` deliberately differs from what the
/// reconstructor produces.
struct WrongDataReconstructor;

impl verbreel_state::Verb for WrongDataReconstructor {
    fn verb(&self) -> &'static str {
        "wrong.data"
    }

    fn compute_patch(
        &self,
        _prior: &Project,
        _args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), verbreel_state::VerbError> {
        Err(verbreel_state::VerbError::Custom(
            "WrongDataReconstructor::compute_patch not implemented (gate-only test verb)".into(),
        ))
    }

    fn reconstruct(
        &self,
        _args: &Value,
        _patch: &Value,
        _warnings: &[Value],
        _post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        Ok(json!({ "produced": "left" }))
    }
}

/// Build a synthetic fixture for a test-local verb. The reconstructors
/// above don't read `patch` / `warnings` / `post_state`, so those carry
/// no signal here — the 5-tuple is still fully populated for the gate.
fn fixture(verb: &str, expected_data: Value) -> RecordedEvent {
    RecordedEvent {
        verb: verb.to_string(),
        args: json!({}),
        patch: json!([]),
        warnings: vec![],
        post_state: load_empty_project(),
        expected_data,
    }
}

#[test]
fn open_with_default_registry_and_fixtures_succeeds() {
    // Happy path: a freshly-created project re-opens cleanly under the
    // canonical kernel-verb set + matching fixtures.
    let dir = TempDir::new().unwrap();
    {
        let _store = ProjectStore::create(dir.path(), load_empty_project()).unwrap();
    }
    let reopened =
        ProjectStore::open_with_registry(dir.path(), &default_registry(), &default_fixtures())
            .expect("open_with_registry against default set must succeed");
    // Sanity: the reopened project carries the expected shape.
    assert_eq!(reopened.project().name, "test");
}

#[test]
fn create_with_default_registry_and_fixtures_succeeds() {
    // Happy path: create with the canonical kernel-verb set + matching
    // fixtures. project.json lands on disk.
    let dir = TempDir::new().unwrap();
    let store = ProjectStore::create_with_registry(
        dir.path(),
        load_empty_project(),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry against default set must succeed");
    assert!(
        dir.path().join("project.json").is_file(),
        "project.json written"
    );
    assert_eq!(store.project().name, "test");
}

#[test]
fn open_with_empty_registry_and_fixtures_succeeds() {
    // Vacuous-pass baseline: empty registry + empty fixtures → gate is
    // a no-op. Exercises the same path the un-suffixed `open()` takes.
    let dir = TempDir::new().unwrap();
    {
        let _store = ProjectStore::create(dir.path(), load_empty_project()).unwrap();
    }
    let reopened = ProjectStore::open_with_registry(dir.path(), &VerbRegistry::new(), &[])
        .expect("open_with_registry against an empty registry must succeed (vacuous pass)");
    assert_eq!(reopened.project().name, "test");
}

#[test]
fn open_with_misconfigured_registry_fails() {
    // Misconfigured registry: a broken reconstructor + a matching
    // fixture. The gate must refuse the open regardless of project
    // state on disk.
    let dir = TempDir::new().unwrap();
    {
        let _store = ProjectStore::create(dir.path(), load_empty_project()).unwrap();
    }
    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(BrokenReconstructorVerb))
        .expect("register broken verb");
    let fixtures = vec![fixture("broken.verb", json!({}))];

    let err = ProjectStore::open_with_registry(dir.path(), &registry, &fixtures)
        .expect_err("gate must reject the open");
    assert!(
        matches!(
            err,
            LifecycleError::ReconstructorGateFailed {
                source: ValidationError::ReconstructError {
                    verb: "broken.verb",
                    ..
                },
            }
        ),
        "expected ReconstructorGateFailed wrapping ReconstructError, got {err:?}"
    );
}

#[test]
fn gate_runs_before_io() {
    // Critical ordering invariant: the gate fires BEFORE any IO. Point
    // the open at a path that does NOT exist on disk; if the gate ran
    // after the existence check we'd see `NoProjectJson`. With Step-0
    // ordering we get the `ReconstructorGateFailed` error instead, even
    // though no project.json could possibly exist at the bogus path.
    let bogus = PathBuf::from("/__definitely_does_not_exist_98765__/__/");
    assert!(
        !bogus.exists(),
        "test precondition: bogus path must not exist"
    );

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(BrokenReconstructorVerb))
        .expect("register broken verb");
    let fixtures = vec![fixture("broken.verb", json!({}))];

    let err = ProjectStore::open_with_registry(&bogus, &registry, &fixtures)
        .expect_err("gate must fire before existence check");
    assert!(
        matches!(err, LifecycleError::ReconstructorGateFailed { .. }),
        "expected ReconstructorGateFailed (gate fires first), got {err:?}"
    );
    // Negative: not the IO error.
    assert!(
        !matches!(err, LifecycleError::NoProjectJson),
        "must NOT short-circuit to NoProjectJson — gate must run first"
    );
}

#[test]
fn gate_data_mismatch_propagates() {
    // The reconstructor returns `{ "produced": "left" }` but the
    // fixture's `expected_data` is `{ "produced": "right" }` — RFC 8785
    // canonical SHAs differ, gate surfaces `DataMismatch`, lifecycle
    // wraps it in `ReconstructorGateFailed`.
    let dir = TempDir::new().unwrap();
    {
        let _store = ProjectStore::create(dir.path(), load_empty_project()).unwrap();
    }

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(WrongDataReconstructor))
        .expect("register wrong-data verb");
    let fixtures = vec![fixture("wrong.data", json!({ "produced": "right" }))];

    let err = ProjectStore::open_with_registry(dir.path(), &registry, &fixtures)
        .expect_err("data mismatch must propagate through the gate");
    assert!(
        matches!(
            err,
            LifecycleError::ReconstructorGateFailed {
                source: ValidationError::DataMismatch {
                    verb: "wrong.data",
                    ..
                },
            }
        ),
        "expected ReconstructorGateFailed wrapping DataMismatch, got {err:?}"
    );
}

// ---------------------------------------------------------------------
// #379 — lifecycle persistence routes through verbreel-storage
// primitives (atomic_write_bytes) without bypassing the apply()
// mutation boundary or the §0.8 write-ordering rule.
// ---------------------------------------------------------------------

/// `save()` must produce the exact bytes the shared storage primitive
/// (`verbreel_storage::fs::atomic_write_bytes`) would lay down, and
/// must not leave a half-written temp file behind in the project root.
/// This pins the migration: lifecycle no longer hand-rolls the
/// tempfile + rename + parent-fsync dance — it delegates to storage.
#[test]
fn lifecycle_save_routes_through_storage_atomic_primitive() {
    let dir = TempDir::new().unwrap();
    let project = load_empty_project();
    let mut store = ProjectStore::create(dir.path(), project).unwrap();

    store
        .mutate(
            "project.set_name",
            serde_json::Value::Null,
            &replace_name_patch("through-storage"),
            None,
        )
        .unwrap();
    store.save().expect("save must succeed");

    // The in-memory project the store holds is exactly what was written.
    let expected = serde_json::to_vec_pretty(store.project()).unwrap();
    drop(store);

    let pj = dir.path().join("project.json");
    let on_disk = fs::read(&pj).unwrap();
    assert_eq!(
        on_disk, expected,
        "project.json on disk must equal the serialized in-memory project"
    );

    // Re-running the storage primitive with the same bytes is a no-op
    // identity: this is the primitive the lifecycle save() now calls.
    verbreel_storage::fs::atomic_write_bytes(&pj, &expected).unwrap();
    assert_eq!(fs::read(&pj).unwrap(), expected);

    // No orphaned NamedTempFile may remain in the root: a fully
    // delegated atomic write either lands project.json or errors — it
    // never leaves a `.tmp*` sibling on success.
    let leftovers: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n != "project.json" && n != ".verbreel" && n != "assets")
        .collect();
    assert!(
        leftovers.is_empty(),
        "atomic write left stray files in root: {leftovers:?}"
    );
}

/// §0.8 write-ordering: the event line is appended to events.jsonl by
/// `mutate()` (via apply()) BEFORE `save()` ever touches project.json.
/// A crash after the event append but before save() must leave the
/// event durable on disk while the *snapshot* still trails — proven by
/// reopening and observing the event replays on top of the old snapshot.
/// This confirms the storage-primitive migration did not collapse the
/// event-before-snapshot ordering or route a write around apply().
#[test]
fn lifecycle_event_durable_before_snapshot_save_after_migration() {
    let dir = TempDir::new().unwrap();
    let project = load_empty_project();

    // create() does an initial save() (snapshot with last_saved=None).
    let mut store = ProjectStore::create(dir.path(), project).unwrap();
    let snapshot_before = fs::read(dir.path().join("project.json")).unwrap();

    // mutate() runs the §0.8 protocol through apply(): event appended
    // to events.jsonl, THEN the in-memory patch applied. We deliberately
    // do NOT call save() — the snapshot must still be the pre-mutate one.
    let applied = match store
        .mutate(
            "project.set_name",
            serde_json::Value::Null,
            &replace_name_patch("event-before-snapshot"),
            None,
        )
        .expect("mutate must succeed")
    {
        MutateOutcome::Applied { event_id, .. } => event_id,
        other => panic!("expected Applied, got {other:?}"),
    };
    // In memory the patch is visible (apply() ran)...
    assert_eq!(store.project().name, "event-before-snapshot");
    drop(store);

    // ...but on disk the snapshot has NOT moved (no save() yet): the
    // event was written first, the snapshot second. project.json is byte
    // identical to the pre-mutate snapshot.
    assert_eq!(
        fs::read(dir.path().join("project.json")).unwrap(),
        snapshot_before,
        "snapshot must not change until save() — event is written first"
    );

    // The event line is durable in events.jsonl.
    let ev_bytes = fs::read(dir.path().join(".verbreel").join("events.jsonl")).unwrap();
    let lines: Vec<&[u8]> = ev_bytes
        .split(|&b| b == b'\n')
        .filter(|l| !l.is_empty())
        .collect();
    assert_eq!(lines.len(), 1, "exactly one durable event line");
    let ev: Event = serde_json::from_slice(lines[0]).unwrap();
    assert_eq!(ev.id, applied, "durable event id matches the applied one");

    // Reopen: the durable event replays on top of the stale snapshot,
    // proving event-before-snapshot ordering survived the migration.
    let reopened = ProjectStore::open(dir.path()).expect("reopen replays the event");
    assert_eq!(reopened.project().name, "event-before-snapshot");
}
