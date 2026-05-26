//! Tests for `validate_command` (§1.4) — sixty-fifth production verb (meta arc).

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::validate_command::{compute_patch, data_envelope_from_post_state};
use verbreel_state::{
    MutateOutcome, Project, SchemaError, ValidateCommandArgs, ValidateCommandData,
    ValidateCommandError, ValidateCommandVerb, Verb, VerbError, VerbRegistry, default_fixtures,
    default_registry, validate_reconstructors,
};

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture → Project")
}

fn fixture_project_id() -> verbreel_types::ProjectId {
    empty_project().id
}

fn args(verb: &str, payload: Value) -> ValidateCommandArgs {
    ValidateCommandArgs {
        project_id: fixture_project_id(),
        verb: verb.to_string(),
        args: payload,
    }
}

fn marker_add_payload() -> Value {
    json!({
        "project_id": FIXTURE_PROJECT_ID,
        "time_tk": 0_i64,
        "label": "ok",
    })
}

// ------- arg deserialization -------

#[test]
fn args_deserialize_ok() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "verb": "marker.add",
        "args": marker_add_payload(),
    });
    let typed: ValidateCommandArgs = serde_json::from_value(raw).expect("ok");
    assert_eq!(typed.project_id.to_string(), FIXTURE_PROJECT_ID);
    assert_eq!(typed.verb, "marker.add");
    assert!(typed.args.is_object());
}

#[test]
fn args_missing_verb_fails_through_verb() {
    let prior = empty_project();
    let verb = ValidateCommandVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({ "project_id": FIXTURE_PROJECT_ID, "args": {} }),
        )
        .expect_err("missing verb → BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_missing_args_fails_through_verb() {
    let prior = empty_project();
    let verb = ValidateCommandVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({ "project_id": FIXTURE_PROJECT_ID, "verb": "marker.add" }),
        )
        .expect_err("missing args → BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_missing_project_id_fails_through_verb() {
    let prior = empty_project();
    let verb = ValidateCommandVerb;
    let err = verb
        .compute_patch(&prior, &json!({ "verb": "marker.add", "args": {} }))
        .expect_err("missing project_id → BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_wrong_type_for_verb_fails_through_verb() {
    let prior = empty_project();
    let verb = ValidateCommandVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "verb": 12345,
                "args": {},
            }),
        )
        .expect_err("non-string verb → BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

// ------- E_UNKNOWN_VERB -------

#[test]
fn unknown_verb_returns_unknown_verb_error() {
    let prior = empty_project();
    let err = compute_patch(&prior, &args("bogus.unknown", json!({})))
        .expect_err("bogus verb → UnknownVerb");
    assert!(
        matches!(err, ValidateCommandError::UnknownVerb { ref verb } if verb == "bogus.unknown")
    );
}

#[test]
fn empty_verb_string_returns_unknown_verb_error() {
    let prior = empty_project();
    let err = compute_patch(&prior, &args("", json!({}))).expect_err("empty verb → UnknownVerb");
    assert!(matches!(err, ValidateCommandError::UnknownVerb { ref verb } if verb.is_empty()));
}

#[test]
fn unknown_verb_maps_to_bad_args_at_verb_surface() {
    let prior = empty_project();
    let verb = ValidateCommandVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "verb": "no.such",
                "args": {},
            }),
        )
        .expect_err("unknown verb → BadArgs");
    let detail = match err {
        VerbError::BadArgs { detail } => detail,
        other => panic!("expected BadArgs, got {other:?}"),
    };
    assert!(detail.contains("E_UNKNOWN_VERB"));
    assert!(detail.contains("no.such"));
}

// ------- happy-valid paths -------

#[test]
fn marker_add_with_valid_args_is_valid() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(
        &prior,
        &args(
            "marker.add",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "time_tk": 0_i64,
                "label": "fixture",
            }),
        ),
    )
    .expect("ok");
    assert!(data.valid);
    assert!(data.errors.is_none());
}

#[test]
fn track_add_with_valid_args_is_valid() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(
        &prior,
        &args(
            "track.add",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "kind": "video",
            }),
        ),
    )
    .expect("ok");
    assert!(data.valid);
    assert!(data.errors.is_none());
}

#[test]
fn help_with_valid_args_is_valid() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(
        &prior,
        &args("help", json!({ "project_id": FIXTURE_PROJECT_ID })),
    )
    .expect("ok");
    assert!(data.valid);
}

#[test]
fn list_capabilities_with_valid_args_is_valid() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(
        &prior,
        &args(
            "list_capabilities",
            json!({ "project_id": FIXTURE_PROJECT_ID }),
        ),
    )
    .expect("ok");
    assert!(data.valid);
}

#[test]
fn self_reference_returns_valid() {
    // validate_command validating its own valid args — no infinite
    // recursion, since the verb resolves itself via the registry and
    // dispatches normally.
    let prior = empty_project();
    let inner_args = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "verb": "marker.add",
        "args": {
            "project_id": FIXTURE_PROJECT_ID,
            "time_tk": 0_i64,
            "label": "ok",
        },
    });
    let (_, _, data) = compute_patch(&prior, &args("validate_command", inner_args)).expect("ok");
    assert!(data.valid);
}

#[test]
fn clip_list_with_valid_args_is_valid_on_empty_project() {
    // clip.list against an empty project succeeds (returns empty list).
    let prior = empty_project();
    let (_, _, data) = compute_patch(
        &prior,
        &args("clip.list", json!({ "project_id": FIXTURE_PROJECT_ID })),
    )
    .expect("ok");
    assert!(data.valid);
}

// ------- happy-invalid paths -------

#[test]
fn clip_add_missing_track_is_invalid() {
    // clip.add requires `track` — missing it fails at serde deserialize.
    let prior = empty_project();
    let (_, _, data) = compute_patch(
        &prior,
        &args(
            "clip.add",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "asset_id": FIXTURE_PROJECT_ID,
                "track_position_tk": 0_i64,
            }),
        ),
    )
    .expect("ok");
    assert!(!data.valid);
    let errors = data.errors.expect("errors populated for Invalid");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].path, "");
    assert!(!errors[0].message.is_empty());
}

#[test]
fn clip_add_wrong_type_for_track_is_invalid() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(
        &prior,
        &args(
            "clip.add",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "asset_id": FIXTURE_PROJECT_ID,
                "track": 42,
                "track_position_tk": 0_i64,
            }),
        ),
    )
    .expect("ok");
    assert!(!data.valid);
    assert_eq!(data.errors.expect("errors").len(), 1);
}

#[test]
fn clip_add_wrong_shape_is_invalid() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args("clip.add", json!([1, 2, 3]))).expect("ok");
    assert!(!data.valid);
    assert_eq!(data.errors.expect("errors").len(), 1);
}

#[test]
fn audio_fade_curve_combo_is_invalid() {
    // `curve` + `curve_in` is the second-stage BadArgs path (caught by
    // validate_curve_combo, not by serde deserialize). Confirms the
    // verb reaches into post-deser arg validation too.
    let prior = empty_project();
    let (_, _, data) = compute_patch(
        &prior,
        &args(
            "audio.fade",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "clip": "0190b8d3-15e3-7000-bd00-000000000099",
                "fade_in_tk": 1_i64,
                "curve": "linear",
                "curve_in": "exp",
            }),
        ),
    )
    .expect("ok");
    assert!(
        !data.valid,
        "curve + curve_in must be rejected by validate_curve_combo as BadArgs"
    );
    let errors = data.errors.expect("errors");
    assert_eq!(errors.len(), 1);
    assert!(
        errors[0].message.contains("curve"),
        "message should mention the curve combo failure, got `{}`",
        errors[0].message
    );
}

// ------- serialization shape -------

#[test]
fn valid_serializes_to_exactly_one_field() {
    let data = ValidateCommandData::valid();
    let value = serde_json::to_value(&data).expect("serializes");
    let obj = value.as_object().expect("object");
    assert_eq!(
        obj.keys().count(),
        1,
        "got keys: {:?}",
        obj.keys().collect::<Vec<_>>()
    );
    assert_eq!(obj.get("valid"), Some(&Value::Bool(true)));
}

#[test]
fn invalid_serializes_to_two_fields() {
    let data = ValidateCommandData::invalid(vec![SchemaError {
        path: String::new(),
        message: "broken".to_string(),
    }]);
    let value = serde_json::to_value(&data).expect("serializes");
    let obj = value.as_object().expect("object");
    assert_eq!(
        obj.keys().count(),
        2,
        "got keys: {:?}",
        obj.keys().collect::<Vec<_>>()
    );
    assert_eq!(obj.get("valid"), Some(&Value::Bool(false)));
    let errors = obj
        .get("errors")
        .and_then(Value::as_array)
        .expect("errors is array");
    assert_eq!(errors.len(), 1);
}

#[test]
fn valid_roundtrips_through_serde() {
    let original = ValidateCommandData::valid();
    let value = serde_json::to_value(&original).expect("serialize");
    let restored: ValidateCommandData = serde_json::from_value(value).expect("deserialize back");
    assert_eq!(original, restored);
}

#[test]
fn invalid_roundtrips_through_serde() {
    let original = ValidateCommandData::invalid(vec![SchemaError {
        path: String::new(),
        message: "x".to_string(),
    }]);
    let value = serde_json::to_value(&original).expect("serialize");
    let restored: ValidateCommandData = serde_json::from_value(value).expect("deserialize back");
    assert_eq!(original, restored);
}

// ------- project-agnostic invariant -------

#[test]
fn verb_is_project_agnostic() {
    let prior_a = empty_project();
    let mut prior_b = empty_project();
    prior_b.name = "different-name".to_string();
    prior_b.duration_tk = verbreel_types::Tick::new(123_456);

    let inputs = args("marker.add", marker_add_payload());
    let (_, _, data_a) = compute_patch(&prior_a, &inputs).expect("a");
    let (_, _, data_b) = compute_patch(&prior_b, &inputs).expect("b");
    assert_eq!(data_a, data_b);
}

// ------- patch / warnings invariants -------

#[test]
fn patch_is_empty() {
    let prior = empty_project();
    let (patch, _, _) =
        compute_patch(&prior, &args("marker.add", marker_add_payload())).expect("ok");
    assert_eq!(patch, json!([]));
}

#[test]
fn warnings_are_empty() {
    let prior = empty_project();
    let (_, warnings, _) =
        compute_patch(&prior, &args("marker.add", marker_add_payload())).expect("ok");
    assert!(warnings.is_empty());
}

// ------- reconstructor / registry -------

#[test]
fn reconstruct_byte_identical_to_compute_patch() {
    let prior = empty_project();
    let inputs = args("marker.add", marker_add_payload());
    let (_, _, data) = compute_patch(&prior, &inputs).expect("ok");
    let envelope = data_envelope_from_post_state(&inputs, &prior).expect("envelope");
    let lhs = serde_json::to_vec(&data).expect("forward serializes");
    let rhs = serde_json::to_vec(&envelope).expect("rebuilt serializes");
    assert_eq!(lhs, rhs);
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "validate_command")
        .expect("default_fixtures includes validate_command");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(ValidateCommandVerb))
        .expect("register validate_command verb");

    let report = validate_reconstructors(&registry, &[fixture])
        .expect("validate_command reconstruct from fixture");
    assert_eq!(report.verbs_checked, vec!["validate_command"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn verb_trait_surface_lookup_via_default_registry() {
    let registry = default_registry();
    let verb = registry
        .get("validate_command")
        .expect("validate_command registered in default_registry");

    let prior = empty_project();
    let (patch, data, warnings) = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "verb": "marker.add",
                "args": marker_add_payload(),
            }),
        )
        .expect("default_registry invocation succeeds");

    assert!(patch.is_empty());
    assert!(warnings.is_empty());
    let typed: ValidateCommandData = serde_json::from_value(data).expect("envelope deserializes");
    assert!(typed.valid);
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
            "validate_command",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "verb": "marker.add",
                "args": marker_add_payload(),
            }),
            None,
        )
        .expect("validate_command should route");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("expected Applied outcome from validate_command");
    };
    assert!(warnings.is_empty());

    let data: ValidateCommandData = serde_json::from_value(data).expect("data deserializes");
    assert!(data.valid);
}

// ------- direct SchemaError construction (re-export sanity) -------

#[test]
fn schema_error_struct_re_exported_via_crate_root() {
    let err = SchemaError {
        path: String::new(),
        message: "x".to_string(),
    };
    let value = serde_json::to_value(&err).expect("serializes");
    let obj = value.as_object().expect("object");
    assert_eq!(obj.keys().count(), 2);
    assert!(obj.contains_key("path"));
    assert!(obj.contains_key("message"));
}
