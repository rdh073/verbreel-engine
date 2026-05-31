//! Tests for [`ProjectStore::mutate_via_verb`] (§0.8 forward path).
//!
//! Slice B3 — covers the kernel-side verb routing: look up the verb in
//! the store's [`VerbRegistry`], call `Verb::compute_patch`, delegate
//! to `apply_write_ordering`, thread the verb's typed `data` envelope
//! through the returned [`MutateOutcome`].
//!
//! Native-only because the lifecycle module is feature-gated. Pairs
//! one-to-one with the scope-list in `task-prompt.md` Step 7 (7 tests).

#![cfg(feature = "native")]

use std::sync::Arc;

use serde_json::{Map, Value, json};
use tempfile::TempDir;
use verbreel_events::{EventBackend, EventBuilder, NativeBackend};
use verbreel_state::{
    LifecycleError, MutateOutcome, Project, ProjectStore, ReconstructError, Verb, VerbError,
    VerbRegistry, default_fixtures, default_registry,
};
use verbreel_types::EventId;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");

fn load_empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture → Project")
}

/// The `project.id` embedded in `empty_project_create.json`. Used so
/// the verb's args carry the same project_id as the in-memory project.
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";

#[test]
fn mutate_via_verb_set_metadata_round_trip() {
    // Happy path: default registry + default fixtures clear the gate
    // at construction; `mutate_via_verb` routes the call through
    // `ProjectSetMetadataVerb::compute_patch`, the patch lands on
    // disk, the in-memory project carries the merged metadata, and
    // the returned `data` is the typed `{ project_id, metadata }`
    // envelope.
    let dir = TempDir::new().unwrap();
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        load_empty_project(),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry must clear the gate and write project.json");

    let args = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "metadata": { "author": "alice" },
    });

    let outcome = store
        .mutate_via_verb("project.set_metadata", args, None)
        .expect("mutate_via_verb on happy path must succeed");

    let MutateOutcome::Applied {
        event_id,
        data,
        warnings,
        ..
    } = outcome
    else {
        panic!("happy path must return Applied, got {outcome:?}");
    };

    // Event id surfaced.
    assert_eq!(
        store.last_applied_event_id(),
        Some(event_id),
        "store tracks the just-applied event"
    );

    // In-memory project reflects the mutation.
    let expected_author = Value::String("alice".to_string());
    assert_eq!(
        store.project().metadata.get("author"),
        Some(&expected_author),
        "in-memory project.metadata.author == alice after mutate_via_verb"
    );

    // Data envelope shape: `{ project_id, metadata }`.
    let expected = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "metadata": { "author": "alice" },
    });
    assert_eq!(
        data, expected,
        "data envelope is the verb's typed `{{ project_id, metadata }}` shape"
    );
    assert!(
        warnings.is_empty(),
        "existing production verbs emit no warnings in Slice 1"
    );
}

#[test]
fn applied_outcome_includes_empty_warnings_from_existing_verbs() {
    let dir = TempDir::new().unwrap();
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        load_empty_project(),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry must clear the gate and write project.json");

    let args = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "metadata": { "author": "bob" },
    });

    let outcome = store
        .mutate_via_verb("project.set_metadata", args, None)
        .expect("happy path must succeed");

    let MutateOutcome::Applied { warnings, .. } = outcome else {
        panic!("applied outcome required, got {outcome:?}");
    };

    assert!(
        warnings.is_empty(),
        "Slice 1 production verbs emit no warnings; outcome must include []"
    );
}

#[test]
fn mutate_via_verb_unknown_verb_errors() {
    // A verb id absent from the registry surfaces as
    // `LifecycleError::UnknownVerb`. No event written, in-memory
    // project unchanged.
    let dir = TempDir::new().unwrap();
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        load_empty_project(),
        &default_registry(),
        &default_fixtures(),
    )
    .unwrap();
    let metadata_before = store.project().metadata.clone();

    let err = store
        .mutate_via_verb("not.a.real.verb", json!({}), None)
        .expect_err("unknown verb must error");

    match err {
        LifecycleError::UnknownVerb { verb_id } => {
            assert_eq!(verb_id, "not.a.real.verb");
        }
        other => panic!("expected UnknownVerb, got {other:?}"),
    }

    // Side-effect free.
    assert_eq!(
        store.project().metadata,
        metadata_before,
        "unknown-verb call must not mutate project"
    );
    assert_eq!(
        store.last_applied_event_id(),
        None,
        "unknown-verb call must not bump last_applied_event_id"
    );
}

#[test]
fn mutate_via_verb_empty_registry_errors() {
    // An un-registry-aware `create()` stores an empty registry under
    // the hood. Calling `mutate_via_verb` on that store returns
    // `UnknownVerb` for every verb id — including ones that DO ship
    // in `default_registry()`. Proves the registry is per-store, not
    // process-global.
    let dir = TempDir::new().unwrap();
    let mut store = ProjectStore::create(dir.path(), load_empty_project())
        .expect("plain create stores an empty VerbRegistry");

    let err = store
        .mutate_via_verb(
            "project.set_metadata",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "metadata": { "author": "alice" },
            }),
            None,
        )
        .expect_err("empty-registry store must reject every verb id");

    match err {
        LifecycleError::UnknownVerb { verb_id } => {
            assert_eq!(verb_id, "project.set_metadata");
        }
        other => panic!("expected UnknownVerb on empty registry, got {other:?}"),
    }
}

#[test]
fn mutate_via_verb_bad_args_errors() {
    // Args with neither `metadata` nor `unset` is the §2.12
    // `E_ARGS_INCOMPATIBLE` case. The verb's `compute_patch` rejects
    // it and the kernel re-raises as
    // `VerbExecutionFailed { source: VerbError::BadArgs }`.
    let dir = TempDir::new().unwrap();
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        load_empty_project(),
        &default_registry(),
        &default_fixtures(),
    )
    .unwrap();

    // `project_id` present, but neither `metadata` nor `unset` →
    // ArgsIncompatibleNeitherMetadataNorUnset (BadArgs).
    let bad_args = json!({ "project_id": FIXTURE_PROJECT_ID });

    let err = store
        .mutate_via_verb("project.set_metadata", bad_args, None)
        .expect_err("bad args must fail");

    match err {
        LifecycleError::VerbExecutionFailed { verb_id, source } => {
            assert_eq!(verb_id, "project.set_metadata");
            assert!(
                matches!(source, VerbError::BadArgs { .. }),
                "expected BadArgs inside VerbExecutionFailed, got {source:?}"
            );
        }
        other => panic!("expected VerbExecutionFailed, got {other:?}"),
    }
}

#[test]
fn mutate_via_verb_cap_violation_errors() {
    // 257 keys in `metadata` overruns the §0.13
    // `METADATA_MAX_KEYS` (256) cap. The verb's `compute_patch`
    // rejects it and the kernel re-raises as
    // `VerbExecutionFailed { source: VerbError::InvariantViolation }`.
    let dir = TempDir::new().unwrap();
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        load_empty_project(),
        &default_registry(),
        &default_fixtures(),
    )
    .unwrap();

    // Build a 257-key metadata bag — one more than the §0.13 cap of
    // 256. Each value is a tiny string so the byte cap doesn't fire
    // first; only the key-count check should trip.
    let mut metadata = Map::new();
    for i in 0..=verbreel_state::METADATA_MAX_KEYS {
        metadata.insert(format!("k{i}"), Value::from("v"));
    }
    assert_eq!(metadata.len(), verbreel_state::METADATA_MAX_KEYS + 1);

    let args = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "metadata": metadata,
    });

    let err = store
        .mutate_via_verb("project.set_metadata", args, None)
        .expect_err("cap-violating metadata must fail");

    match err {
        LifecycleError::VerbExecutionFailed { verb_id, source } => {
            assert_eq!(verb_id, "project.set_metadata");
            assert!(
                matches!(source, VerbError::InvariantViolation { .. }),
                "expected InvariantViolation inside VerbExecutionFailed, got {source:?}"
            );
        }
        other => panic!("expected VerbExecutionFailed, got {other:?}"),
    }
}

#[test]
fn mutate_via_verb_idempotency_replays() {
    // Same key + same args → second call returns Replayed with the
    // same `event_id` as the first. events.jsonl still holds exactly
    // one event line.
    let dir = TempDir::new().unwrap();
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        load_empty_project(),
        &default_registry(),
        &default_fixtures(),
    )
    .unwrap();

    let args = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "metadata": { "author": "alice" },
    });

    let first = store
        .mutate_via_verb("project.set_metadata", args.clone(), Some("idem-1".into()))
        .expect("first keyed call must succeed");
    let MutateOutcome::Applied {
        event_id: first_id,
        data: first_data,
        warnings: first_warnings,
        ..
    } = first
    else {
        panic!("first call should be Applied, got {first:?}");
    };
    assert_eq!(
        first_warnings,
        Vec::<Value>::new(),
        "first apply returns no replay warning"
    );

    let second = store
        .mutate_via_verb("project.set_metadata", args, Some("idem-1".into()))
        .expect("second (replay) call must succeed");
    let MutateOutcome::Replayed {
        event_id: replay_id,
        data: replay_data,
        warnings: replay_warnings,
        ..
    } = second
    else {
        panic!("second call should be Replayed, got {second:?}");
    };

    assert_eq!(
        first_id, replay_id,
        "replay surfaces the original event id, not a fresh one"
    );
    // §0.8 replay reconstructs `data` from the on-disk event line
    // (see `mutate_via_verb`'s "Replay path semantics"). Same args +
    // same patch + same warnings + same post-state ⇒ same envelope.
    assert_eq!(
        first_data, replay_data,
        "replay envelope matches original (reconstructed from disk via Verb::reconstruct)"
    );
    assert_eq!(
        replay_warnings,
        vec![json!({"code":"W_REPLAY","message":"idempotent replay"})],
        "replay appends W_REPLAY warning"
    );

    // Exactly one event line on disk (lock release on drop is
    // required before the test reads the file).
    drop(store);
    let events_path = dir.path().join(".verbreel").join("events.jsonl");
    let bytes = std::fs::read(&events_path).unwrap();
    let lines: Vec<&[u8]> = bytes
        .split(|&b| b == b'\n')
        .filter(|l| !l.is_empty())
        .collect();
    assert_eq!(lines.len(), 1, "replay must not write a second event line");
}

#[test]
fn replay_appends_w_replay_warning() {
    let dir = TempDir::new().unwrap();
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        load_empty_project(),
        &default_registry(),
        &default_fixtures(),
    )
    .unwrap();

    let args = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "metadata": { "author": "charlie" },
    });

    let first = store
        .mutate_via_verb(
            "project.set_metadata",
            args.clone(),
            Some("idem-replay-2".into()),
        )
        .expect("first call must apply");
    let MutateOutcome::Applied { warnings, .. } = first else {
        panic!("first call should be Applied, got {first:?}");
    };
    assert!(
        warnings.is_empty(),
        "first call does not include replay marker"
    );

    let second = store
        .mutate_via_verb("project.set_metadata", args, Some("idem-replay-2".into()))
        .expect("second call must replay");
    let MutateOutcome::Replayed { warnings, .. } = second else {
        panic!("second call should be Replayed, got {second:?}");
    };
    assert_eq!(
        warnings,
        vec![json!({"code":"W_REPLAY","message":"idempotent replay"})],
        "replay appends exactly one replay warning"
    );
}

#[test]
fn mutate_via_verb_idempotency_conflict() {
    // Same key + DIFFERENT args → second call returns
    // `IdempotencyConflict`. The in-memory project is unchanged from
    // the first call's result.
    let dir = TempDir::new().unwrap();
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        load_empty_project(),
        &default_registry(),
        &default_fixtures(),
    )
    .unwrap();

    let args_one = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "metadata": { "author": "alice" },
    });
    let args_two = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "metadata": { "author": "bob" },
    });

    store
        .mutate_via_verb(
            "project.set_metadata",
            args_one,
            Some("conflict-key".into()),
        )
        .expect("first keyed call must succeed");
    assert_eq!(
        store.project().metadata.get("author"),
        Some(&Value::String("alice".to_string()))
    );

    let err = store
        .mutate_via_verb(
            "project.set_metadata",
            args_two,
            Some("conflict-key".into()),
        )
        .expect_err("different-args replay under same key must conflict");

    match err {
        LifecycleError::IdempotencyConflict { key, .. } => {
            assert_eq!(key, "conflict-key");
        }
        other => panic!("expected IdempotencyConflict, got {other:?}"),
    }

    // Author still alice — the conflicting call did not overwrite.
    assert_eq!(
        store.project().metadata.get("author"),
        Some(&Value::String("alice".to_string())),
        "conflict must not mutate the project"
    );
}

// ---------------------------------------------------------------------
// Issue #63 — read_by_id + mutate_via_verb replay wiring
// ---------------------------------------------------------------------

/// Defensive: an idempotency-index `Completed { event_id }` whose id is
/// absent from `events.jsonl` must surface as
/// [`LifecycleError::ReplayEventMissing`] rather than silently
/// fabricating an envelope.
///
/// Setup directly pokes the index via [`ProjectStore::idempotency`] —
/// the public read-only handle exposes `start` + `complete`, which is
/// enough to plant a phantom `Completed` entry pointing at a fresh
/// `EventId::now()` (guaranteed not in the freshly-created log).
#[test]
fn replay_event_missing_returns_error() {
    let dir = TempDir::new().unwrap();
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        load_empty_project(),
        &default_registry(),
        &default_fixtures(),
    )
    .unwrap();

    // Args + matching fingerprint. The replay path runs `compute_patch`
    // BEFORE delegating to `mutate()`, so the args must shape-validate
    // against the verb — use the same metadata-set shape the other
    // happy-path tests use.
    let args = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "metadata": { "author": "alice" },
    });
    let fingerprint = verbreel_canon::sha256_hex(&args).expect("args canonicalize");

    // Plant a phantom Completed entry pointing at a synthetic event id
    // that does NOT exist on disk.
    let phantom_id = EventId::now();
    store
        .idempotency()
        .start("missing-key".into(), fingerprint)
        .expect("phantom start must succeed on empty index");
    store.idempotency().complete("missing-key", phantom_id);

    let err = store
        .mutate_via_verb("project.set_metadata", args, Some("missing-key".into()))
        .expect_err("replay against missing event must error");

    match err {
        LifecycleError::ReplayEventMissing { event_id } => {
            assert_eq!(
                event_id, phantom_id,
                "error carries the phantom id we planted, not a fresh one"
            );
        }
        other => panic!("expected ReplayEventMissing, got {other:?}"),
    }
}

// ---------------------------------------------------------------------
// Test-only verbs (Issue #63)
// ---------------------------------------------------------------------

/// `compute_patch` is a no-op (empty patch, null data). `reconstruct`
/// always returns `Err(ReconstructError::Custom("test"))`.
///
/// Register **without** a fixture so the §0.8 startup gate passes
/// vacuously for this verb — `validate_reconstructors` walks fixtures,
/// not the registry (see `reconstructor.rs` module docs).
struct BadReconstructVerb;

impl Verb for BadReconstructVerb {
    fn verb(&self) -> &'static str {
        "test.bad_reconstruct"
    }

    fn compute_patch(
        &self,
        _prior: &Project,
        _args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        // Emit a non-empty patch so the §0.8 write-ordering + idempotency
        // replay path is exercised — an empty patch would route to the
        // `MutateOutcome::NoOp` read-only fast-path, which never reaches
        // the replay reconstructor under test here.
        let patch: json_patch::Patch = serde_json::from_value(json!([
            { "op": "add", "path": "/metadata/_replay_probe", "value": "x" }
        ]))
        .expect("static probe patch is valid RFC 6902");
        Ok((patch, Value::Null, Vec::new()))
    }

    fn reconstruct(
        &self,
        _args: &Value,
        _patch: &Value,
        _warnings: &[Value],
        _post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        Err(ReconstructError::Custom("test".into()))
    }
}

/// `compute_patch` returns `{warnings_count: 0}` (compute time has no
/// access to warnings). `reconstruct` returns `{warnings_count: N}`
/// where N is the count of recorded warnings.
///
/// The difference between the two return values is the lever used by
/// `replay_reads_disk_event` to prove the replay path reads the on-disk
/// warnings vector rather than the duplicate call's freshly-computed
/// envelope.
struct WarningsCountVerb;

impl Verb for WarningsCountVerb {
    fn verb(&self) -> &'static str {
        "test.warnings_count"
    }

    fn compute_patch(
        &self,
        _prior: &Project,
        _args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        // Non-empty patch so the keyed replay path is reached rather than
        // the empty-patch `NoOp` fast-path. The replay never applies this
        // patch (it returns the cached envelope), so the value is inert.
        let patch: json_patch::Patch = serde_json::from_value(json!([
            { "op": "add", "path": "/metadata/_replay_probe", "value": "x" }
        ]))
        .expect("static probe patch is valid RFC 6902");
        Ok((patch, json!({ "warnings_count": 0 }), Vec::new()))
    }

    fn reconstruct(
        &self,
        _args: &Value,
        _patch: &Value,
        warnings: &[Value],
        _post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        Ok(json!({ "warnings_count": warnings.len() }))
    }
}

/// A reconstructor that errors at replay time must surface as
/// [`LifecycleError::ReplayReconstructFailed`], with the verb id and
/// event id threaded through and the underlying [`ReconstructError`]
/// chained via `#[source]`.
///
/// Drives a custom registry with `BadReconstructVerb` registered (no
/// fixture — gate passes vacuously per the module-doc rule that the
/// validator walks fixtures, not the registry).
#[test]
fn replay_reconstruct_error_propagates() {
    let dir = TempDir::new().unwrap();
    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(BadReconstructVerb))
        .expect("register BadReconstructVerb");

    let mut store =
        ProjectStore::create_with_registry(dir.path(), load_empty_project(), &registry, &[])
            .expect("create_with_registry with custom registry + no fixtures");

    // First call: Applied (compute_patch emits a non-empty probe patch
    // so the call writes an event and registers the idempotency key,
    // which the replay path below then re-reads).
    let args = json!({ "any": "args" });
    let first = store
        .mutate_via_verb("test.bad_reconstruct", args.clone(), Some("k".into()))
        .expect("first keyed call must succeed");
    let MutateOutcome::Applied { event_id, .. } = first else {
        panic!("first call should be Applied, got {first:?}");
    };

    // Second call: replay path → read_by_id succeeds → reconstruct
    // returns Err → ReplayReconstructFailed wraps it.
    let err = store
        .mutate_via_verb("test.bad_reconstruct", args, Some("k".into()))
        .expect_err("replay must propagate the reconstruct error");

    match err {
        LifecycleError::ReplayReconstructFailed {
            event_id: err_event_id,
            verb_id,
            source,
        } => {
            assert_eq!(
                err_event_id, event_id,
                "error carries the original event id, not a fresh one"
            );
            assert_eq!(verb_id, "test.bad_reconstruct");
            match source {
                ReconstructError::Custom(msg) => assert_eq!(msg, "test"),
                other => panic!("expected ReconstructError::Custom, got {other:?}"),
            }
        }
        other => panic!("expected ReplayReconstructFailed, got {other:?}"),
    }
}

/// The replay path reconstructs `data` from the **on-disk** event line,
/// not the duplicate call's freshly-computed `compute_patch` envelope.
///
/// Setup pre-seeds `events.jsonl` with a hand-crafted event carrying
/// `warnings: [{"code":"W_TEST"}]` + `idempotency_key: "k1"` so the
/// open-time index rebuild populates `Completed { event_id }` for the
/// pre-seeded id. The store is then opened with `WarningsCountVerb`
/// registered; calling `mutate_via_verb` with the same args + key fires
/// the replay path.
///
/// Lever: `WarningsCountVerb::compute_patch` always returns
/// `{warnings_count: 0}` (it never sees warnings), while
/// `WarningsCountVerb::reconstruct` returns `{warnings_count: N}` based
/// on the recorded `warnings` slice. Observing `{warnings_count: 1}` in
/// the replay envelope proves the wiring reads `recorded.warnings`
/// from disk rather than re-using compute-time data.
#[test]
fn replay_preserves_recorded_warnings() {
    let dir = TempDir::new().unwrap();

    // (1) Bootstrap the project root: project.json + .verbreel dir +
    // hand-crafted events.jsonl line.
    let project = load_empty_project();
    let project_json = serde_json::to_vec_pretty(&project).unwrap();
    std::fs::write(dir.path().join("project.json"), &project_json).unwrap();
    let verbreel_dir = dir.path().join(".verbreel");
    std::fs::create_dir_all(&verbreel_dir).unwrap();

    // Hand-build the seeded event. Use EventBuilder so we validate the
    // warnings field is serialized and read from disk.
    let seeded = EventBuilder::new()
        .verb("test.warnings_count")
        .args(json!({}))
        .patch(json_patch::Patch(Vec::new()))
        .warnings(vec![json!({ "code": "W_TEST" })])
        .idempotency_key("k1")
        .build();
    let seeded_id = seeded.id;
    let seeded_line = serde_json::to_string(&seeded).unwrap();
    {
        let backend = NativeBackend::open(verbreel_dir.join("events.jsonl")).unwrap();
        backend.append(seeded_line.as_bytes()).unwrap();
    }

    // (2) Open the store with WarningsCountVerb registered. The
    // open-time replay applies the (empty) patch and the index rebuild
    // populates `k1 → Completed { event_id: seeded_id }`.
    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(WarningsCountVerb))
        .expect("register WarningsCountVerb");
    let mut store = ProjectStore::open_with_registry(dir.path(), &registry, &[])
        .expect("open_with_registry must succeed against pre-seeded layout");

    // (3) Replay: same args + same key.
    let outcome = store
        .mutate_via_verb("test.warnings_count", json!({}), Some("k1".into()))
        .expect("replay must succeed");

    let MutateOutcome::Replayed {
        event_id,
        data,
        warnings,
        ..
    } = outcome
    else {
        panic!("expected Replayed (idempotency dedup), got {outcome:?}");
    };

    assert_eq!(
        event_id, seeded_id,
        "replay surfaces the pre-seeded event id"
    );

    // The lever: reconstruct produced `warnings_count: 1` because it
    // saw the on-disk `warnings: [W_TEST]`. If the old code path were
    // still in place, `data` would be the duplicate call's freshly-
    // computed `{warnings_count: 0}` (compute_patch has no access to
    // warnings).
    assert_eq!(
        data,
        json!({ "warnings_count": 1 }),
        "replay data must reflect the on-disk warnings slice, not compute_patch's empty default"
    );
    assert_eq!(
        warnings,
        vec![
            json!({ "code": "W_TEST" }),
            json!({"code":"W_REPLAY","message":"idempotent replay"}),
        ],
        "replay result prepends recorded warnings then appends W_REPLAY"
    );
}
