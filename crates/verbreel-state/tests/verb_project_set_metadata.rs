//! Tests for `project.set_metadata` (§2.12) — the first real verb.
//!
//! These cover the `compute_patch` order-of-operations
//! (merge / replace / unset / null-removal), the §0.13 cap rejections,
//! the args-incompatible matrix, the `data_envelope` helper, and one
//! full round-trip through [`validate_reconstructors`] proving the
//! reconstructor is replay-deterministic against a recorded fixture.

use std::sync::Arc;

use serde_json::{Map, Value, json};
use verbreel_state::{
    METADATA_MAX_BYTES, METADATA_MAX_KEYS, Project, ProjectSetMetadataArgs,
    ProjectSetMetadataError, ProjectSetMetadataVerb, RecordedEvent, VerbRegistry,
    validate_reconstructors,
    verbs::project_set_metadata::{compute_patch, data_envelope},
};
use verbreel_types::ProjectId;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");

/// Load the canonical empty-project fixture as the prior state.
fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture → Project")
}

/// Build a `Project` whose `metadata` field is set to `m`. Everything
/// else mirrors `empty_project_create.json`.
fn project_with_metadata(m: Map<String, Value>) -> Project {
    let mut p = empty_project();
    p.metadata = m;
    p
}

/// The fixture project's own id — every test reuses it as
/// `args.project_id` so the round-trip envelope's `project_id` field
/// matches by construction.
fn fixture_project_id() -> ProjectId {
    empty_project().id
}

/// Convenience: build `ProjectSetMetadataArgs` with `replace=false`,
/// `unset=None`, and the fixture project id.
fn merge_args(metadata: Option<Map<String, Value>>) -> ProjectSetMetadataArgs {
    ProjectSetMetadataArgs {
        project_id: fixture_project_id(),
        metadata,
        replace: false,
        unset: None,
    }
}

/// Convenience: build a `Map<String, Value>` from a list of `(k, v)`
/// pairs while preserving declaration order.
fn map_from(pairs: &[(&str, Value)]) -> Map<String, Value> {
    let mut m = Map::new();
    for (k, v) in pairs {
        m.insert((*k).to_owned(), v.clone());
    }
    m
}

/// Convenience: extract the `replace`-op value off the patch. Panics
/// (test-only) if the patch shape isn't the wholesale-replace form.
fn patch_value(patch: &Value) -> Map<String, Value> {
    let arr = patch.as_array().expect("patch is an array");
    assert_eq!(arr.len(), 1, "wholesale-replace patch has exactly one op");
    let op = arr[0].as_object().expect("op is an object");
    assert_eq!(op.get("op").and_then(Value::as_str), Some("replace"));
    assert_eq!(op.get("path").and_then(Value::as_str), Some("/metadata"));
    op.get("value")
        .and_then(Value::as_object)
        .cloned()
        .expect("replace op carries an object value")
}

// ---------------------------------------------------------------------
// Merge mode
// ---------------------------------------------------------------------

#[test]
fn compute_patch_merge_with_empty_prior() {
    let prior = empty_project();
    let args = merge_args(Some(map_from(&[("author", json!("alice"))])));
    let (patch, new_meta) = compute_patch(&prior, &args).expect("merge with empty prior ok");
    let expected = map_from(&[("author", json!("alice"))]);
    assert_eq!(new_meta, expected);
    assert_eq!(patch_value(&patch), expected);
}

#[test]
fn compute_patch_merge_overwrites_existing_key() {
    let prior = project_with_metadata(map_from(&[("author", json!("alice"))]));
    let args = merge_args(Some(map_from(&[("author", json!("bob"))])));
    let (_, new_meta) = compute_patch(&prior, &args).expect("overwrite ok");
    let expected = map_from(&[("author", json!("bob"))]);
    assert_eq!(new_meta, expected);
}

#[test]
fn compute_patch_merge_preserves_absent_keys() {
    let prior = project_with_metadata(map_from(&[("a", json!(1)), ("b", json!(2))]));
    let args = merge_args(Some(map_from(&[("a", json!(3))])));
    let (_, new_meta) = compute_patch(&prior, &args).expect("partial merge ok");
    let expected = map_from(&[("a", json!(3)), ("b", json!(2))]);
    assert_eq!(new_meta, expected);
}

#[test]
fn compute_patch_merge_null_value_removes_key() {
    let prior = project_with_metadata(map_from(&[("a", json!(1))]));
    let args = merge_args(Some(map_from(&[("a", Value::Null)])));
    let (_, new_meta) = compute_patch(&prior, &args).expect("null-removes-key ok");
    assert!(
        new_meta.is_empty(),
        "null-value should remove the key, got {new_meta:?}"
    );
}

// ---------------------------------------------------------------------
// Unset mode
// ---------------------------------------------------------------------

#[test]
fn compute_patch_unset_removes_listed_keys() {
    let prior = project_with_metadata(map_from(&[("a", json!(1)), ("b", json!(2))]));
    let args = ProjectSetMetadataArgs {
        project_id: fixture_project_id(),
        metadata: None,
        replace: false,
        unset: Some(vec!["a".to_owned()]),
    };
    let (_, new_meta) = compute_patch(&prior, &args).expect("unset removes ok");
    let expected = map_from(&[("b", json!(2))]);
    assert_eq!(new_meta, expected);
}

// ---------------------------------------------------------------------
// Replace mode
// ---------------------------------------------------------------------

#[test]
fn compute_patch_replace_true_wipes_then_applies() {
    let prior = project_with_metadata(map_from(&[("a", json!(1)), ("b", json!(2))]));
    let args = ProjectSetMetadataArgs {
        project_id: fixture_project_id(),
        metadata: Some(map_from(&[("c", json!(3))])),
        replace: true,
        unset: None,
    };
    let (_, new_meta) = compute_patch(&prior, &args).expect("replace ok");
    let expected = map_from(&[("c", json!(3))]);
    assert_eq!(new_meta, expected);
}

#[test]
fn compute_patch_replace_drops_null_values() {
    // Per the null-value semantics: `null` always means "remove". In
    // replace mode the prior is wiped first, so a `null`-valued entry
    // can't actually remove anything — but it MUST NOT land in the
    // result either (the semantics divergence still applies).
    let prior = project_with_metadata(map_from(&[("a", json!(1))]));
    let args = ProjectSetMetadataArgs {
        project_id: fixture_project_id(),
        metadata: Some(map_from(&[("b", Value::Null), ("c", json!(2))])),
        replace: true,
        unset: None,
    };
    let (_, new_meta) = compute_patch(&prior, &args).expect("replace + null ok");
    let expected = map_from(&[("c", json!(2))]);
    assert_eq!(new_meta, expected);
}

// ---------------------------------------------------------------------
// Args-incompatible matrix
// ---------------------------------------------------------------------

#[test]
fn compute_patch_args_incompatible_replace_and_unset() {
    let prior = empty_project();
    let args = ProjectSetMetadataArgs {
        project_id: fixture_project_id(),
        metadata: Some(map_from(&[("a", json!(1))])),
        replace: true,
        unset: Some(vec!["b".to_owned()]),
    };
    let err = compute_patch(&prior, &args).expect_err("replace + unset must reject");
    assert_eq!(
        err,
        ProjectSetMetadataError::ArgsIncompatibleReplaceAndUnset
    );
}

#[test]
fn compute_patch_args_incompatible_neither_supplied() {
    let prior = empty_project();
    let args = ProjectSetMetadataArgs {
        project_id: fixture_project_id(),
        metadata: None,
        replace: false,
        unset: None,
    };
    let err = compute_patch(&prior, &args).expect_err("neither metadata nor unset must reject");
    assert_eq!(
        err,
        ProjectSetMetadataError::ArgsIncompatibleNeitherMetadataNorUnset
    );
}

// ---------------------------------------------------------------------
// Cap rejections
// ---------------------------------------------------------------------

#[test]
fn compute_patch_unset_too_long() {
    let prior = empty_project();
    let too_many: Vec<String> = (0..=METADATA_MAX_KEYS).map(|i| format!("k{i}")).collect();
    assert_eq!(too_many.len(), METADATA_MAX_KEYS + 1);
    let args = ProjectSetMetadataArgs {
        project_id: fixture_project_id(),
        metadata: None,
        replace: false,
        unset: Some(too_many),
    };
    match compute_patch(&prior, &args).expect_err("unset over cap must reject") {
        ProjectSetMetadataError::UnsetTooLong { actual, cap } => {
            assert_eq!(actual, METADATA_MAX_KEYS + 1);
            assert_eq!(cap, METADATA_MAX_KEYS);
        }
        other => panic!("expected UnsetTooLong, got {other:?}"),
    }
}

#[test]
fn compute_patch_keys_over_cap() {
    let prior = empty_project();
    let mut m = Map::new();
    for i in 0..=METADATA_MAX_KEYS {
        m.insert(format!("k{i}"), json!(0));
    }
    assert_eq!(m.len(), METADATA_MAX_KEYS + 1);
    let args = merge_args(Some(m));
    match compute_patch(&prior, &args).expect_err("post-merge over cap must reject") {
        ProjectSetMetadataError::KeysOverCap { actual, cap } => {
            assert_eq!(actual, METADATA_MAX_KEYS + 1);
            assert_eq!(cap, METADATA_MAX_KEYS);
        }
        other => panic!("expected KeysOverCap, got {other:?}"),
    }
}

#[test]
fn compute_patch_bytes_over_cap() {
    let prior = empty_project();
    // Build a single-key map whose serialization exceeds the byte cap.
    // One large string value is enough; the key-count check (=1) sails
    // through and the byte check fires.
    let blob = "x".repeat(METADATA_MAX_BYTES + 100);
    let mut m = Map::new();
    m.insert("k".to_owned(), json!(blob));
    let args = merge_args(Some(m));
    match compute_patch(&prior, &args).expect_err("post-merge bytes over cap must reject") {
        ProjectSetMetadataError::BytesOverCap { actual, cap } => {
            assert!(actual > cap, "actual={actual}, cap={cap}");
            assert_eq!(cap, METADATA_MAX_BYTES);
        }
        other => panic!("expected BytesOverCap, got {other:?}"),
    }
}

// ---------------------------------------------------------------------
// Envelope helper
// ---------------------------------------------------------------------

#[test]
fn data_envelope_returns_post_state_metadata() {
    let post_state = project_with_metadata(map_from(&[("after", json!(true))]));
    let args = merge_args(Some(map_from(&[("after", json!(true))])));
    let env = data_envelope(&args, &post_state);
    assert_eq!(env.project_id, args.project_id);
    assert_eq!(env.metadata, post_state.metadata);
}

// ---------------------------------------------------------------------
// Reconstructor round-trip — the §0.8 startup-gate exercise
// ---------------------------------------------------------------------

#[test]
fn reconstructor_round_trip() {
    // 1. Build prior with some metadata.
    let prior = project_with_metadata(map_from(&[("seed", json!("v1"))]));

    // 2. Build args — overwrite the seeded key, add another.
    let args = merge_args(Some(map_from(&[
        ("seed", json!("v2")),
        ("note", json!("hello")),
    ])));

    // 3. Compute the patch and the post-merge metadata.
    let (patch, new_metadata) = compute_patch(&prior, &args).expect("compute_patch ok");

    // 4. Simulate `apply()` by setting `post_state.metadata =
    //    new_metadata` directly. The real kernel would run
    //    `Project::apply(&patch)` here; this slice doesn't wire that
    //    integration, so the test reaches the same post-state by
    //    hand-applying the patch's `replace` op.
    let mut post_state = prior.clone();
    post_state.metadata = new_metadata;

    // 5. Build the expected envelope via the helper and serialize it
    //    so we can compare apples-to-apples with what the
    //    reconstructor produces.
    let expected_envelope = data_envelope(&args, &post_state);
    let expected_data = serde_json::to_value(&expected_envelope).expect("envelope → Value");

    // 6. Build the recorded event the gate exercises against.
    let recorded = RecordedEvent {
        verb: "project.set_metadata".to_owned(),
        args: serde_json::to_value(&args).expect("args → Value"),
        patch,
        warnings: vec![],
        post_state,
        expected_data,
    };

    // 7. Register the reconstructor and run the validator.
    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(ProjectSetMetadataVerb))
        .expect("register ok");

    let report = validate_reconstructors(&registry, &[recorded])
        .expect("reconstructor round-trip must pass");
    assert_eq!(report.verbs_checked, vec!["project.set_metadata"]);
    assert_eq!(report.fixtures_run, 1);
}
