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

// --- from_uuid: validating raw-Uuid constructor ---------------------------

#[test]
fn from_uuid_accepts_a_real_v7() {
    let raw = uuid::Uuid::now_v7();
    let id = IrNodeId::from_uuid(raw).expect("now_v7 must validate as v7");
    assert_eq!(id.as_uuid(), raw);
}

#[test]
fn from_uuid_rejects_nil_uuid() {
    let nil = uuid::Uuid::nil();
    let err = IrNodeId::from_uuid(nil).unwrap_err();
    // Don't pin exact variant — verbreel_types::id::IdError is the
    // source of truth; this test asserts only that nil is refused.
    assert!(
        !format!("{err:?}").is_empty(),
        "rejection must surface a non-empty error"
    );
}

#[test]
fn from_uuid_rejects_non_v7_versions() {
    // A v4 (random) UUID is the most common non-v7 input agents would
    // accidentally pass.
    let v4: uuid::Uuid = "550e8400-e29b-41d4-a716-446655440000"
        .parse()
        .expect("literal v4 must parse");
    let err = IrNodeId::from_uuid(v4).unwrap_err();
    assert!(
        !format!("{err:?}").is_empty(),
        "rejection must surface a non-empty error"
    );
}

// --- Ord: lexicographic-by-uuid-bytes ordering ---------------------------

#[test]
fn ord_equal_ids_compare_equal() {
    let id = IrNodeId::now();
    assert_eq!(id.cmp(&id), std::cmp::Ordering::Equal);
}

#[test]
fn ord_distinguishes_distinct_ids() {
    let a = IrNodeId::now();
    let b = IrNodeId::now();
    assert_ne!(a, b, "two now()-minted ids must differ");
    assert_ne!(a.cmp(&b), std::cmp::Ordering::Equal);
}

#[test]
fn ord_is_lexicographic_on_uuid_bytes() {
    // Construct two IrNodeIds from raw v7 UUIDs whose byte order is
    // known by construction — manually parse known v7 strings so the
    // test doesn't depend on the system clock.
    let lo_uuid: uuid::Uuid = "01900000-0000-7000-8000-000000000001"
        .parse()
        .expect("hardcoded UUID must parse");
    let hi_uuid: uuid::Uuid = "01900000-0000-7000-8000-000000000002"
        .parse()
        .expect("hardcoded UUID must parse");
    let lo = IrNodeId::from_uuid(lo_uuid).unwrap();
    let hi = IrNodeId::from_uuid(hi_uuid).unwrap();
    assert!(lo < hi, "lexicographic-byte order must place lo < hi");
    assert!(hi > lo);
    assert_eq!(lo.cmp(&hi), std::cmp::Ordering::Less);
    assert_eq!(hi.cmp(&lo), std::cmp::Ordering::Greater);
}

#[test]
fn ord_v7_timestamp_prefix_yields_monotonic_creation_order() {
    // UUIDv7 leads with a 48-bit Unix-ms timestamp. Two ids minted
    // sequentially must sort in creation order — Ord is monotonic
    // with respect to the wall clock at the millisecond grain.
    // Insert a tiny sleep to force the millisecond tick.
    let first = IrNodeId::now();
    std::thread::sleep(std::time::Duration::from_millis(2));
    let second = IrNodeId::now();
    assert!(
        first < second,
        "later-minted id must sort after earlier-minted: first={first} second={second}"
    );
}
