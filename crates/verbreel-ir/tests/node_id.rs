//! Integration tests for [`IrNodeId`] — the strict-`UUIDv7` newtype that
//! identifies a composition-graph node.
//!
//! Mirrors the test style established in `crates/verbreel-types/src/id.rs`:
//! string round-trips, version-nibble enforcement, serde shape.

use std::str::FromStr;

use verbreel_ir::IrNodeId;
use verbreel_types::id::UuidV7;

// A synthetic v7 UUID — bit 0x70 in the version nibble.
const KNOWN_V7: &str = "0190b8d3-15e3-7000-b8d3-15e370a30000";

// A v4 UUID — must be rejected by FromStr.
const KNOWN_V4: &str = "550e8400-e29b-41d4-a716-446655440000";

#[test]
fn from_str_accepts_v7_string() {
    let id: IrNodeId = KNOWN_V7.parse().unwrap();
    assert_eq!(id.to_string(), KNOWN_V7);
}

#[test]
fn display_round_trips_through_from_str() {
    let original = IrNodeId::now();
    let s = original.to_string();
    let back = IrNodeId::from_str(&s).unwrap();
    assert_eq!(original, back);
}

#[test]
fn from_str_rejects_v4_string() {
    let err = KNOWN_V4.parse::<IrNodeId>().unwrap_err();
    // Mirror the verbreel-types pattern: we don't pin the exact variant
    // beyond confirming an error surfaced.
    assert!(
        !err.to_string().is_empty(),
        "v4 rejection must surface a non-empty error"
    );
}

#[test]
fn from_str_rejects_nil_uuid() {
    let nil = "00000000-0000-0000-0000-000000000000";
    let err = nil.parse::<IrNodeId>().unwrap_err();
    assert!(
        !err.to_string().is_empty(),
        "nil rejection must surface a non-empty error"
    );
}

#[test]
fn now_mints_distinct_ids() {
    // UUIDv7 embeds a ms-resolution timestamp + 74 random bits — back-to-back
    // calls may share the millisecond but the random tail makes collisions
    // astronomically unlikely.
    let a = IrNodeId::now();
    let b = IrNodeId::now();
    assert_ne!(a, b);
}

#[test]
fn as_uuid_v7_returns_inner() {
    let inner: UuidV7 = KNOWN_V7.parse().unwrap();
    let id = IrNodeId::from_uuid_v7(inner);
    assert_eq!(id.as_uuid_v7(), inner);
}

#[test]
fn as_uuid_matches_string() {
    let id: IrNodeId = KNOWN_V7.parse().unwrap();
    assert_eq!(id.as_uuid().hyphenated().to_string(), KNOWN_V7);
}

#[test]
fn serde_round_trips_as_uuid_string() {
    let id: IrNodeId = KNOWN_V7.parse().unwrap();
    let json = serde_json::to_string(&id).unwrap();
    assert_eq!(json, format!("\"{KNOWN_V7}\""));
    let back: IrNodeId = serde_json::from_str(&json).unwrap();
    assert_eq!(id, back);
}

#[test]
fn serde_rejects_v4_string_payload() {
    let payload = format!("\"{KNOWN_V4}\"");
    let res: Result<IrNodeId, _> = serde_json::from_str(&payload);
    assert!(res.is_err(), "v4 payload must fail deserialization");
}
