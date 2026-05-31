//! Integration tests for [`verbreel_state::Engine`] — the shared
//! verb-dispatch surface (§commands.md:3-9) + [`verbreel_state::Envelope`]
//! (§0.1).
//!
//! Covers the issue #443 "Test target" floor: project-less reads, the
//! create → open → mutate persistence path, the project-scoped read
//! fast-path, unknown verbs, project-not-found, close eviction,
//! idempotency replay, dry-run, and the exact §0.1 envelope shape.

#![cfg(feature = "native")]

use serde_json::{Value, json};
use tempfile::TempDir;
use verbreel_state::{Engine, Envelope};

/// Create a fresh project under `dir` and return its `project_id`. Drives
/// the engine's `project.create` lifecycle handler, which writes the
/// project to disk AND opens it into the engine's map.
fn create_project(engine: &mut Engine, dir: &TempDir, name: &str) -> String {
    let env = engine.dispatch(
        "project.create",
        json!({
            "name": name,
            "canvas": "1080x1920",
            "at": dir.path(),
        }),
    );
    assert!(env.is_ok(), "project.create must succeed: {env:?}");
    let Envelope::Ok { data, .. } = env else {
        unreachable!("checked is_ok");
    };
    data["project_id"]
        .as_str()
        .expect("create data carries project_id")
        .to_string()
}

/// Count `events.jsonl` lines under a project root.
fn event_count(root: &std::path::Path) -> usize {
    let log = root.join(".verbreel").join("events.jsonl");
    match std::fs::read_to_string(&log) {
        Ok(s) => s.lines().filter(|l| !l.trim().is_empty()).count(),
        Err(_) => 0,
    }
}

/// Count content-addressed objects under `<root>/assets/`
/// (`assets/<aa>/<sha256>.<ext>`). A missing `assets/` dir counts as 0.
fn cas_object_count(root: &std::path::Path) -> usize {
    fn walk(dir: &std::path::Path, n: &mut usize) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, n);
            } else {
                *n += 1;
            }
        }
    }
    let mut n = 0;
    walk(&root.join("assets"), &mut n);
    n
}

// ---- Test 1: project-less read → Ok, event_id == "", patch == [] -----

#[test]
fn list_capabilities_is_ok_with_empty_event_and_patch() {
    let dir = TempDir::new().unwrap();
    let mut engine = Engine::new(dir.path());

    let env = engine.dispatch("list_capabilities", json!({ "project_id": "ignored" }));

    let Envelope::Ok {
        patch, event_id, ..
    } = &env
    else {
        panic!("list_capabilities must be Ok, got {env:?}");
    };
    assert_eq!(*patch, json!([]), "read verb returns an empty patch");
    assert_eq!(event_id, "", "read verb returns event_id \"\"");
}

// ---- Test 2: create → mutating verb persists --------------------------

#[test]
fn create_then_mutate_persists_event_and_returns_patch() {
    let dir = TempDir::new().unwrap();
    let mut engine = Engine::new(dir.path());
    let project_id = create_project(&mut engine, &dir, "alpha");
    let root = dir.path().join("alpha");

    let before = event_count(&root);
    let env = engine.dispatch(
        "marker.add",
        json!({
            "project_id": project_id,
            "time_tk": 1_000,
            "label": "Intro",
        }),
    );

    let Envelope::Ok {
        patch, event_id, ..
    } = &env
    else {
        panic!("marker.add must be Ok, got {env:?}");
    };
    assert_ne!(*patch, json!([]), "a mutation produces a non-empty patch");
    assert_ne!(event_id, "", "a persisted mutation carries a real event_id");

    let after = event_count(&root);
    assert_eq!(
        after,
        before + 1,
        "marker.add must append exactly one event"
    );
}

// ---- Test 3: project-scoped read verb → empty patch, no event --------

#[test]
fn project_scoped_read_verb_is_noop_with_no_event() {
    let dir = TempDir::new().unwrap();
    let mut engine = Engine::new(dir.path());
    let project_id = create_project(&mut engine, &dir, "beta");
    let root = dir.path().join("beta");

    // Seed one marker so the list has content but the LIST itself is a
    // read.
    engine.dispatch(
        "marker.add",
        json!({ "project_id": project_id, "time_tk": 500, "label": "m" }),
    );
    let before = event_count(&root);

    let env = engine.dispatch("marker.list", json!({ "project_id": project_id }));
    let Envelope::Ok {
        patch, event_id, ..
    } = &env
    else {
        panic!("marker.list must be Ok, got {env:?}");
    };
    assert_eq!(*patch, json!([]), "read verb returns empty patch");
    assert_eq!(event_id, "", "read verb returns event_id \"\"");

    let after = event_count(&root);
    assert_eq!(after, before, "a read verb must not write an event line");
}

// ---- Test 4: unknown verb → E_UNKNOWN_VERB ---------------------------

#[test]
fn unknown_verb_returns_e_unknown_verb() {
    let dir = TempDir::new().unwrap();
    let mut engine = Engine::new(dir.path());

    let env = engine.dispatch("nope.frobnicate", json!({ "project_id": "x" }));
    let Envelope::Err { code, .. } = &env else {
        panic!("unknown verb must be Err, got {env:?}");
    };
    assert_eq!(code, "E_UNKNOWN_VERB");
}

// ---- Test 5: mutating verb on a non-open project → E_PROJECT_NOT_FOUND

#[test]
fn mutating_verb_on_unopened_project_returns_not_found() {
    let dir = TempDir::new().unwrap();
    let mut engine = Engine::new(dir.path());

    let env = engine.dispatch(
        "marker.add",
        json!({
            "project_id": "0190b8d3-15e3-7000-bd00-0000000000aa",
            "time_tk": 1,
            "label": "x",
        }),
    );
    let Envelope::Err { code, details, .. } = &env else {
        panic!("must be Err, got {env:?}");
    };
    assert_eq!(code, "E_PROJECT_NOT_FOUND");
    assert_eq!(
        details
            .as_ref()
            .and_then(|d| d.get("project_id"))
            .and_then(Value::as_str),
        Some("0190b8d3-15e3-7000-bd00-0000000000aa"),
        "E_PROJECT_NOT_FOUND carries details.project_id (§0.12)"
    );
}

// ---- Test 6: close evicts → subsequent verb → E_PROJECT_NOT_FOUND ----

#[test]
fn close_evicts_then_verb_is_not_found() {
    let dir = TempDir::new().unwrap();
    let mut engine = Engine::new(dir.path());
    let project_id = create_project(&mut engine, &dir, "gamma");
    assert_eq!(engine.open_count(), 1);

    let env = engine.dispatch("project.close", json!({ "project_id": project_id }));
    assert!(env.is_ok(), "close must succeed: {env:?}");
    assert_eq!(
        engine.open_count(),
        0,
        "close evicts the project from the map"
    );

    let after = engine.dispatch(
        "marker.add",
        json!({ "project_id": project_id, "time_tk": 1, "label": "x" }),
    );
    let Envelope::Err { code, .. } = &after else {
        panic!("verb after close must be Err, got {after:?}");
    };
    assert_eq!(code, "E_PROJECT_NOT_FOUND");
}

// ---- Test 7: idempotency → same key → Replayed with identical event_id

#[test]
fn idempotent_retry_returns_same_event_id() {
    let dir = TempDir::new().unwrap();
    let mut engine = Engine::new(dir.path());
    let project_id = create_project(&mut engine, &dir, "delta");

    let args = json!({
        "project_id": project_id,
        "time_tk": 2_000,
        "label": "Intro",
        "idempotency_key": "k-marker",
    });

    let first = engine.dispatch("marker.add", args.clone());
    let Envelope::Ok { event_id: id1, .. } = &first else {
        panic!("first must be Ok, got {first:?}");
    };
    assert_ne!(id1, "", "first call writes a real event");

    let second = engine.dispatch("marker.add", args);
    let Envelope::Ok {
        event_id: id2,
        warnings,
        ..
    } = &second
    else {
        panic!("replay must be Ok, got {second:?}");
    };
    assert_eq!(id1, id2, "idempotent replay surfaces the original event_id");
    assert!(
        warnings.iter().any(|w| w["code"] == "W_REPLAY"),
        "replay carries W_REPLAY"
    );
}

// ---- Test 8: dry_run → patch computed, event_id == "", no persistence -

#[test]
fn dry_run_computes_patch_without_persisting() {
    let dir = TempDir::new().unwrap();
    let mut engine = Engine::new(dir.path());
    let project_id = create_project(&mut engine, &dir, "epsilon");
    let root = dir.path().join("epsilon");

    let before = event_count(&root);
    let env = engine.dispatch(
        "marker.add",
        json!({
            "project_id": project_id,
            "time_tk": 3_000,
            "label": "Dry",
            "dry_run": true,
        }),
    );
    let Envelope::Ok {
        patch, event_id, ..
    } = &env
    else {
        panic!("dry_run must be Ok, got {env:?}");
    };
    assert_ne!(
        *patch,
        json!([]),
        "dry_run still returns the would-be patch (§0.5.1)"
    );
    assert_eq!(event_id, "", "dry_run returns event_id \"\"");

    let after = event_count(&root);
    assert_eq!(after, before, "dry_run must not persist an event");
    // And the in-memory marker must NOT have been added.
    let listed = engine.dispatch("marker.list", json!({ "project_id": project_id }));
    let Envelope::Ok { data, .. } = listed else {
        panic!("marker.list must be Ok");
    };
    assert_eq!(
        data["markers"].as_array().map(Vec::len),
        Some(0),
        "dry_run must not mutate in-memory state"
    );
}

// ---- Test 8a: malformed dry_run on a project-scoped verb is rejected ---
//
// §0.5.1 regression: a present-but-non-boolean `dry_run` (`"true"`, `1`,
// `null`, `{}`) must NOT coerce to `false` and persist the real mutation on
// the project-scoped path. `marker.add` is a persistent verb here; a malformed
// dry_run must be rejected at the dispatch boundary and write no event.

#[test]
fn malformed_dry_run_on_project_scoped_verb_is_rejected_and_persists_nothing() {
    let dir = TempDir::new().unwrap();
    let mut engine = Engine::new(dir.path());
    let project_id = create_project(&mut engine, &dir, "badtype-scoped");
    let root = dir.path().join("badtype-scoped");
    let before = event_count(&root);

    for bad in [json!("true"), json!(1), json!(null), json!({})] {
        let env = engine.dispatch(
            "marker.add",
            json!({
                "project_id": project_id,
                "time_tk": 3_000,
                "label": "Bad",
                "dry_run": bad,
            }),
        );
        let Envelope::Err { code, details, .. } = &env else {
            panic!("non-boolean dry_run on marker.add must be rejected, got {env:?}");
        };
        assert_eq!(code, "E_SCHEMA_VIOLATION");
        assert_eq!(
            details
                .as_ref()
                .and_then(|d| d.get("arg"))
                .and_then(Value::as_str),
            Some("dry_run"),
            "rejection must name dry_run as the offending arg (dispatch-boundary type gate)"
        );
    }

    assert_eq!(
        event_count(&root),
        before,
        "a rejected malformed-dry_run call must not persist any event"
    );
    let listed = engine.dispatch("marker.list", json!({ "project_id": project_id }));
    let Envelope::Ok { data, .. } = listed else {
        panic!("marker.list must be Ok");
    };
    assert_eq!(
        data["markers"].as_array().map(Vec::len),
        Some(0),
        "a rejected malformed-dry_run call must not mutate in-memory state"
    );
}

// ---- Test 8b: dry_run asset.import → would-be patch, NO CAS write -----
//
// §0.5.1 regression: a dry `asset.import` MUST compute the real sha256 +
// would-be patch but MUST NOT copy the source bytes into the CAS (no
// orphaned object under `<root>/assets/`).

#[test]
fn dry_run_asset_import_does_not_write_cas() {
    let dir = TempDir::new().unwrap();
    let mut engine = Engine::new(dir.path());
    let project_id = create_project(&mut engine, &dir, "import-dry");
    let root = dir.path().join("import-dry");

    // A real on-disk source file to import.
    let src = dir.path().join("source.srt");
    std::fs::write(&src, b"1\n00:00:00,000 --> 00:00:01,000\nhello\n").unwrap();

    let cas_before = cas_object_count(&root);
    let events_before = event_count(&root);

    let env = engine.dispatch(
        "asset.import",
        json!({
            "project_id": project_id,
            "paths": [src.to_string_lossy()],
            "dry_run": true,
        }),
    );

    let Envelope::Ok {
        patch, event_id, ..
    } = &env
    else {
        panic!("dry asset.import must be Ok, got {env:?}");
    };
    // (a) Ok with a NON-EMPTY would-be patch and event_id == "".
    assert_ne!(
        *patch,
        json!([]),
        "dry asset.import returns the would-be patch (§0.5.1)"
    );
    let ops = patch.as_array().expect("patch is an array");
    assert_eq!(ops.len(), 1, "one add op per imported path");
    assert_eq!(ops[0]["op"], "add");
    assert_eq!(ops[0]["path"], "/assets/-");
    // The patch carries the REAL content hash (the §0.5.1 "patch reflects
    // real probed values" guarantee).
    assert!(
        ops[0]["value"]["hash"]
            .as_str()
            .is_some_and(|h| h.len() == 64),
        "dry import patch carries the real sha256 hash"
    );
    assert_eq!(event_id, "", "dry_run returns event_id \"\"");

    // (b) No new CAS object on disk; no event written.
    assert_eq!(
        cas_object_count(&root),
        cas_before,
        "dry asset.import must NOT write a CAS object (§0.5.1 persist nothing)"
    );
    assert_eq!(
        event_count(&root),
        events_before,
        "dry asset.import must not persist an event"
    );
}

// ---- Test 8c: real asset.import DOES write the CAS object -------------
//
// Contrasting persist-path test: proves the §0.8 mutate path still copies
// the source bytes into the content-addressed store byte-for-byte.

#[test]
fn real_asset_import_writes_cas_object() {
    let dir = TempDir::new().unwrap();
    let mut engine = Engine::new(dir.path());
    let project_id = create_project(&mut engine, &dir, "import-real");
    let root = dir.path().join("import-real");

    let bytes = b"1\n00:00:00,000 --> 00:00:01,000\nhello\n";
    let src = dir.path().join("source.srt");
    std::fs::write(&src, bytes).unwrap();

    let cas_before = cas_object_count(&root);
    let events_before = event_count(&root);

    let env = engine.dispatch(
        "asset.import",
        json!({
            "project_id": project_id,
            "paths": [src.to_string_lossy()],
        }),
    );

    let Envelope::Ok {
        patch, event_id, ..
    } = &env
    else {
        panic!("real asset.import must be Ok, got {env:?}");
    };
    assert_ne!(*patch, json!([]), "real import produces a non-empty patch");
    assert_ne!(event_id, "", "real import persists a real event");

    // Exactly one new CAS object exists, and its bytes match the source.
    assert_eq!(
        cas_object_count(&root),
        cas_before + 1,
        "real asset.import writes exactly one CAS object"
    );
    let rel = patch.as_array().unwrap()[0]["value"]["path"]
        .as_str()
        .expect("asset value carries a CAS-relative path");
    let cas_path = root.join(rel);
    assert!(cas_path.exists(), "the CAS object exists at {rel}");
    assert_eq!(
        std::fs::read(&cas_path).unwrap(),
        bytes,
        "CAS object bytes match the source byte-for-byte"
    );
    assert_eq!(
        event_count(&root),
        events_before + 1,
        "real asset.import appends exactly one event"
    );
}

// ---- Test 10: Envelope::to_json exact §0.1 shape (Ok + Err) ----------
// (Issue #443 test target #9 — register → resolve — lives in
// `tests/resolve.rs` because the resolver is storage-side, not engine.
// `resolve_root_through_index` below also exercises it via the engine.)

#[test]
fn envelope_to_json_matches_spec_0_1_shape() {
    let ok = Envelope::Ok {
        data: json!({ "x": 1 }),
        patch: json!([{ "op": "add", "path": "/a", "value": 1 }]),
        warnings: vec![json!({ "code": "W_X" })],
        event_id: "0190b8d3-15e3-7000-bd00-000000000099".to_string(),
    };
    assert_eq!(
        ok.to_json(),
        json!({
            "ok": true,
            "data": { "x": 1 },
            "patch": [{ "op": "add", "path": "/a", "value": 1 }],
            "warnings": [{ "code": "W_X" }],
            "event_id": "0190b8d3-15e3-7000-bd00-000000000099",
        })
    );

    // Err with no hint/details → those keys are absent (the §0.1 `?`
    // fields), not emitted as null.
    let err = Envelope::Err {
        code: "E_UNKNOWN_VERB".to_string(),
        message: "unknown verb: nope".to_string(),
        hint: None,
        details: None,
    };
    assert_eq!(
        err.to_json(),
        json!({
            "ok": false,
            "code": "E_UNKNOWN_VERB",
            "message": "unknown verb: nope",
        })
    );

    // Err WITH hint + details → both present.
    let err2 = Envelope::Err {
        code: "E_PROJECT_NOT_FOUND".to_string(),
        message: "not open".to_string(),
        hint: Some("open it first".to_string()),
        details: Some(json!({ "project_id": "p" })),
    };
    assert_eq!(
        err2.to_json(),
        json!({
            "ok": false,
            "code": "E_PROJECT_NOT_FOUND",
            "message": "not open",
            "hint": "open it first",
            "details": { "project_id": "p" },
        })
    );
}

// ---- Extra: verb_ids includes registry + lifecycle, schema_for shape -

#[test]
fn verb_ids_include_lifecycle_and_registry() {
    let dir = TempDir::new().unwrap();
    let engine = Engine::new(dir.path());
    let ids = engine.verb_ids();

    for lifecycle in [
        "project.create",
        "project.open",
        "project.save",
        "project.close",
        "project.duplicate",
        "project.forget",
    ] {
        assert!(ids.iter().any(|v| v == lifecycle), "missing {lifecycle}");
    }
    assert!(
        ids.iter().any(|v| v == "marker.add"),
        "missing registry verb"
    );
    assert!(ids.iter().any(|v| v == "list_capabilities"));
    // sorted + deduped
    let mut sorted = ids.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(ids, sorted, "verb_ids must be sorted and deduped");
}

#[test]
fn schema_for_returns_object_schema() {
    let dir = TempDir::new().unwrap();
    let engine = Engine::new(dir.path());

    // Well-known verb → curated schema.
    let known = engine.schema_for("clip.list");
    assert_eq!(known["type"], "object");

    // Unknown / unlisted verb → permissive v1 floor.
    let floor = engine.schema_for("clip.add");
    assert_eq!(floor["type"], "object");
    assert_eq!(floor["additionalProperties"], true);
}

// ---- Extra: open a previously-created project, then mutate -----------

#[test]
fn open_existing_project_then_mutate() {
    let dir = TempDir::new().unwrap();
    let root = {
        // Create then close so the project exists on disk but is not
        // open in a second engine.
        let mut engine = Engine::new(dir.path());
        let project_id = create_project(&mut engine, &dir, "zeta");
        engine.dispatch("project.close", json!({ "project_id": project_id }));
        dir.path().join("zeta")
    };

    let mut engine = Engine::new(dir.path());
    let env = engine.dispatch("project.open", json!({ "path": root }));
    let Envelope::Ok { data, .. } = &env else {
        panic!("project.open must be Ok, got {env:?}");
    };
    let project_id = data["project_id"]
        .as_str()
        .expect("open returns project_id");
    assert_eq!(engine.open_count(), 1);

    let mutated = engine.dispatch(
        "marker.add",
        json!({ "project_id": project_id, "time_tk": 9, "label": "after-open" }),
    );
    assert!(
        mutated.is_ok(),
        "mutation after open must succeed: {mutated:?}"
    );
}

// ---- Lifecycle error-path mappings (one test per new E_* code) -------

#[test]
fn create_duplicate_name_returns_project_exists() {
    let dir = TempDir::new().unwrap();
    let mut engine = Engine::new(dir.path());
    let _id = create_project(&mut engine, &dir, "dup");

    // Second create at the same destination → E_PROJECT_EXISTS.
    let env = engine.dispatch(
        "project.create",
        json!({ "name": "dup", "canvas": "1080x1920", "at": dir.path() }),
    );
    let Envelope::Err { code, .. } = &env else {
        panic!("duplicate create must be Err, got {env:?}");
    };
    assert_eq!(code, "E_PROJECT_EXISTS");
}

#[test]
fn create_malformed_args_returns_schema_violation() {
    let dir = TempDir::new().unwrap();
    let mut engine = Engine::new(dir.path());

    // `canvas` of the wrong shape → the free fn's InvalidCanvas →
    // E_SCHEMA_VIOLATION.
    let env = engine.dispatch(
        "project.create",
        json!({ "name": "x", "canvas": "not-a-canvas", "at": dir.path() }),
    );
    let Envelope::Err { code, .. } = &env else {
        panic!("malformed create must be Err, got {env:?}");
    };
    assert_eq!(code, "E_SCHEMA_VIOLATION");
}

#[test]
fn project_scoped_invalid_project_id_returns_schema_violation() {
    let dir = TempDir::new().unwrap();
    let mut engine = Engine::new(dir.path());

    // A registered verb with a non-UUID project_id reaches the
    // project_id parse branch (the verb is known, so it does NOT short
    // out on E_UNKNOWN_VERB) and surfaces E_SCHEMA_VIOLATION.
    let env = engine.dispatch(
        "marker.add",
        json!({ "project_id": "not-a-uuid", "time_tk": 1, "label": "x" }),
    );
    let Envelope::Err { code, .. } = &env else {
        panic!("invalid project_id must be Err, got {env:?}");
    };
    assert_eq!(code, "E_SCHEMA_VIOLATION");
}

#[test]
fn forget_with_both_args_returns_args_incompatible() {
    let dir = TempDir::new().unwrap();
    let mut engine = Engine::new(dir.path());

    // project.forget requires path XOR project_id; supplying both →
    // E_ARGS_INCOMPATIBLE.
    let env = engine.dispatch(
        "project.forget",
        json!({ "path": "/tmp/x", "project_id": "0190b8d3-15e3-7000-bd00-000000000001" }),
    );
    let Envelope::Err { code, .. } = &env else {
        panic!("forget with both args must be Err, got {env:?}");
    };
    assert_eq!(code, "E_ARGS_INCOMPATIBLE");
}

#[test]
fn save_unopened_project_returns_not_found() {
    let dir = TempDir::new().unwrap();
    let mut engine = Engine::new(dir.path());

    let env = engine.dispatch(
        "project.save",
        json!({ "project_id": "0190b8d3-15e3-7000-bd00-0000000000bb" }),
    );
    let Envelope::Err { code, .. } = &env else {
        panic!("save on unopened project must be Err, got {env:?}");
    };
    assert_eq!(code, "E_PROJECT_NOT_FOUND");
}

#[test]
fn open_with_relative_path_registers_absolute_root() {
    use std::sync::Mutex;
    // `std::env::set_current_dir` is process-global; serialize the cwd
    // window so a parallel test never observes the temporary cwd. (No other
    // engine test mutates cwd or opens with a relative path, so this lock is
    // the only cwd writer.)
    static CWD_LOCK: Mutex<()> = Mutex::new(());
    let _cwd_guard = CWD_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let original_cwd = std::env::current_dir().unwrap();

    let dir = TempDir::new().unwrap();
    let abs_root = dir.path().join("rel-open");
    {
        let mut engine = Engine::new(dir.path());
        let id = create_project(&mut engine, &dir, "rel-open");
        engine.dispatch("project.close", json!({ "project_id": id }));
    }
    // Wipe the index the create wrote, so the only entry under test is the
    // one `project.open` registers.
    let index = dir.path().join(".verbreel").join("projects-index");
    if index.exists() {
        std::fs::remove_file(&index).unwrap();
    }

    // cd into the temp dir so "rel-open" is a valid RELATIVE path to the
    // project; `project.open`'s `to_absolute` resolves it against this cwd.
    std::env::set_current_dir(dir.path()).unwrap();
    let opened = {
        let mut engine = Engine::new(dir.path());
        engine.dispatch("project.open", json!({ "path": "rel-open" }))
    };
    // Restore cwd before any assertion that could unwind the test.
    std::env::set_current_dir(&original_cwd).unwrap();

    let Envelope::Ok { data, .. } = &opened else {
        panic!("project.open(relative) must be Ok, got {opened:?}");
    };
    let id = data["project_id"]
        .as_str()
        .expect("open returns project_id");

    // A fresh engine (empty open-map) must resolve the id to the ABSOLUTE
    // root, not the raw relative "rel-open" — otherwise a later one-shot
    // process resolves it against the wrong cwd. Regression guard for the
    // `typed.path`-vs-`store.root()` registration bug.
    let fresh = Engine::new(dir.path());
    let resolved = fresh
        .resolve_root(id)
        .expect("open must register a resolvable root");
    assert!(
        resolved.is_absolute(),
        "registered root must be absolute, got {resolved:?}"
    );
    // Canonicalize both sides: on macOS the temp dir lives under a symlinked
    // `/var` → `/private/var`, and `current_dir()` (used by `to_absolute`)
    // resolves it, so the registered root is the canonical form while
    // `abs_root` (built from `dir.path()`) is not. The point under test —
    // that an ABSOLUTE root was registered, not the raw relative path — is
    // the `is_absolute` assert above; this confirms it's the same directory.
    assert_eq!(
        resolved.canonicalize().expect("resolved root exists"),
        abs_root.canonicalize().expect("project root exists"),
        "project.open(relative) must register the absolutized root, not the raw relative path"
    );
}

#[test]
fn reused_idempotency_key_with_different_args_returns_conflict() {
    let dir = TempDir::new().unwrap();
    let mut engine = Engine::new(dir.path());
    let project_id = create_project(&mut engine, &dir, "idem-conflict");

    // First call: writes an event and records the key → fingerprint.
    let first = engine.dispatch(
        "marker.add",
        json!({
            "project_id": project_id,
            "time_tk": 1_000,
            "label": "Intro",
            "idempotency_key": "k-conflict",
        }),
    );
    assert!(first.is_ok(), "first keyed call must succeed: {first:?}");

    // Same key, DIFFERENT args (time_tk differs ⇒ different fingerprint).
    // The §0.8 index returns ConflictingFingerprint → E_IDEMPOTENCY_CONFLICT.
    let second = engine.dispatch(
        "marker.add",
        json!({
            "project_id": project_id,
            "time_tk": 9_999,
            "label": "Intro",
            "idempotency_key": "k-conflict",
        }),
    );
    let Envelope::Err { code, .. } = &second else {
        panic!("reused key with new args must be Err, got {second:?}");
    };
    assert_eq!(code, "E_IDEMPOTENCY_CONFLICT");
}

#[test]
fn forget_empty_path_returns_bad_range() {
    let dir = TempDir::new().unwrap();
    let mut engine = Engine::new(dir.path());

    // project.forget with an empty `path` reaches validate_path → BadRange
    // → forget_error_to_envelope → E_BAD_RANGE. (Empty path is the
    // deterministic malformed-range trigger; a NUL byte also works.)
    let env = engine.dispatch("project.forget", json!({ "path": "" }));
    let Envelope::Err { code, .. } = &env else {
        panic!("forget with empty path must be Err, got {env:?}");
    };
    assert_eq!(code, "E_BAD_RANGE");
}

#[test]
fn open_already_locked_project_returns_project_locked() {
    let dir = TempDir::new().unwrap();

    // Engine 1 creates the project and keeps it open — it holds the
    // exclusive `events.jsonl` flock for the lifetime of its store.
    let mut engine1 = Engine::new(dir.path());
    let _id = create_project(&mut engine1, &dir, "locked");
    let root = dir.path().join("locked");

    // Engine 2 tries to open the same path → the flock try_lock returns
    // WouldBlock → ProjectOpenError::ProjectLocked → E_PROJECT_LOCKED.
    let mut engine2 = Engine::new(dir.path());
    let env = engine2.dispatch("project.open", json!({ "path": root }));
    let Envelope::Err { code, .. } = &env else {
        panic!("opening a locked project must be Err, got {env:?}");
    };
    assert_eq!(code, "E_PROJECT_LOCKED");
}

// ---- Engine::resolve_root threads home + storage resolver -----------

#[test]
fn resolve_root_through_index() {
    // The engine's `resolve_root` delegates to the storage resolver
    // against `self.home`. Register an id → resolve returns the root;
    // an unknown id → NotFound. This test registers directly (not via a
    // lifecycle verb) to isolate the resolve delegation; the
    // create/open-register-then-resolve round-trips live in
    // `create_registers_in_index_resolvable_fresh_engine` and
    // `open_registers_in_index_resolvable_fresh_engine` below.
    use verbreel_storage::layout::{ResolveError, register_project};

    let home = TempDir::new().unwrap();
    let engine = Engine::new(home.path());

    let id = "0190b8d3-15e3-7000-bd00-0000000000cc";
    let root = home.path().join("projects/resolved");
    register_project(home.path(), id, "resolved", &root, "2025-01-01T00:00:00Z").unwrap();

    assert_eq!(engine.resolve_root(id).unwrap(), root);

    let err = engine
        .resolve_root("0190b8d3-15e3-7000-bd00-0000000000dd")
        .unwrap_err();
    assert!(
        matches!(err, ResolveError::NotFound(_)),
        "unknown id must resolve to NotFound, got {err:?}"
    );
}

// ---- Regression: create populates the index so a fresh engine resolves
//
// The root-cause guard for the inert-CLI bug: before this fix, the
// `project.create` handler inserted the project into the in-memory open-map
// but never wrote `<home>/.verbreel/projects-index`. A one-shot CLI process
// that does `project create` then exits leaves an empty index, so the NEXT
// process (`verbreel clip add <id>`) builds a fresh `Engine` whose
// `resolve_root(id)` returns NotFound — every project-scoped CLI verb is
// inert. Resolving through a *fresh* `Engine` (distinct from the one that
// created the project) is exactly the cross-process path the CLI exercises.

#[test]
fn create_registers_in_index_resolvable_fresh_engine() {
    let dir = TempDir::new().unwrap();
    let id = {
        let mut engine = Engine::new(dir.path());
        create_project(&mut engine, &dir, "indexed-create")
    };
    let expected_root = dir.path().join("indexed-create");

    // A brand-new engine on the SAME home — its open-map is empty, so the
    // only way it can resolve the id is the on-disk index the create wrote.
    let fresh = Engine::new(dir.path());
    assert_eq!(
        fresh
            .resolve_root(&id)
            .expect("create must populate the index"),
        expected_root,
        "project.create must register id→root so a fresh engine resolves it"
    );
}

// ---- Regression: open (re-)registers in the index ---------------------
//
// `project.open` must also write the index (§2.6). We force a closed-on-disk
// project by creating + closing inside one engine, deleting the index file
// it wrote, then opening in a fresh engine and asserting the fresh open
// re-populated the index for cross-process resolution.

#[test]
fn open_registers_in_index_resolvable_fresh_engine() {
    let dir = TempDir::new().unwrap();
    let root = {
        let mut engine = Engine::new(dir.path());
        let id = create_project(&mut engine, &dir, "indexed-open");
        engine.dispatch("project.close", json!({ "project_id": id }));
        dir.path().join("indexed-open")
    };

    // Wipe the index the create wrote, so the only entry under test is the
    // one project.open is responsible for writing.
    let index = dir.path().join(".verbreel").join("projects-index");
    if index.exists() {
        std::fs::remove_file(&index).unwrap();
    }

    let id = {
        let mut engine = Engine::new(dir.path());
        let env = engine.dispatch("project.open", json!({ "path": &root }));
        let Envelope::Ok { data, .. } = &env else {
            panic!("project.open must be Ok, got {env:?}");
        };
        data["project_id"]
            .as_str()
            .expect("open returns project_id")
            .to_string()
    };

    // Fresh engine, empty open-map → resolution can only come from the
    // index entry project.open wrote.
    let fresh = Engine::new(dir.path());
    assert_eq!(
        fresh
            .resolve_root(&id)
            .expect("open must populate the index"),
        root,
        "project.open must register id→root so a fresh engine resolves it"
    );
}

// ---- Non-fatal index-write failure ------------------------------------
//
// If writing the projects-index fails, `project.create` must still succeed
// (the project IS on disk + open in the engine) and surface a
// `W_INDEX_WRITE_FAILED` warning rather than failing the whole call or
// swallowing the error. We make the index write fail deterministically by
// planting a regular FILE where `register_project` needs the `.verbreel`
// DIRECTORY: its `create_dir_all(<home>/.verbreel)` then errors, which is
// the documented non-fatal path.

#[test]
fn create_index_write_failure_is_nonfatal_warns() {
    let dir = TempDir::new().unwrap();

    // The project lives under a separate `projects/` subdir so the home's
    // `.verbreel` can be a file without colliding with the project root's
    // own `.verbreel/` directory.
    let home = dir.path().join("home");
    let at = dir.path().join("projects");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&at).unwrap();
    // Block `<home>/.verbreel` from being a directory.
    std::fs::write(home.join(".verbreel"), b"not a dir").unwrap();

    let mut engine = Engine::new(&home);
    let env = engine.dispatch(
        "project.create",
        json!({ "name": "warns", "canvas": "1080x1920", "at": at }),
    );

    let Envelope::Ok { warnings, .. } = &env else {
        panic!("create must stay Ok despite an index-write failure, got {env:?}");
    };
    assert!(
        warnings.iter().any(|w| w["code"] == "W_INDEX_WRITE_FAILED"),
        "an index-write failure must surface W_INDEX_WRITE_FAILED, got {warnings:?}"
    );
    // And the project is genuinely open in the engine despite the warning.
    assert_eq!(
        engine.open_count(),
        1,
        "the project is created + open even though indexing failed"
    );
}

// ---- Regression: universal args on lifecycle verbs (issue #446) -------
//
// The six lifecycle arg structs all carry `#[serde(deny_unknown_fields)]`.
// `strip_universal_args` used to run only in `dispatch_project_scoped`, so a
// lifecycle call carrying a §0.5 universal arg (`idempotency_key` / `dry_run`)
// deserialized straight into the typed struct, hit the deny-unknown gate, and
// wrongly returned E_SCHEMA_VIOLATION — even though §0.8 designates
// `project.create` / `.duplicate` as legitimate idempotency_key surfaces.
// Each assertion below FAILS before the fix (the call 400s) and passes after.

#[test]
fn create_with_idempotency_key_is_not_schema_violation() {
    let dir = TempDir::new().unwrap();
    let mut engine = Engine::new(dir.path());

    let env = engine.dispatch(
        "project.create",
        json!({
            "name": "idem-create",
            "canvas": "1080x1920",
            "at": dir.path(),
            "idempotency_key": "k",
        }),
    );
    assert!(
        env.is_ok(),
        "project.create with idempotency_key must succeed, not 400 E_SCHEMA_VIOLATION: {env:?}"
    );
}

#[test]
fn open_with_universal_args_is_not_schema_violation() {
    let dir = TempDir::new().unwrap();
    let mut engine = Engine::new(dir.path());
    let id = create_project(&mut engine, &dir, "idem-open");
    engine.dispatch("project.close", json!({ "project_id": id }));
    let root = dir.path().join("idem-open");

    let env = engine.dispatch(
        "project.open",
        json!({ "path": &root, "idempotency_key": "k", "dry_run": false }),
    );
    assert!(
        env.is_ok(),
        "project.open with universal args must succeed, not E_SCHEMA_VIOLATION: {env:?}"
    );
}

#[test]
fn save_with_universal_args_is_not_schema_violation() {
    let dir = TempDir::new().unwrap();
    let mut engine = Engine::new(dir.path());
    let id = create_project(&mut engine, &dir, "idem-save");

    let env = engine.dispatch(
        "project.save",
        json!({ "project_id": id, "idempotency_key": "k", "dry_run": false }),
    );
    assert!(
        env.is_ok(),
        "project.save with universal args must succeed, not E_SCHEMA_VIOLATION: {env:?}"
    );
}

#[test]
fn close_with_universal_args_is_not_schema_violation() {
    let dir = TempDir::new().unwrap();
    let mut engine = Engine::new(dir.path());
    let id = create_project(&mut engine, &dir, "idem-close");

    let env = engine.dispatch(
        "project.close",
        json!({ "project_id": id, "idempotency_key": "k", "dry_run": false }),
    );
    assert!(
        env.is_ok(),
        "project.close with universal args must succeed, not E_SCHEMA_VIOLATION: {env:?}"
    );
}

#[test]
fn duplicate_with_idempotency_key_is_not_schema_violation() {
    let dir = TempDir::new().unwrap();
    let mut engine = Engine::new(dir.path());
    let id = create_project(&mut engine, &dir, "idem-dup-src");
    let source_path = dir.path().join("idem-dup-src");

    let env = engine.dispatch(
        "project.duplicate",
        json!({
            "project_id": id,
            "name": "idem-dup-dst",
            "source_path": source_path,
            "idempotency_key": "k",
        }),
    );
    assert!(
        env.is_ok(),
        "project.duplicate with idempotency_key must succeed, not E_SCHEMA_VIOLATION: {env:?}"
    );
}

#[test]
fn forget_with_universal_args_is_not_schema_violation() {
    let dir = TempDir::new().unwrap();
    let mut engine = Engine::new(dir.path());
    let id = create_project(&mut engine, &dir, "idem-forget");
    // forget removes the on-disk root; close first so the flock is released.
    engine.dispatch("project.close", json!({ "project_id": id }));
    let root = dir.path().join("idem-forget");

    let env = engine.dispatch(
        "project.forget",
        json!({ "path": &root, "idempotency_key": "k", "dry_run": false }),
    );
    assert!(
        env.is_ok(),
        "project.forget with universal args must succeed, not E_SCHEMA_VIOLATION: {env:?}"
    );
}

// ---- dry_run on lifecycle verbs is rejected, never silently performed ----
//
// §0.5.1: `dry_run: true` guarantees no persistent side effect. The lifecycle
// free functions have no compute-only path — they always persist. So the
// engine must REJECT `dry_run: true` on a lifecycle verb rather than strip it
// and proceed (which would perform the real, unconditional mutation). The
// canonical hazard is `project.forget --dry_run` deleting the on-disk root
// under a preview flag; the first test pins that the root survives.

#[test]
fn forget_with_dry_run_is_rejected_and_does_not_remove_root() {
    let dir = TempDir::new().unwrap();
    let mut engine = Engine::new(dir.path());
    let id = create_project(&mut engine, &dir, "dryrun-forget");
    engine.dispatch("project.close", json!({ "project_id": id }));
    let root = dir.path().join("dryrun-forget");
    assert!(
        root.exists(),
        "project root exists before the dry-run forget"
    );

    let env = engine.dispatch("project.forget", json!({ "path": &root, "dry_run": true }));

    let Envelope::Err { code, .. } = &env else {
        panic!("project.forget with dry_run must be rejected, got {env:?}");
    };
    assert_eq!(
        code, "E_SCHEMA_VIOLATION",
        "dry_run on a lifecycle verb is rejected at the dispatch boundary"
    );
    assert!(
        root.exists(),
        "project.forget --dry_run must NOT remove the on-disk root (§0.5.1: no persistent side effect)"
    );
}

#[test]
fn create_with_dry_run_is_rejected_and_writes_nothing() {
    let dir = TempDir::new().unwrap();
    let mut engine = Engine::new(dir.path());

    let env = engine.dispatch(
        "project.create",
        json!({
            "name": "dryrun-create",
            "canvas": "1080x1920",
            "at": dir.path(),
            "dry_run": true,
        }),
    );

    let Envelope::Err { code, .. } = &env else {
        panic!("project.create with dry_run must be rejected, got {env:?}");
    };
    assert_eq!(code, "E_SCHEMA_VIOLATION");
    assert!(
        !dir.path().join("dryrun-create").exists(),
        "project.create --dry_run must NOT write a project to disk (§0.5.1)"
    );
    assert_eq!(
        engine.open_count(),
        0,
        "a rejected dry-run create must not enter a project into the open-map"
    );
}

#[test]
fn forget_with_non_boolean_dry_run_is_rejected_and_does_not_remove_root() {
    // A present-but-not-bool `dry_run` (e.g. the string "true" or 1) must NOT
    // coerce to false and fall through to a real, destructive forget. The
    // boundary rejects any present dry_run that is not the boolean `false`.
    let dir = TempDir::new().unwrap();
    let mut engine = Engine::new(dir.path());
    let id = create_project(&mut engine, &dir, "badtype-forget");
    engine.dispatch("project.close", json!({ "project_id": id }));
    let root = dir.path().join("badtype-forget");
    assert!(
        root.exists(),
        "root exists before the malformed dry-run forget"
    );

    for bad in [json!("true"), json!(1), json!(null), json!({})] {
        let env = engine.dispatch("project.forget", json!({ "path": &root, "dry_run": bad }));
        let Envelope::Err { code, details, .. } = &env else {
            panic!("non-boolean dry_run on project.forget must be rejected, got {env:?}");
        };
        assert_eq!(code, "E_SCHEMA_VIOLATION");
        // Pin that the dry_run type-gate fired, not some unrelated schema error.
        assert_eq!(
            details
                .as_ref()
                .and_then(|d| d.get("arg"))
                .and_then(Value::as_str),
            Some("dry_run"),
            "rejection must name dry_run as the offending arg"
        );
        assert!(
            root.exists(),
            "malformed dry_run must NOT trigger a real forget that removes the root"
        );
    }
}

#[test]
fn forget_with_explicit_dry_run_false_is_accepted() {
    // The escape valve: an explicit `dry_run: false` is the documented
    // non-dry-run signal and must still perform the real forget.
    let dir = TempDir::new().unwrap();
    let mut engine = Engine::new(dir.path());
    let id = create_project(&mut engine, &dir, "falsedry-forget");
    engine.dispatch("project.close", json!({ "project_id": id }));
    let root = dir.path().join("falsedry-forget");

    let env = engine.dispatch("project.forget", json!({ "path": &root, "dry_run": false }));
    assert!(
        env.is_ok(),
        "explicit dry_run:false must be accepted as a real call: {env:?}"
    );
}

#[test]
fn save_with_dry_run_is_rejected() {
    let dir = TempDir::new().unwrap();
    let mut engine = Engine::new(dir.path());
    let id = create_project(&mut engine, &dir, "dryrun-save");

    let env = engine.dispatch("project.save", json!({ "project_id": id, "dry_run": true }));

    let Envelope::Err { code, .. } = &env else {
        panic!("project.save with dry_run must be rejected, got {env:?}");
    };
    assert_eq!(code, "E_SCHEMA_VIOLATION");
}

// ---- project.list engine handler (§2.6) — open/closed marking, prune ----

/// Helper: extract the `Ok` envelope parts or panic.
fn ok_parts(env: Envelope) -> (Value, Value, Vec<Value>, String) {
    let Envelope::Ok {
        data,
        patch,
        warnings,
        event_id,
    } = env
    else {
        panic!("expected Ok envelope, got {env:?}");
    };
    (data, patch, warnings, event_id)
}

#[test]
fn project_list_marks_open_closed_prunes_stale_and_sorts() {
    let dir = TempDir::new().unwrap();
    let mut engine = Engine::new(dir.path());

    // (1) A live project created + opened in this engine -> state "open".
    let open_id = create_project(&mut engine, &dir, "open-proj");

    // (2) A second real project, created then closed -> registered, on
    // disk, but not in the open-map -> state "closed".
    let closed_id = create_project(&mut engine, &dir, "closed-proj");
    engine.dispatch("project.close", json!({ "project_id": closed_id }));

    // (3) A stale registration pointing at a path that does not exist ->
    // pruned on list with one W_INDEX_STALE warning.
    let stale_id = "0192f3a0-0000-7000-8000-0000000000ff";
    verbreel_storage::layout::register_project(
        dir.path(),
        stale_id,
        "stale-proj",
        std::path::Path::new("/no/such/path/ever/list-test"),
        "2025-01-01T00:00:00Z",
    )
    .expect("register stale entry");

    let env = engine.dispatch("project.list", json!({}));
    let (data, patch, warnings, event_id) = ok_parts(env);

    // Read-shaped verb: empty patch + event_id.
    assert_eq!(patch, json!([]));
    assert_eq!(event_id, "");

    // Exactly one W_INDEX_STALE for the pruned entry.
    let stale: Vec<&Value> = warnings
        .iter()
        .filter(|w| w["code"] == json!("W_INDEX_STALE"))
        .collect();
    assert_eq!(stale.len(), 1, "one stale entry pruned: {warnings:?}");
    assert_eq!(stale[0]["details"]["project_id"], json!(stale_id));

    let projects = data["projects"].as_array().expect("projects array");
    assert_eq!(projects.len(), 2, "stale entry pruned from listing");

    // Sorted by id ascending.
    let ids: Vec<&str> = projects.iter().map(|p| p["id"].as_str().unwrap()).collect();
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    assert_eq!(ids, sorted, "projects sorted by id");

    // open/closed marking from the live open-map.
    let state_of = |id: &str| -> String {
        projects
            .iter()
            .find(|p| p["id"] == json!(id))
            .and_then(|p| p["state"].as_str())
            .unwrap_or("missing")
            .to_string()
    };
    assert_eq!(
        state_of(&open_id),
        "open",
        "the open project is marked open"
    );
    assert_eq!(
        state_of(&closed_id),
        "closed",
        "the closed project is marked closed"
    );

    // The stale entry is gone from the durable index after the prune.
    let index = verbreel_storage::layout::read_index(dir.path()).unwrap();
    assert!(
        !index.contains_key(stale_id),
        "stale entry pruned from disk"
    );
}

#[test]
fn project_list_does_not_prune_an_open_project_with_unreachable_path() {
    // An open project whose path is (registered as) unreachable must NOT
    // be dropped by project.list — the engine exempts its open ids. We
    // simulate this by overwriting the open project's index entry with a
    // gone path, then listing: it must survive and stay "open".
    let dir = TempDir::new().unwrap();
    let mut engine = Engine::new(dir.path());
    let open_id = create_project(&mut engine, &dir, "still-open");

    // Re-register the open id pointing at a path that does not exist.
    verbreel_storage::layout::register_project(
        dir.path(),
        &open_id,
        "still-open",
        std::path::Path::new("/no/such/path/ever/open-exempt"),
        "2025-01-01T00:00:00Z",
    )
    .expect("overwrite open entry with gone path");

    let env = engine.dispatch("project.list", json!({}));
    let (data, _, warnings, _) = ok_parts(env);

    assert!(
        warnings.iter().all(|w| w["code"] != json!("W_INDEX_STALE")),
        "an open project must not be pruned: {warnings:?}"
    );
    let projects = data["projects"].as_array().expect("projects array");
    let entry = projects
        .iter()
        .find(|p| p["id"] == json!(open_id))
        .expect("open project still listed");
    assert_eq!(entry["state"], json!("open"));

    // And it survives in the durable index.
    let index = verbreel_storage::layout::read_index(dir.path()).unwrap();
    assert!(index.contains_key(&open_id), "open entry preserved on disk");
}

#[test]
fn project_list_corrupt_index_warns_unreadable_not_silent_empty() {
    let dir = TempDir::new().unwrap();
    let mut engine = Engine::new(dir.path());

    // Corrupt the index file directly.
    let vdir = dir.path().join(".verbreel");
    std::fs::create_dir_all(&vdir).unwrap();
    std::fs::write(vdir.join("projects-index"), b"{ not json").unwrap();

    let env = engine.dispatch("project.list", json!({}));
    let (data, _, warnings, _) = ok_parts(env);

    assert!(
        warnings
            .iter()
            .any(|w| w["code"] == json!("W_INDEX_UNREADABLE")),
        "a corrupt index must surface W_INDEX_UNREADABLE, not a silent empty list: {warnings:?}"
    );
    assert!(
        data["projects"].as_array().unwrap().is_empty(),
        "unreadable index lists no projects"
    );
}

// ---- project.forget removes the projects-index entry (#452, §2.8) ----

#[test]
fn forget_by_id_removes_entry_then_id_no_longer_resolves() {
    let dir = TempDir::new().unwrap();
    let mut engine = Engine::new(dir.path());

    // create registers the project in the durable index (§2.6).
    let id = create_project(&mut engine, &dir, "forget-by-id");
    assert!(
        verbreel_storage::layout::read_index(dir.path())
            .unwrap()
            .contains_key(&id),
        "create must register the project in the index"
    );

    let env = engine.dispatch("project.forget", json!({ "project_id": id }));
    let (data, _, _, _) = ok_parts(env);
    assert_eq!(
        data["was_in_index"],
        json!(true),
        "forgetting an indexed id reports was_in_index: true"
    );
    assert!(
        data["removed_path"].is_string(),
        "forget echoes the resolved removed_path"
    );

    // The id is gone from the index: resolving it now fails NotFound.
    assert!(
        !verbreel_storage::layout::read_index(dir.path())
            .unwrap()
            .contains_key(&id),
        "forget-by-id must remove the index entry"
    );
    let resolved = verbreel_storage::layout::resolve_root_for_project_id(dir.path(), &id);
    assert!(
        matches!(
            resolved,
            Err(verbreel_storage::layout::ResolveError::NotFound(_))
        ),
        "the forgotten id must no longer resolve, got {resolved:?}"
    );
}

#[test]
fn forget_by_path_indexed_reports_was_in_index_true() {
    let dir = TempDir::new().unwrap();
    let mut engine = Engine::new(dir.path());

    let id = create_project(&mut engine, &dir, "forget-by-path");
    // The path the create registered (absolutised root) is what the path
    // form must match.
    let path = verbreel_storage::layout::read_index(dir.path()).unwrap()[&id]
        .path
        .clone();

    let env = engine.dispatch("project.forget", json!({ "path": path }));
    let (data, _, _, _) = ok_parts(env);
    assert_eq!(
        data["was_in_index"],
        json!(true),
        "forgetting an indexed path reports was_in_index: true"
    );
    assert!(
        !verbreel_storage::layout::read_index(dir.path())
            .unwrap()
            .contains_key(&id),
        "forget-by-path must remove the matching index entry"
    );
}

#[test]
fn forget_by_path_unindexed_reports_was_in_index_false() {
    let dir = TempDir::new().unwrap();
    let mut engine = Engine::new(dir.path());

    // A well-formed path never registered in this engine's index.
    let env = engine.dispatch(
        "project.forget",
        json!({ "path": "/tmp/never-registered-by-this-engine" }),
    );
    let (data, _, _, _) = ok_parts(env);
    assert_eq!(
        data["was_in_index"],
        json!(false),
        "an unindexed path reports was_in_index: false"
    );
    assert_eq!(
        data["removed_path"],
        json!("/tmp/never-registered-by-this-engine"),
        "removed_path echoes the input path verbatim"
    );
}

#[test]
fn forget_by_unknown_id_returns_project_not_found() {
    let dir = TempDir::new().unwrap();
    let mut engine = Engine::new(dir.path());

    let env = engine.dispatch(
        "project.forget",
        json!({ "project_id": "0190b8d3-15e3-7000-bd00-0000000000ff" }),
    );
    let Envelope::Err { code, .. } = &env else {
        panic!("forget of an unknown id must be Err, got {env:?}");
    };
    assert_eq!(code, "E_PROJECT_NOT_FOUND");
}
