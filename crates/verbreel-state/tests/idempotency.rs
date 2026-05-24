//! Unit tests for [`verbreel_state::IdempotencyIndex`] — the §0.8
//! in-memory dedup index.
//!
//! Time-based tests use [`MockClock`] + a small explicit TTL so they
//! run deterministically without `sleep`. Real-time tests are
//! forbidden here: CI flakes are not an acceptable cost for a clock
//! abstraction we already have.

use std::time::{Duration, SystemTime};

use verbreel_events::Event;
use verbreel_state::{EntryState, IdempotencyIndex, LookupOutcome, MockClock};
use verbreel_types::EventId;

fn fake_event_id() -> EventId {
    EventId::now()
}

// ---------------------------------------------------------------------
// Basics
// ---------------------------------------------------------------------

#[test]
fn idempotency_index_new_is_empty() {
    let idx = IdempotencyIndex::new();
    assert!(idx.is_empty());
    assert_eq!(idx.len(), 0);
}

#[test]
fn idempotency_index_lookup_absent() {
    let idx = IdempotencyIndex::new();
    assert_eq!(idx.lookup("missing-key", "fp"), LookupOutcome::Absent);
}

#[test]
fn idempotency_index_start_then_lookup_in_progress() {
    let idx = IdempotencyIndex::new();
    idx.start("k".into(), "fp".into()).unwrap();
    assert_eq!(idx.len(), 1);
    assert_eq!(idx.lookup("k", "fp"), LookupOutcome::InProgress);
}

#[test]
fn idempotency_index_start_then_complete_then_lookup_completed() {
    let idx = IdempotencyIndex::new();
    let eid = fake_event_id();
    idx.start("k".into(), "fp".into()).unwrap();
    idx.complete("k", eid);
    assert_eq!(
        idx.lookup("k", "fp"),
        LookupOutcome::Completed { event_id: eid }
    );
}

#[test]
fn idempotency_index_start_twice_errors() {
    let idx = IdempotencyIndex::new();
    idx.start("k".into(), "fp".into()).unwrap();
    let err = idx
        .start("k".into(), "fp".into())
        .expect_err("second start must fail");
    assert_eq!(err.key, "k");
    assert_eq!(err.existing_state, EntryState::InProgress);
}

#[test]
fn idempotency_index_complete_without_start_is_silent_noop() {
    // Spec-defined: `complete` on an absent key is a silent no-op so
    // the caller doesn't have to special-case the race where another
    // path (abort, evict_expired) just removed the entry. Verify the
    // index stays empty and a subsequent lookup returns Absent.
    let idx = IdempotencyIndex::new();
    idx.complete("never-started", fake_event_id());
    assert!(idx.is_empty());
    assert_eq!(idx.lookup("never-started", "fp"), LookupOutcome::Absent);
}

#[test]
fn idempotency_index_lookup_conflicting_fingerprint() {
    let idx = IdempotencyIndex::new();
    idx.start("k".into(), "fp_A".into()).unwrap();
    idx.complete("k", fake_event_id());
    match idx.lookup("k", "fp_B") {
        LookupOutcome::ConflictingFingerprint {
            existing_fingerprint,
        } => assert_eq!(existing_fingerprint, "fp_A"),
        other => panic!("expected ConflictingFingerprint, got {other:?}"),
    }
}

#[test]
fn idempotency_index_abort_removes_entry() {
    let idx = IdempotencyIndex::new();
    idx.start("k".into(), "fp".into()).unwrap();
    assert_eq!(idx.len(), 1);
    idx.abort("k");
    assert!(idx.is_empty());
    assert_eq!(idx.lookup("k", "fp"), LookupOutcome::Absent);
}

// ---------------------------------------------------------------------
// TTL + stale-in_progress
// ---------------------------------------------------------------------

#[test]
fn idempotency_index_evict_expired_removes_old_entries() {
    // 1-second TTL, 100ms stale-in_progress. MockClock advance pushes
    // the completed entry past TTL.
    let clock_handle = std::sync::Arc::new(MockClock::new(SystemTime::UNIX_EPOCH));
    let idx = IdempotencyIndex::with_clock_and_ttl(
        Box::new(SharedMockClock(clock_handle.clone())),
        Duration::from_secs(1),
        Duration::from_millis(100),
    );

    idx.start("k1".into(), "fp".into()).unwrap();
    idx.complete("k1", fake_event_id());

    // Move past TTL.
    clock_handle.advance(Duration::from_secs(2));
    let removed = idx.evict_expired();
    assert_eq!(removed, 1, "TTL-expired completed entry must be evicted");
    assert!(idx.is_empty());
}

#[test]
fn idempotency_index_evict_expired_keeps_fresh() {
    let clock_handle = std::sync::Arc::new(MockClock::new(SystemTime::UNIX_EPOCH));
    let idx = IdempotencyIndex::with_clock_and_ttl(
        Box::new(SharedMockClock(clock_handle.clone())),
        Duration::from_secs(10),
        Duration::from_secs(5),
    );

    idx.start("fresh".into(), "fp".into()).unwrap();
    idx.complete("fresh", fake_event_id());

    // 1s elapsed — well within both TTLs.
    clock_handle.advance(Duration::from_secs(1));
    assert_eq!(idx.evict_expired(), 0);
    assert_eq!(idx.len(), 1);
}

#[test]
fn idempotency_index_stale_in_progress_treated_as_expired() {
    // InProgress entries past the 5-min (here 100ms) stale timer
    // surface as Expired on lookup even though they're well within
    // the global TTL.
    let clock_handle = std::sync::Arc::new(MockClock::new(SystemTime::UNIX_EPOCH));
    let idx = IdempotencyIndex::with_clock_and_ttl(
        Box::new(SharedMockClock(clock_handle.clone())),
        Duration::from_secs(10),    // generous global TTL
        Duration::from_millis(100), // tight stale-in_progress
    );

    idx.start("stuck".into(), "fp".into()).unwrap();
    assert_eq!(idx.lookup("stuck", "fp"), LookupOutcome::InProgress);

    // Push past the stale timer but well inside the global TTL.
    clock_handle.advance(Duration::from_millis(500));
    assert_eq!(
        idx.lookup("stuck", "fp"),
        LookupOutcome::Expired,
        "stale in_progress entry surfaces as Expired",
    );

    // start() on the same key must succeed now — the slot is free.
    idx.start("stuck".into(), "fp".into())
        .expect("expired slot must be overwritable");
    assert_eq!(idx.lookup("stuck", "fp"), LookupOutcome::InProgress);
}

#[test]
fn idempotency_index_start_overwrites_ttl_expired_completed() {
    // Step (6) of the §0.8 "six paths": a key whose entry is past TTL
    // behaves like the first-call path — `start()` succeeds and the
    // old entry is replaced.
    let clock_handle = std::sync::Arc::new(MockClock::new(SystemTime::UNIX_EPOCH));
    let idx = IdempotencyIndex::with_clock_and_ttl(
        Box::new(SharedMockClock(clock_handle.clone())),
        Duration::from_millis(100),
        Duration::from_millis(10),
    );

    idx.start("aged".into(), "fp_old".into()).unwrap();
    idx.complete("aged", fake_event_id());

    clock_handle.advance(Duration::from_millis(500));
    // Aged out — lookup says Expired and start() succeeds.
    assert_eq!(idx.lookup("aged", "fp_old"), LookupOutcome::Expired);
    idx.start("aged".into(), "fp_new".into())
        .expect("expired entry must be overwritable");
    assert_eq!(idx.lookup("aged", "fp_new"), LookupOutcome::InProgress);
}

// ---------------------------------------------------------------------
// Rebuild from events
// ---------------------------------------------------------------------

#[test]
fn idempotency_index_rebuild_from_events_populates_completed() {
    // Build a small events slice: one event with a key, one without,
    // one with a different key. Rebuild and confirm the two keyed
    // entries land as Completed with the correct fingerprints.
    let patch: json_patch::Patch =
        serde_json::from_str(r#"[{"op":"replace","path":"/name","value":"x"}]"#).unwrap();

    let mut ev_a = Event::new(
        "project.set_name",
        serde_json::json!({"name":"A"}),
        patch.clone(),
    );
    ev_a.idempotency_key = Some("key-A".into());

    let mut ev_unkeyed = Event::new(
        "project.set_name",
        serde_json::json!({"name":"unkeyed"}),
        patch.clone(),
    );
    // No idempotency_key set — must not land in the index.
    ev_unkeyed.idempotency_key = None;

    let mut ev_b = Event::new("project.set_name", serde_json::json!({"name":"B"}), patch);
    ev_b.idempotency_key = Some("key-B".into());

    let events = vec![ev_a.clone(), ev_unkeyed, ev_b.clone()];

    let idx = IdempotencyIndex::new();
    idx.rebuild_from_events(&events);

    assert_eq!(idx.len(), 2, "two keyed events → two entries");

    let fp_a = verbreel_canon::sha256_hex(&ev_a.args).unwrap();
    let fp_b = verbreel_canon::sha256_hex(&ev_b.args).unwrap();

    assert_eq!(
        idx.lookup("key-A", &fp_a),
        LookupOutcome::Completed { event_id: ev_a.id }
    );
    assert_eq!(
        idx.lookup("key-B", &fp_b),
        LookupOutcome::Completed { event_id: ev_b.id }
    );
    // Mismatched fingerprint surfaces as conflict.
    assert!(matches!(
        idx.lookup("key-A", "deadbeef"),
        LookupOutcome::ConflictingFingerprint { .. }
    ));
}

#[test]
fn idempotency_index_rebuild_last_write_wins() {
    // Same key emitted twice across events.jsonl — the later event's
    // fingerprint + event_id win. Mirrors the §0.8 "events.jsonl is
    // the source of truth, last write wins" invariant.
    let patch: json_patch::Patch =
        serde_json::from_str(r#"[{"op":"replace","path":"/name","value":"x"}]"#).unwrap();

    let mut ev_first = Event::new(
        "project.set_name",
        serde_json::json!({"v":1}),
        patch.clone(),
    );
    ev_first.idempotency_key = Some("key".into());

    let mut ev_second = Event::new("project.set_name", serde_json::json!({"v":2}), patch);
    ev_second.idempotency_key = Some("key".into());

    let idx = IdempotencyIndex::new();
    idx.rebuild_from_events(&[ev_first, ev_second.clone()]);

    let fp_second = verbreel_canon::sha256_hex(&ev_second.args).unwrap();
    assert_eq!(
        idx.lookup("key", &fp_second),
        LookupOutcome::Completed {
            event_id: ev_second.id
        }
    );
}

// ---------------------------------------------------------------------
// SharedMockClock — wrapper so the same MockClock can be shared
// between the test (which advances it) and the index (which only
// reads). Box<dyn Clock> doesn't let us reach back into the index's
// clock, so we wrap an Arc<MockClock>.
// ---------------------------------------------------------------------

struct SharedMockClock(std::sync::Arc<MockClock>);

impl verbreel_state::Clock for SharedMockClock {
    fn now(&self) -> SystemTime {
        self.0.now()
    }
}
