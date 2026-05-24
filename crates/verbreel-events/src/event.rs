//! events.jsonl line shape per Appendix C.
//!
//! Phase 2 events-integration (#31): extended from the v0 3-field
//! placeholder to the **Appendix C minimal-but-real** shape:
//!   `id, verb, ts, args, patch, warnings, idempotency_key, parent_event_id`
//!
//! `idempotency_key` and `parent_event_id` are wired structurally now but
//! their semantics (dedup index, retry chains) land in follow-up slices.
//! `warnings` may carry any `W_*` warnings emitted by verb-layer code
//! (JSON objects of shape `{ code: string, message: string, details?: any }`).
//!
//! Single-line invariant: serialize via `serde_json::to_string(&event)`
//! (NOT `to_string_pretty`) so the result contains no embedded `\n`.
//! [`crate::native::NativeBackend::append`] adds the trailing newline.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use verbreel_types::EventId;

use time::{OffsetDateTime, format_description::well_known::Rfc3339};

/// A single event log line — Appendix C minimal-but-real shape.
///
/// All eight fields are mandatory on the wire so consumers can parse
/// uniformly. Optional semantics carried via `Option<T>`:
/// `idempotency_key` and `parent_event_id` are `null` on the line when
/// absent; `warnings` is `[]` when empty.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Event {
    /// Strongly-typed `UuidV7` event id. Spec §0.3.
    pub id: EventId,

    /// Verb name, e.g. `"clip.add"`, `"project.set_metadata"`. Engine-
    /// layer registers known verbs; this crate keeps the field opaque.
    pub verb: String,

    /// RFC 3339 timestamp (`OffsetDateTime::now_utc()` formatted).
    pub ts: String,

    /// Verb args bag. Opaque at this slice — verb-arg typing lands per-
    /// verb in Phase 3. Empty `{}` is acceptable for verbs with no args.
    pub args: Value,

    /// The RFC 6902 JSON Patch the verb computed. Applied to the
    /// project graph in §0.8 step 3. Serializes as a JSON array.
    pub patch: json_patch::Patch,

    /// Warning codes emitted by the verb. `W_*` per spec.
    #[serde(default)]
    pub warnings: Vec<Value>,

    /// Idempotency token. `None` here; the in-memory dedup index that
    /// catches same-key retries is a follow-up slice (spec §0.8).
    #[serde(default)]
    pub idempotency_key: Option<String>,

    /// Parent event id when this event was emitted as part of a retry
    /// chain. `None` at this slice; chain semantics land with the
    /// idempotency index slice.
    #[serde(default)]
    pub parent_event_id: Option<EventId>,
}

impl Event {
    /// Construct a fresh event with `id = EventId::now()` and
    /// `ts = OffsetDateTime::now_utc().format(Rfc3339)`. `args` and
    /// `patch` come from the verb-layer; `warnings`, `idempotency_key`,
    /// `parent_event_id` default to empty / None.
    #[must_use]
    pub fn new(verb: impl Into<String>, args: Value, patch: json_patch::Patch) -> Self {
        Self {
            id: EventId::now(),
            verb: verb.into(),
            ts: timestamp_rfc3339_now(),
            args,
            patch,
            warnings: Vec::new(),
            idempotency_key: None,
            parent_event_id: None,
        }
    }
}

/// Builder for [`Event`] — convenience for partial-field construction.
#[derive(Debug, Default)]
pub struct EventBuilder {
    verb: Option<String>,
    args: Option<Value>,
    patch: Option<json_patch::Patch>,
    warnings: Option<Vec<Value>>,
    idempotency_key: Option<String>,
    parent_event_id: Option<EventId>,
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

    /// Set args (defaults to `serde_json::Value::Null` if unset).
    #[must_use]
    pub fn args(mut self, args: Value) -> Self {
        self.args = Some(args);
        self
    }

    /// Set patch (defaults to empty `Patch(vec![])` if unset).
    #[must_use]
    pub fn patch(mut self, patch: json_patch::Patch) -> Self {
        self.patch = Some(patch);
        self
    }

    /// Set warnings (defaults to empty `[]` if unset).
    #[must_use]
    pub fn warnings(mut self, warnings: Vec<Value>) -> Self {
        self.warnings = Some(warnings);
        self
    }

    /// Set idempotency key.
    #[must_use]
    pub fn idempotency_key(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(key.into());
        self
    }

    /// Set parent event id.
    #[must_use]
    pub fn parent_event_id(mut self, id: EventId) -> Self {
        self.parent_event_id = Some(id);
        self
    }

    /// Build the [`Event`] using current time for `ts` and a fresh `UuidV7` for `id`.
    #[must_use]
    pub fn build(self) -> Event {
        Event {
            id: EventId::now(),
            verb: self.verb.unwrap_or_default(),
            ts: timestamp_rfc3339_now(),
            args: self.args.unwrap_or(Value::Null),
            patch: self.patch.unwrap_or_else(|| json_patch::Patch(Vec::new())),
            warnings: self.warnings.unwrap_or_default(),
            idempotency_key: self.idempotency_key,
            parent_event_id: self.parent_event_id,
        }
    }
}

/// A line as serialized bytes (no trailing `\n`).
pub type EventLine = Vec<u8>;

/// RFC 3339 UTC timestamp from system time, via [`time::OffsetDateTime`].
///
/// Replaces the v0 placeholder which used a hand-rolled day-counter that
/// ignored leap years. The `time` crate handles every calendrical edge.
///
/// # Panics
///
/// Panics only if `time::OffsetDateTime::now_utc()` produces a value
/// that the RFC 3339 well-known format can't render — this is an
/// upstream invariant of the `time` crate (the format spec is a
/// compile-time constant, the date is always valid) and treated here
/// as a "would-indicate-an-upstream-bug" expectation rather than a
/// runtime error worth propagating.
#[must_use]
pub fn timestamp_rfc3339_now() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .expect("Rfc3339 format of OffsetDateTime::now_utc is infallible")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn event_serialization_roundtrip() {
        let ev = EventBuilder::new().verb("clip.add").build();
        let s = serde_json::to_string(&ev).unwrap();
        let back: Event = serde_json::from_str(&s).unwrap();
        assert_eq!(back.verb, "clip.add");
        assert!(back.warnings.is_empty());
        assert!(back.idempotency_key.is_none());
        assert!(back.parent_event_id.is_none());
    }

    #[test]
    fn event_builder_carries_warnings() {
        let warnings = vec![json!({"code": "W_TEST", "message": "..."})];
        let ev = EventBuilder::new()
            .verb("test")
            .warnings(warnings.clone())
            .build();

        assert_eq!(ev.warnings, warnings);
    }

    #[test]
    fn timestamp_is_rfc3339_shape() {
        let ts = timestamp_rfc3339_now();
        // RFC 3339 form: YYYY-MM-DDTHH:MM:SS[.fraction]Z or with offset
        assert!(ts.contains('T'), "ts must contain 'T' separator");
        // Either ends with Z or has a `+HH:MM`/`-HH:MM` offset.
        assert!(
            ts.ends_with('Z') || ts.matches(':').count() >= 3,
            "ts must end with Z or carry an offset: {ts}"
        );
        // Parses back via the `time` crate's Rfc3339.
        let _ = OffsetDateTime::parse(&ts, &Rfc3339).expect("ts must parse as RFC 3339");
    }

    #[test]
    fn event_serialized_form_has_no_embedded_newlines() {
        // Single-line invariant: serializing an Event with a non-trivial
        // patch must produce a single line. NativeBackend adds the
        // trailing newline; this string itself must have none.
        let patch: json_patch::Patch =
            serde_json::from_str(r#"[{"op":"replace","path":"/name","value":"x"}]"#).unwrap();
        let ev = Event::new(
            "project.set_metadata",
            serde_json::json!({"key":"value"}),
            patch,
        );
        let s = serde_json::to_string(&ev).unwrap();
        assert!(
            !s.contains('\n'),
            "serialized event must be a single line, got: {s:?}"
        );
    }
}
