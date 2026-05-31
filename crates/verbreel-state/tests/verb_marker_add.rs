//! Tests for `marker.add` (§13.1) — first ID-minting verb and first `add` op.
//!
//! Covers validation semantics (`time_tk`, label length/charset checks, color
//! normalization, note cap), RFC 6902 patch shape, envelope reconstruction
//! through the patch payload, reconstructor round-trip, and idempotent replay
//! behavior.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::{
    DEFAULT_MARKER_COLOR, Marker, MarkerAddArgs, MarkerAddData, MarkerAddError, MarkerAddVerb,
    MutateOutcome, Project, RecordedEvent, VerbRegistry, validate_reconstructors,
    verbs::marker_add::{compute_patch, data_envelope_from_patch},
};
use verbreel_types::MarkerId;

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture → Project")
}

fn patch_marker_value(patch: &Value) -> &Value {
    let arr = patch.as_array().expect("patch is an array");
    assert_eq!(arr.len(), 1, "marker.add emits single-op add");
    let op = arr[0].as_object().expect("patch op is an object");
    assert_eq!(op.get("op").and_then(Value::as_str), Some("add"));
    assert_eq!(op.get("path").and_then(Value::as_str), Some("/markers/-"));
    op.get("value").expect("add op carries value")
}

fn marker_id_from_patch(patch: &Value) -> String {
    patch_marker_value(patch)
        .get("id")
        .and_then(Value::as_str)
        .expect("marker value has id")
        .to_string()
}

#[test]
fn compute_patch_minimal_marker_succeeds() {
    let prior = empty_project();
    let (patch, warnings) = compute_patch(
        &prior,
        &MarkerAddArgs {
            project_id: FIXTURE_PROJECT_ID
                .parse()
                .expect("fixture project id parses"),
            time_tk: 0,
            label: "Hello".to_string(),
            color: None,
            note: None,
        },
    )
    .expect("happy-path minimal args");

    let value = patch_marker_value(&patch);
    assert_eq!(value["id"].as_str().unwrap().len(), 36);
    assert_eq!(value["time_tk"], 0);
    assert_eq!(value["label"], "Hello");
    assert_eq!(value["color"], DEFAULT_MARKER_COLOR);
    assert!(
        value.get("note").is_none(),
        "note is omitted unless provided"
    );
    assert!(warnings.is_empty());
}

#[test]
fn compute_patch_with_color_succeeds() {
    let prior = empty_project();
    let args = MarkerAddArgs {
        project_id: FIXTURE_PROJECT_ID
            .parse()
            .expect("fixture project id parses"),
        time_tk: 123,
        label: "Hello".to_string(),
        color: Some("#00ff00ff".to_string()),
        note: None,
    };

    let (patch, _) = compute_patch(&prior, &args).expect("explicit color accepted");
    let value = patch_marker_value(&patch);
    assert_eq!(value["color"], "#00ff00ff");
}

#[test]
fn compute_patch_with_note_succeeds() {
    let prior = empty_project();
    let args = MarkerAddArgs {
        project_id: FIXTURE_PROJECT_ID
            .parse()
            .expect("fixture project id parses"),
        time_tk: 123,
        label: "Hello".to_string(),
        color: None,
        note: Some("A note about this point".to_string()),
    };

    let (patch, _) = compute_patch(&prior, &args).expect("explicit note accepted");
    let value = patch_marker_value(&patch);
    assert_eq!(value["note"], "A note about this point");
}

#[test]
fn compute_patch_with_all_optionals_succeeds() {
    let prior = empty_project();
    let args = MarkerAddArgs {
        project_id: FIXTURE_PROJECT_ID
            .parse()
            .expect("fixture project id parses"),
        time_tk: 123,
        label: "Hello".to_string(),
        color: Some("#00ff00ff".to_string()),
        note: Some("A note about this point".to_string()),
    };

    let (patch, _) = compute_patch(&prior, &args).expect("color + note accepted");
    let value = patch_marker_value(&patch);
    assert_eq!(value["color"], "#00ff00ff");
    assert_eq!(value["note"], "A note about this point");
}

#[test]
fn compute_patch_default_color_filled() {
    let prior = empty_project();
    let (patch, _) = compute_patch(
        &prior,
        &MarkerAddArgs {
            project_id: FIXTURE_PROJECT_ID
                .parse()
                .expect("fixture project id parses"),
            time_tk: 0,
            label: "Intro".to_string(),
            color: None,
            note: None,
        },
    )
    .expect("default-color path");

    assert_eq!(patch_marker_value(&patch)["color"], DEFAULT_MARKER_COLOR);
}

#[test]
fn compute_patch_uppercase_color_normalized() {
    let prior = empty_project();
    let (patch, _) = compute_patch(
        &prior,
        &MarkerAddArgs {
            project_id: FIXTURE_PROJECT_ID
                .parse()
                .expect("fixture project id parses"),
            time_tk: 0,
            label: "Intro".to_string(),
            color: Some("#FFAA00FF".to_string()),
            note: None,
        },
    )
    .expect("upper-case color accepted and normalized");

    assert_eq!(patch_marker_value(&patch)["color"], "#ffaa00ff");
}

#[test]
fn compute_patch_negative_time_errors() {
    let prior = empty_project();
    let err = compute_patch(
        &prior,
        &MarkerAddArgs {
            project_id: FIXTURE_PROJECT_ID
                .parse()
                .expect("fixture project id parses"),
            time_tk: -1,
            label: "Hello".to_string(),
            color: None,
            note: None,
        },
    )
    .expect_err("negative time must reject");

    assert!(matches!(
        err,
        MarkerAddError::TimeBeforeProjectStart { time_tk: -1 }
    ));
}

#[test]
fn compute_patch_empty_label_errors() {
    let prior = empty_project();
    let err = compute_patch(
        &prior,
        &MarkerAddArgs {
            project_id: FIXTURE_PROJECT_ID
                .parse()
                .expect("fixture project id parses"),
            time_tk: 0,
            label: "".to_string(),
            color: None,
            note: None,
        },
    )
    .expect_err("empty label must reject");

    assert!(matches!(err, MarkerAddError::LabelEmpty));
}

#[test]
fn compute_patch_257_char_label_errors() {
    let prior = empty_project();
    let err = compute_patch(
        &prior,
        &MarkerAddArgs {
            project_id: FIXTURE_PROJECT_ID
                .parse()
                .expect("fixture project id parses"),
            time_tk: 0,
            label: "a".repeat(257),
            color: None,
            note: None,
        },
    )
    .expect_err("257-char ASCII label must reject");

    match err {
        MarkerAddError::LabelTooLong { actual, max } => {
            assert_eq!(actual, 257);
            assert_eq!(max, 256);
        }
        other => panic!("expected LabelTooLong, got {other:?}"),
    }
}

#[test]
fn compute_patch_unicode_label_257_chars_errors() {
    let prior = empty_project();
    let label = "界".repeat(257);
    assert_eq!(label.chars().count(), 257);

    let err = compute_patch(
        &prior,
        &MarkerAddArgs {
            project_id: FIXTURE_PROJECT_ID
                .parse()
                .expect("fixture project id parses"),
            time_tk: 0,
            label,
            color: None,
            note: None,
        },
    )
    .expect_err("257-char unicode label must reject");

    match err {
        MarkerAddError::LabelTooLong { actual, max } => {
            assert_eq!(actual, 257);
            assert_eq!(max, 256);
        }
        other => panic!("expected LabelTooLong, got {other:?}"),
    }
}

#[test]
fn compute_patch_4097_char_note_errors() {
    let prior = empty_project();
    let err = compute_patch(
        &prior,
        &MarkerAddArgs {
            project_id: FIXTURE_PROJECT_ID
                .parse()
                .expect("fixture project id parses"),
            time_tk: 0,
            label: "Hello".to_string(),
            color: None,
            note: Some("x".repeat(4097)),
        },
    )
    .expect_err("4097-char note must reject");

    match err {
        MarkerAddError::NoteTooLong { actual, max } => {
            assert_eq!(actual, 4097);
            assert_eq!(max, 4096);
        }
        other => panic!("expected NoteTooLong, got {other:?}"),
    }
}

#[test]
fn compute_patch_invalid_color_errors() {
    let prior = empty_project();
    let err = compute_patch(
        &prior,
        &MarkerAddArgs {
            project_id: FIXTURE_PROJECT_ID
                .parse()
                .expect("fixture project id parses"),
            time_tk: 0,
            label: "Hello".to_string(),
            color: Some("rgb(0,0,0)".to_string()),
            note: None,
        },
    )
    .expect_err("invalid color format must reject");

    assert!(matches!(err, MarkerAddError::ColorInvalid { .. }));
}

#[test]
fn compute_patch_no_warnings_emitted() {
    let prior = empty_project();
    let cases = [
        MarkerAddArgs {
            project_id: FIXTURE_PROJECT_ID
                .parse()
                .expect("fixture project id parses"),
            time_tk: 0,
            label: "A".to_string(),
            color: None,
            note: None,
        },
        MarkerAddArgs {
            project_id: FIXTURE_PROJECT_ID
                .parse()
                .expect("fixture project id parses"),
            time_tk: 10,
            label: "🙂".to_string(),
            color: Some("#00ff00ff".to_string()),
            note: Some("note".to_string()),
        },
    ];

    for args in cases {
        let (_, warnings) =
            compute_patch(&prior, &args).expect("valid args should emit no warnings");
        assert!(warnings.is_empty(), "marker.add emits no warnings");
    }
}

#[test]
fn compute_patch_mints_uuidv7_id() {
    let prior = empty_project();
    let args = MarkerAddArgs {
        project_id: FIXTURE_PROJECT_ID
            .parse()
            .expect("fixture project id parses"),
        time_tk: 0,
        label: "Hello".to_string(),
        color: None,
        note: None,
    };

    let (patch, _) = compute_patch(&prior, &args).expect("happy-path compute_patch");
    let id: MarkerId = marker_id_from_patch(&patch)
        .parse()
        .expect("patched marker.id must be a valid UUIDv7");
    assert_ne!(id.to_string().len(), 0);
}

#[test]
fn compute_patch_id_is_unique_per_call() {
    let prior = empty_project();
    let args = MarkerAddArgs {
        project_id: FIXTURE_PROJECT_ID
            .parse()
            .expect("fixture project id parses"),
        time_tk: 0,
        label: "Hello".to_string(),
        color: None,
        note: None,
    };

    let (first, _) = compute_patch(&prior, &args).expect("first compute_patch call");
    let (second, _) = compute_patch(&prior, &args).expect("second compute_patch call");
    assert_ne!(marker_id_from_patch(&first), marker_id_from_patch(&second));
}

#[test]
fn data_envelope_from_patch_round_trip() {
    let prior = empty_project();
    let args = MarkerAddArgs {
        project_id: FIXTURE_PROJECT_ID
            .parse()
            .expect("fixture project id parses"),
        time_tk: 42,
        label: "Hello".to_string(),
        color: Some("#00ff00ff".to_string()),
        note: Some("note".to_string()),
    };

    let (patch, _) = compute_patch(&prior, &args).expect("compute_patch for reconstructor test");
    let env = data_envelope_from_patch(&patch).expect("envelope must parse from patch");
    let marker_from_patch: Marker = serde_json::from_value(patch_marker_value(&patch).clone())
        .expect("marker payload parses as Marker");

    assert_eq!(env.marker, marker_from_patch);
}

#[test]
fn reconstructor_round_trip() {
    let prior = empty_project();
    let args = MarkerAddArgs {
        project_id: FIXTURE_PROJECT_ID
            .parse()
            .expect("fixture project id parses"),
        time_tk: 12,
        label: "Hello".to_string(),
        color: Some("#00ff00ff".to_string()),
        note: None,
    };

    let (patch, _) = compute_patch(&prior, &args).expect("compute_patch ok");
    let patch_typed: json_patch::Patch = serde_json::from_value(patch.clone())
        .expect("patch value should parse to json_patch::Patch");
    let post_state = prior.apply(&patch_typed).expect("fixture patch applies");

    let expected_data =
        serde_json::to_value(data_envelope_from_patch(&patch).expect("envelope from patch"))
            .expect("envelope serializes");

    let recorded = RecordedEvent {
        verb: "marker.add".to_string(),
        args: serde_json::to_value(&args).expect("args serializes"),
        patch,
        warnings: vec![],
        post_state,
        expected_data,
    };

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(MarkerAddVerb))
        .expect("register marker.add fixture verb");

    let report = validate_reconstructors(&registry, std::slice::from_ref(&recorded))
        .expect("reconstructor validation must pass");
    assert_eq!(report.verbs_checked, vec!["marker.add"]);
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
    .expect("create_with_registry clears the gate and writes project.json");

    let args = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "time_tk": 1_000,
        "label": "Intro",
    });

    let outcome = store
        .mutate_via_verb("marker.add", args, None)
        .expect("mutate_via_verb happy path");

    let MutateOutcome::Applied {
        event_id: _event_id,
        data,
        warnings,
        ..
    } = outcome
    else {
        panic!("happy path must return Applied, got {outcome:?}");
    };

    let data: MarkerAddData = serde_json::from_value(data).expect("data is MarkerAddData");
    assert_eq!(store.project().markers.len(), 1);
    assert_eq!(store.project().markers[0].label, "Intro");
    assert_eq!(store.project().markers[0].time_tk.get(), 1_000);
    assert_eq!(
        store.project().markers[0].color.as_str(),
        DEFAULT_MARKER_COLOR
    );
    assert!(store.project().markers[0].note.is_none());
    assert_eq!(data.marker, store.project().markers[0]);
    assert!(warnings.is_empty());
}

#[cfg(feature = "native")]
#[test]
fn replay_via_idempotency_returns_same_marker_id() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        empty_project(),
        &verbreel_state::default_registry(),
        &verbreel_state::default_fixtures(),
    )
    .expect("create_with_registry clears the gate and writes project.json");

    let args = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "time_tk": 2_000,
        "label": "Intro",
    });

    let first = store
        .mutate_via_verb("marker.add", args.clone(), Some("idem-marker".into()))
        .expect("first insert");
    let MutateOutcome::Applied {
        data: first_data, ..
    } = first
    else {
        panic!("first call must be Applied, got {first:?}");
    };
    let first_data: MarkerAddData = serde_json::from_value(first_data).expect("first marker data");

    let second = store
        .mutate_via_verb("marker.add", args, Some("idem-marker".into()))
        .expect("replay call");
    let MutateOutcome::Replayed {
        data: second_data,
        warnings,
        event_id: _event_id,
        ..
    } = second
    else {
        panic!("replay path must return Replayed, got {second:?}");
    };
    let second_data: MarkerAddData =
        serde_json::from_value(second_data).expect("replay marker data is MarkerAddData");

    assert_eq!(first_data.marker.id, second_data.marker.id);
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], "W_REPLAY");
}
