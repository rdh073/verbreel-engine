//! Tests for `project.rename` (§2.9) — the fourth production verb.
//!
//! Covers `compute_patch` happy paths, unicode-safe boundary checks,
//! `data_envelope`, reconstructor round-trip, mutate-via-verb routing,
//! and the invariant that this verb never emits warnings.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::{
    MutateOutcome, Project, ProjectRenameArgs, ProjectRenameError, ProjectRenameVerb, ProjectStore,
    RecordedEvent, VerbRegistry, default_fixtures, default_registry, validate_reconstructors,
    verbs::project_rename::{PROJECT_NAME_MAX, compute_patch, data_envelope},
};
use verbreel_types::ProjectId;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture → Project")
}

fn fixture_project_id() -> ProjectId {
    empty_project().id
}

fn patch_name_value(patch: &Value) -> Value {
    let arr = patch.as_array().expect("patch is an array");
    assert_eq!(arr.len(), 1, "rename is a single replace op");
    let op = arr[0].as_object().expect("op is an object");
    assert_eq!(op.get("op").and_then(Value::as_str), Some("replace"));
    assert_eq!(op.get("path").and_then(Value::as_str), Some("/name"));
    op.get("value")
        .cloned()
        .expect("replace op carries a value")
}

#[test]
fn compute_patch_simple_rename_succeeds() {
    let prior = empty_project();
    let args = ProjectRenameArgs {
        project_id: fixture_project_id(),
        name: "New Name".to_string(),
    };

    let (patch, new_name, warnings) = compute_patch(&prior, &args).expect("happy-path rename");
    let patch_name = patch_name_value(&patch);
    assert_eq!(patch_name, "New Name");
    assert_eq!(new_name, args.name);
    assert!(warnings.is_empty(), "rename emits no warnings");
}

#[test]
fn compute_patch_minimum_length_one_char_accepted() {
    let prior = empty_project();
    let args = ProjectRenameArgs {
        project_id: fixture_project_id(),
        name: "A".to_string(),
    };

    let (patch, new_name, _warnings) =
        compute_patch(&prior, &args).expect("single-char name should be accepted");
    let patch_name = patch_name_value(&patch);
    assert_eq!(patch_name, "A");
    assert_eq!(new_name, "A");
}

#[test]
fn compute_patch_maximum_length_256_chars_accepted() {
    let prior = empty_project();
    let name = "a".repeat(256);
    let args = ProjectRenameArgs {
        project_id: fixture_project_id(),
        name: name.clone(),
    };

    let (patch, new_name, _warnings) =
        compute_patch(&prior, &args).expect("256-char name should be accepted");
    assert_eq!(patch_name_value(&patch), name);
    assert_eq!(new_name, name);
    assert_eq!(new_name.chars().count(), PROJECT_NAME_MAX);
}

#[test]
fn compute_patch_empty_name_errors() {
    let prior = empty_project();
    let err = compute_patch(
        &prior,
        &ProjectRenameArgs {
            project_id: fixture_project_id(),
            name: "".to_string(),
        },
    )
    .expect_err("empty name must fail");

    assert!(matches!(err, ProjectRenameError::NameEmpty));
}

#[test]
fn compute_patch_257_char_name_errors() {
    let prior = empty_project();
    let args = ProjectRenameArgs {
        project_id: fixture_project_id(),
        name: "a".repeat(257),
    };

    match compute_patch(&prior, &args).expect_err("257 chars must fail") {
        ProjectRenameError::NameTooLong { actual, max } => {
            assert_eq!(actual, 257);
            assert_eq!(max, 256);
        }
        other => panic!("expected NameTooLong, got {other:?}"),
    }
}

#[test]
fn compute_patch_unicode_chars_counted_correctly() {
    let prior = empty_project();
    let name = "界".repeat(100);
    assert_eq!(name.chars().count(), 100);
    assert!(name.len() > 256); // byte count clearly larger than char count.

    let args = ProjectRenameArgs {
        project_id: fixture_project_id(),
        name,
    };

    let (patch, new_name, _warnings) =
        compute_patch(&prior, &args).expect("unicode names should validate by char count");
    assert_eq!(patch_name_value(&patch), new_name);
}

#[test]
fn compute_patch_unicode_256_chars_accepted() {
    let prior = empty_project();
    let name = "🙂".repeat(256);
    assert_eq!(name.chars().count(), 256);

    let args = ProjectRenameArgs {
        project_id: fixture_project_id(),
        name: name.clone(),
    };

    let (patch, new_name, _warnings) =
        compute_patch(&prior, &args).expect("256 unicode chars should be accepted");
    assert_eq!(patch_name_value(&patch), name);
    assert_eq!(new_name, name);
}

#[test]
fn compute_patch_unicode_257_chars_errors() {
    let prior = empty_project();
    let args = ProjectRenameArgs {
        project_id: fixture_project_id(),
        name: "🙂".repeat(257),
    };

    match compute_patch(&prior, &args).expect_err("257 unicode chars should fail") {
        ProjectRenameError::NameTooLong { actual, max } => {
            assert_eq!(actual, 257);
            assert_eq!(max, 256);
        }
        other => panic!("expected NameTooLong, got {other:?}"),
    }
}

#[test]
fn data_envelope_returns_post_state_name() {
    let mut post_state = empty_project();
    post_state.name = "Renamed".to_string();

    let args = ProjectRenameArgs {
        project_id: fixture_project_id(),
        name: "Renamed".to_string(),
    };

    let env = data_envelope(&args, &post_state);
    assert_eq!(env.project_id, args.project_id);
    assert_eq!(env.name, post_state.name);
}

#[test]
fn reconstructor_round_trip() {
    let prior = empty_project();
    let args = ProjectRenameArgs {
        project_id: fixture_project_id(),
        name: "Renamed".to_string(),
    };

    let (patch, new_name, _warnings) = compute_patch(&prior, &args).expect("compute_patch ok");

    let mut post_state = prior.clone();
    post_state.name = new_name;

    let expected_envelope = data_envelope(&args, &post_state);
    let expected_data = serde_json::to_value(&expected_envelope).expect("envelope → Value");

    let recorded = RecordedEvent {
        verb: "project.rename".to_owned(),
        args: serde_json::to_value(&args).expect("args → Value"),
        patch,
        warnings: vec![],
        post_state,
        expected_data,
    };

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(ProjectRenameVerb))
        .expect("register ok");

    let report = validate_reconstructors(&registry, &[recorded])
        .expect("reconstructor round-trip must pass");
    assert_eq!(report.verbs_checked, vec!["project.rename"]);
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
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry clears the gate and writes project.json");

    let args = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "name": "Renamed"
    });

    let outcome = store
        .mutate_via_verb("project.rename", args, None)
        .expect("mutate_via_verb happy path");

    let MutateOutcome::Applied {
        event_id,
        data,
        warnings,
    } = outcome
    else {
        panic!("happy path must return Applied, got {outcome:?}");
    };

    assert_eq!(
        store.last_applied_event_id(),
        Some(event_id),
        "store tracks the just-applied event"
    );
    assert_eq!(store.project().name, "Renamed");
    assert_eq!(warnings, Vec::<Value>::new());

    let expected = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "name": "Renamed",
    });
    assert_eq!(
        data, expected,
        "data envelope is the verb's typed `{{ project_id, name }}` shape"
    );
}

#[test]
fn warnings_always_empty() {
    let prior = empty_project();
    let cases = [
        ProjectRenameArgs {
            project_id: fixture_project_id(),
            name: "A".to_string(),
        },
        ProjectRenameArgs {
            project_id: fixture_project_id(),
            name: "界面設計".to_string(),
        },
        ProjectRenameArgs {
            project_id: fixture_project_id(),
            name: "🙂".repeat(10),
        },
    ];

    for args in cases {
        let (_, _, warnings) =
            compute_patch(&prior, &args).expect("happy-path inputs must produce no warnings");
        assert!(
            warnings.is_empty(),
            "all rename variants must emit no warnings"
        );
    }
}
