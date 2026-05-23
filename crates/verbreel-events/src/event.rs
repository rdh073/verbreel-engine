//! events.jsonl line shape per Appendix C.
//!
//! v0 placeholder: just enough to make the backend trait usable. The full
//! shape (`idempotency_key`, `parent_event_id`, `warnings`, `patch`) comes in
//! follow-up issues once the state crate lands.

use serde::{Deserialize, Serialize};
use verbreel_types::EventId;

/// A single event log line — Appendix C minimal shape.
///
/// More fields are added as verbs land. The id + verb + ts triple is the
/// stable subset every event carries.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Event {
    /// `UuidV7` event id.
    pub id: EventId,
    /// Verb name, e.g. `"clip.add"`.
    pub verb: String,
    /// RFC 3339 timestamp.
    pub ts: String,
}

/// Builder for [`Event`]. Stub — completes once we have verb-args types.
#[derive(Debug, Default)]
pub struct EventBuilder {
    verb: Option<String>,
}

impl EventBuilder {
    /// New empty builder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set verb name.
    #[must_use]
    pub fn verb(mut self, v: impl Into<String>) -> Self {
        self.verb = Some(v.into());
        self
    }

    /// Build the [`Event`] using current time for `ts` and a fresh `UuidV7` for `id`.
    #[must_use]
    pub fn build(self) -> Event {
        Event {
            id: EventId::now(),
            verb: self.verb.unwrap_or_default(),
            ts: timestamp_rfc3339_now(),
        }
    }
}

/// A line as serialized bytes (no trailing `\n`).
pub type EventLine = Vec<u8>;

/// RFC 3339 timestamp from system time, using only `std`. Stub — switch to
/// `time` crate when state crate lands and needs more sophistication.
fn timestamp_rfc3339_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    format_unix_secs(secs)
}

fn format_unix_secs(secs: u64) -> String {
    // Minimal correct ISO 8601 for UTC. Good enough for a v0 placeholder.
    // Switch to `time` crate when verb-args land — leap-year handling is
    // deliberately skipped here; this string is for shape compatibility only.
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let h = rem / 3600;
    let m = (rem % 3600) / 60;
    let s = rem % 60;
    let year = 1970 + days / 365;
    format!("{year:04}-01-01T{h:02}:{m:02}:{s:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_serialization_roundtrip() {
        let ev = EventBuilder::new().verb("clip.add").build();
        let s = serde_json::to_string(&ev).unwrap();
        let back: Event = serde_json::from_str(&s).unwrap();
        assert_eq!(back.verb, "clip.add");
    }

    #[test]
    fn timestamp_is_iso8601_shape() {
        let ts = timestamp_rfc3339_now();
        assert!(ts.ends_with('Z'));
        assert!(ts.contains('T'));
        assert_eq!(ts.len(), 20); // YYYY-MM-DDTHH:MM:SSZ
    }
}
