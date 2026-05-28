//! project_hash field-projection contract, locked through the typed
//! [`Project`] shape.
//!
//! [`verbreel_canon::project_hash`] strips `updated_at` and
//! `last_saved_event_id` before canonicalizing and hashing. This test
//! lifts that contract up through serde — mutating those two fields
//! on a typed [`Project`] must NOT change the hash.

use serde_json::Value;
use verbreel_canon::project_hash;
use verbreel_events::Timestamp;
use verbreel_state::Project;
use verbreel_types::{EventId, UuidV7};

const FIXTURE: &str = include_str!("fixtures/empty_project_create.json");

const KNOWN_V7: &str = "0190b8d3-15e3-7000-bd00-00000000beef";

fn hash_of(p: &Project) -> String {
    let v: Value = serde_json::to_value(p).expect("Project → Value");
    project_hash(&v).expect("project_hash must succeed on a well-formed Project")
}

#[test]
fn project_hash_stable_across_serde_roundtrip() {
    // Deserialize → serialize → deserialize → hash. The hash must be
    // identical to the hash of the original — locks the contract that
    // typed Project carries no hidden mutation across the round trip.
    let p1: Project = serde_json::from_str(FIXTURE).expect("fixture → Project");
    let json = serde_json::to_string(&p1).expect("Project → JSON");
    let p2: Project = serde_json::from_str(&json).expect("JSON → Project");

    let h1 = hash_of(&p1);
    let h2 = hash_of(&p2);

    assert_eq!(
        h1, h2,
        "project_hash must be stable across a serialize-deserialize round trip"
    );
}

#[test]
fn project_hash_ignores_updated_at_and_last_saved_event_id() {
    let mut p1: Project = serde_json::from_str(FIXTURE).expect("fixture → Project");
    let h_baseline = hash_of(&p1);

    // Mutate `updated_at` — projection rule strips this before
    // canonicalization, so the hash must NOT change.
    p1.updated_at = Timestamp::parse("9999-12-31T23:59:59Z").unwrap();
    let h_after_updated_at = hash_of(&p1);
    assert_eq!(
        h_baseline, h_after_updated_at,
        "mutating updated_at must NOT change project_hash (canon projects this field out)"
    );

    // Mutate `last_saved_event_id` from None → Some(...). Same rule.
    let known: EventId = EventId::from_uuid_v7(KNOWN_V7.parse::<UuidV7>().unwrap());
    p1.last_saved_event_id = Some(known);
    let h_after_event_id = hash_of(&p1);
    assert_eq!(
        h_baseline, h_after_event_id,
        "mutating last_saved_event_id must NOT change project_hash (canon projects this field out)"
    );

    // Sanity check: mutating a NON-projected field DOES change the
    // hash. If this fails, the projection in `verbreel-canon` has
    // accidentally been broadened.
    p1.name = "renamed".to_string();
    let h_after_name = hash_of(&p1);
    assert_ne!(
        h_baseline, h_after_name,
        "mutating a non-projected field (name) MUST change project_hash; \
         if this assertion fails, verbreel-canon's projection is too broad"
    );
}
