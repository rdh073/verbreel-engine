//! Tests for the §0.8 reconstructor-purity startup gate (Slice A).
//!
//! These exercise the freestanding [`validate_reconstructors`] validator
//! and the [`VerbRegistry`] against purpose-built test verbs. No
//! production verb is registered here — the only [`VerbReconstructor`]
//! impls in the tree live in this file.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::{
    Project, ReconstructError, RecordedEvent, RegistryError, ValidationError, VerbReconstructor,
    VerbRegistry, validate_reconstructors,
};

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture → Project")
}

/// Build a fixture with an empty patch / no warnings / empty post-state.
/// The test verbs do not read patch / warnings / post-state, so those
/// carry no signal here — the 5-tuple is still fully populated.
fn fixture(verb: &str, args: Value, expected_data: Value) -> RecordedEvent {
    RecordedEvent {
        verb: verb.to_string(),
        args,
        patch: json!([]),
        warnings: vec![],
        post_state: empty_project(),
        expected_data,
    }
}

/// A pure echo verb: `data` is `args["echo"]` verbatim.
struct TestEchoVerb;

impl VerbReconstructor for TestEchoVerb {
    fn verb(&self) -> &'static str {
        "test.echo"
    }

    fn reconstruct(
        &self,
        args: &Value,
        _patch: &Value,
        _warnings: &[Value],
        _post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        args.get("echo")
            .cloned()
            .ok_or(ReconstructError::MissingField { name: "echo" })
    }
}

/// A verb whose reconstructor always fails — recorded inputs are
/// (pretend-)malformed. Models a verb-author bug caught at the gate.
struct BrokenReconstructorVerb;

impl VerbReconstructor for BrokenReconstructorVerb {
    fn verb(&self) -> &'static str {
        "broken.verb"
    }

    fn reconstruct(
        &self,
        _args: &Value,
        _patch: &Value,
        _warnings: &[Value],
        _post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        Err(ReconstructError::MissingField { name: "echo" })
    }
}

/// A verb whose reconstructor returns a fixed object with a deliberately
/// reordered key set. Used to prove the validator compares by RFC 8785
/// canonical SHA (key-order-insensitive), not by raw serialization.
struct TestKeyOrderVerb;

impl VerbReconstructor for TestKeyOrderVerb {
    fn verb(&self) -> &'static str {
        "test.keyorder"
    }

    fn reconstruct(
        &self,
        _args: &Value,
        _patch: &Value,
        _warnings: &[Value],
        _post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        // Insertion order b, a — the opposite of the fixture's expected.
        Ok(json!({ "b": 2, "a": 1 }))
    }
}

#[test]
fn empty_registry_empty_fixtures_passes() {
    let registry = VerbRegistry::new();
    let report = validate_reconstructors(&registry, &[]).expect("vacuous pass");
    assert!(report.verbs_checked.is_empty());
    assert_eq!(report.fixtures_run, 0);
}

#[test]
fn single_pure_verb_passes() {
    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(TestEchoVerb))
        .expect("first register ok");

    let echo = json!({ "hello": "world" });
    let fixtures = [fixture("test.echo", json!({ "echo": echo.clone() }), echo)];

    let report = validate_reconstructors(&registry, &fixtures).expect("pure verb should pass");
    assert_eq!(report.fixtures_run, 1);
    assert_eq!(report.verbs_checked, vec!["test.echo"]);
}

#[test]
fn mismatch_returns_data_mismatch() {
    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(TestEchoVerb))
        .expect("register ok");

    // args.echo is {"echo":"hello"} but expected_data is {"echo":"different"}.
    let fixtures = [fixture(
        "test.echo",
        json!({ "echo": { "echo": "hello" } }),
        json!({ "echo": "different" }),
    )];

    let err = validate_reconstructors(&registry, &fixtures).expect_err("mismatch must error");
    assert!(
        matches!(
            err,
            ValidationError::DataMismatch {
                verb: "test.echo",
                ..
            }
        ),
        "expected DataMismatch for test.echo, got {err:?}"
    );
}

#[test]
fn reconstructor_error_propagates() {
    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(BrokenReconstructorVerb))
        .expect("register ok");

    let fixtures = [fixture("broken.verb", json!({}), json!({}))];

    let err = validate_reconstructors(&registry, &fixtures).expect_err("reconstructor err");
    assert!(
        matches!(
            err,
            ValidationError::ReconstructError {
                verb: "broken.verb",
                source: ReconstructError::MissingField { name: "echo" },
            }
        ),
        "expected ReconstructError wrapping MissingField, got {err:?}"
    );
}

#[test]
fn unknown_verb_in_fixture_errors() {
    // Registry has no verbs; the fixture references one anyway.
    let registry = VerbRegistry::new();
    let fixtures = [fixture("test.echo", json!({ "echo": 1 }), json!(1))];

    let err = validate_reconstructors(&registry, &fixtures).expect_err("unknown verb");
    match err {
        ValidationError::UnknownVerb { verb } => assert_eq!(verb, "test.echo"),
        other => panic!("expected UnknownVerb, got {other:?}"),
    }
}

#[test]
fn duplicate_register_rejected() {
    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(TestEchoVerb))
        .expect("first register ok");

    let err = registry
        .register(Arc::new(TestEchoVerb))
        .expect_err("second register must reject");
    assert_eq!(err, RegistryError::DuplicateVerb { verb: "test.echo" });
}

#[test]
fn sha_canonicalization_used() {
    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(TestKeyOrderVerb))
        .expect("register ok");

    // expected_data uses key order a, b; the reconstructor returns b, a.
    let expected_data = json!({ "a": 1, "b": 2 });
    let produced = json!({ "b": 2, "a": 1 });

    // Sanity: the two values genuinely differ by key order at the
    // serialization level (serde_json preserve_order is on). If this
    // assertion ever fails, the regression guard below is vacuous.
    assert_ne!(
        serde_json::to_string(&expected_data).unwrap(),
        serde_json::to_string(&produced).unwrap(),
        "key orders must differ for this guard to be meaningful"
    );

    let fixtures = [fixture("test.keyorder", json!({}), expected_data)];

    // Despite the key-order difference, canonical SHA equality holds →
    // the fixture passes. A naive `==`-on-serialized-string comparison
    // would (incorrectly) fail here.
    let report =
        validate_reconstructors(&registry, &fixtures).expect("key-order diff must still pass");
    assert_eq!(report.fixtures_run, 1);
    assert_eq!(report.verbs_checked, vec!["test.keyorder"]);
}
