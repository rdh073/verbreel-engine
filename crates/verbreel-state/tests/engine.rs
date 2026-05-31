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

// ---- Engine::resolve_root threads home + storage resolver -----------

#[test]
fn resolve_root_through_index() {
    // The engine's `resolve_root` delegates to the storage resolver
    // against `self.home`. Register an id → resolve returns the root;
    // an unknown id → NotFound. (Index registration is a surface
    // concern; project.create does not yet register, so the test
    // registers directly to exercise the delegation.)
    use verbreel_storage::layout::{ResolveError, register_project};

    let home = TempDir::new().unwrap();
    let engine = Engine::new(home.path());

    let id = "0190b8d3-15e3-7000-bd00-0000000000cc";
    let root = home.path().join("projects/resolved");
    register_project(home.path(), id, &root).unwrap();

    assert_eq!(engine.resolve_root(id).unwrap(), root);

    let err = engine
        .resolve_root("0190b8d3-15e3-7000-bd00-0000000000dd")
        .unwrap_err();
    assert!(
        matches!(err, ResolveError::NotFound(_)),
        "unknown id must resolve to NotFound, got {err:?}"
    );
}
