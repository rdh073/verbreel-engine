//! [`Timestamp`] — the one typed boundary for RFC 3339 timestamps.
//!
//! Issue #382: before this slice, 50+ call-sites threaded raw
//! `String` RFC 3339 values into event data and project-graph state.
//! Nothing forced those strings to actually *be* RFC 3339, and nothing
//! stopped an un-normalized value from reaching canonical event data.
//!
//! [`Timestamp`] closes that gap. It is the single place that:
//!
//! - **constructs** the wall-clock value ([`Timestamp::now`]), so every
//!   "current time" call goes through `time::OffsetDateTime::now_utc`
//!   formatted as RFC 3339 once;
//! - **validates** any string that claims to be a timestamp
//!   ([`Timestamp::parse`] / `TryFrom<String>`), rejecting non-RFC-3339
//!   input at construction rather than letting it rot in stored state;
//! - **serializes** transparently as a JSON string (`from`/`into`
//!   = `"String"`), so on-disk and on-wire the field is still a plain
//!   `"format": "date-time"` string exactly matching the schema — no
//!   wire-format change for existing consumers.
//!
//! Mirrors the `verbreel-state` [`Color`]/[`Sha256`] newtype pattern:
//! validation on construction, serde transparency, `Display`, no public
//! way to bypass the constructor.
//!
//! [`Color`]: ../../verbreel_state/newtypes/struct.Color.html

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

/// A canonical RFC 3339 timestamp.
///
/// The inner string is **always** a value that round-trips through the
/// `time` crate's RFC 3339 parser — the only constructors
/// ([`Timestamp::now`], [`Timestamp::parse`], `TryFrom<String>`,
/// `FromStr`) all validate. There is no public constructor that wraps an
/// unvalidated string, so any `Timestamp` in scope is guaranteed
/// well-formed.
///
/// Serde representation is transparent: a `Timestamp` serializes as the
/// JSON string it wraps and deserializes by validating that string, so
/// event data and project-graph JSON keep their `"format": "date-time"`
/// string shape.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Timestamp(String);

/// Error returned when constructing a [`Timestamp`] from a string that
/// is not a valid RFC 3339 date-time.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("Timestamp must be an RFC 3339 date-time (schema `format: date-time`), got {got:?}")]
pub struct TimestampParseError {
    /// The offending input.
    pub got: String,
}

impl Timestamp {
    /// Current wall-clock time as a canonical RFC 3339 UTC timestamp.
    ///
    /// This is the single construction site for "now" across the
    /// engine — `Event::new`, `project.create`, `project.duplicate`,
    /// asset import, etc. all route through it so there is exactly one
    /// formatting path.
    ///
    /// # Panics
    ///
    /// Panics only if `time::OffsetDateTime::now_utc()` produces a value
    /// the RFC 3339 well-known format cannot render. The format spec is
    /// a compile-time constant and `now_utc()` is always a valid
    /// calendar instant, so this is an upstream-invariant assertion, not
    /// a runtime error worth propagating.
    #[must_use]
    pub fn now() -> Self {
        let s = OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .expect("Rfc3339 format of OffsetDateTime::now_utc is infallible");
        Timestamp(s)
    }

    /// Validate `s` as RFC 3339 and wrap it.
    ///
    /// The stored string is the caller's input verbatim once it parses
    /// (RFC 3339 permits both `Z` and numeric-offset forms; the spec
    /// `format: date-time` accepts either, so we preserve what was
    /// given rather than re-normalizing the offset).
    ///
    /// # Errors
    ///
    /// Returns [`TimestampParseError`] if `s` is not a valid RFC 3339
    /// date-time.
    pub fn parse(s: impl Into<String>) -> Result<Self, TimestampParseError> {
        let s = s.into();
        match OffsetDateTime::parse(&s, &Rfc3339) {
            Ok(_) => Ok(Timestamp(s)),
            Err(_) => Err(TimestampParseError { got: s }),
        }
    }

    /// Borrow the inner RFC 3339 string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Timestamp {
    type Error = TimestampParseError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Timestamp::parse(value)
    }
}

impl FromStr for Timestamp {
    type Err = TimestampParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Timestamp::parse(s)
    }
}

impl From<Timestamp> for String {
    fn from(value: Timestamp) -> Self {
        value.0
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn now_is_valid_rfc3339() {
        let ts = Timestamp::now();
        // Round-trips back through the validating constructor.
        let reparsed = Timestamp::parse(ts.as_str()).expect("now() must produce valid RFC 3339");
        assert_eq!(ts, reparsed);
        assert!(ts.as_str().contains('T'), "RFC 3339 needs the T separator");
    }

    #[test]
    fn parse_rejects_malformed() {
        assert!(Timestamp::parse("not-a-timestamp").is_err());
        assert!(Timestamp::parse("2026-13-01T00:00:00Z").is_err());
        assert!(Timestamp::parse("").is_err());
    }

    #[test]
    fn parse_accepts_z_and_offset_forms() {
        assert!(Timestamp::parse("2026-05-29T12:00:00Z").is_ok());
        assert!(Timestamp::parse("2026-05-29T12:00:00+02:00").is_ok());
        assert!(Timestamp::parse("2026-05-29T12:00:00.123456Z").is_ok());
    }

    #[test]
    fn serde_is_transparent_string() {
        let ts = Timestamp::parse("2026-05-29T12:00:00Z").unwrap();
        let json = serde_json::to_string(&ts).unwrap();
        assert_eq!(json, "\"2026-05-29T12:00:00Z\"");
        let back: Timestamp = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ts);
    }

    #[test]
    fn deserialize_rejects_malformed() {
        let err = serde_json::from_str::<Timestamp>("\"garbage\"");
        assert!(err.is_err(), "deserialize must reject non-RFC-3339 input");
    }

    #[test]
    fn ordering_is_lexicographic_on_canonical_form() {
        let a = Timestamp::parse("2026-01-01T00:00:00Z").unwrap();
        let b = Timestamp::parse("2026-12-31T23:59:59Z").unwrap();
        assert!(a < b);
    }
}
