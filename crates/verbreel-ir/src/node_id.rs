//! `IrNodeId` — composition-graph node identifier (strict `UUIDv7`).
//!
//! Mirrors the strict-`UUIDv7` newtype shape established in
//! `verbreel_types::id` ([`ProjectId`], [`ClipId`], etc.): non-nil, version
//! nibble pinned to 7, transparent serde over the lowercase-hyphenated UUID
//! string. The type-level distinction prevents passing e.g. a
//! [`verbreel_types::ProjectId`] where the engine expects an [`IrNodeId`].
//!
//! Spec references:
//! - §0.3 — IDs are strict `UUIDv7`, no v4, no nil.
//! - Research 01 line 114 — `cache_hash` is a pure function of
//!   `(node_id, args_hash, upstream_cache_hashes, tick)`.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use verbreel_types::id::{IdError, UuidV7};

/// Identifier for a composition-graph node.
///
/// Transparent over [`UuidV7`] in serde, so the JSON shape is the
/// lowercase-hyphenated UUID string. The newtype exists to prevent mixing
/// node identifiers with project/clip/track identifiers at the type level.
///
/// ## Constructor naming
///
/// Two constructors exist by design, matching the pair in
/// `verbreel_types::id`: [`from_uuid`](Self::from_uuid) takes a raw
/// [`uuid::Uuid`] and runs the v7 check, returning a [`Result`];
/// [`from_uuid_v7`](Self::from_uuid_v7) takes an already-typed
/// [`UuidV7`] and wraps infallibly. Callers carrying raw input use
/// `from_uuid`; callers already inside the strict-v7 type system use
/// `from_uuid_v7` and skip the redundant check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct IrNodeId(UuidV7);

// Hand-implemented Ord because `UuidV7` upstream does not derive Ord.
// Compare on the inner raw `uuid::Uuid`'s byte representation —
// `UUIDv7`'s leading 48-bit timestamp prefix makes byte-order Ord
// equivalent to creation-order for ids minted on the same clock.
impl Ord for IrNodeId {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_uuid().as_bytes().cmp(other.as_uuid().as_bytes())
    }
}

impl PartialOrd for IrNodeId {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl IrNodeId {
    /// Wrap an existing [`UuidV7`].
    #[must_use]
    pub const fn from_uuid_v7(uuid: UuidV7) -> Self {
        Self(uuid)
    }

    /// Wrap a raw [`uuid::Uuid`] after verifying it is a non-nil v7.
    /// Returns [`IdError`] if the input fails the v7 invariant — same
    /// validating contract as [`UuidV7::from_uuid`].
    ///
    /// # Errors
    ///
    /// Propagates any [`IdError`] from [`UuidV7::from_uuid`].
    pub fn from_uuid(uuid: uuid::Uuid) -> Result<Self, IdError> {
        Ok(Self(UuidV7::from_uuid(uuid)?))
    }

    /// Mint a fresh node identifier (`UUIDv7` from system time).
    #[must_use]
    pub fn now() -> Self {
        Self(UuidV7::now())
    }

    /// Inner [`UuidV7`].
    #[must_use]
    pub fn as_uuid_v7(&self) -> UuidV7 {
        self.0
    }

    /// Inner raw [`uuid::Uuid`]. Convenience for [`crate::cache_key::CacheKey::derive`]
    /// which needs the 16-byte representation.
    #[must_use]
    pub fn as_uuid(&self) -> uuid::Uuid {
        self.0.as_uuid()
    }
}

impl FromStr for IrNodeId {
    type Err = IdError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(s.parse()?))
    }
}

impl fmt::Display for IrNodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
