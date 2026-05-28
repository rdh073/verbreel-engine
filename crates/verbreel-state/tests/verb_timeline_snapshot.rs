//! Tests for `timeline.snapshot` (§12.1) — sixty-first production verb.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_events::Timestamp;
use verbreel_state::verbs::timeline_snapshot::{compute_patch, data_envelope_from_post_state};
use verbreel_state::{
    MutateOutcome, Project, TimelineSnapshotArgs, TimelineSnapshotData, TimelineSnapshotVerb, Verb,
    VerbError, VerbRegistry, default_fixtures, default_registry, validate_reconstructors,
};
use verbreel_types::EventId;

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const FIXTURE_EVENT_ID: &str = "0190b8d3-15e3-7000-bd00-00000000ee01";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture → Project")
}

fn fixture_project_id() -> verbreel_types::ProjectId {
    FIXTURE_PROJECT_ID
        .parse()
        .expect("fixture project id parses")
}

fn fixture_event_id() -> EventId {
    FIXTURE_EVENT_ID.parse().expect("fixture event id parses")
}

fn make_args() -> TimelineSnapshotArgs {
    TimelineSnapshotArgs {
        project_id: fixture_project_id(),
    }
}

#[test]
fn args_deserialize_ok() {
    let args: TimelineSnapshotArgs = serde_json::from_value(json!({
        "project_id": FIXTURE_PROJECT_ID,
    }))
    .expect("valid args deserialize");
    assert_eq!(args.project_id, fixture_project_id());
}

#[test]
fn args_missing_project_id_field_is_bad_args() {
    let prior = empty_project();
    let verb = TimelineSnapshotVerb;
    let err = verb
        .compute_patch(&prior, &json!({}))
        .expect_err("missing project_id should fail");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_wrong_type_is_bad_args() {
    let prior = empty_project();
    let verb = TimelineSnapshotVerb;
    let err = verb
        .compute_patch(&prior, &json!({ "project_id": 42 }))
        .expect_err("integer project_id should fail");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn empty_project_yields_empty_sentinel() {
    let mut prior = empty_project();
    prior.last_saved_event_id = None;
    let (_, _, data) = compute_patch(&prior, &make_args()).expect("compute");
    assert_eq!(
        data.event_id, "empty",
        "None last_saved_event_id must surface as literal \"empty\" sentinel per §12.1"
    );
}

#[test]
fn empty_sentinel_is_literal_not_alias() {
    // Defends against drift to "none", "" or null.
    let mut prior = empty_project();
    prior.last_saved_event_id = None;
    let (_, _, data) = compute_patch(&prior, &make_args()).expect("compute");
    assert_ne!(data.event_id, "none");
    assert_ne!(data.event_id, "");
    assert_ne!(data.event_id, "null");
    assert_eq!(data.event_id, "empty");
}

#[test]
fn saved_project_yields_event_id_string() {
    let mut prior = empty_project();
    let id = fixture_event_id();
    prior.last_saved_event_id = Some(id);
    let (_, _, data) = compute_patch(&prior, &make_args()).expect("compute");
    assert_eq!(
        data.event_id,
        id.to_string(),
        "Some last_saved_event_id must surface as its stringified UUID"
    );
}

#[test]
fn project_hash_is_64_char_lowercase_hex() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &make_args()).expect("compute");
    assert_eq!(data.project_hash.len(), 64, "SHA-256 hex is 64 chars");
    assert!(
        data.project_hash
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
        "project_hash must be lowercase ascii-hex: {}",
        data.project_hash
    );
}

#[test]
fn project_hash_unchanged_by_updated_at_mutation() {
    let mut prior = empty_project();
    let (_, _, before) = compute_patch(&prior, &make_args()).expect("compute");

    prior.updated_at = Timestamp::parse("2099-12-31T23:59:59Z").unwrap();
    let (_, _, after) = compute_patch(&prior, &make_args()).expect("compute");

    assert_eq!(
        before.project_hash, after.project_hash,
        "project_hash MUST NOT change when only updated_at differs (§0.5.2 projection)"
    );
}

#[test]
fn project_hash_unchanged_by_last_saved_event_id_mutation() {
    // load → save → reload invariant: changing only last_saved_event_id
    // (the save-bookkeeping field) must not invalidate cache keys.
    let mut prior = empty_project();
    prior.last_saved_event_id = None;
    let (_, _, before) = compute_patch(&prior, &make_args()).expect("compute");

    prior.last_saved_event_id = Some(fixture_event_id());
    let (_, _, after) = compute_patch(&prior, &make_args()).expect("compute");

    assert_eq!(
        before.project_hash, after.project_hash,
        "project_hash MUST NOT change when only last_saved_event_id differs (§0.5.2 projection)"
    );
}

#[test]
fn project_hash_changes_when_real_field_changes() {
    let mut prior = empty_project();
    let (_, _, before) = compute_patch(&prior, &make_args()).expect("compute");

    prior.name = "renamed-project".to_string();
    let (_, _, after) = compute_patch(&prior, &make_args()).expect("compute");

    assert_ne!(
        before.project_hash, after.project_hash,
        "project_hash MUST change when graph content (name) changes"
    );
}

#[test]
fn project_hash_stable_across_input_key_order() {
    // Sanity check that the verb routes through the canon (which is
    // RFC-8785 key-sorting) rather than `serde_json::to_string()`.
    let prior = empty_project();
    let (_, _, data_first) = compute_patch(&prior, &make_args()).expect("compute");

    // Round-trip through Value (which preserves insertion order on
    // serde_json::Map) and back via the verb — the canon must produce
    // the same hash because it sorts keys.
    let as_value = serde_json::to_value(&prior).expect("serialize prior");
    let reconstituted: Project =
        serde_json::from_value(as_value).expect("deserialize prior round-trip");
    let (_, _, data_second) = compute_patch(&reconstituted, &make_args()).expect("compute");

    assert_eq!(
        data_first.project_hash, data_second.project_hash,
        "project_hash must be stable across input round-trips (canon sorts keys)"
    );
}

#[test]
fn empty_patch_returned() {
    let prior = empty_project();
    let (patch, _, _) = compute_patch(&prior, &make_args()).expect("compute");
    assert_eq!(patch, json!([]), "timeline.snapshot is read-only");
    assert!(
        patch.as_array().expect("patch is array").is_empty(),
        "patch must be empty"
    );
}

#[test]
fn empty_warnings_returned() {
    let prior = empty_project();
    let (_, warnings, _) = compute_patch(&prior, &make_args()).expect("compute");
    assert!(warnings.is_empty(), "timeline.snapshot emits no warnings");
}

#[test]
fn reconstructor_round_trip_matches_compute_patch() {
    let mut prior = empty_project();
    prior.last_saved_event_id = Some(fixture_event_id());
    let args = make_args();
    let (patch, _, expected) = compute_patch(&prior, &args).expect("compute");
    let patch: json_patch::Patch = serde_json::from_value(patch).expect("valid patch");
    let post_state = prior
        .apply(&patch)
        .expect("applying empty patch should succeed");

    let data = data_envelope_from_post_state(&args, &post_state)
        .expect("data envelope from post state should rebuild");
    assert_eq!(
        data, expected,
        "reconstructor must rebuild byte-identical envelope from post-state alone"
    );

    // And the byte-identical claim — serialize both and compare bytes.
    let expected_bytes = serde_json::to_vec(&expected).expect("serialize expected");
    let actual_bytes = serde_json::to_vec(&data).expect("serialize actual");
    assert_eq!(expected_bytes, actual_bytes);
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "timeline.snapshot")
        .expect("default_fixtures includes timeline.snapshot");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(TimelineSnapshotVerb))
        .expect("register timeline.snapshot");

    let report = validate_reconstructors(&registry, &[fixture])
        .expect("timeline.snapshot reconstruct from fixture should pass");
    assert_eq!(report.verbs_checked, vec!["timeline.snapshot"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn verb_trait_surface_via_serde_json_value() {
    let mut prior = empty_project();
    prior.last_saved_event_id = Some(fixture_event_id());
    let verb = TimelineSnapshotVerb;
    let (patch, data, warnings) = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
            }),
        )
        .expect("verb should route ok");
    assert_eq!(patch.0.len(), 0, "patch must be empty");
    assert!(warnings.is_empty());
    let typed: TimelineSnapshotData = serde_json::from_value(data).expect("data parses");
    assert_eq!(typed.event_id, fixture_event_id().to_string());
    assert_eq!(typed.project_hash.len(), 64);
}

#[test]
fn verb_lookup_via_default_registry() {
    let registry = default_registry();
    let verb = registry
        .get("timeline.snapshot")
        .expect("timeline.snapshot must be in default_registry");
    assert_eq!(verb.verb(), "timeline.snapshot");

    let prior = empty_project();
    let (patch, data, warnings) = verb
        .compute_patch(&prior, &json!({ "project_id": FIXTURE_PROJECT_ID }))
        .expect("registry-routed compute_patch ok");
    assert_eq!(patch.0.len(), 0);
    assert!(warnings.is_empty());
    let typed: TimelineSnapshotData = serde_json::from_value(data).expect("data parses");
    // The empty_project fixture comes with last_saved_event_id absent
    // (defaulted to None) — verify the sentinel is returned.
    assert_eq!(typed.event_id, "empty");
}

#[test]
fn data_serializes_with_exactly_two_fields() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &make_args()).expect("compute");
    let v: Value = serde_json::to_value(&data).expect("serialize");
    let obj = v.as_object().expect("data is object");
    assert_eq!(
        obj.len(),
        2,
        "envelope must carry exactly {{event_id, project_hash}}"
    );
    assert!(obj.contains_key("event_id"));
    assert!(obj.contains_key("project_hash"));
}

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        empty_project(),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry succeeds");

    let outcome = store
        .mutate_via_verb(
            "timeline.snapshot",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
            }),
            None,
        )
        .expect("timeline.snapshot should route");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("timeline.snapshot expected Applied outcome");
    };

    assert!(warnings.is_empty());
    let data: TimelineSnapshotData =
        serde_json::from_value(data).expect("timeline.snapshot data deserializes");
    // Freshly created store has no saved events yet — None last_saved.
    assert_eq!(data.event_id, "empty");
    assert_eq!(data.project_hash.len(), 64);
}
