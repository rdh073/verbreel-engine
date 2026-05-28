//! Tests for the `well_known` schema registry — pins the contract that
//! every shipped schema compiles, validates a well-formed input, and
//! rejects the two common malformations (missing required field /
//! extra unknown key).

use serde_json::{Value, json};
use verbreel_args::well_known::{
    ASSET_LIST_SCHEMA, FONT_LIST_SCHEMA, HELP_SCHEMA, LIST_CAPABILITIES_SCHEMA,
    PROJECT_LIST_SCHEMA, default_registry,
};
use verbreel_args::{Schema, ValidationError, validate};

/// Canonical UUIDv7 used across the accept-case tests.
const VALID_UUID: &str = "01900000-0000-7000-8000-000000000001";

fn parse(raw: &str) -> Value {
    serde_json::from_str(raw).expect("well-known schema literal must parse as JSON")
}

fn compile(raw: &str) -> Schema {
    Schema::from_value(parse(raw)).expect("well-known schema literal must compile")
}

// --- 1. Every schema compiles -------------------------------------------

#[test]
fn every_well_known_schema_compiles() {
    let _ = compile(HELP_SCHEMA);
    let _ = compile(PROJECT_LIST_SCHEMA);
    let _ = compile(LIST_CAPABILITIES_SCHEMA);
    let _ = compile(FONT_LIST_SCHEMA);
    let _ = compile(ASSET_LIST_SCHEMA);
}

#[test]
fn schema_literals_are_valid_json_objects() {
    for raw in [
        HELP_SCHEMA,
        PROJECT_LIST_SCHEMA,
        LIST_CAPABILITIES_SCHEMA,
        FONT_LIST_SCHEMA,
        ASSET_LIST_SCHEMA,
    ] {
        let value = parse(raw);
        assert!(
            value.is_object(),
            "every schema literal must parse to a JSON object, got: {value}"
        );
    }
}

// --- 2. default_registry shape ------------------------------------------

#[test]
fn default_registry_contains_exactly_five_entries() {
    let registry = default_registry();
    assert_eq!(registry.len(), 5);
}

#[test]
fn default_registry_resolves_every_verb_id() {
    let registry = default_registry();
    for verb in [
        "help",
        "project.list",
        "list_capabilities",
        "font.list",
        "asset.list",
    ] {
        assert!(
            registry.get(verb).is_some(),
            "default_registry must resolve verb `{verb}`"
        );
    }
}

#[test]
fn default_registry_rejects_unrelated_verb_id() {
    let registry = default_registry();
    assert!(registry.get("totally.not.a.real.verb").is_none());
}

// --- 3. Per-verb accept-case --------------------------------------------

#[test]
fn help_accepts_valid_uuid_only() {
    let registry = default_registry();
    validate("help", &json!({ "project_id": VALID_UUID }), &registry)
        .expect("uuid-only payload must validate against help schema");
}

#[test]
fn project_list_accepts_valid_uuid_only() {
    let registry = default_registry();
    validate(
        "project.list",
        &json!({ "project_id": VALID_UUID }),
        &registry,
    )
    .expect("uuid-only payload must validate against project.list schema");
}

#[test]
fn list_capabilities_accepts_valid_uuid_only() {
    let registry = default_registry();
    validate(
        "list_capabilities",
        &json!({ "project_id": VALID_UUID }),
        &registry,
    )
    .expect("uuid-only payload must validate against list_capabilities schema");
}

#[test]
fn font_list_accepts_valid_uuid_only() {
    let registry = default_registry();
    validate("font.list", &json!({ "project_id": VALID_UUID }), &registry)
        .expect("uuid-only payload must validate against font.list schema");
}

#[test]
fn asset_list_accepts_valid_uuid_only() {
    let registry = default_registry();
    validate(
        "asset.list",
        &json!({ "project_id": VALID_UUID }),
        &registry,
    )
    .expect("uuid-only payload must validate against asset.list schema");
}

// --- 4. Missing-required-field rejection --------------------------------

#[test]
fn help_rejects_empty_object() {
    let registry = default_registry();
    let err = validate("help", &json!({}), &registry).unwrap_err();
    assert!(matches!(err, ValidationError::SchemaViolation { .. }));
}

#[test]
fn project_list_rejects_empty_object() {
    let registry = default_registry();
    let err = validate("project.list", &json!({}), &registry).unwrap_err();
    assert!(matches!(err, ValidationError::SchemaViolation { .. }));
}

#[test]
fn asset_list_rejects_empty_object() {
    let registry = default_registry();
    let err = validate("asset.list", &json!({}), &registry).unwrap_err();
    assert!(matches!(err, ValidationError::SchemaViolation { .. }));
}

// --- 5. Unknown-key rejection (additionalProperties: false) -------------

#[test]
fn help_rejects_unknown_key() {
    let registry = default_registry();
    let err = validate(
        "help",
        &json!({ "project_id": VALID_UUID, "garbage": "x" }),
        &registry,
    )
    .unwrap_err();
    assert!(matches!(err, ValidationError::SchemaViolation { .. }));
}

#[test]
fn project_list_rejects_unknown_key() {
    let registry = default_registry();
    let err = validate(
        "project.list",
        &json!({ "project_id": VALID_UUID, "limit": 10 }),
        &registry,
    )
    .unwrap_err();
    assert!(matches!(err, ValidationError::SchemaViolation { .. }));
}

#[test]
fn asset_list_rejects_unknown_key() {
    let registry = default_registry();
    let err = validate(
        "asset.list",
        &json!({ "project_id": VALID_UUID, "extra": true }),
        &registry,
    )
    .unwrap_err();
    assert!(matches!(err, ValidationError::SchemaViolation { .. }));
}

// --- 6. asset.list `kind` enum ------------------------------------------

#[test]
fn asset_list_accepts_each_kind_variant() {
    let registry = default_registry();
    for variant in ["video", "audio", "image", "subtitle"] {
        validate(
            "asset.list",
            &json!({ "project_id": VALID_UUID, "kind": variant }),
            &registry,
        )
        .unwrap_or_else(|e| {
            panic!("asset.list must accept kind={variant}, got error: {e:?}");
        });
    }
}

#[test]
fn asset_list_rejects_unknown_kind_value() {
    let registry = default_registry();
    let err = validate(
        "asset.list",
        &json!({ "project_id": VALID_UUID, "kind": "bogus" }),
        &registry,
    )
    .unwrap_err();
    assert!(matches!(err, ValidationError::SchemaViolation { .. }));
}

#[test]
fn asset_list_rejects_wrong_type_for_kind() {
    let registry = default_registry();
    let err = validate(
        "asset.list",
        &json!({ "project_id": VALID_UUID, "kind": 42 }),
        &registry,
    )
    .unwrap_err();
    assert!(matches!(err, ValidationError::SchemaViolation { .. }));
}

// --- 7. UnknownVerb path stays intact ----------------------------------

#[test]
fn validate_unknown_verb_against_default_registry_returns_unknown_verb() {
    let registry = default_registry();
    let err = validate(
        "totally.fake.verb",
        &json!({ "project_id": VALID_UUID }),
        &registry,
    )
    .unwrap_err();
    assert!(matches!(err, ValidationError::UnknownVerb { .. }));
}

// --- 8. help.topic optional field ---------------------------------------

#[test]
fn help_accepts_topic_string() {
    let registry = default_registry();
    validate(
        "help",
        &json!({ "project_id": VALID_UUID, "topic": "clip" }),
        &registry,
    )
    .expect("help must accept topic as a string");
}

#[test]
fn help_accepts_full_verb_id_as_topic() {
    let registry = default_registry();
    validate(
        "help",
        &json!({ "project_id": VALID_UUID, "topic": "clip.add" }),
        &registry,
    )
    .expect("help must accept a full verb id as topic");
}

#[test]
fn help_rejects_non_string_topic() {
    let registry = default_registry();
    let err = validate(
        "help",
        &json!({ "project_id": VALID_UUID, "topic": 42 }),
        &registry,
    )
    .unwrap_err();
    assert!(matches!(err, ValidationError::SchemaViolation { .. }));
}

// --- 9. Coverage parity for list_capabilities + font.list --------------

#[test]
fn list_capabilities_rejects_empty_object() {
    let registry = default_registry();
    let err = validate("list_capabilities", &json!({}), &registry).unwrap_err();
    assert!(matches!(err, ValidationError::SchemaViolation { .. }));
}

#[test]
fn font_list_rejects_empty_object() {
    let registry = default_registry();
    let err = validate("font.list", &json!({}), &registry).unwrap_err();
    assert!(matches!(err, ValidationError::SchemaViolation { .. }));
}

#[test]
fn list_capabilities_rejects_unknown_key() {
    let registry = default_registry();
    let err = validate(
        "list_capabilities",
        &json!({ "project_id": VALID_UUID, "stranger": 1 }),
        &registry,
    )
    .unwrap_err();
    assert!(matches!(err, ValidationError::SchemaViolation { .. }));
}

#[test]
fn font_list_rejects_unknown_key() {
    let registry = default_registry();
    let err = validate(
        "font.list",
        &json!({ "project_id": VALID_UUID, "family": "Helvetica" }),
        &registry,
    )
    .unwrap_err();
    assert!(matches!(err, ValidationError::SchemaViolation { .. }));
}

// --- 10. Optional fields accept null --------------------------------------

#[test]
fn help_topic_accepts_null() {
    // `HelpArgs.topic: Option<String>` deserializes both omitted AND
    // `null` as `None`. Schema must accept both shapes to match the
    // typed Args contract.
    let registry = default_registry();
    validate(
        "help",
        &json!({ "project_id": VALID_UUID, "topic": null }),
        &registry,
    )
    .expect("help schema must accept topic: null");
}

#[test]
fn asset_list_kind_accepts_null() {
    let registry = default_registry();
    validate(
        "asset.list",
        &json!({ "project_id": VALID_UUID, "kind": null }),
        &registry,
    )
    .expect("asset.list schema must accept kind: null");
}
