//! Empty-project byte-equal round trip.
//!
//! Loads the hand-written fixture, deserializes into [`Project`], then
//! serializes back to JSON. Compares the parsed `serde_json::Value` of
//! input and output (NOT raw string bytes — pretty-print whitespace and
//! key order differ between hand-written JSON and serde output, and
//! `Value` equality is the right comparator). Also asserts `Project ==
//! Project` after the round trip.

use serde_json::Value;
use verbreel_state::Project;

const FIXTURE: &str = include_str!("fixtures/empty_project_create.json");

#[test]
fn round_trip_empty_project_create() {
    // 1) Hand-written JSON → typed Project.
    let p1: Project = serde_json::from_str(FIXTURE).expect("fixture must deserialize into Project");

    // 2) Typed Project → JSON string.
    let s = serde_json::to_string_pretty(&p1).expect("Project must serialize");

    // 3) Hand-written JSON → Value (golden).
    let v_in: Value = serde_json::from_str(FIXTURE).expect("fixture must parse as Value");

    // 4) Serialized JSON → Value (round-tripped).
    let v_out: Value = serde_json::from_str(&s).expect("round-trip JSON must parse");

    // Equality compares by structure — Map insertion order and pretty
    // whitespace are both ignored.
    assert_eq!(
        v_in, v_out,
        "round-trip JSON Value must equal the fixture Value (left = fixture, right = serialized)"
    );

    // 5) Project PartialEq round-trip.
    let p2: Project = serde_json::from_str(&s).expect("round-trip JSON must deserialize");
    assert_eq!(p1, p2, "Project PartialEq must hold across the round trip");
}
