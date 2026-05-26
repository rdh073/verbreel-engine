//! Tests for `tracker.remove` (§18.5) — seventieth production verb.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::tracker::Tracker;
use verbreel_state::verbs::tracker_remove::{
    TrackerRemoveArgs, TrackerRemoveData, TrackerRemoveError, TrackerRemoveVerb,
    W_TRACKER_REMOVE_ENVELOPE_CODE, compute_patch, data_envelope_from_args_warnings,
};
use verbreel_state::{
    Project, VerbRegistry, default_fixtures, default_registry, validate_reconstructors,
};

#[cfg(feature = "native")]
use verbreel_state::{MutateOutcome, ProjectStore};

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";

const TRACKER_ID_1: &str = "01900000-0000-7000-8000-0000000bb001";
const TRACKER_ID_2: &str = "01900000-0000-7000-8000-0000000bb002";
const TRACKER_ID_3: &str = "01900000-0000-7000-8000-0000000bb003";
const MISSING_TRACKER: &str = "01900000-0000-7000-8000-0000000bb999";

const CACHE_PATH: &str =
    "cache/trackers/53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.json";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture -> Project")
}

fn fixture_project_id() -> verbreel_types::ProjectId {
    FIXTURE_PROJECT_ID
        .parse()
        .expect("fixture project id parses")
}

fn tracker_from_json(value: Value) -> Tracker {
    serde_json::from_value(value).expect("tracker fixture parses")
}

fn base_args(tracker_id: &str, purge_cache: Option<bool>) -> TrackerRemoveArgs {
    TrackerRemoveArgs {
        project_id: fixture_project_id(),
        tracker_id: tracker_id.to_string(),
        purge_cache,
    }
}

fn patch_ops(patch: &Value) -> &[Value] {
    patch.as_array().expect("patch is array")
}

fn apply_patch(prior: &Project, patch: Value) -> Project {
    let patch: json_patch::Patch = serde_json::from_value(patch).expect("patch parses");
    prior.apply(&patch).expect("patch applies")
}

fn project_with_one_tracker(tracker_id: &str, cache_path: &str) -> Project {
    let mut project = empty_project();
    project.trackers.push(tracker_from_json(json!({
        "tracker_id": tracker_id,
        "source_clip_id": "",
        "algorithm": "object",
        "applied_to_clip_ids": [],
        "cache_hash": "",
        "cache_path": cache_path,
    })));
    project
}

fn project_with_three_trackers() -> Project {
    let mut project = empty_project();
    project.trackers.push(tracker_from_json(json!({
        "tracker_id": TRACKER_ID_1,
        "source_clip_id": "",
        "algorithm": "object",
        "applied_to_clip_ids": [],
        "cache_hash": "",
        "cache_path": "",
    })));
    project.trackers.push(tracker_from_json(json!({
        "tracker_id": TRACKER_ID_2,
        "source_clip_id": "",
        "algorithm": "face",
        "applied_to_clip_ids": [],
        "cache_hash": "",
        "cache_path": "",
    })));
    project.trackers.push(tracker_from_json(json!({
        "tracker_id": TRACKER_ID_3,
        "source_clip_id": "",
        "algorithm": "optical_flow",
        "applied_to_clip_ids": [],
        "cache_hash": "",
        "cache_path": "",
    })));
    project
}

// ---------------------------------------------------------------------
// Args shape / deserialization
// ---------------------------------------------------------------------

#[test]
fn args_deserialize_ok_minimal() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "tracker_id": TRACKER_ID_1,
    });
    let parsed: TrackerRemoveArgs = serde_json::from_value(raw).expect("minimal args parse");
    assert_eq!(parsed.tracker_id, TRACKER_ID_1);
    // Default per §18.5: purge_cache defaults to Some(true) when omitted.
    assert_eq!(parsed.purge_cache, Some(true));
}

#[test]
fn args_deserialize_ok_with_purge_cache_false() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "tracker_id": TRACKER_ID_1,
        "purge_cache": false,
    });
    let parsed: TrackerRemoveArgs = serde_json::from_value(raw).expect("args parse");
    assert_eq!(parsed.purge_cache, Some(false));
}

#[test]
fn args_deserialize_ok_with_purge_cache_true_explicit() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "tracker_id": TRACKER_ID_1,
        "purge_cache": true,
    });
    let parsed: TrackerRemoveArgs = serde_json::from_value(raw).expect("args parse");
    assert_eq!(parsed.purge_cache, Some(true));
}

#[test]
fn args_missing_tracker_id_fails() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
    });
    let result: Result<TrackerRemoveArgs, _> = serde_json::from_value(raw);
    assert!(
        result.is_err(),
        "missing tracker_id must fail to deserialize"
    );
}

#[test]
fn args_missing_project_id_fails() {
    let raw = json!({
        "tracker_id": TRACKER_ID_1,
    });
    let result: Result<TrackerRemoveArgs, _> = serde_json::from_value(raw);
    assert!(
        result.is_err(),
        "missing project_id must fail to deserialize"
    );
}

#[test]
fn args_wrong_type_for_tracker_id_fails() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "tracker_id": 12345,
    });
    let result: Result<TrackerRemoveArgs, _> = serde_json::from_value(raw);
    assert!(
        result.is_err(),
        "numeric tracker_id must fail to deserialize"
    );
}

// ---------------------------------------------------------------------
// Error variants
// ---------------------------------------------------------------------

#[test]
fn compute_patch_unknown_tracker_returns_tracker_not_found() {
    let prior = project_with_one_tracker(TRACKER_ID_1, "");
    let args = base_args(MISSING_TRACKER, Some(true));
    let err = compute_patch(&prior, &args).expect_err("unknown tracker");
    assert!(matches!(
        err,
        TrackerRemoveError::TrackerNotFound { ref tracker_id } if tracker_id == MISSING_TRACKER
    ));
}

#[test]
fn compute_patch_empty_trackers_returns_tracker_not_found() {
    let prior = empty_project();
    let args = base_args(TRACKER_ID_1, Some(true));
    let err = compute_patch(&prior, &args).expect_err("empty trackers");
    assert!(matches!(err, TrackerRemoveError::TrackerNotFound { .. }));
}

#[test]
fn compute_patch_empty_tracker_id_returns_bad_selector() {
    let prior = project_with_one_tracker(TRACKER_ID_1, "");
    let args = base_args("", Some(true));
    let err = compute_patch(&prior, &args).expect_err("empty tracker_id");
    assert!(matches!(err, TrackerRemoveError::BadSelector { .. }));
}

// ---------------------------------------------------------------------
// Happy paths
// ---------------------------------------------------------------------

#[test]
fn compute_patch_happy_path_emits_single_remove_op_at_trackers_index() {
    let prior = project_with_one_tracker(TRACKER_ID_1, "");
    let args = base_args(TRACKER_ID_1, Some(true));
    let (patch, _warnings, _data) = compute_patch(&prior, &args).expect("happy path");

    let ops = patch_ops(&patch);
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0]["op"], "remove");
    assert_eq!(ops[0]["path"], "/trackers/0");
}

#[test]
fn compute_patch_returns_tracker_id_matching_args() {
    let prior = project_with_one_tracker(TRACKER_ID_1, "");
    let args = base_args(TRACKER_ID_1, Some(true));
    let (_patch, _warnings, data) = compute_patch(&prior, &args).expect("happy path");
    assert_eq!(data.tracker_id, TRACKER_ID_1);
}

#[test]
fn compute_patch_with_non_empty_cache_path_returns_some_path() {
    let prior = project_with_one_tracker(TRACKER_ID_1, CACHE_PATH);
    let args = base_args(TRACKER_ID_1, Some(true));
    let (_patch, _warnings, data) = compute_patch(&prior, &args).expect("happy path");
    assert_eq!(data.cache_path, Some(CACHE_PATH.to_string()));
}

#[test]
fn compute_patch_with_empty_cache_path_returns_none() {
    let prior = project_with_one_tracker(TRACKER_ID_1, "");
    let args = base_args(TRACKER_ID_1, Some(true));
    let (_patch, _warnings, data) = compute_patch(&prior, &args).expect("happy path");
    assert_eq!(data.cache_path, None);
}

#[test]
fn compute_patch_with_missing_cache_path_field_returns_none() {
    // Tracker placeholder without `cache_path` key at all.
    let mut prior = empty_project();
    prior.trackers.push(tracker_from_json(json!({
        "tracker_id": TRACKER_ID_1,
        "source_clip_id": "",
        "algorithm": "object",
    })));
    let args = base_args(TRACKER_ID_1, Some(true));
    let (_patch, _warnings, data) = compute_patch(&prior, &args).expect("happy path");
    assert_eq!(data.cache_path, None);
}

#[test]
fn cache_purged_always_false_v1_floor_when_purge_cache_true() {
    // v1 floor: even when `purge_cache: true` is explicit, no unlink
    // happens. Locks the deferral documented in the module-level note.
    let prior = project_with_one_tracker(TRACKER_ID_1, CACHE_PATH);
    let args = base_args(TRACKER_ID_1, Some(true));
    let (_patch, _warnings, data) = compute_patch(&prior, &args).expect("happy path");
    assert!(!data.cache_purged);
}

#[test]
fn cache_purged_false_when_purge_cache_false() {
    // Non-purge intent — also `false` (same v1 behavior).
    let prior = project_with_one_tracker(TRACKER_ID_1, CACHE_PATH);
    let args = base_args(TRACKER_ID_1, Some(false));
    let (_patch, _warnings, data) = compute_patch(&prior, &args).expect("happy path");
    assert!(!data.cache_purged);
}

#[test]
fn patch_removes_correct_tracker_by_index_when_three_trackers_present() {
    // Remove the middle tracker. Patch must target /trackers/1.
    let prior = project_with_three_trackers();
    let args = base_args(TRACKER_ID_2, Some(true));
    let (patch, _warnings, _data) = compute_patch(&prior, &args).expect("happy path");

    let ops = patch_ops(&patch);
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0]["path"], "/trackers/1");

    // After applying, only T1 and T3 remain in order.
    let post = apply_patch(&prior, patch);
    assert_eq!(post.trackers.len(), 2);
    assert_eq!(
        post.trackers[0].0.get("tracker_id").and_then(Value::as_str),
        Some(TRACKER_ID_1)
    );
    assert_eq!(
        post.trackers[1].0.get("tracker_id").and_then(Value::as_str),
        Some(TRACKER_ID_3)
    );
}

#[test]
fn apply_patch_decreases_trackers_length_by_one() {
    let prior = project_with_three_trackers();
    let args = base_args(TRACKER_ID_2, Some(true));
    let (patch, _warnings, _data) = compute_patch(&prior, &args).expect("happy path");
    let post = apply_patch(&prior, patch);
    assert_eq!(post.trackers.len(), prior.trackers.len() - 1);
}

#[test]
fn multi_tracker_isolation_other_trackers_preserved_byte_identical() {
    // Removing T1 must leave T2 and T3 in `prior.trackers` with their
    // original Map content intact.
    let prior = project_with_three_trackers();
    let original_t2 = prior.trackers[1].clone();
    let original_t3 = prior.trackers[2].clone();

    let args = base_args(TRACKER_ID_1, Some(true));
    let (patch, _warnings, _data) = compute_patch(&prior, &args).expect("happy path");
    let post = apply_patch(&prior, patch);

    assert_eq!(post.trackers.len(), 2);
    assert_eq!(post.trackers[0], original_t2);
    assert_eq!(post.trackers[1], original_t3);
}

// ---------------------------------------------------------------------
// Envelope warning
// ---------------------------------------------------------------------

#[test]
fn envelope_warning_emitted_on_success() {
    let prior = project_with_one_tracker(TRACKER_ID_1, "");
    let args = base_args(TRACKER_ID_1, Some(true));
    let (_patch, warnings, _data) = compute_patch(&prior, &args).expect("happy path");
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_TRACKER_REMOVE_ENVELOPE_CODE);
}

#[test]
fn envelope_carries_removed_tracker_id_matching_args() {
    let prior = project_with_one_tracker(TRACKER_ID_1, "");
    let args = base_args(TRACKER_ID_1, Some(true));
    let (_patch, warnings, _data) = compute_patch(&prior, &args).expect("happy path");
    assert_eq!(warnings[0]["details"]["removed_tracker_id"], TRACKER_ID_1);
}

#[test]
fn envelope_carries_cache_path_some_when_tracker_has_one() {
    let prior = project_with_one_tracker(TRACKER_ID_1, CACHE_PATH);
    let args = base_args(TRACKER_ID_1, Some(true));
    let (_patch, warnings, _data) = compute_patch(&prior, &args).expect("happy path");
    assert_eq!(warnings[0]["details"]["cache_path"], CACHE_PATH);
}

#[test]
fn envelope_carries_cache_path_null_when_tracker_has_none() {
    let prior = project_with_one_tracker(TRACKER_ID_1, "");
    let args = base_args(TRACKER_ID_1, Some(true));
    let (_patch, warnings, _data) = compute_patch(&prior, &args).expect("happy path");
    assert_eq!(warnings[0]["details"]["cache_path"], Value::Null);
}

#[test]
fn envelope_carries_cache_purged_false() {
    let prior = project_with_one_tracker(TRACKER_ID_1, CACHE_PATH);
    let args = base_args(TRACKER_ID_1, Some(true));
    let (_patch, warnings, _data) = compute_patch(&prior, &args).expect("happy path");
    assert_eq!(warnings[0]["details"]["cache_purged"], json!(false));
}

// ---------------------------------------------------------------------
// Data envelope serialization
// ---------------------------------------------------------------------

#[test]
fn data_serialization_omits_cache_path_when_none() {
    let prior = project_with_one_tracker(TRACKER_ID_1, "");
    let args = base_args(TRACKER_ID_1, Some(true));
    let (_patch, _warnings, data) = compute_patch(&prior, &args).expect("happy path");
    let value: Value = serde_json::to_value(&data).expect("envelope serializes");
    let obj = value.as_object().expect("data is JSON object");
    assert!(
        !obj.contains_key("cache_path"),
        "cache_path must be omitted when None, got: {value}"
    );
}

#[test]
fn data_serialization_includes_cache_path_when_some() {
    let prior = project_with_one_tracker(TRACKER_ID_1, CACHE_PATH);
    let args = base_args(TRACKER_ID_1, Some(true));
    let (_patch, _warnings, data) = compute_patch(&prior, &args).expect("happy path");
    let value: Value = serde_json::to_value(&data).expect("envelope serializes");
    assert_eq!(value["cache_path"], CACHE_PATH);
}

// ---------------------------------------------------------------------
// Reconstructor round-trips
// ---------------------------------------------------------------------

#[test]
fn reconstruct_round_trip_with_none_cache_path() {
    let prior = project_with_one_tracker(TRACKER_ID_1, "");
    let args = base_args(TRACKER_ID_1, Some(true));
    let (_patch, warnings, data) = compute_patch(&prior, &args).expect("happy path");

    let reconstructed = data_envelope_from_args_warnings(&args, &warnings).expect("round-trip");
    assert_eq!(data, reconstructed);

    // Byte-identical when serialized.
    let lhs = serde_json::to_vec(&data).expect("forward data serializes");
    let rhs = serde_json::to_vec(&reconstructed).expect("reconstructed serializes");
    assert_eq!(lhs, rhs);
}

#[test]
fn reconstruct_round_trip_with_some_cache_path() {
    let prior = project_with_one_tracker(TRACKER_ID_1, CACHE_PATH);
    let args = base_args(TRACKER_ID_1, Some(true));
    let (_patch, warnings, data) = compute_patch(&prior, &args).expect("happy path");

    let reconstructed = data_envelope_from_args_warnings(&args, &warnings).expect("round-trip");
    assert_eq!(data, reconstructed);

    let lhs = serde_json::to_vec(&data).expect("forward data serializes");
    let rhs = serde_json::to_vec(&reconstructed).expect("reconstructed serializes");
    assert_eq!(lhs, rhs);
}

#[test]
fn reconstruct_rejects_warning_set_without_envelope() {
    let args = base_args(TRACKER_ID_1, Some(true));
    let warnings: Vec<Value> = vec![json!({
        "code": "W_OTHER",
        "message": "not the envelope",
        "details": {},
    })];
    let err = data_envelope_from_args_warnings(&args, &warnings)
        .expect_err("missing envelope warning must surface");
    assert!(matches!(
        err,
        verbreel_state::ReconstructError::MissingField { .. }
    ));
}

// ---------------------------------------------------------------------
// Verb-trait + registry integration
// ---------------------------------------------------------------------

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "tracker.remove")
        .expect("default_fixtures includes tracker.remove");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(TrackerRemoveVerb))
        .expect("register tracker.remove verb");

    let report = validate_reconstructors(&registry, &[fixture])
        .expect("tracker.remove reconstructor passes");
    assert_eq!(report.verbs_checked, vec!["tracker.remove"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn verb_trait_surface_lookup_via_default_registry() {
    let registry = default_registry();
    let verb = registry
        .get("tracker.remove")
        .expect("tracker.remove registered in default_registry");
    assert_eq!(verb.verb(), "tracker.remove");
}

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let prior = project_with_one_tracker(TRACKER_ID_1, CACHE_PATH);
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        prior,
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry succeeds");

    let outcome = store
        .mutate_via_verb(
            "tracker.remove",
            serde_json::to_value(base_args(TRACKER_ID_1, Some(true))).expect("args serialize"),
            None,
        )
        .expect("tracker.remove should route");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("happy path must apply");
    };
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_TRACKER_REMOVE_ENVELOPE_CODE);
    let envelope: TrackerRemoveData = serde_json::from_value(data).expect("data parses");
    assert_eq!(envelope.tracker_id, TRACKER_ID_1);
    assert_eq!(envelope.cache_path, Some(CACHE_PATH.to_string()));
    assert!(!envelope.cache_purged);
}
