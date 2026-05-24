//! §0.8 in-memory idempotency dedup index.
//!
//! Phase 2 idempotency-dedup-index slice (#53). This module owns the
//! **in-memory working state** half of the §0.8 idempotency contract:
//!
//! - **Source of truth** is still `events.jsonl` — the events file
//!   carries every event's `idempotency_key`, `args`, and `patch`. It
//!   is append-only and never pruned.
//! - **Working state** is a `HashMap<key, Entry>` rebuilt on
//!   `project.open` from a single scan of `events.jsonl`. Each entry
//!   tracks the verb-args fingerprint, the current state
//!   ([`EntryState::InProgress`] or [`EntryState::Completed`]), and the
//!   wall-clock timestamp it was inserted.
//!
//! Two TTLs apply at lookup + eviction time:
//! - **24h TTL** — entries older than `ttl` are treated as
//!   [`LookupOutcome::Expired`] and may be safely overwritten by a new
//!   first-call execution.
//! - **5-min stale-`in_progress`** — an [`EntryState::InProgress`]
//!   entry older than `stale_in_progress` is also treated as
//!   [`LookupOutcome::Expired`] (the original holder crashed before
//!   transitioning to [`EntryState::Completed`]; the slot is reusable).
//!
//! ## Scope of this slice
//!
//! In-memory index core only — insert, lookup, state transitions, TTL
//! eviction, and the rebuild-from-events scan. The pieces deliberately
//! deferred to follow-up slices are documented at the `verbreel-state`
//! crate root:
//! - Step (2) failure recovery + events.jsonl tail-scan window.
//! - `~/.verbreel/idempotency.json` global file (only used by
//!   `project.create` / `project.duplicate` which don't exist yet).
//! - Per-verb reconstructor purity startup validation.
//! - Replay envelope `data` reconstruction — [`LookupOutcome::Completed`]
//!   here carries the [`EventId`] and lets the caller re-fetch the
//!   original event for itself.
//!
//! ## Concurrency
//!
//! The index uses interior mutability (`Mutex<HashMap>`) so a future
//! concurrent-mutators scenario only has to wrap the index in `Arc`
//! without re-shaping the API. For this slice, single-thread access
//! through [`ProjectStore`] is the only call pattern.
//!
//! ## Clock injection
//!
//! TTL semantics are driven through a [`Clock`] trait so tests can
//! advance time deterministically with [`MockClock`] instead of
//! `std::thread::sleep`. Production uses [`SystemClock`] (wraps
//! `SystemTime::now`).
//!
//! [`ProjectStore`]: crate::lifecycle::ProjectStore

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use verbreel_events::Event;
use verbreel_types::EventId;

/// Default §0.8 24-hour entry TTL.
///
/// After 24h, both `InProgress` and `Completed` entries are
/// [`LookupOutcome::Expired`] and may be safely replaced.
pub const DEFAULT_TTL: Duration = Duration::from_hours(24);

/// Default §0.8 5-minute stale-`in_progress` timer.
///
/// An [`EntryState::InProgress`] entry older than this is treated as
/// abandoned (the original holder crashed mid-execution) and the slot
/// is reusable by the next first-call execution.
pub const DEFAULT_STALE_IN_PROGRESS: Duration = Duration::from_mins(5);

// ---------------------------------------------------------------------
// Clock
// ---------------------------------------------------------------------

/// Wall-clock abstraction for TTL bookkeeping.
///
/// `Send + Sync` so the index can sit behind an `Arc` in a future
/// concurrent-mutators scenario. Production uses [`SystemClock`]; tests
/// use [`MockClock`] to advance time deterministically.
pub trait Clock: Send + Sync {
    /// Return "now" as a [`SystemTime`].
    fn now(&self) -> SystemTime;
}

/// Production [`Clock`] — wraps [`SystemTime::now`].
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }
}

/// Test-only deterministic [`Clock`].
///
/// Initialized with a [`SystemTime`]; [`Self::advance`] moves it
/// forward by a [`Duration`]. Internally a `Mutex<SystemTime>` so the
/// `&self` API satisfies [`Clock::now`].
#[derive(Debug)]
pub struct MockClock {
    time: Mutex<SystemTime>,
}

impl MockClock {
    /// New [`MockClock`] starting at `start`.
    #[must_use]
    pub fn new(start: SystemTime) -> Self {
        Self {
            time: Mutex::new(start),
        }
    }

    /// New [`MockClock`] starting at [`SystemTime::UNIX_EPOCH`].
    #[must_use]
    pub fn unix_epoch() -> Self {
        Self::new(SystemTime::UNIX_EPOCH)
    }

    /// Advance the clock by `by`.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned — which can only happen
    /// if a previous caller panicked while holding the lock; tests fail
    /// hard rather than silently using a stale time.
    pub fn advance(&self, by: Duration) {
        let mut t = self.time.lock().expect("MockClock mutex poisoned");
        *t += by;
    }
}

impl Clock for MockClock {
    fn now(&self) -> SystemTime {
        *self.time.lock().expect("MockClock mutex poisoned")
    }
}

// ---------------------------------------------------------------------
// Entry / EntryState / LookupOutcome
// ---------------------------------------------------------------------

/// State half of an [`Entry`]. `InProgress` is held only in memory;
/// `Completed` is the form that gets mirrored from `events.jsonl` on
/// rebuild.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryState {
    /// First call is mid-flight. The slot is reserved; concurrent calls
    /// with the same key see [`LookupOutcome::InProgress`].
    InProgress,
    /// First call completed and emitted `event_id`. Re-fetch the event
    /// from `events.jsonl` for replay envelope reconstruction (deferred
    /// slice).
    Completed {
        /// Id of the event the first call emitted.
        event_id: EventId,
    },
}

/// One row of the index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// `sha256_hex(canonicalize(args))` — see [`verbreel_canon`]. Used
    /// to distinguish "same key, same args" (replay) from "same key,
    /// different args" (conflict).
    pub fingerprint: String,
    /// Current state.
    pub state: EntryState,
    /// Wall-clock instant the entry was inserted — drives TTL +
    /// stale-`in_progress` checks at lookup + eviction time.
    pub created_at: SystemTime,
}

/// Result of [`IdempotencyIndex::lookup`]. TTL handling is folded in:
/// any entry past `ttl` (`Completed`) or `stale_in_progress`
/// (`InProgress`) surfaces as [`Self::Expired`] rather than its true
/// state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LookupOutcome {
    /// No entry for the key — first-call path.
    Absent,
    /// Same-key call is mid-flight — caller must surface `E_BUSY` /
    /// `IdempotencyBusy`.
    InProgress,
    /// Same key + same fingerprint, first call completed — caller
    /// returns the cached envelope (replay path).
    Completed {
        /// Id of the event the first call emitted.
        event_id: EventId,
    },
    /// Same key, **different** fingerprint — caller must surface
    /// `E_IDEMPOTENCY_CONFLICT` / `IdempotencyConflict`.
    ConflictingFingerprint {
        /// The fingerprint already on file (the original first call's).
        existing_fingerprint: String,
    },
    /// Entry is past its TTL (`Completed`) or stale-`in_progress`
    /// timer (`InProgress`) — caller treats it as [`Self::Absent`] and
    /// may safely overwrite.
    Expired,
}

// ---------------------------------------------------------------------
// AlreadyExists
// ---------------------------------------------------------------------

/// [`IdempotencyIndex::start`] tried to insert a key that already has a
/// non-expired entry.
///
/// Surfaced when the caller didn't check [`IdempotencyIndex::lookup`]
/// first or raced another caller. Carries the existing entry's state
/// so the caller can decide whether to retry as a busy / conflict /
/// replay.
#[derive(Debug, thiserror::Error)]
#[error("idempotency key {key:?} already in the index ({existing_state:?})")]
pub struct AlreadyExists {
    /// Key that was already present.
    pub key: String,
    /// State of the existing entry — `InProgress` or `Completed { .. }`.
    pub existing_state: EntryState,
}

// ---------------------------------------------------------------------
// IdempotencyIndex
// ---------------------------------------------------------------------

/// In-memory `key → Entry` map with §0.8 TTL semantics.
///
/// Interior mutability via `Mutex<HashMap>` so the index can sit
/// behind an `Arc` in a future concurrent-mutators scenario. For this
/// slice, all access goes through [`crate::lifecycle::ProjectStore`]
/// on a single thread.
pub struct IdempotencyIndex {
    entries: Mutex<HashMap<String, Entry>>,
    ttl: Duration,
    stale_in_progress: Duration,
    clock: Box<dyn Clock>,
}

impl std::fmt::Debug for IdempotencyIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let entries = self.entries.lock().map(|m| m.len()).unwrap_or_default();
        // Skip the clock — it's a trait object that doesn't implement
        // Debug, and the type identity is irrelevant for diagnostics.
        f.debug_struct("IdempotencyIndex")
            .field("entries", &entries)
            .field("ttl", &self.ttl)
            .field("stale_in_progress", &self.stale_in_progress)
            .finish_non_exhaustive()
    }
}

impl Default for IdempotencyIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl IdempotencyIndex {
    /// Build an empty index with spec-default TTLs ([`DEFAULT_TTL`],
    /// [`DEFAULT_STALE_IN_PROGRESS`]) and a [`SystemClock`].
    #[must_use]
    pub fn new() -> Self {
        Self::with_clock_and_ttl(
            Box::new(SystemClock),
            DEFAULT_TTL,
            DEFAULT_STALE_IN_PROGRESS,
        )
    }

    /// Build an empty index with custom TTLs (testing knob).
    ///
    /// Uses [`SystemClock`] — tests that need deterministic time
    /// should use [`Self::with_clock_and_ttl`] instead.
    #[must_use]
    pub fn with_ttl(ttl: Duration, stale_in_progress: Duration) -> Self {
        Self::with_clock_and_ttl(Box::new(SystemClock), ttl, stale_in_progress)
    }

    /// Build an empty index with a custom clock + TTLs (fully
    /// deterministic — preferred for tests).
    #[must_use]
    pub fn with_clock_and_ttl(
        clock: Box<dyn Clock>,
        ttl: Duration,
        stale_in_progress: Duration,
    ) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            ttl,
            stale_in_progress,
            clock,
        }
    }

    /// Number of entries currently in the index (post-eviction state
    /// not re-computed — call [`Self::evict_expired`] first for an
    /// authoritative live count).
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned — only possible if a
    /// previous caller panicked while holding the lock, which is a
    /// programmer error worth surfacing.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries
            .lock()
            .expect("IdempotencyIndex mutex poisoned")
            .len()
    }

    /// True when the index has zero entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Look up `key` and classify the result for the verb-layer's
    /// "six return paths" decision.
    ///
    /// TTL semantics applied here (before the conflict / replay
    /// classification):
    /// - Entry older than `ttl` → [`LookupOutcome::Expired`].
    /// - [`EntryState::InProgress`] older than `stale_in_progress` →
    ///   [`LookupOutcome::Expired`] (the original holder crashed).
    ///
    /// Read-only — does not evict. Callers that want the storage
    /// reclaimed should invoke [`Self::evict_expired`] periodically.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    #[must_use]
    pub fn lookup(&self, key: &str, fingerprint: &str) -> LookupOutcome {
        let entries = self
            .entries
            .lock()
            .expect("IdempotencyIndex mutex poisoned");
        let Some(entry) = entries.get(key) else {
            return LookupOutcome::Absent;
        };
        if self.is_expired(entry) {
            return LookupOutcome::Expired;
        }
        if entry.fingerprint != fingerprint {
            return LookupOutcome::ConflictingFingerprint {
                existing_fingerprint: entry.fingerprint.clone(),
            };
        }
        match &entry.state {
            EntryState::InProgress => LookupOutcome::InProgress,
            EntryState::Completed { event_id } => LookupOutcome::Completed {
                event_id: *event_id,
            },
        }
    }

    /// Insert an [`EntryState::InProgress`] entry for `key`.
    ///
    /// Used by the first-call path between §0.8 step 1 (validate) and
    /// step 2 (write the event). If the slot is already populated by a
    /// non-expired entry, returns [`AlreadyExists`] so the caller can
    /// re-classify via [`Self::lookup`].
    ///
    /// An expired entry occupying the slot is silently overwritten —
    /// that's the §0.8 "Stale `in_progress` / 24h TTL aged out" path.
    ///
    /// # Errors
    ///
    /// Returns [`AlreadyExists`] when the slot is occupied by a
    /// non-expired entry.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    pub fn start(&self, key: String, fingerprint: String) -> Result<(), AlreadyExists> {
        let mut entries = self
            .entries
            .lock()
            .expect("IdempotencyIndex mutex poisoned");
        if let Some(existing) = entries.get(&key)
            && !self.is_expired(existing)
        {
            return Err(AlreadyExists {
                key,
                existing_state: existing.state.clone(),
            });
        }
        // Either absent or expired — insert/overwrite.
        entries.insert(
            key,
            Entry {
                fingerprint,
                state: EntryState::InProgress,
                created_at: self.clock.now(),
            },
        );
        Ok(())
    }

    /// Transition an existing [`EntryState::InProgress`] entry to
    /// [`EntryState::Completed`] with `event_id`.
    ///
    /// Used by the first-call path after §0.8 step 3 (apply) succeeds.
    /// Silent no-op when the key is absent or already completed — the
    /// caller has already lost the race to a concurrent
    /// [`Self::abort`] / [`Self::evict_expired`] / another `complete`,
    /// and there's nothing useful to do beyond leaving the slot in its
    /// observed state.
    ///
    /// `created_at` is refreshed to `now` so the 24h TTL starts from
    /// the completion, not the original `start`.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    pub fn complete(&self, key: &str, event_id: EventId) {
        let mut entries = self
            .entries
            .lock()
            .expect("IdempotencyIndex mutex poisoned");
        if let Some(entry) = entries.get_mut(key) {
            entry.state = EntryState::Completed { event_id };
            entry.created_at = self.clock.now();
        }
    }

    /// Remove `key` from the index.
    ///
    /// Used by the first-call path when §0.8 step 2 (event write) or
    /// step 3 (apply) fails — the reserved `InProgress` slot must be
    /// released so retries can proceed. Silent no-op when the key is
    /// absent.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    pub fn abort(&self, key: &str) {
        let mut entries = self
            .entries
            .lock()
            .expect("IdempotencyIndex mutex poisoned");
        entries.remove(key);
    }

    /// Drop every expired entry from the map. Returns the count
    /// removed.
    ///
    /// Cheap enough to call after every mutation — the typical index
    /// size is small (< 1k entries) and the comparison is a single
    /// `SystemTime` subtract.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    pub fn evict_expired(&self) -> usize {
        let mut entries = self
            .entries
            .lock()
            .expect("IdempotencyIndex mutex poisoned");
        let before = entries.len();
        entries.retain(|_, e| !is_expired_impl(e, &*self.clock, self.ttl, self.stale_in_progress));
        before - entries.len()
    }

    /// Rebuild the index from an events.jsonl scan.
    ///
    /// Walks `events` in chronological order — for each event with a
    /// non-`None` `idempotency_key`, computes the args fingerprint via
    /// [`verbreel_canon::sha256_hex`] and inserts an
    /// [`EntryState::Completed`] entry. Later events with the same key
    /// overwrite earlier ones (events.jsonl is the source of truth;
    /// the last write wins).
    ///
    /// Events whose `args` aren't canonicalizable (e.g. carry a
    /// non-finite number — see [`verbreel_canon::CanonError`]) are
    /// skipped with a `tracing::warn!`. A malformed historical event
    /// shouldn't prevent the engine from opening; the verb-layer
    /// emits canonicalizable args, so the only way this triggers is
    /// the events.jsonl being hand-edited.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    pub fn rebuild_from_events(&self, events: &[Event]) {
        let mut entries = self
            .entries
            .lock()
            .expect("IdempotencyIndex mutex poisoned");
        for event in events {
            let Some(key) = &event.idempotency_key else {
                continue;
            };
            let fingerprint = match verbreel_canon::sha256_hex(&event.args) {
                Ok(fp) => fp,
                Err(err) => {
                    tracing::warn!(
                        key = %key,
                        event_id = %event.id,
                        error = %err,
                        "idempotency: skipping event with non-canonicalizable args during rebuild",
                    );
                    continue;
                }
            };
            entries.insert(
                key.clone(),
                Entry {
                    fingerprint,
                    state: EntryState::Completed { event_id: event.id },
                    created_at: self.clock.now(),
                },
            );
        }
    }

    /// True when `entry` is past TTL ([`EntryState::Completed`]) or
    /// past the stale-`in_progress` timer ([`EntryState::InProgress`]).
    fn is_expired(&self, entry: &Entry) -> bool {
        is_expired_impl(entry, &*self.clock, self.ttl, self.stale_in_progress)
    }
}

/// Free-function form of the expiry check — needed because
/// [`HashMap::retain`] takes a `&mut self` closure that can't
/// re-borrow `&self` through [`IdempotencyIndex::is_expired`].
fn is_expired_impl(
    entry: &Entry,
    clock: &dyn Clock,
    ttl: Duration,
    stale_in_progress: Duration,
) -> bool {
    // `created_at` is in the future relative to "now" — clock moved
    // backwards (NTP correction). Treat as not expired so we don't
    // accidentally drop a fresh entry.
    let Ok(age) = clock.now().duration_since(entry.created_at) else {
        return false;
    };
    if age > ttl {
        return true;
    }
    matches!(entry.state, EntryState::InProgress) && age > stale_in_progress
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_event_id() -> EventId {
        EventId::now()
    }

    #[test]
    fn new_is_empty() {
        let idx = IdempotencyIndex::new();
        assert!(idx.is_empty());
        assert_eq!(idx.len(), 0);
    }

    #[test]
    fn lookup_absent_returns_absent() {
        let idx = IdempotencyIndex::new();
        assert_eq!(idx.lookup("k", "fp"), LookupOutcome::Absent);
    }

    #[test]
    fn start_then_lookup_in_progress() {
        let idx = IdempotencyIndex::new();
        idx.start("k".into(), "fp".into()).unwrap();
        assert_eq!(idx.lookup("k", "fp"), LookupOutcome::InProgress);
    }

    #[test]
    fn start_then_complete_then_lookup_completed() {
        let idx = IdempotencyIndex::new();
        idx.start("k".into(), "fp".into()).unwrap();
        let eid = fake_event_id();
        idx.complete("k", eid);
        assert_eq!(
            idx.lookup("k", "fp"),
            LookupOutcome::Completed { event_id: eid }
        );
    }

    #[test]
    fn start_twice_errors() {
        let idx = IdempotencyIndex::new();
        idx.start("k".into(), "fp".into()).unwrap();
        let err = idx
            .start("k".into(), "fp".into())
            .expect_err("second start must fail");
        assert_eq!(err.key, "k");
        assert_eq!(err.existing_state, EntryState::InProgress);
    }
}
