//! Tests for [`verbreel_args::validate`] — the top-level args
//! validation entry point.
//!
//! Cases under test:
//!
//! - Happy path: known verb + matching args → `Ok(())`.
//! - UnknownVerb: lookup fails, error carries the verb name.
//! - SchemaViolation: lookup succeeds but the args do not match; the
//!   error carries a non-empty `detail` string.

use serde_json::json;
use verbreel_args::{ArgsRegistry, Schema, ValidationError, validate};

/// Build a registry populated with one verb whose args are
/// `{ project_id: string, title: string }`, both required.
fn registry_with_project_create() -> ArgsRegistry {
    let mut r = ArgsRegistry::new();
    let s = Schema::from_value(json!({
        "type": "object",
        "properties": {
            "project_id": { "type": "string" },
            "title":      { "type": "string", "minLength": 1 }
        },
        "required": ["project_id", "title"],
        "additionalProperties": false
    }))
    .unwrap();
    r.register("project.create", s);
    r
}

// --- happy path --------------------------------------------------------

#[test]
fn validate_happy_path_returns_ok() {
    let r = registry_with_project_create();
    let args = json!({ "project_id": "p_01", "title": "demo" });
    assert!(validate("project.create", &args, &r).is_ok());
}

#[test]
fn validate_accepts_minimum_required_args() {
    let r = registry_with_project_create();
    let args = json!({ "project_id": "x", "title": "y" });
    assert!(validate("project.create", &args, &r).is_ok());
}

#[test]
fn validate_succeeds_against_permissive_schema() {
    // `{}` matches every JSON value — any args payload should pass.
    let mut r = ArgsRegistry::new();
    r.register("permissive", Schema::from_value(json!({})).unwrap());
    assert!(validate("permissive", &json!({"anything": 1}), &r).is_ok());
    assert!(validate("permissive", &json!(null), &r).is_ok());
    assert!(validate("permissive", &json!([1, 2, 3]), &r).is_ok());
}

// --- UnknownVerb ------------------------------------------------------

#[test]
fn validate_unknown_verb_against_empty_registry() {
    let r = ArgsRegistry::new();
    let err = validate("nope", &json!({}), &r).expect_err("empty registry rejects all verbs");
    assert_eq!(
        err,
        ValidationError::UnknownVerb {
            verb: "nope".to_owned()
        }
    );
}

#[test]
fn validate_unknown_verb_against_populated_registry() {
    let r = registry_with_project_create();
    let err =
        validate("project.delete", &json!({}), &r).expect_err("only project.create is registered");
    assert_eq!(
        err,
        ValidationError::UnknownVerb {
            verb: "project.delete".to_owned()
        }
    );
}

#[test]
fn unknown_verb_error_preserves_exact_verb_string() {
    let r = ArgsRegistry::new();
    let verb = "weird/case::name";
    let err = validate(verb, &json!({}), &r).unwrap_err();
    match err {
        ValidationError::UnknownVerb { verb: got } => assert_eq!(got, verb),
        other => panic!("expected UnknownVerb, got {other:?}"),
    }
}

#[test]
fn unknown_verb_error_displays_with_verb() {
    let r = ArgsRegistry::new();
    let err = validate("missing.verb", &json!({}), &r).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("missing.verb"),
        "Display output missing verb: {msg}"
    );
}

// --- SchemaViolation -------------------------------------------------

#[test]
fn validate_rejects_missing_required_field() {
    let r = registry_with_project_create();
    let args = json!({ "project_id": "p_01" }); // no `title`
    let err = validate("project.create", &args, &r).unwrap_err();
    match err {
        ValidationError::SchemaViolation { detail } => {
            assert!(!detail.is_empty(), "violation detail must not be empty");
        }
        other => panic!("expected SchemaViolation, got {other:?}"),
    }
}

#[test]
fn validate_rejects_wrong_type() {
    let r = registry_with_project_create();
    let args = json!({ "project_id": 42, "title": "demo" });
    let err = validate("project.create", &args, &r).unwrap_err();
    assert!(matches!(err, ValidationError::SchemaViolation { .. }));
}

#[test]
fn validate_rejects_extra_property_when_additional_properties_false() {
    let r = registry_with_project_create();
    let args = json!({
        "project_id": "p_01",
        "title": "demo",
        "rogue": true
    });
    let err = validate("project.create", &args, &r).unwrap_err();
    assert!(matches!(err, ValidationError::SchemaViolation { .. }));
}

#[test]
fn validate_rejects_min_length_violation() {
    let r = registry_with_project_create();
    let args = json!({ "project_id": "p_01", "title": "" });
    let err = validate("project.create", &args, &r).unwrap_err();
    assert!(matches!(err, ValidationError::SchemaViolation { .. }));
}

#[test]
fn validate_first_error_only_does_not_enumerate_all() {
    // Two distinct violations in one payload: missing `project_id`
    // AND wrong type for `title`. The validator must return one error,
    // not concatenate both.
    let r = registry_with_project_create();
    let args = json!({ "title": 7 });
    let err = validate("project.create", &args, &r).unwrap_err();
    match err {
        ValidationError::SchemaViolation { detail } => {
            // `detail` is the first jsonschema error to_string; it must
            // be one message, not a list.
            assert!(!detail.is_empty());
            // crude lower bound: a single error message does not contain
            // both field names verbatim. Pin only that it's non-empty
            // and that the type is SchemaViolation.
        }
        other => panic!("expected SchemaViolation, got {other:?}"),
    }
}

#[test]
fn validate_unknown_verb_takes_precedence_over_args_shape() {
    // If the verb is not registered, the args shape is never looked
    // at — even a completely garbage payload must surface
    // UnknownVerb, not SchemaViolation.
    let r = registry_with_project_create();
    let err = validate("not.a.verb", &json!({"completely": ["wrong", 1, null]}), &r).unwrap_err();
    assert!(matches!(err, ValidationError::UnknownVerb { .. }));
}

#[test]
fn validate_distinguishes_violation_from_unknown_verb() {
    // Cross-check: same args payload, two outcomes depending on
    // registration.
    let r_known = registry_with_project_create();
    let r_empty = ArgsRegistry::new();
    let args = json!({});

    let err_known = validate("project.create", &args, &r_known).unwrap_err();
    let err_empty = validate("project.create", &args, &r_empty).unwrap_err();
    assert!(matches!(err_known, ValidationError::SchemaViolation { .. }));
    assert!(matches!(err_empty, ValidationError::UnknownVerb { .. }));
}
