//! Tests for [`verbreel_args::Schema`] construction and accessors.
//!
//! Surface under test:
//!
//! - [`Schema::from_value`] — well-formed schema compiles, ill-formed
//!   schema returns the right [`SchemaError`] variant.
//! - [`Schema::as_value`] — round-trips the raw schema unchanged.
//! - [`Schema::validator`] — hands back the compiled artefact that
//!   actually validates instances.

use serde_json::{Value, json};
use verbreel_args::{Schema, SchemaError};

// --- happy path ----------------------------------------------------------

#[test]
fn from_value_accepts_empty_object_schema() {
    // `{}` matches every JSON value. It is still a well-formed schema.
    let s = Schema::from_value(json!({})).expect("empty object is a valid schema");
    assert!(s.validator().is_valid(&json!(null)));
    assert!(s.validator().is_valid(&json!(42)));
    assert!(s.validator().is_valid(&json!({"any": "thing"})));
}

#[test]
fn from_value_accepts_typical_args_schema() {
    let raw = json!({
        "type": "object",
        "properties": {
            "project_id": { "type": "string" },
            "title": { "type": "string", "minLength": 1 }
        },
        "required": ["project_id", "title"],
        "additionalProperties": false
    });
    let s = Schema::from_value(raw).expect("standard args schema must compile");
    assert!(s.validator().is_valid(&json!({
        "project_id": "p_01",
        "title": "demo"
    })));
}

#[test]
fn from_value_accepts_string_only_schema() {
    let s = Schema::from_value(json!({ "type": "string" })).unwrap();
    assert!(s.validator().is_valid(&json!("hello")));
    assert!(!s.validator().is_valid(&json!(7)));
}

#[test]
fn from_value_accepts_enum_schema() {
    let s = Schema::from_value(json!({ "enum": ["a", "b", "c"] })).unwrap();
    assert!(s.validator().is_valid(&json!("a")));
    assert!(!s.validator().is_valid(&json!("z")));
}

// --- as_value: round-trip -----------------------------------------------

#[test]
fn as_value_returns_exact_input_object() {
    let raw = json!({
        "type": "object",
        "properties": { "x": { "type": "number" } }
    });
    let s = Schema::from_value(raw.clone()).unwrap();
    assert_eq!(s.as_value(), &raw);
}

#[test]
fn as_value_preserves_nested_structure() {
    let raw = json!({
        "type": "object",
        "properties": {
            "outer": {
                "type": "object",
                "properties": {
                    "inner": { "type": "integer", "minimum": 0 }
                }
            }
        }
    });
    let s = Schema::from_value(raw.clone()).unwrap();
    // Walk back through serde_json to be sure nothing was rewritten.
    let serialised = serde_json::to_value(s.as_value()).unwrap();
    assert_eq!(serialised, raw);
}

#[test]
fn as_value_is_borrowed_not_cloned() {
    let raw = json!({"type": "boolean"});
    let s = Schema::from_value(raw).unwrap();
    let p1 = s.as_value() as *const Value;
    let p2 = s.as_value() as *const Value;
    assert_eq!(p1, p2, "as_value must return the same borrow each call");
}

// --- validator: pre-compiled artefact -----------------------------------

#[test]
fn validator_handle_runs_against_instances() {
    let s = Schema::from_value(json!({
        "type": "object",
        "required": ["k"]
    }))
    .unwrap();
    assert!(s.validator().is_valid(&json!({ "k": 1 })));
    assert!(!s.validator().is_valid(&json!({})));
}

#[test]
fn validator_returns_first_error_via_iter_errors() {
    let s = Schema::from_value(json!({
        "type": "object",
        "required": ["a", "b"]
    }))
    .unwrap();
    let instance = json!({});
    let mut errs = s.validator().iter_errors(&instance);
    let first = errs.next().expect("missing required field must surface");
    let msg = first.to_string();
    // The validator reports one missing-property error; we don't pin
    // exact wording, just that it mentions a required field.
    assert!(
        msg.contains("required") || msg.contains("\"a\"") || msg.contains("\"b\""),
        "unexpected error message: {msg}"
    );
}

// --- ill-formed shapes: NotAnObject -------------------------------------

#[test]
fn from_value_rejects_null() {
    let err = Schema::from_value(json!(null)).expect_err("null is not an object");
    assert!(matches!(err, SchemaError::NotAnObject("null")));
}

#[test]
fn from_value_rejects_bare_bool_true() {
    // JSON Schema 2020-12 allows `true`/`false` as schemas, but this
    // layer rejects them — see the SchemaError::NotAnObject doc.
    let err = Schema::from_value(json!(true)).expect_err("bool rejected at this layer");
    assert!(matches!(err, SchemaError::NotAnObject("bool")));
}

#[test]
fn from_value_rejects_bare_bool_false() {
    let err = Schema::from_value(json!(false)).expect_err("bool rejected at this layer");
    assert!(matches!(err, SchemaError::NotAnObject("bool")));
}

#[test]
fn from_value_rejects_number() {
    let err = Schema::from_value(json!(7)).expect_err("number is not an object");
    assert!(matches!(err, SchemaError::NotAnObject("number")));
}

#[test]
fn from_value_rejects_string() {
    let err = Schema::from_value(json!("schema")).expect_err("string is not an object");
    assert!(matches!(err, SchemaError::NotAnObject("string")));
}

#[test]
fn from_value_rejects_array() {
    let err = Schema::from_value(json!([1, 2, 3])).expect_err("array is not an object");
    assert!(matches!(err, SchemaError::NotAnObject("array")));
}

// --- ill-formed shapes: Compile -----------------------------------------

#[test]
fn from_value_rejects_invalid_keyword_value() {
    // `type` must be a string or an array of strings; an object here
    // triggers jsonschema's meta-validation.
    let err = Schema::from_value(json!({ "type": { "nested": true } }))
        .expect_err("malformed type keyword must reject");
    assert!(matches!(err, SchemaError::Compile(_)));
}

#[test]
fn from_value_compile_error_preserves_validator_message() {
    // The Compile variant must carry the underlying validator message
    // so callers can surface a useful diagnostic. We don't pin exact
    // wording — only that the string is non-empty.
    let err = Schema::from_value(json!({ "type": "not-a-real-json-type" }))
        .expect_err("bogus type literal must reject");
    match err {
        SchemaError::Compile(msg) => assert!(!msg.is_empty()),
        other => panic!("expected Compile, got {other:?}"),
    }
}
