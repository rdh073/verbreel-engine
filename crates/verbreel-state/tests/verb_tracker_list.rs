//! Tests for `tracker.list` (§18.4) — sixty-ninth production verb.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::tracker::Tracker;
use verbreel_state::verbs::tracker_list::{compute_patch, data_envelope_from_post_state};
use verbreel_state::{
    Project, Track, TrackerCacheStatus, TrackerEntry, TrackerListArgs, TrackerListData,
    TrackerListVerb, Verb, VerbError, VerbRegistry, default_fixtures, default_registry,
    validate_reconstructors,
};

#[cfg(feature = "native")]
use verbreel_state::{MutateOutcome, ProjectStore};

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const TRACKER_ID_1: &str = "01900000-0000-7000-8000-0000000bb001";
const TRACKER_ID_2: &str = "01900000-0000-7000-8000-0000000bb002";
const TRACKER_ID_3: &str = "01900000-0000-7000-8000-0000000bb003";
const SOURCE_CLIP_ID_LIVE: &str = "01900000-0000-7000-8000-0000000cc001";
const SOURCE_CLIP_ID_ORPHAN: &str = "01900000-0000-7000-8000-0000000cc999";
const TRACK_VIDEO_LIVE: &str = "01900000-0000-7000-8000-0000000aa001";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture → Project")
}

fn args() -> TrackerListArgs {
    TrackerListArgs {
        project_id: empty_project().id,
    }
}

fn tracker_from_json(value: Value) -> Tracker {
    serde_json::from_value(value).expect("tracker fixture parses")
}

fn live_clip_video_track(track_id: &str, clip_id: &str) -> Track {
    serde_json::from_value(json!({
        "id": track_id,
        "kind": "video",
        "name": "Tracker Source",
        "locked": false,
        "clips": [{
            "id": clip_id,
            "name": "Tracker Source Clip",
            "asset_id": "00000000-0000-0000-0000-000000000000",
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 240_000,
            "locked": false,
        }],
    }))
    .expect("video track fixture parses")
}

#[test]
fn args_deserialize_ok() {
    let raw = json!({ "project_id": FIXTURE_PROJECT_ID });
    let typed: TrackerListArgs =
        serde_json::from_value(raw).expect("project_id is the only required arg field");
    assert_eq!(typed.project_id.to_string(), FIXTURE_PROJECT_ID);
}

#[test]
fn args_missing_project_id_fails_through_verb() {
    let prior = empty_project();
    let verb = TrackerListVerb;

    let err = verb
        .compute_patch(&prior, &json!({}))
        .expect_err("missing project_id should map to BadArgs");

    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_wrong_type_fails_through_verb() {
    let prior = empty_project();
    let verb = TrackerListVerb;

    let err = verb
        .compute_patch(&prior, &json!({ "project_id": 12345 }))
        .expect_err("non-string project_id should map to BadArgs");

    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn empty_trackers_returns_empty_list() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    assert!(data.trackers.is_empty());
}

#[test]
fn single_tracker_returns_singleton_with_all_fields() {
    let mut prior = empty_project();
    prior.trackers.push(tracker_from_json(json!({
        "tracker_id": TRACKER_ID_1,
        "source_clip_id": SOURCE_CLIP_ID_ORPHAN,
        "algorithm": "median_flow",
        "sample_count": 480,
        "applied_to_clip_ids": [SOURCE_CLIP_ID_ORPHAN],
        "cache_hash": "",
        "cache_path": "",
    })));

    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    assert_eq!(data.trackers.len(), 1);
    let entry = &data.trackers[0];
    assert_eq!(entry.tracker_id, TRACKER_ID_1);
    assert_eq!(entry.source_clip_id, SOURCE_CLIP_ID_ORPHAN);
    assert_eq!(entry.algorithm, "median_flow");
    assert_eq!(entry.sample_count, 480);
    assert_eq!(
        entry.applied_to_clip_ids,
        vec![SOURCE_CLIP_ID_ORPHAN.to_string()]
    );
    assert_eq!(entry.cache_hash, "");
    assert_eq!(entry.cache_path, "");
    assert_eq!(entry.cache_status, TrackerCacheStatus::Unrun);
    assert!(!entry.source_clip_exists);
    assert!(entry.last_run_at.is_none());
}

#[test]
fn multiple_trackers_preserve_insertion_order() {
    let mut prior = empty_project();
    prior.trackers.push(tracker_from_json(json!({
        "tracker_id": TRACKER_ID_3,
        "source_clip_id": "",
        "algorithm": "kanade",
    })));
    prior.trackers.push(tracker_from_json(json!({
        "tracker_id": TRACKER_ID_1,
        "source_clip_id": "",
        "algorithm": "median_flow",
    })));
    prior.trackers.push(tracker_from_json(json!({
        "tracker_id": TRACKER_ID_2,
        "source_clip_id": "",
        "algorithm": "csrt",
    })));

    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    assert_eq!(data.trackers.len(), 3);
    assert_eq!(data.trackers[0].tracker_id, TRACKER_ID_3);
    assert_eq!(data.trackers[1].tracker_id, TRACKER_ID_1);
    assert_eq!(data.trackers[2].tracker_id, TRACKER_ID_2);
}

#[test]
fn source_clip_exists_true_when_clip_present_on_any_track() {
    let mut prior = empty_project();
    prior
        .tracks
        .push(live_clip_video_track(TRACK_VIDEO_LIVE, SOURCE_CLIP_ID_LIVE));
    prior.trackers.push(tracker_from_json(json!({
        "tracker_id": TRACKER_ID_1,
        "source_clip_id": SOURCE_CLIP_ID_LIVE,
        "algorithm": "median_flow",
    })));

    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    assert!(data.trackers[0].source_clip_exists);
}

#[test]
fn source_clip_exists_false_when_clip_not_found() {
    let mut prior = empty_project();
    prior.trackers.push(tracker_from_json(json!({
        "tracker_id": TRACKER_ID_1,
        "source_clip_id": SOURCE_CLIP_ID_ORPHAN,
        "algorithm": "median_flow",
    })));

    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    assert!(!data.trackers[0].source_clip_exists);
}

#[test]
fn sample_count_defaults_to_minus_one_when_absent() {
    let mut prior = empty_project();
    prior.trackers.push(tracker_from_json(json!({
        "tracker_id": TRACKER_ID_1,
        "source_clip_id": "",
        "algorithm": "median_flow",
    })));

    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    assert_eq!(data.trackers[0].sample_count, -1);
}

#[test]
fn cache_hash_defaults_to_empty_when_absent() {
    let mut prior = empty_project();
    prior.trackers.push(tracker_from_json(json!({
        "tracker_id": TRACKER_ID_1,
        "source_clip_id": "",
        "algorithm": "median_flow",
    })));

    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    assert_eq!(data.trackers[0].cache_hash, "");
}

#[test]
fn cache_path_defaults_to_empty_when_absent() {
    let mut prior = empty_project();
    prior.trackers.push(tracker_from_json(json!({
        "tracker_id": TRACKER_ID_1,
        "source_clip_id": "",
        "algorithm": "median_flow",
    })));

    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    assert_eq!(data.trackers[0].cache_path, "");
}

#[test]
fn cache_status_always_unrun_even_with_populated_cache_hash() {
    // v1 floor: cache_status is always Unrun. This test locks the
    // deferral even when the tracker's Map carries a non-empty
    // cache_hash (which can only happen via direct project.json edit
    // since no tracker.run exists in v1).
    let mut prior = empty_project();
    prior.trackers.push(tracker_from_json(json!({
        "tracker_id": TRACKER_ID_1,
        "source_clip_id": "",
        "algorithm": "median_flow",
        "cache_hash": "53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658",
        "cache_path": "cache/trackers/53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.json",
    })));

    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    assert_eq!(data.trackers[0].cache_status, TrackerCacheStatus::Unrun);
}

#[test]
fn last_run_at_absent_from_serialization_when_none() {
    let mut prior = empty_project();
    prior.trackers.push(tracker_from_json(json!({
        "tracker_id": TRACKER_ID_1,
        "source_clip_id": "",
        "algorithm": "median_flow",
    })));

    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    let value: Value = serde_json::to_value(&data).expect("envelope → Value");
    let entry = &value["trackers"][0];
    assert!(
        entry.get("last_run_at").is_none(),
        "last_run_at must be omitted when None, got: {entry}"
    );
}

#[test]
fn last_run_at_present_when_set() {
    let mut prior = empty_project();
    prior.trackers.push(tracker_from_json(json!({
        "tracker_id": TRACKER_ID_1,
        "source_clip_id": "",
        "algorithm": "median_flow",
        "last_run_at": "2026-05-24T12:00:00Z",
    })));

    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    assert_eq!(
        data.trackers[0].last_run_at.as_deref(),
        Some("2026-05-24T12:00:00Z")
    );

    let value: Value = serde_json::to_value(&data).expect("envelope → Value");
    assert_eq!(
        value["trackers"][0]["last_run_at"],
        json!("2026-05-24T12:00:00Z")
    );
}

#[test]
fn data_envelope_has_exactly_one_field_named_trackers() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    let value: Value = serde_json::to_value(&data).expect("envelope → Value");
    let obj = value.as_object().expect("envelope is a JSON object");

    assert_eq!(obj.keys().count(), 1);
    let keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    assert_eq!(keys, vec!["trackers"]);
}

#[test]
fn tracker_entry_serializes_with_nine_required_fields_when_no_last_run_at() {
    let mut prior = empty_project();
    prior.trackers.push(tracker_from_json(json!({
        "tracker_id": TRACKER_ID_1,
        "source_clip_id": "",
        "algorithm": "median_flow",
    })));

    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    let value: Value = serde_json::to_value(&data).expect("envelope → Value");
    let entry = value["trackers"][0]
        .as_object()
        .expect("entry is JSON object");

    let mut keys: Vec<&str> = entry.keys().map(String::as_str).collect();
    keys.sort_unstable();
    let mut expected = vec![
        "applied_to_clip_ids",
        "algorithm",
        "cache_hash",
        "cache_path",
        "cache_status",
        "sample_count",
        "source_clip_exists",
        "source_clip_id",
        "tracker_id",
    ];
    expected.sort_unstable();
    assert_eq!(keys, expected);
    assert_eq!(entry.keys().count(), 9);
}

#[test]
fn tracker_entry_serializes_with_ten_fields_when_last_run_at_set() {
    let mut prior = empty_project();
    prior.trackers.push(tracker_from_json(json!({
        "tracker_id": TRACKER_ID_1,
        "source_clip_id": "",
        "algorithm": "median_flow",
        "last_run_at": "2026-05-24T12:00:00Z",
    })));

    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    let value: Value = serde_json::to_value(&data).expect("envelope → Value");
    let entry = value["trackers"][0]
        .as_object()
        .expect("entry is JSON object");

    assert_eq!(entry.keys().count(), 10);
    assert!(entry.contains_key("last_run_at"));
}

#[test]
fn tracker_cache_status_fresh_serializes_lowercase() {
    let value = serde_json::to_value(TrackerCacheStatus::Fresh).expect("Fresh → Value");
    assert_eq!(value, json!("fresh"));
}

#[test]
fn tracker_cache_status_dangling_serializes_lowercase() {
    let value = serde_json::to_value(TrackerCacheStatus::Dangling).expect("Dangling → Value");
    assert_eq!(value, json!("dangling"));
}

#[test]
fn tracker_cache_status_unrun_serializes_lowercase() {
    let value = serde_json::to_value(TrackerCacheStatus::Unrun).expect("Unrun → Value");
    assert_eq!(value, json!("unrun"));
}

#[test]
fn patch_is_empty() {
    let prior = empty_project();
    let (patch, _, _) = compute_patch(&prior, &args()).expect("happy path");
    assert_eq!(patch, json!([]));
}

#[test]
fn warnings_are_empty() {
    let prior = empty_project();
    let (_, warnings, _) = compute_patch(&prior, &args()).expect("happy path");
    assert!(warnings.is_empty());
}

#[test]
fn reconstruct_byte_identical_to_compute_patch() {
    let mut prior = empty_project();
    prior.trackers.push(tracker_from_json(json!({
        "tracker_id": TRACKER_ID_1,
        "source_clip_id": SOURCE_CLIP_ID_ORPHAN,
        "algorithm": "median_flow",
        "sample_count": 100,
        "applied_to_clip_ids": [SOURCE_CLIP_ID_ORPHAN],
    })));

    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");

    let envelope = data_envelope_from_post_state(&args(), &prior).expect("envelope rebuilds");

    let lhs = serde_json::to_vec(&data).expect("forward data serializes");
    let rhs = serde_json::to_vec(&envelope).expect("reconstructed envelope serializes");
    assert_eq!(lhs, rhs);
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "tracker.list")
        .expect("default_fixtures includes tracker.list");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(TrackerListVerb))
        .expect("register tracker.list verb");

    let report = validate_reconstructors(&registry, &[fixture])
        .expect("tracker.list reconstruct from fixture");
    assert_eq!(report.verbs_checked, vec!["tracker.list"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn verb_trait_surface_lookup_via_default_registry() {
    let registry = default_registry();
    let verb = registry
        .get("tracker.list")
        .expect("tracker.list registered in default_registry");

    let prior = empty_project();
    let (patch, data, warnings) = verb
        .compute_patch(&prior, &json!({ "project_id": FIXTURE_PROJECT_ID }))
        .expect("default_registry invocation succeeds");

    assert!(patch.is_empty());
    assert!(warnings.is_empty());
    let typed: TrackerListData =
        serde_json::from_value(data).expect("envelope deserializes to TrackerListData");
    assert!(typed.trackers.is_empty());
}

#[test]
fn tracker_entry_round_trip() {
    let original = TrackerEntry {
        tracker_id: TRACKER_ID_1.to_string(),
        source_clip_id: SOURCE_CLIP_ID_LIVE.to_string(),
        source_clip_exists: true,
        algorithm: "median_flow".to_string(),
        sample_count: 240,
        applied_to_clip_ids: vec![SOURCE_CLIP_ID_LIVE.to_string()],
        cache_hash: "abc".to_string(),
        cache_path: "cache/trackers/abc.json".to_string(),
        cache_status: TrackerCacheStatus::Unrun,
        last_run_at: Some("2026-05-24T00:00:00Z".to_string()),
    };
    let value = serde_json::to_value(&original).expect("TrackerEntry → Value");
    let parsed: TrackerEntry = serde_json::from_value(value).expect("Value → TrackerEntry");
    assert_eq!(parsed, original);
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
            "tracker.list",
            json!({"project_id": FIXTURE_PROJECT_ID}),
            None,
        )
        .expect("tracker.list should route");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("expected Applied outcome from tracker.list");
    };
    assert!(warnings.is_empty());

    let data: TrackerListData =
        serde_json::from_value(data).expect("tracker.list data deserializes");
    assert!(data.trackers.is_empty());
}
