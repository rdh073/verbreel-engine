//! Regression tests for issue #382 — the single typed timestamp
//! boundary.
//!
//! Acceptance criterion (verbatim): "Timestamp construction and event
//! data serialization flow through one typed newtype boundary."
//!
//! These tests pin both halves of that criterion:
//!   1. **construction** validates/normalizes — a malformed RFC 3339
//!      string is rejected at construction (and at deserialize), and a
//!      valid one is accepted.
//!   2. **event-data serialization** goes through the newtype — the
//!      `Event.ts` field is a `Timestamp`, it serializes as a plain
//!      JSON string, and a malformed `ts` on the wire fails to
//!      deserialize at the boundary rather than landing in state.

use verbreel_events::{Event, Timestamp};

#[test]
fn malformed_timestamp_rejected_at_construction() {
    assert!(
        Timestamp::parse("2026-99-99T99:99:99Z").is_err(),
        "out-of-range RFC 3339 must be rejected at construction"
    );
    assert!(
        Timestamp::parse("not a date").is_err(),
        "non-date string must be rejected at construction"
    );
    assert!(
        Timestamp::parse("2026-05-29").is_err(),
        "date-only (no time component) is not RFC 3339 date-time"
    );
}

#[test]
fn valid_timestamp_constructs_and_round_trips_canonically() {
    let ts = Timestamp::parse("2026-05-29T12:34:56Z").expect("valid RFC 3339");
    let json = serde_json::to_string(&ts).expect("serialize");
    assert_eq!(
        json, "\"2026-05-29T12:34:56Z\"",
        "serializes as a bare string"
    );
    let back: Timestamp = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(
        back, ts,
        "round-trips byte-identically through the boundary"
    );
}

#[test]
fn now_flows_through_the_boundary() {
    // Construction via Timestamp::now is the single "current time" path;
    // its output must itself satisfy the validating constructor.
    let now = Timestamp::now();
    let reparsed = Timestamp::parse(now.as_str()).expect("now() output is valid RFC 3339");
    assert_eq!(now, reparsed);
}

#[test]
fn event_ts_serializes_through_typed_boundary() {
    let ev = Event::new(
        "clip.add",
        serde_json::json!({}),
        json_patch::Patch(Vec::new()),
    );
    let value = serde_json::to_value(&ev).expect("serialize event");
    let ts = value
        .get("ts")
        .and_then(serde_json::Value::as_str)
        .expect("event ts serializes as a JSON string");
    // The serialized ts must itself be a valid Timestamp — i.e. event
    // data cannot carry an un-normalized RFC 3339 value.
    Timestamp::parse(ts).expect("event ts on the wire is valid RFC 3339");
}

#[test]
fn event_with_malformed_ts_fails_to_deserialize_at_boundary() {
    // A hand-crafted event line carrying a garbage `ts` must be rejected
    // when parsed back into an `Event`, not silently accepted — the
    // typed `ts` field guards the deserialize seam.
    let line = r#"{
        "id": "0190b8d3-15e3-7000-bd00-000000000001",
        "verb": "clip.add",
        "ts": "garbage-not-a-timestamp",
        "args": {},
        "patch": [],
        "warnings": [],
        "idempotency_key": null,
        "parent_event_id": null
    }"#;
    let parsed = serde_json::from_str::<Event>(line);
    assert!(
        parsed.is_err(),
        "malformed ts must be rejected at the deserialize boundary"
    );
}

#[test]
fn event_with_valid_ts_deserializes() {
    let line = r#"{
        "id": "0190b8d3-15e3-7000-bd00-000000000001",
        "verb": "clip.add",
        "ts": "2026-05-29T12:34:56Z",
        "args": {},
        "patch": [],
        "warnings": [],
        "idempotency_key": null,
        "parent_event_id": null
    }"#;
    let ev: Event = serde_json::from_str(line).expect("valid event line deserializes");
    assert_eq!(ev.ts.as_str(), "2026-05-29T12:34:56Z");
}
