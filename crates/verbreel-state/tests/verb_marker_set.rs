//! Tests for `marker.set` (§13.2) — first verb using serde `Option<Option<T>>`.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::marker_set::{compute_patch, data_envelope_from_post_state};
use verbreel_state::{
    DEFAULT_MARKER_COLOR, Marker, MarkerSetArgs, MarkerSetData, MarkerSetError, MarkerSetVerb,
    MutateOutcome, Project, RecordedEvent, VerbRegistry, validate_reconstructors,
};

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const SAMPLE_MARKER_ID: &str = "0190b8d3-15e3-7000-bd00-0000000cd001";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture → Project")
}

fn project_with_one_marker() -> Project {
    let mut prior = empty_project();
    let marker = json!({
        "id": SAMPLE_MARKER_ID,
        "time_tk": 1_000,
        "label": "Intro",
        "color": "#00ff00ff",
        "note": "old note",
    });
    prior
        .markers
        .push(serde_json::from_value(marker).expect("sample marker parses"));
    prior
}

fn patch_marker_value(patch: &Value) -> &Value {
    let arr = patch.as_array().expect("patch is an array");
    assert_eq!(arr.len(), 1, "marker.set emits single-op replace");
    let op = arr[0].as_object().expect("patch op is an object");
    assert_eq!(op.get("op").and_then(Value::as_str), Some("replace"));
    assert_eq!(op.get("path").and_then(Value::as_str), Some("/markers/0"));
    op.get("value").expect("replace op carries value")
}

#[test]
fn serde_distinguishes_absent_vs_null_color() {
    let base = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "marker": SAMPLE_MARKER_ID,
    });

    let absent: MarkerSetArgs = serde_json::from_value(base.clone()).expect("absent color parses");
    assert!(absent.color.is_none(), "omitted color is None");

    let null = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "marker": SAMPLE_MARKER_ID,
        "color": null,
    });
    let null: MarkerSetArgs = serde_json::from_value(null).expect("null color parses");
    assert_eq!(null.color, Some(None), "present null color is Some(None)");

    let value = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "marker": SAMPLE_MARKER_ID,
        "color": "#ff0000ff",
    });
    let value: MarkerSetArgs = serde_json::from_value(value).expect("value color parses");
    assert_eq!(
        value.color,
        Some(Some("#ff0000ff".to_string())),
        "present string color is Some(Some(..))"
    );
}

#[test]
fn compute_patch_set_label_succeeds() {
    let prior = project_with_one_marker();
    let args = MarkerSetArgs {
        project_id: FIXTURE_PROJECT_ID
            .parse()
            .expect("fixture project id parses"),
        marker: SAMPLE_MARKER_ID.to_string(),
        time_tk: None,
        label: Some("Renamed".to_string()),
        color: None,
        note: None,
    };

    let (patch, _warnings) = compute_patch(&prior, &args).expect("label update succeeds");
    let value = patch_marker_value(&patch);
    assert_eq!(value["label"], "Renamed");
    assert_eq!(value["id"], SAMPLE_MARKER_ID);
}

#[test]
fn compute_patch_set_time_tk_succeeds() {
    let prior = project_with_one_marker();
    let args = MarkerSetArgs {
        project_id: FIXTURE_PROJECT_ID
            .parse()
            .expect("fixture project id parses"),
        marker: SAMPLE_MARKER_ID.to_string(),
        time_tk: Some(9_999),
        label: None,
        color: None,
        note: None,
    };

    let (patch, _) = compute_patch(&prior, &args).expect("time_tk update succeeds");
    let value = patch_marker_value(&patch);
    assert_eq!(value["time_tk"], 9_999);
}

#[test]
fn compute_patch_set_color_value_succeeds() {
    let prior = project_with_one_marker();
    let args = MarkerSetArgs {
        project_id: FIXTURE_PROJECT_ID
            .parse()
            .expect("fixture project id parses"),
        marker: SAMPLE_MARKER_ID.to_string(),
        time_tk: None,
        label: None,
        color: Some(Some("#00ff00ff".to_string())),
        note: None,
    };

    let (patch, _) = compute_patch(&prior, &args).expect("explicit color succeeds");
    let value = patch_marker_value(&patch);
    assert_eq!(value["color"], "#00ff00ff");
}

#[test]
fn compute_patch_set_color_uppercase_normalized() {
    let prior = project_with_one_marker();
    let args = MarkerSetArgs {
        project_id: FIXTURE_PROJECT_ID
            .parse()
            .expect("fixture project id parses"),
        marker: SAMPLE_MARKER_ID.to_string(),
        time_tk: None,
        label: None,
        color: Some(Some("#FFAA00FF".to_string())),
        note: None,
    };

    let (patch, _) =
        compute_patch(&prior, &args).expect("uppercase color is validated and normalized");
    let value = patch_marker_value(&patch);
    assert_eq!(value["color"], "#ffaa00ff");
}

#[test]
fn compute_patch_color_null_reverts_to_default() {
    let prior = project_with_one_marker();
    let args = MarkerSetArgs {
        project_id: FIXTURE_PROJECT_ID
            .parse()
            .expect("fixture project id parses"),
        marker: SAMPLE_MARKER_ID.to_string(),
        time_tk: None,
        label: None,
        color: Some(None),
        note: None,
    };

    let (patch, _) = compute_patch(&prior, &args).expect("null color reverts to default");
    let value = patch_marker_value(&patch);
    assert_eq!(value["color"], DEFAULT_MARKER_COLOR);
}

#[test]
fn compute_patch_note_value_succeeds() {
    let prior = project_with_one_marker();
    let args = MarkerSetArgs {
        project_id: FIXTURE_PROJECT_ID
            .parse()
            .expect("fixture project id parses"),
        marker: SAMPLE_MARKER_ID.to_string(),
        time_tk: None,
        label: None,
        color: None,
        note: Some(Some("updated note".to_string())),
    };

    let (patch, _) = compute_patch(&prior, &args).expect("note value succeeds");
    let value = patch_marker_value(&patch);
    assert_eq!(value["note"], "updated note");
}

#[test]
fn compute_patch_note_null_removes_field() {
    let prior = project_with_one_marker();
    let args = MarkerSetArgs {
        project_id: FIXTURE_PROJECT_ID
            .parse()
            .expect("fixture project id parses"),
        marker: SAMPLE_MARKER_ID.to_string(),
        time_tk: None,
        label: None,
        color: None,
        note: Some(None),
    };

    let (patch, _) = compute_patch(&prior, &args).expect("null note removes field");
    let value = patch_marker_value(&patch);
    assert!(value.get("note").is_none(), "note None removes field");
}

#[test]
fn compute_patch_no_args_changes_emits_self_replace_patch() {
    let prior = project_with_one_marker();
    let marker_before = prior.markers[0].clone();
    let args = MarkerSetArgs {
        project_id: FIXTURE_PROJECT_ID
            .parse()
            .expect("fixture project id parses"),
        marker: SAMPLE_MARKER_ID.to_string(),
        time_tk: None,
        label: None,
        color: None,
        note: None,
    };

    let (patch, _) = compute_patch(&prior, &args).expect("no-op args still emit a replace patch");
    let value = patch_marker_value(&patch);
    let expected = serde_json::to_value(&marker_before).expect("marker serializes");
    assert_eq!(value, &expected);
}

#[test]
fn compute_patch_marker_id_invalid_uuid_errors() {
    let prior = project_with_one_marker();
    let args = MarkerSetArgs {
        project_id: FIXTURE_PROJECT_ID
            .parse()
            .expect("fixture project id parses"),
        marker: "not-a-uuid".to_string(),
        time_tk: None,
        label: None,
        color: None,
        note: None,
    };

    let err = compute_patch(&prior, &args).expect_err("invalid marker id must error");
    assert!(matches!(err, MarkerSetError::MarkerIdInvalid { .. }));
}

#[test]
fn compute_patch_marker_not_found_errors() {
    let prior = project_with_one_marker();
    let args = MarkerSetArgs {
        project_id: FIXTURE_PROJECT_ID
            .parse()
            .expect("fixture project id parses"),
        marker: "0190b8d3-15e3-7000-bd00-0000000cd999".to_string(),
        time_tk: None,
        label: None,
        color: None,
        note: None,
    };

    let err = compute_patch(&prior, &args).expect_err("missing marker id must error");
    assert!(matches!(err, MarkerSetError::MarkerNotFound { .. }));
}

#[test]
fn compute_patch_negative_time_errors() {
    let prior = project_with_one_marker();
    let args = MarkerSetArgs {
        project_id: FIXTURE_PROJECT_ID
            .parse()
            .expect("fixture project id parses"),
        marker: SAMPLE_MARKER_ID.to_string(),
        time_tk: Some(-1),
        label: None,
        color: None,
        note: None,
    };

    let err = compute_patch(&prior, &args).expect_err("negative time must reject");
    assert!(matches!(
        err,
        MarkerSetError::TimeBeforeProjectStart { time_tk: -1 }
    ));
}

#[test]
fn compute_patch_empty_label_errors() {
    let prior = project_with_one_marker();
    let err = compute_patch(
        &prior,
        &MarkerSetArgs {
            project_id: FIXTURE_PROJECT_ID
                .parse()
                .expect("fixture project id parses"),
            marker: SAMPLE_MARKER_ID.to_string(),
            time_tk: None,
            label: Some("".to_string()),
            color: None,
            note: None,
        },
    )
    .expect_err("empty label must reject");

    assert!(matches!(err, MarkerSetError::LabelEmpty));
}

#[test]
fn compute_patch_257_char_label_errors() {
    let prior = project_with_one_marker();
    let err = compute_patch(
        &prior,
        &MarkerSetArgs {
            project_id: FIXTURE_PROJECT_ID
                .parse()
                .expect("fixture project id parses"),
            marker: SAMPLE_MARKER_ID.to_string(),
            time_tk: None,
            label: Some("a".repeat(257)),
            color: None,
            note: None,
        },
    )
    .expect_err("257-char label must reject");

    match err {
        MarkerSetError::LabelTooLong { actual, max } => {
            assert_eq!(actual, 257);
            assert_eq!(max, 256);
        }
        other => panic!("expected LabelTooLong, got {other:?}"),
    }
}

#[test]
fn compute_patch_invalid_color_errors() {
    let prior = project_with_one_marker();
    let err = compute_patch(
        &prior,
        &MarkerSetArgs {
            project_id: FIXTURE_PROJECT_ID
                .parse()
                .expect("fixture project id parses"),
            marker: SAMPLE_MARKER_ID.to_string(),
            time_tk: None,
            label: None,
            color: Some(Some("rgb(0,0,0)".to_string())),
            note: None,
        },
    )
    .expect_err("invalid color must reject");

    assert!(matches!(err, MarkerSetError::ColorInvalid { .. }));
}

#[test]
fn compute_patch_4097_char_note_errors() {
    let prior = project_with_one_marker();
    let err = compute_patch(
        &prior,
        &MarkerSetArgs {
            project_id: FIXTURE_PROJECT_ID
                .parse()
                .expect("fixture project id parses"),
            marker: SAMPLE_MARKER_ID.to_string(),
            time_tk: None,
            label: None,
            color: None,
            note: Some(Some("x".repeat(4097))),
        },
    )
    .expect_err("4097-char note must reject");

    match err {
        MarkerSetError::NoteTooLong { actual, max } => {
            assert_eq!(actual, 4097);
            assert_eq!(max, 4096);
        }
        other => panic!("expected NoteTooLong, got {other:?}"),
    }
}

#[test]
fn data_envelope_returns_post_state_marker() {
    let prior = project_with_one_marker();
    let args = MarkerSetArgs {
        project_id: FIXTURE_PROJECT_ID
            .parse()
            .expect("fixture project id parses"),
        marker: SAMPLE_MARKER_ID.to_string(),
        time_tk: None,
        label: Some("Renamed".to_string()),
        color: None,
        note: None,
    };

    let (patch, _) = compute_patch(&prior, &args).expect("patch computes");
    let patch_typed: json_patch::Patch = serde_json::from_value(patch.clone())
        .expect("marker.set compute returns valid RFC 6902 patch");
    let post_state = prior.apply(&patch_typed).expect("patch applies");

    let from_post = data_envelope_from_post_state(&args, &post_state)
        .expect("post-state marker should be found");

    let patched_marker: Marker = serde_json::from_value(patch_marker_value(&patch).clone())
        .expect("patched value parses to Marker");
    assert_eq!(from_post.marker, patched_marker);
}

#[test]
fn reconstructor_round_trip() {
    let prior = project_with_one_marker();
    let args = MarkerSetArgs {
        project_id: FIXTURE_PROJECT_ID
            .parse()
            .expect("fixture project id parses"),
        marker: SAMPLE_MARKER_ID.to_string(),
        time_tk: Some(2_000),
        label: Some("Updated".to_string()),
        color: Some(Some("#00ff00ff".to_string())),
        note: Some(Some("note".to_string())),
    };

    let (patch, _warnings) = compute_patch(&prior, &args).expect("compute_patch ok");
    let patch_typed: json_patch::Patch =
        serde_json::from_value(patch.clone()).expect("patch parses to typed JSON patch");
    let post_state = prior.apply(&patch_typed).expect("patch applies");

    let expected_data =
        serde_json::to_value(data_envelope_from_post_state(&args, &post_state).expect("envelope"))
            .expect("envelope serializes");

    let recorded = RecordedEvent {
        verb: "marker.set".to_string(),
        args: serde_json::to_value(&args).expect("args serializes"),
        patch,
        warnings: vec![],
        post_state,
        expected_data,
    };

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(MarkerSetVerb))
        .expect("register marker.set verb");

    let report = validate_reconstructors(&registry, std::slice::from_ref(&recorded))
        .expect("reconstructor validation should pass");
    assert_eq!(report.verbs_checked, vec!["marker.set"]);
    assert_eq!(report.fixtures_run, 1);
}

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = verbreel_state::ProjectStore::create_with_registry(
        dir.path(),
        empty_project(),
        &verbreel_state::default_registry(),
        &verbreel_state::default_fixtures(),
    )
    .expect("create_with_registry succeeds");

    let add = store
        .mutate_via_verb(
            "marker.add",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "time_tk": 1_000,
                "label": "Intro",
            }),
            None,
        )
        .expect("seed marker fixture via marker.add should apply");
    assert!(matches!(
        add,
        verbreel_state::MutateOutcome::Applied {
            event_id: _,
            data: _,
            warnings: _
        }
    ));

    let prior_marker_id = store
        .project()
        .markers
        .first()
        .expect("seeded marker.add should create one marker")
        .id
        .to_string();

    let outcome = store
        .mutate_via_verb(
            "marker.set",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "marker": prior_marker_id,
                "label": "Intro Updated",
                "time_tk": 2_500,
            }),
            None,
        )
        .expect("mutate_via_verb should succeed");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("happy path must return Applied, got {outcome:?}");
    };

    let data: MarkerSetData = serde_json::from_value(data).expect("data is MarkerSetData");
    let marker = &store.project().markers[0];
    assert_eq!(marker.label, "Intro Updated");
    assert_eq!(marker.time_tk.get(), 2_500);
    assert_eq!(data.marker, *marker);
    assert!(warnings.is_empty());
}

#[cfg(feature = "native")]
#[test]
fn replay_returns_same_marker_state() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = verbreel_state::ProjectStore::create_with_registry(
        dir.path(),
        empty_project(),
        &verbreel_state::default_registry(),
        &verbreel_state::default_fixtures(),
    )
    .expect("create_with_registry succeeds");

    let add = store
        .mutate_via_verb(
            "marker.add",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "time_tk": 1_000,
                "label": "Intro",
            }),
            None,
        )
        .expect("seed marker fixture via marker.add should apply");
    assert!(matches!(
        add,
        verbreel_state::MutateOutcome::Applied {
            event_id: _,
            data: _,
            warnings: _
        }
    ));

    let marker_id = store
        .project()
        .markers
        .first()
        .expect("seeded marker.add should create one marker")
        .id
        .to_string();

    let args = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "marker": marker_id,
        "time_tk": 12_000,
        "note": "same note across replay",
    });

    let first = store
        .mutate_via_verb("marker.set", args.clone(), Some("idem-marker-set".into()))
        .expect("first call should apply");
    let MutateOutcome::Applied {
        data: first_data, ..
    } = first
    else {
        panic!("first call must be Applied, got {first:?}");
    };
    let first_data: MarkerSetData =
        serde_json::from_value(first_data).expect("first data deserializes");

    let second = store
        .mutate_via_verb("marker.set", args, Some("idem-marker-set".into()))
        .expect("replay path should be used");
    let MutateOutcome::Replayed {
        data: second_data,
        warnings,
        ..
    } = second
    else {
        panic!("second call must be Replayed, got {second:?}");
    };
    let second_data: MarkerSetData =
        serde_json::from_value(second_data).expect("second data deserializes");

    assert_eq!(first_data.marker, second_data.marker);
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], "W_REPLAY");
}
