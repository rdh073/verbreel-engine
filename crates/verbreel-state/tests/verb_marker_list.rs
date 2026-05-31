//! Tests for `marker.list` (§13.4) — first read-only verb.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_canon::project_hash;
use verbreel_state::verbs::marker_list::{compute_patch, data_envelope};
use verbreel_state::{
    Marker, MarkerListArgs, MarkerListData, MarkerListVerb, MutateOutcome, Project, RecordedEvent,
    VerbRegistry, validate_reconstructors,
};

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const MARKER_ID_0: &str = "0190b8d3-15e3-7000-bd00-0000000cd001";
const MARKER_ID_1: &str = "0190b8d3-15e3-7000-bd00-0000000cd002";
const MARKER_ID_2: &str = "0190b8d3-15e3-7000-bd00-0000000cd003";
const MARKER_ID_3: &str = "0190b8d3-15e3-7000-bd00-0000000cd004";
const MARKER_ID_TIE_1: &str = "01900000-0000-7000-8000-000000000001";
const MARKER_ID_TIE_2: &str = "01900000-0000-7000-8000-000000000002";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture → Project")
}

fn marker(id: &str, time_tk: i64, label: &str) -> Marker {
    serde_json::from_value(json!({
        "id": id,
        "time_tk": time_tk,
        "label": label,
        "color": "#ffaa00ff",
    }))
    .expect("marker fixture parses")
}

fn project_with_markers(entries: Vec<(&str, i64, &str)>) -> Project {
    let mut prior = empty_project();
    for (id, time_tk, label) in entries {
        prior.markers.push(marker(id, time_tk, label));
    }
    prior
}

fn project_hash_of(project: &Project) -> String {
    let value: Value = serde_json::to_value(project).expect("project serializes to value");
    project_hash(&value).expect("project_hash canonicalizes and hashes")
}

#[cfg(feature = "native")]
fn count_event_lines(verbreel_dir: &std::path::Path) -> usize {
    let events_path = verbreel_dir.join(".verbreel").join("events.jsonl");
    let bytes = std::fs::read(&events_path).expect("events.jsonl exists");
    bytes
        .split(|&b| b == b'\n')
        .filter(|line| !line.is_empty())
        .count()
}

#[test]
fn compute_patch_empty_markers_returns_empty_list() {
    let prior = project_with_markers(Vec::new());
    let args = MarkerListArgs {
        project_id: FIXTURE_PROJECT_ID
            .parse()
            .expect("fixture project id parses"),
    };

    let (patch, warnings) = compute_patch(&prior, &args);
    let data = data_envelope(&prior);

    assert_eq!(patch, json!([]));
    assert!(warnings.is_empty());
    assert!(data.markers.is_empty());
}

#[test]
fn compute_patch_single_marker_returns_singleton() {
    let prior = project_with_markers(vec![(MARKER_ID_0, 1_000, "Intro")]);
    let args = MarkerListArgs {
        project_id: FIXTURE_PROJECT_ID
            .parse()
            .expect("fixture project id parses"),
    };

    let data = data_envelope(&prior);
    let (patch, _warnings) = compute_patch(&prior, &args);

    assert_eq!(patch, json!([]));
    assert_eq!(data.markers.len(), 1);
    assert_eq!(data.markers[0], marker(MARKER_ID_0, 1_000, "Intro"));
}

#[test]
fn compute_patch_two_markers_sorted_by_time_tk_ascending() {
    let prior = project_with_markers(vec![
        (MARKER_ID_0, 1_000, "Later"),
        (MARKER_ID_1, 500, "Earlier"),
    ]);
    let args = MarkerListArgs {
        project_id: FIXTURE_PROJECT_ID
            .parse()
            .expect("fixture project id parses"),
    };

    let data = data_envelope(&prior);
    let (patch, _warnings) = compute_patch(&prior, &args);

    assert_eq!(patch, json!([]));
    assert_eq!(data.markers.len(), 2);
    assert_eq!(data.markers[0], marker(MARKER_ID_1, 500, "Earlier"));
    assert_eq!(data.markers[1], marker(MARKER_ID_0, 1_000, "Later"));
}

#[test]
fn compute_patch_tiebreaker_by_marker_id_when_same_time_tk() {
    let _ = MARKER_ID_TIE_1
        .parse::<verbreel_types::MarkerId>()
        .expect("marker id parses");
    let _ = MARKER_ID_TIE_2
        .parse::<verbreel_types::MarkerId>()
        .expect("marker id parses");

    let prior = project_with_markers(vec![
        (MARKER_ID_TIE_2, 1_000, "second-id"),
        (MARKER_ID_TIE_1, 1_000, "first-id"),
    ]);
    let args = MarkerListArgs {
        project_id: FIXTURE_PROJECT_ID
            .parse()
            .expect("fixture project id parses"),
    };

    let data = data_envelope(&prior);
    let (_patch, _warnings) = compute_patch(&prior, &args);

    let expected = vec![
        marker(MARKER_ID_TIE_1, 1_000, "first-id"),
        marker(MARKER_ID_TIE_2, 1_000, "second-id"),
    ];
    assert_eq!(data.markers, expected);
}

#[test]
fn compute_patch_patch_is_always_empty() {
    let prior = project_with_markers(vec![
        (MARKER_ID_0, 1_000, "Intro"),
        (MARKER_ID_1, 500, "Outro"),
    ]);
    let args = MarkerListArgs {
        project_id: FIXTURE_PROJECT_ID
            .parse()
            .expect("fixture project id parses"),
    };

    let (patch, _warnings) = compute_patch(&prior, &args);
    assert_eq!(patch, json!([]));

    let no_warnings = compute_patch(&prior, &args);
    assert_eq!(no_warnings.0, json!([]));
}

#[test]
fn compute_patch_warnings_always_empty() {
    let prior = project_with_markers(vec![(MARKER_ID_0, 1_000, "Intro")]);
    let args = MarkerListArgs {
        project_id: FIXTURE_PROJECT_ID
            .parse()
            .expect("fixture project id parses"),
    };

    let (_patch, warnings) = compute_patch(&prior, &args);
    assert!(warnings.is_empty());
}

#[test]
fn data_envelope_returns_sorted_markers() {
    let prior = project_with_markers(vec![
        (MARKER_ID_0, 2_000, "Late"),
        (MARKER_ID_1, 1_000, "Early"),
        (MARKER_ID_2, 1_000, "Tiebreaker-2"),
        (MARKER_ID_3, 1_000, "Tiebreaker-1"),
    ]);

    let envelope = data_envelope(&prior);
    let expected = vec![
        marker(MARKER_ID_1, 1_000, "Early"),
        marker(MARKER_ID_2, 1_000, "Tiebreaker-2"),
        marker(MARKER_ID_3, 1_000, "Tiebreaker-1"),
        marker(MARKER_ID_0, 2_000, "Late"),
    ];
    assert_eq!(envelope.markers, expected);
}

#[test]
fn reconstructor_round_trip() {
    let prior = project_with_markers(vec![
        (MARKER_ID_0, 1_000, "Intro"),
        (MARKER_ID_1, 2_000, "Outro"),
    ]);
    let args = MarkerListArgs {
        project_id: FIXTURE_PROJECT_ID
            .parse()
            .expect("fixture project id parses"),
    };

    let (patch_value, warnings) = compute_patch(&prior, &args);

    let post_state = prior.clone();
    let expected_data =
        serde_json::to_value(data_envelope(&post_state)).expect("marker list data serializes");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(MarkerListVerb))
        .expect("register marker.list verb");

    let recorded = RecordedEvent {
        verb: "marker.list".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch: patch_value,
        warnings,
        post_state,
        expected_data,
    };

    let report = validate_reconstructors(&registry, std::slice::from_ref(&recorded))
        .expect("reconstructor validation should pass");

    assert_eq!(report.verbs_checked, vec!["marker.list"]);
    assert_eq!(report.fixtures_run, 1);
}

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        empty_project(),
        &verbreel_state::default_registry(),
        &verbreel_state::default_fixtures(),
    )
    .expect("create_with_registry succeeds");

    store
        .mutate_via_verb(
            "marker.add",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "time_tk": 1_000,
                "label": "Second",
            }),
            None,
        )
        .expect("seed second marker");
    store
        .mutate_via_verb(
            "marker.add",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "time_tk": 500,
                "label": "First",
            }),
            None,
        )
        .expect("seed first marker");

    let before = count_event_lines(dir.path());
    let outcome = store
        .mutate_via_verb(
            "marker.list",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
            }),
            None,
        )
        .expect("marker.list should route");

    // marker.list is read-only (empty patch) — the §0.6/§0.8 fast-path
    // routes it to `NoOp` with no event line.
    let MutateOutcome::NoOp { data, warnings, .. } = outcome else {
        panic!("marker.list should be NoOp (read-only, empty patch), got {outcome:?}");
    };

    assert!(warnings.is_empty());
    let data: MarkerListData = serde_json::from_value(data).expect("marker.list data deserializes");
    assert_eq!(data.markers.len(), 2);
    assert_eq!(data.markers[0].time_tk.get(), 500);
    assert_eq!(data.markers[1].time_tk.get(), 1_000);

    let after = count_event_lines(dir.path());
    assert_eq!(
        after, before,
        "marker.list is read-only and must not write an event line"
    );
}

#[cfg(feature = "native")]
#[test]
fn read_only_no_state_mutation() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        empty_project(),
        &verbreel_state::default_registry(),
        &verbreel_state::default_fixtures(),
    )
    .expect("create_with_registry succeeds");

    store
        .mutate_via_verb(
            "marker.add",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "time_tk": 1_000,
                "label": "Intro",
            }),
            None,
        )
        .expect("seed marker fixture");

    let before = project_hash_of(store.project());
    let outcome = store
        .mutate_via_verb(
            "marker.list",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
            }),
            None,
        )
        .expect("marker.list should apply");

    let MutateOutcome::NoOp { data, warnings, .. } = outcome else {
        panic!("marker.list should be NoOp (read-only), got {outcome:?}");
    };

    let _data: MarkerListData =
        serde_json::from_value(data).expect("marker.list data deserializes");
    assert!(warnings.is_empty());
    assert_eq!(
        before,
        project_hash_of(store.project()),
        "marker.list must not mutate project state"
    );
}

#[cfg(feature = "native")]
#[test]
fn keyed_read_verb_is_noop_not_replay() {
    // A read verb (empty patch) writes no event, so an `idempotency_key`
    // on it never lands in the §0.8 index — a same-key second call is
    // another fresh `NoOp`, not a `Replayed` with `W_REPLAY`. The
    // contract callers depend on (same args → same data) still holds
    // because the verb recomputes its data from current state.
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        empty_project(),
        &verbreel_state::default_registry(),
        &verbreel_state::default_fixtures(),
    )
    .expect("create_with_registry succeeds");

    store
        .mutate_via_verb(
            "marker.add",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "time_tk": 1_000,
                "label": "Intro",
            }),
            None,
        )
        .expect("seed first marker");

    store
        .mutate_via_verb(
            "marker.add",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "time_tk": 500,
                "label": "Outro",
            }),
            None,
        )
        .expect("seed second marker");

    let args = json!({
        "project_id": FIXTURE_PROJECT_ID,
    });

    let first = store
        .mutate_via_verb(
            "marker.list",
            args.clone(),
            Some("marker-list-replay".into()),
        )
        .expect("first marker.list call");
    let MutateOutcome::NoOp {
        data: first_data, ..
    } = first
    else {
        panic!("first call should be NoOp (read verb), got {first:?}");
    };
    let first_data: MarkerListData =
        serde_json::from_value(first_data).expect("marker.list data deserializes");

    let second = store
        .mutate_via_verb("marker.list", args, Some("marker-list-replay".into()))
        .expect("second call");
    let MutateOutcome::NoOp {
        data: second_data,
        warnings,
        ..
    } = second
    else {
        panic!("second call should also be NoOp (read verbs do not replay), got {second:?}");
    };

    let second_data: MarkerListData =
        serde_json::from_value(second_data).expect("marker.list second data deserializes");
    assert_eq!(first_data, second_data);
    // No event was ever written, so there is nothing to replay — no
    // `W_REPLAY` warning is appended.
    assert!(
        warnings.iter().all(|w| w["code"] != "W_REPLAY"),
        "a read verb's keyed retry must not produce W_REPLAY"
    );
}
