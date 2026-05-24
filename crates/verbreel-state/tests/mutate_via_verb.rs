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

use serde_json::{Map, Value, json};
use tempfile::TempDir;
use verbreel_state::{
    LifecycleError, MutateOutcome, Project, ProjectStore, VerbError, default_fixtures,
    default_registry,
};

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

    let MutateOutcome::Applied { event_id, data } = outcome else {
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
    } = first
    else {
        panic!("first call should be Applied, got {first:?}");
    };

    let second = store
        .mutate_via_verb("project.set_metadata", args, Some("idem-1".into()))
        .expect("second (replay) call must succeed");
    let MutateOutcome::Replayed {
        event_id: replay_id,
        data: replay_data,
    } = second
    else {
        panic!("second call should be Replayed, got {second:?}");
    };

    assert_eq!(
        first_id, replay_id,
        "replay surfaces the original event id, not a fresh one"
    );
    // The replay envelope (per the Slice B3 known-gap) is the freshly-
    // computed value from this call's compute_patch — for `same args ⇒
    // same envelope` it matches the original.
    assert_eq!(
        first_data, replay_data,
        "replay envelope matches original (same args → same compute_patch output)"
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
