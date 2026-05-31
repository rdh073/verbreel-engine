//! Tests for `marker.remove` (§13.3) — first batch + soft + empty-noop verb.

use std::sync::Arc;

use serde_json::json;
use verbreel_state::{
    DEFAULT_MARKER_COLOR, MARKERS_MAX_BATCH, Marker, MarkerRemoveArgs, MarkerRemoveData,
    MarkerRemoveError, MarkerRemoveVerb, MutateOutcome, Project, RecordedEvent, VerbRegistry,
    validate_reconstructors,
};

#[cfg(feature = "native")]
use verbreel_state::{LifecycleError, ProjectStore, VerbError};

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const MARKER_ID_0: &str = "0190b8d3-15e3-7000-bd00-0000000cd001";
const MARKER_ID_1: &str = "0190b8d3-15e3-7000-bd00-0000000cd002";
const MARKER_ID_2: &str = "0190b8d3-15e3-7000-bd00-0000000cd003";
const MARKER_ID_3: &str = "0190b8d3-15e3-7000-bd00-0000000cd004";
const MARKER_ID_4: &str = "0190b8d3-15e3-7000-bd00-0000000cd005";
const MISSING_MARKER_ID: &str = "0190b8d3-15e3-7000-bd00-0000000cd999";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture → Project")
}

fn marker(id: &str, time_tk: i64, label: &str) -> Marker {
    serde_json::from_value(json!({
        "id": id,
        "time_tk": time_tk,
        "label": label,
        "color": DEFAULT_MARKER_COLOR,
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

fn count_event_lines(verbreel_dir: &std::path::Path) -> usize {
    let events_path = verbreel_dir.join(".verbreel").join("events.jsonl");
    let bytes = std::fs::read(&events_path).expect("events.jsonl exists");
    bytes
        .split(|&b| b == b'\n')
        .filter(|line| !line.is_empty())
        .count()
}

#[test]
fn compute_patch_empty_array_is_noop() {
    let prior = project_with_markers(vec![(MARKER_ID_0, 1_000, "Intro")]);
    let args = MarkerRemoveArgs {
        project_id: FIXTURE_PROJECT_ID
            .parse()
            .expect("fixture project id parses"),
        markers: Vec::new(),
        soft: false,
    };

    let (patch, warnings, data) =
        verbreel_state::verbs::marker_remove::compute_patch(&prior, &args)
            .expect("empty array no-op");

    let patch_ops = patch.as_array().expect("patch is array");
    assert_eq!(
        patch_ops.len(),
        0,
        "empty array must emit no patch operations"
    );
    assert!(warnings.is_empty(), "empty array no warnings");
    assert!(
        data.removed_marker_ids.is_empty(),
        "empty array removed list empty"
    );
    assert!(
        data.missing_marker_ids.is_empty(),
        "empty array missing list empty"
    );
}

#[test]
fn compute_patch_single_marker_succeeds() {
    let prior = project_with_markers(vec![(MARKER_ID_0, 1_000, "Intro")]);
    let args = MarkerRemoveArgs {
        project_id: FIXTURE_PROJECT_ID
            .parse()
            .expect("fixture project id parses"),
        markers: vec![MARKER_ID_0.to_string()],
        soft: false,
    };

    let (patch, warnings, data) =
        verbreel_state::verbs::marker_remove::compute_patch(&prior, &args)
            .expect("one marker remove");

    let arr = patch.as_array().expect("patch is array");
    assert_eq!(arr.len(), 1, "single remove emits one op");
    assert_eq!(arr[0]["op"], "remove");
    assert_eq!(arr[0]["path"], "/markers/0");
    assert!(warnings.is_empty());
    assert_eq!(data.removed_marker_ids, vec![MARKER_ID_0.to_string()]);
    assert!(data.missing_marker_ids.is_empty());
}

#[test]
fn compute_patch_multiple_markers_removed_descending_order() {
    let prior = project_with_markers(vec![
        (MARKER_ID_0, 1_000, "A"),
        (MARKER_ID_1, 2_000, "B"),
        (MARKER_ID_2, 3_000, "C"),
        (MARKER_ID_3, 4_000, "D"),
        (MARKER_ID_4, 5_000, "E"),
    ]);
    let args = MarkerRemoveArgs {
        project_id: FIXTURE_PROJECT_ID
            .parse()
            .expect("fixture project id parses"),
        markers: vec![
            MARKER_ID_0.to_string(),
            MARKER_ID_2.to_string(),
            MARKER_ID_4.to_string(),
        ],
        soft: false,
    };

    let (patch, _warnings, data) =
        verbreel_state::verbs::marker_remove::compute_patch(&prior, &args)
            .expect("multiple remove");
    let arr = patch.as_array().expect("patch is array");
    assert_eq!(arr.len(), 3, "three removes");
    assert_eq!(arr[0]["path"], "/markers/4");
    assert_eq!(arr[1]["path"], "/markers/2");
    assert_eq!(arr[2]["path"], "/markers/0");
    assert_eq!(
        data.removed_marker_ids,
        vec![
            MARKER_ID_0.to_string(),
            MARKER_ID_2.to_string(),
            MARKER_ID_4.to_string()
        ]
    );
}

#[test]
fn compute_patch_strict_missing_errors() {
    let prior = project_with_markers(vec![(MARKER_ID_0, 1_000, "Intro")]);
    let args = MarkerRemoveArgs {
        project_id: FIXTURE_PROJECT_ID
            .parse()
            .expect("fixture project id parses"),
        markers: vec![MISSING_MARKER_ID.to_string()],
        soft: false,
    };

    let err = verbreel_state::verbs::marker_remove::compute_patch(&prior, &args)
        .expect_err("missing marker is a strict error");

    match err {
        MarkerRemoveError::MarkerNotFound {
            marker_id,
            failed_index,
        } => {
            assert_eq!(marker_id, MISSING_MARKER_ID);
            assert_eq!(failed_index, 0);
        }
        _ => panic!("expected MarkerNotFound"),
    }
}

#[test]
fn compute_patch_soft_missing_emits_w_noop() {
    let prior = empty_project();
    let args = MarkerRemoveArgs {
        project_id: FIXTURE_PROJECT_ID
            .parse()
            .expect("fixture project id parses"),
        markers: vec![MISSING_MARKER_ID.to_string()],
        soft: true,
    };

    let (_patch, warnings, data) =
        verbreel_state::verbs::marker_remove::compute_patch(&prior, &args).expect("soft missing");

    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], "W_NOOP");
    assert_eq!(warnings[0]["message"], "marker not found (soft skip)");
    assert_eq!(warnings[0]["details"]["marker_id"], MISSING_MARKER_ID);
    assert_eq!(warnings[0]["details"]["input_index"], 0);
    assert_eq!(data.removed_marker_ids, Vec::<String>::new());
    assert_eq!(data.missing_marker_ids, vec![MISSING_MARKER_ID.to_string()]);
}

#[test]
fn compute_patch_soft_mixed_present_and_missing() {
    let prior = project_with_markers(vec![(MARKER_ID_0, 1_000, "A"), (MARKER_ID_1, 2_000, "B")]);
    let args = MarkerRemoveArgs {
        project_id: FIXTURE_PROJECT_ID
            .parse()
            .expect("fixture project id parses"),
        markers: vec![
            MARKER_ID_0.to_string(),
            MISSING_MARKER_ID.to_string(),
            MARKER_ID_1.to_string(),
        ],
        soft: true,
    };

    let (patch, warnings, data) =
        verbreel_state::verbs::marker_remove::compute_patch(&prior, &args).expect("mixed list");
    let arr = patch.as_array().expect("patch is array");
    assert_eq!(arr.len(), 2, "present markers are removed");
    assert_eq!(arr[0]["path"], "/markers/1");
    assert_eq!(arr[1]["path"], "/markers/0");
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["details"]["marker_id"], MISSING_MARKER_ID);
    assert_eq!(
        data.removed_marker_ids,
        vec![MARKER_ID_0.to_string(), MARKER_ID_1.to_string()]
    );
    assert_eq!(data.missing_marker_ids, vec![MISSING_MARKER_ID.to_string()]);
}

#[test]
fn compute_patch_batch_over_1000_errors() {
    let ids = vec![MISSING_MARKER_ID.to_string(); MARKERS_MAX_BATCH + 1];
    let prior = empty_project();
    let args = MarkerRemoveArgs {
        project_id: FIXTURE_PROJECT_ID
            .parse()
            .expect("fixture project id parses"),
        markers: ids,
        soft: true,
    };

    let err = verbreel_state::verbs::marker_remove::compute_patch(&prior, &args)
        .expect_err("batch > 1000 must fail");

    assert!(matches!(
        err,
        MarkerRemoveError::BatchTooLarge {
            actual,
            max
        } if actual == MARKERS_MAX_BATCH + 1 && max == MARKERS_MAX_BATCH
    ));
}

#[test]
fn compute_patch_marker_id_invalid_uuid_errors() {
    let prior = empty_project();
    let args = MarkerRemoveArgs {
        project_id: FIXTURE_PROJECT_ID
            .parse()
            .expect("fixture project id parses"),
        markers: vec!["not-a-uuid".to_string()],
        soft: false,
    };

    let err = verbreel_state::verbs::marker_remove::compute_patch(&prior, &args)
        .expect_err("invalid uuid must fail");

    assert!(matches!(
        err,
        MarkerRemoveError::MarkerIdInvalid {
            marker_index: 0,
            ..
        }
    ));
}

#[test]
fn compute_patch_batch_at_1000_boundary_succeeds() {
    let prior = empty_project();
    let ids = vec![MARKER_ID_0.to_string(); MARKERS_MAX_BATCH];
    let args = MarkerRemoveArgs {
        project_id: FIXTURE_PROJECT_ID
            .parse()
            .expect("fixture project id parses"),
        markers: ids,
        soft: true,
    };

    let (patch, warnings, data) =
        verbreel_state::verbs::marker_remove::compute_patch(&prior, &args)
            .expect("boundary succeeds");
    let arr = patch.as_array().expect("patch is array");
    assert_eq!(arr.len(), 0, "all missing ids yields empty patch");
    assert_eq!(warnings.len(), MARKERS_MAX_BATCH);
    assert_eq!(data.removed_marker_ids.len(), 0);
    assert_eq!(data.missing_marker_ids.len(), MARKERS_MAX_BATCH);
}

#[test]
fn compute_patch_duplicate_ids_in_args() {
    let prior = project_with_markers(vec![(MARKER_ID_0, 1_000, "Intro")]);
    let args = MarkerRemoveArgs {
        project_id: FIXTURE_PROJECT_ID
            .parse()
            .expect("fixture project id parses"),
        markers: vec![MARKER_ID_0.to_string(), MARKER_ID_0.to_string()],
        soft: true,
    };

    let (patch, warnings, data) =
        verbreel_state::verbs::marker_remove::compute_patch(&prior, &args).expect("duplicate args");

    let arr = patch.as_array().expect("patch is array");
    assert_eq!(arr.len(), 1, "duplicate present id removes once");
    assert_eq!(arr[0]["path"], "/markers/0");
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["details"]["input_index"], 1);
    assert_eq!(data.removed_marker_ids, vec![MARKER_ID_0.to_string()]);
    assert_eq!(data.missing_marker_ids, vec![MARKER_ID_0.to_string()]);
}

#[test]
fn compute_patch_duplicate_ids_in_args_strict_fails_after_removal() {
    let prior = project_with_markers(vec![(MARKER_ID_0, 1_000, "Intro")]);
    let args = MarkerRemoveArgs {
        project_id: FIXTURE_PROJECT_ID
            .parse()
            .expect("fixture project id parses"),
        markers: vec![MARKER_ID_0.to_string(), MARKER_ID_0.to_string()],
        soft: false,
    };

    let err = verbreel_state::verbs::marker_remove::compute_patch(&prior, &args)
        .expect_err("strict duplicate should fail after first removal");

    assert!(matches!(
        err,
        MarkerRemoveError::MarkerNotFound {
            marker_id,
            failed_index: 1,
        } if marker_id == MARKER_ID_0
    ));
}

#[test]
fn data_envelope_recovers_from_args_warnings_alone() {
    let args = MarkerRemoveArgs {
        project_id: FIXTURE_PROJECT_ID
            .parse()
            .expect("fixture project id parses"),
        markers: vec![
            MARKER_ID_0.to_string(),
            MISSING_MARKER_ID.to_string(),
            MARKER_ID_1.to_string(),
            MISSING_MARKER_ID.to_string(),
            MARKER_ID_2.to_string(),
        ],
        soft: true,
    };

    let warnings = vec![
        json!({
            "code": "W_NOOP",
            "details": {
                "marker_id": MISSING_MARKER_ID,
                "input_index": 1
            }
        }),
        json!({
            "code": "W_NOOP",
            "details": {
                "marker_id": MISSING_MARKER_ID,
                "input_index": 3
            }
        }),
    ];

    let data =
        verbreel_state::verbs::marker_remove::data_envelope_from_args_warnings(&args, &warnings);

    assert_eq!(
        data.removed_marker_ids,
        vec![
            MARKER_ID_0.to_string(),
            MARKER_ID_1.to_string(),
            MARKER_ID_2.to_string()
        ]
    );
    assert_eq!(
        data.missing_marker_ids,
        vec![MISSING_MARKER_ID.to_string(), MISSING_MARKER_ID.to_string()]
    );
}

#[test]
fn data_envelope_ignores_non_w_noop_warnings() {
    let args = MarkerRemoveArgs {
        project_id: FIXTURE_PROJECT_ID
            .parse()
            .expect("fixture project id parses"),
        markers: vec![MARKER_ID_0.to_string(), MARKER_ID_1.to_string()],
        soft: true,
    };

    let warnings = vec![json!({
        "code": "W_OTHER",
        "details": {
            "marker_id": MISSING_MARKER_ID,
        }
    })];

    let data =
        verbreel_state::verbs::marker_remove::data_envelope_from_args_warnings(&args, &warnings);

    assert_eq!(
        data.removed_marker_ids,
        vec![MARKER_ID_0.to_string(), MARKER_ID_1.to_string()]
    );
    assert!(data.missing_marker_ids.is_empty());
}

#[test]
fn reconstructor_round_trip() {
    let prior = project_with_markers(vec![(MARKER_ID_0, 1_000, "Intro")]);
    let args = MarkerRemoveArgs {
        project_id: FIXTURE_PROJECT_ID
            .parse()
            .expect("fixture project id parses"),
        markers: vec![MARKER_ID_0.to_string()],
        soft: false,
    };

    let (patch, _warnings, data) =
        verbreel_state::verbs::marker_remove::compute_patch(&prior, &args).expect("compute patch");
    let patch_typed: json_patch::Patch = serde_json::from_value(patch.clone())
        .expect("marker.remove compute returns valid RFC 6902 patch");
    let post_state = prior.apply(&patch_typed).expect("patch applies");

    let expected_data = serde_json::to_value(data).expect("data serializes");
    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(MarkerRemoveVerb))
        .expect("register marker.remove verb");

    let recorded = RecordedEvent {
        verb: "marker.remove".to_string(),
        args: serde_json::to_value(&args).expect("args serializes"),
        patch,
        warnings: Vec::new(),
        post_state,
        expected_data,
    };

    let report = validate_reconstructors(&registry, std::slice::from_ref(&recorded))
        .expect("reconstructor validation should pass");

    assert_eq!(report.verbs_checked, vec!["marker.remove"]);
    assert_eq!(report.fixtures_run, 1);
}

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb_strict_succeeds() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        empty_project(),
        &verbreel_state::default_registry(),
        &verbreel_state::default_fixtures(),
    )
    .expect("create_with_registry succeeds");

    let seed = store
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
    assert!(
        matches!(seed, MutateOutcome::Applied { .. }),
        "marker.add fixture should apply"
    );

    let marker_id = store
        .project()
        .markers
        .first()
        .expect("seeded marker exists")
        .id
        .to_string();

    let outcome = store
        .mutate_via_verb(
            "marker.remove",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "markers": [marker_id.clone()],
                "soft": false,
            }),
            None,
        )
        .expect("strict remove succeeds");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("strict remove should be Applied, got {outcome:?}");
    };

    let data: MarkerRemoveData = serde_json::from_value(data).expect("remove data serializes");
    assert!(warnings.is_empty(), "strict remove emits no warnings");
    assert_eq!(data.removed_marker_ids, vec![marker_id]);
    assert_eq!(
        store.project().markers.len(),
        0,
        "marker removed from project"
    );
}

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb_soft_with_missing() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        empty_project(),
        &verbreel_state::default_registry(),
        &verbreel_state::default_fixtures(),
    )
    .expect("create_with_registry succeeds");

    let _seed = store
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

    let marker_id = store
        .project()
        .markers
        .first()
        .expect("seeded marker exists")
        .id
        .to_string();

    let outcome = store
        .mutate_via_verb(
            "marker.remove",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "markers": [marker_id.clone(), MISSING_MARKER_ID.to_string()],
                "soft": true,
            }),
            None,
        )
        .expect("soft remove with one missing");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("soft remove should be Applied, got {outcome:?}");
    };

    let data: MarkerRemoveData = serde_json::from_value(data).expect("remove data serializes");
    assert_eq!(data.removed_marker_ids, vec![marker_id]);
    assert_eq!(data.missing_marker_ids, vec![MISSING_MARKER_ID.to_string()]);
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], "W_NOOP");
    assert_eq!(warnings[0]["details"]["marker_id"], MISSING_MARKER_ID);
}

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb_strict_with_missing_fails() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        empty_project(),
        &verbreel_state::default_registry(),
        &verbreel_state::default_fixtures(),
    )
    .expect("create_with_registry succeeds");

    let outcome = store
        .mutate_via_verb(
            "marker.remove",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "markers": [MISSING_MARKER_ID.to_string()],
            }),
            None,
        )
        .expect_err("strict missing must fail");

    match outcome {
        LifecycleError::VerbExecutionFailed {
            verb_id,
            source: VerbError::BadArgs { .. },
        } => assert_eq!(verb_id, "marker.remove"),
        other => panic!("expected VerbExecutionFailed/BadArgs, got {other:?}"),
    }
}

#[cfg(feature = "native")]
#[test]
fn replay_returns_same_data_envelope_and_w_replay() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        empty_project(),
        &verbreel_state::default_registry(),
        &verbreel_state::default_fixtures(),
    )
    .expect("create_with_registry succeeds");

    let _seed = store
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

    let marker_id = store
        .project()
        .markers
        .first()
        .expect("seeded marker exists")
        .id
        .to_string();

    let args = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "markers": [marker_id.clone(), MISSING_MARKER_ID.to_string()],
        "soft": true,
    });

    let first = store
        .mutate_via_verb("marker.remove", args.clone(), Some("k1".into()))
        .expect("first keyed call");

    let MutateOutcome::Applied {
        data: first_data,
        warnings: first_warnings,
        ..
    } = first
    else {
        panic!("first call should be Applied, got {first:?}");
    };
    let first_data: MarkerRemoveData =
        serde_json::from_value(first_data).expect("first data deserializes");
    assert_eq!(first_data.removed_marker_ids, vec![marker_id.clone()]);
    assert_eq!(
        first_data.missing_marker_ids,
        vec![MISSING_MARKER_ID.to_string()]
    );
    assert_eq!(first_warnings.len(), 1);
    assert_eq!(first_warnings[0]["code"], "W_NOOP");

    let second = store
        .mutate_via_verb("marker.remove", args, Some("k1".into()))
        .expect("replay call");

    let MutateOutcome::Replayed {
        data: second_data,
        warnings: second_warnings,
        ..
    } = second
    else {
        panic!("second call should be Replayed, got {second:?}");
    };

    let second_data: MarkerRemoveData =
        serde_json::from_value(second_data).expect("second data deserializes");
    assert_eq!(first_data, second_data, "replay must return identical data");
    assert_eq!(second_warnings.len(), 2);
    assert_eq!(second_warnings[0]["code"], "W_NOOP");
    assert_eq!(second_warnings[1]["code"], "W_REPLAY");
}

#[cfg(feature = "native")]
#[test]
fn compute_patch_empty_array_noop_does_not_write_event_if_kernel_skips() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        empty_project(),
        &verbreel_state::default_registry(),
        &verbreel_state::default_fixtures(),
    )
    .expect("create_with_registry succeeds");

    let before = count_event_lines(dir.path());

    let args = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "markers": Vec::<String>::new(),
        "soft": false,
    });

    let outcome = store
        .mutate_via_verb("marker.remove", args, Some("no-op-array".into()))
        .expect("empty array can still route");
    // marker.remove over an empty `markers` array computes an empty
    // patch — the §0.6/§0.8 fast-path routes it to `NoOp` and writes no
    // event line.
    let MutateOutcome::NoOp { data, warnings, .. } = outcome else {
        panic!("empty-array no-op must be NoOp (no event), got {outcome:?}");
    };
    let data: MarkerRemoveData = serde_json::from_value(data).expect("data deserializes");
    assert_eq!(data.removed_marker_ids, Vec::<String>::new());
    assert_eq!(data.missing_marker_ids, Vec::<String>::new());
    assert!(warnings.is_empty());

    drop(store);
    let after = count_event_lines(dir.path());

    assert_eq!(
        after, before,
        "an empty-patch no-op must not write an event line"
    );
}
