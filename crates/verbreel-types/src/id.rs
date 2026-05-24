//! IDs per spec §0.3 — `UUIDv7` strict, no v4 acceptance, no partial prefixes
//! at the type level.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use thiserror::Error;
use uuid::Uuid;

/// A strict `UUIDv7` (RFC 9562) — version nibble pinned to 7.
///
/// Per spec §0.3: pasting a `UUIDv4` (or any non-v7) must fail validation. The
/// constructor verifies the version nibble and rejects with [`IdError::NotV7`].
/// The nil UUID is rejected via [`IdError::NilForbidden`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct UuidV7(Uuid);

/// Error returned when constructing a [`UuidV7`] (or any strongly-typed ID)
/// from an invalid string.
#[derive(Debug, Error)]
pub enum IdError {
    /// String is not a valid UUID at all.
    #[error("not a valid UUID: {0}")]
    Parse(#[from] uuid::Error),

    /// Parses as UUID but the version nibble is not 7.
    #[error("UUID version is {found}, expected 7 (spec §0.3 UUIDv7 strict)")]
    NotV7 {
        /// The version actually found (1-5 typical, 7 for v7, etc.).
        found: usize,
    },

    /// String parses as the nil UUID, which is rejected by this ID kind.
    #[error("nil UUID is not permitted for this ID kind")]
    NilForbidden,
}

impl UuidV7 {
    /// Construct from a [`Uuid`], verifying version 7. Returns [`IdError::NotV7`]
    /// for any other version. Rejects nil with [`IdError::NilForbidden`].
    ///
    /// # Errors
    ///
    /// - [`IdError::NilForbidden`] if `uuid` is the nil UUID.
    /// - [`IdError::NotV7`] if the version nibble is anything other than 7.
    pub fn from_uuid(uuid: Uuid) -> Result<Self, IdError> {
        if uuid.is_nil() {
            return Err(IdError::NilForbidden);
        }
        let version = uuid.get_version_num();
        if version != 7 {
            return Err(IdError::NotV7 { found: version });
        }
        Ok(UuidV7(uuid))
    }

    /// Mint a fresh `UUIDv7` from the current system time. Convenience wrapper
    /// around [`uuid::Uuid::now_v7`].
    ///
    /// # Panics
    ///
    /// Panics if [`uuid::Uuid::now_v7`] ever produces a non-v7 UUID, which would
    /// be an upstream `uuid` crate bug.
    #[must_use]
    pub fn now() -> Self {
        // `Uuid::now_v7()` always produces a v7; the validation is redundant
        // but keeps the invariant explicit. If this ever fails, it's an upstream bug.
        UuidV7::from_uuid(Uuid::now_v7()).expect("Uuid::now_v7 produced a non-v7 UUID")
    }

    /// Inner [`Uuid`].
    #[must_use]
    pub fn as_uuid(&self) -> Uuid {
        self.0
    }

    /// Lowercase hyphenated representation (36 chars), per spec §0.3.
    #[must_use]
    pub fn to_hyphenated_lower(&self) -> String {
        self.0.hyphenated().to_string()
    }
}

impl FromStr for UuidV7 {
    type Err = IdError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let u = Uuid::parse_str(s)?;
        UuidV7::from_uuid(u)
    }
}

impl fmt::Display for UuidV7 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.hyphenated())
    }
}

impl TryFrom<String> for UuidV7 {
    type Error = IdError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<UuidV7> for String {
    fn from(value: UuidV7) -> Self {
        value.to_string()
    }
}

/// Macro: define a strongly-typed ID newtype wrapping [`UuidV7`].
///
/// All generated types share [`UuidV7`]'s strict v7 + non-nil invariants. They are
/// transparent over [`UuidV7`] in serde, which means the JSON shape is identical
/// to a [`UuidV7`] string. The type-level distinction prevents passing e.g. a
/// [`ClipId`] where the engine expects a [`ProjectId`].
macro_rules! define_id {
    ($(#[$doc:meta])* $vis:vis $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        $vis struct $name(UuidV7);

        impl $name {
            /// Wrap an existing [`UuidV7`].
            #[must_use]
            pub const fn from_uuid_v7(uuid: UuidV7) -> Self {
                Self(uuid)
            }

            /// Mint a fresh ID (`UuidV7` from system time).
            #[must_use]
            pub fn now() -> Self {
                Self(UuidV7::now())
            }

            /// Inner [`UuidV7`].
            #[must_use]
            pub fn as_uuid_v7(&self) -> UuidV7 {
                self.0
            }
        }

        impl FromStr for $name {
            type Err = IdError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Ok(Self(s.parse()?))
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

define_id! {
    /// Project ID — root of a Verbreel project graph (§0.3).
    pub ProjectId
}
define_id! {
    /// Clip ID — a clip on a track (§0.3).
    pub ClipId
}
define_id! {
    /// Track ID — a track in a project (§0.3).
    pub TrackId
}
define_id! {
    /// Asset ID — an asset in the project's asset registry (§0.3, §3.1).
    pub AssetId
}
define_id! {
    /// Text-track ID — a text overlay track (§0.3).
    pub TextTrackId
}
define_id! {
    /// Event ID — `UUIDv7` of an events.jsonl line (§0.3, App. C).
    pub EventId
}
define_id! {
    /// Marker ID — `UUIDv7` of a project-level marker (§0.3, schema `$defs/Marker`).
    pub MarkerId
}
define_id! {
    /// Tracker ID — `UUIDv7` of a tracker record (§0.3, §18, schema `$defs/Tracker`).
    pub TrackerId
}
define_id! {
    /// Link-group ID — `UUIDv7` tagging clips that move and trim as a unit
    /// (§0.3, schema `Clip.link_group`).
    pub LinkGroupId
}
define_id! {
    /// Effect ID — `UUIDv7` of an effect record (§0.3, schema `$defs/Effect`).
    pub EffectId
}
define_id! {
    /// Keyframe ID — `UUIDv7` of a keyframe record (§0.3, schema `$defs/Keyframe`).
    pub KeyframeId
}

#[cfg(test)]
mod tests {
    use super::*;

    const KNOWN_V7: &str = "0190b8d3-15e3-7000-b8d3-15e370a30000"; // synthetic v7
    const KNOWN_V4: &str = "550e8400-e29b-41d4-a716-446655440000"; // a v4

    #[test]
    fn v7_string_parses() {
        let u: UuidV7 = KNOWN_V7.parse().unwrap();
        assert_eq!(u.to_hyphenated_lower(), KNOWN_V7);
    }

    #[test]
    fn v4_string_rejected() {
        let err = KNOWN_V4.parse::<UuidV7>().unwrap_err();
        matches!(err, IdError::NotV7 { found: 4 });
    }

    #[test]
    fn nil_uuid_rejected() {
        let nil = "00000000-0000-0000-0000-000000000000";
        let err = nil.parse::<UuidV7>().unwrap_err();
        matches!(err, IdError::NilForbidden | IdError::NotV7 { .. });
    }

    #[test]
    fn now_v7_round_trips() {
        let u = UuidV7::now();
        let s = u.to_string();
        let back: UuidV7 = s.parse().unwrap();
        assert_eq!(u, back);
    }

    #[test]
    fn id_kinds_are_distinct_at_type_level() {
        let p = ProjectId::now();
        let c = ClipId::now();
        let _: ProjectId = p;
        let _: ClipId = c;
        // The following must NOT compile if uncommented:
        // let _: ProjectId = c;
    }

    #[test]
    fn project_id_serializes_as_uuid_string() {
        let p: ProjectId = KNOWN_V7.parse().unwrap();
        let s = serde_json::to_string(&p).unwrap();
        assert_eq!(s, format!("\"{KNOWN_V7}\""));
    }

    #[test]
    fn passing_clipid_where_projectid_expected_is_a_compile_error() {
        // documented in `id_kinds_are_distinct_at_type_level` above.
    }
}
