//! Tests for the `well_known` schema registry — pins the contract that
//! every shipped schema compiles, validates a well-formed input, and
//! rejects the two common malformations (missing required field /
//! extra unknown key).

use serde_json::{Value, json};
use verbreel_args::well_known::{
    ASSET_LIST_SCHEMA, CLIP_LIST_SCHEMA, COMPOUND_FLATTEN_SCHEMA, FONT_LIST_SCHEMA, HELP_SCHEMA,
    KEYFRAME_LIST_SCHEMA, LIST_CAPABILITIES_SCHEMA, PROJECT_LIST_SCHEMA, TIMELINE_HISTORY_SCHEMA,
    TIMELINE_UNDO_SCHEMA, TRACKER_LIST_SCHEMA, default_registry,
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

/// Every schema literal shipped by the module, paired with its verb id.
/// New entries land here so the compile / valid-JSON-object contracts
/// cover them without per-schema boilerplate.
const ALL_SCHEMAS: &[(&str, &str)] = &[
    ("help", HELP_SCHEMA),
    ("project.list", PROJECT_LIST_SCHEMA),
    ("list_capabilities", LIST_CAPABILITIES_SCHEMA),
    ("font.list", FONT_LIST_SCHEMA),
    ("asset.list", ASSET_LIST_SCHEMA),
    ("tracker.list", TRACKER_LIST_SCHEMA),
    ("compound.flatten", COMPOUND_FLATTEN_SCHEMA),
    ("timeline.undo", TIMELINE_UNDO_SCHEMA),
    ("timeline.history", TIMELINE_HISTORY_SCHEMA),
    ("keyframe.list", KEYFRAME_LIST_SCHEMA),
    ("clip.list", CLIP_LIST_SCHEMA),
];

#[test]
fn every_well_known_schema_compiles() {
    for (_verb, raw) in ALL_SCHEMAS {
        let _ = compile(raw);
    }
}

#[test]
fn schema_literals_are_valid_json_objects() {
    for (_verb, raw) in ALL_SCHEMAS {
        let value = parse(raw);
        assert!(
            value.is_object(),
            "every schema literal must parse to a JSON object, got: {value}"
        );
    }
}

// --- 2. default_registry shape ------------------------------------------

#[test]
fn default_registry_contains_one_entry_per_well_known_schema() {
    let registry = default_registry();
    assert_eq!(registry.len(), ALL_SCHEMAS.len());
}

#[test]
fn default_registry_resolves_every_verb_id() {
    let registry = default_registry();
    for (verb, _raw) in ALL_SCHEMAS {
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

// --- 11. Sprint-2 minimal-args slice (table-driven) ---------------------
//
// Each newly-registered verb is exercised by one table of (label,
// payload, expectation) rows: at least one accept-case and the
// relevant reject-cases (missing required field, unknown key, wrong
// type). `Expect::Ok` asserts the payload validates; `Expect::Schema`
// asserts a `ValidationError::SchemaViolation`. Adding a verb = adding
// a `case![...]` block, no new test fn.

enum Expect {
    Ok,
    Schema,
}

macro_rules! case {
    ($name:ident, $verb:expr, [ $( ($label:expr, $payload:expr, $exp:expr) ),+ $(,)? ]) => {
        #[test]
        fn $name() {
            let registry = default_registry();
            $(
                let payload: Value = $payload;
                let outcome = validate($verb, &payload, &registry);
                match $exp {
                    Expect::Ok => {
                        outcome.unwrap_or_else(|e| {
                            panic!("{}: `{}` must validate, got error: {e:?}", $verb, $label)
                        });
                    }
                    Expect::Schema => {
                        let err = outcome.expect_err(&format!(
                            "{}: `{}` must be rejected, but validated",
                            $verb, $label
                        ));
                        assert!(
                            matches!(err, ValidationError::SchemaViolation { .. }),
                            "{}: `{}` must fail with SchemaViolation, got: {err:?}",
                            $verb,
                            $label
                        );
                    }
                }
            )+
        }
    };
}

case!(
    tracker_list_table,
    "tracker.list",
    [
        ("uuid-only", json!({ "project_id": VALID_UUID }), Expect::Ok),
        ("missing project_id", json!({}), Expect::Schema),
        (
            "project_id wrong type",
            json!({ "project_id": 42 }),
            Expect::Schema
        ),
        (
            "unknown key",
            json!({ "project_id": VALID_UUID, "extra": 1 }),
            Expect::Schema
        ),
    ]
);

case!(
    compound_flatten_table,
    "compound.flatten",
    [
        (
            "uuid + clip",
            json!({ "project_id": VALID_UUID, "clip": "01900000-0000-7000-8000-0000000000aa" }),
            Expect::Ok
        ),
        (
            "clip prefix selector",
            json!({ "project_id": VALID_UUID, "clip": "clip:01900000-0000-7000-8000-0000000000aa" }),
            Expect::Ok
        ),
        (
            "missing clip",
            json!({ "project_id": VALID_UUID }),
            Expect::Schema
        ),
        ("missing project_id", json!({ "clip": "x" }), Expect::Schema),
        (
            "clip wrong type",
            json!({ "project_id": VALID_UUID, "clip": 7 }),
            Expect::Schema
        ),
        (
            "unknown key",
            json!({ "project_id": VALID_UUID, "clip": "x", "extra": true }),
            Expect::Schema
        ),
    ]
);

case!(
    timeline_undo_table,
    "timeline.undo",
    [
        (
            "uuid-only (steps omitted)",
            json!({ "project_id": VALID_UUID }),
            Expect::Ok
        ),
        (
            "steps = 1",
            json!({ "project_id": VALID_UUID, "steps": 1 }),
            Expect::Ok
        ),
        (
            "steps = 5",
            json!({ "project_id": VALID_UUID, "steps": 5 }),
            Expect::Ok
        ),
        (
            "steps null",
            json!({ "project_id": VALID_UUID, "steps": null }),
            Expect::Ok
        ),
        (
            "steps = 0 below minimum",
            json!({ "project_id": VALID_UUID, "steps": 0 }),
            Expect::Schema
        ),
        (
            "steps negative",
            json!({ "project_id": VALID_UUID, "steps": -3 }),
            Expect::Schema
        ),
        (
            "steps non-integer",
            json!({ "project_id": VALID_UUID, "steps": "two" }),
            Expect::Schema
        ),
        ("missing project_id", json!({ "steps": 1 }), Expect::Schema),
        (
            "unknown key",
            json!({ "project_id": VALID_UUID, "limit": 10 }),
            Expect::Schema
        ),
    ]
);

case!(
    timeline_history_table,
    "timeline.history",
    [
        ("uuid-only", json!({ "project_id": VALID_UUID }), Expect::Ok),
        (
            "all optional fields",
            json!({ "project_id": VALID_UUID, "limit": 50, "since": "empty", "include_undone": true }),
            Expect::Ok
        ),
        (
            "all optional null",
            json!({ "project_id": VALID_UUID, "limit": null, "since": null, "include_undone": null }),
            Expect::Ok
        ),
        ("missing project_id", json!({ "limit": 10 }), Expect::Schema),
        (
            "limit wrong type",
            json!({ "project_id": VALID_UUID, "limit": "10" }),
            Expect::Schema
        ),
        (
            "since wrong type",
            json!({ "project_id": VALID_UUID, "since": 1 }),
            Expect::Schema
        ),
        (
            "include_undone wrong type",
            json!({ "project_id": VALID_UUID, "include_undone": "yes" }),
            Expect::Schema
        ),
        (
            "unknown key",
            json!({ "project_id": VALID_UUID, "stranger": 1 }),
            Expect::Schema
        ),
    ]
);

case!(
    keyframe_list_table,
    "keyframe.list",
    [
        (
            "uuid + clip",
            json!({ "project_id": VALID_UUID, "clip": "01900000-0000-7000-8000-0000000000bb" }),
            Expect::Ok
        ),
        (
            "with property filter",
            json!({ "project_id": VALID_UUID, "clip": "01900000-0000-7000-8000-0000000000bb", "property": "transform.scale" }),
            Expect::Ok
        ),
        (
            "property null",
            json!({ "project_id": VALID_UUID, "clip": "01900000-0000-7000-8000-0000000000bb", "property": null }),
            Expect::Ok
        ),
        (
            "missing clip",
            json!({ "project_id": VALID_UUID }),
            Expect::Schema
        ),
        ("missing project_id", json!({ "clip": "x" }), Expect::Schema),
        (
            "clip wrong type",
            json!({ "project_id": VALID_UUID, "clip": 9 }),
            Expect::Schema
        ),
        (
            "property wrong type",
            json!({ "project_id": VALID_UUID, "clip": "x", "property": 9 }),
            Expect::Schema
        ),
        (
            "unknown key",
            json!({ "project_id": VALID_UUID, "clip": "x", "extra": 1 }),
            Expect::Schema
        ),
    ]
);

case!(
    clip_list_table,
    "clip.list",
    [
        ("uuid-only", json!({ "project_id": VALID_UUID }), Expect::Ok),
        (
            "with track filter",
            json!({ "project_id": VALID_UUID, "track": "01900000-0000-7000-8000-0000000000cc" }),
            Expect::Ok
        ),
        (
            "with at_tk filter",
            json!({ "project_id": VALID_UUID, "at_tk": 4800 }),
            Expect::Ok
        ),
        (
            "both filters null",
            json!({ "project_id": VALID_UUID, "track": null, "at_tk": null }),
            Expect::Ok
        ),
        (
            "missing project_id",
            json!({ "track": "x" }),
            Expect::Schema
        ),
        (
            "track wrong type",
            json!({ "project_id": VALID_UUID, "track": 1 }),
            Expect::Schema
        ),
        (
            "at_tk non-integer",
            json!({ "project_id": VALID_UUID, "at_tk": "4800" }),
            Expect::Schema
        ),
        (
            "unknown key",
            json!({ "project_id": VALID_UUID, "limit": 10 }),
            Expect::Schema
        ),
    ]
);
