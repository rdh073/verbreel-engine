//! Newtypes for content-addressed asset fields: [`Sha256`],
//! [`AssetPath`], [`AssetRef`].
//!
//! All three carry a `try_from = "String"` + `into = "String"` serde
//! representation: on-disk and on-wire they look like plain strings,
//! exactly matching the patterns in `spec/project-schema.json`
//! `$defs/Sha256`, `$defs/AssetPath`, `$defs/AssetRef`. Deserialize
//! runs the regex; an unparseable input surfaces as a serde error.
//!
//! The regexes are compiled exactly once per process via
//! [`std::sync::OnceLock`]. No `once_cell` dep is needed — the
//! workspace `rust-version = "1.92"` MSRV well exceeds the std
//! `OnceLock::get_or_init` 1.70 stability gate.

use std::fmt;
use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use verbreel_types::{AssetId, UuidV7};

// ---------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------

/// Error returned when constructing one of the asset newtypes from an
/// invalid string. Each variant matches a specific schema constraint
/// so callers can pattern-match on the failure mode.
#[derive(Debug, Error)]
pub enum AssetNewtypeError {
    /// String did not match the SHA-256 pattern `^[0-9a-f]{64}$`.
    #[error("Sha256 must be 64 lowercase hex chars (schema $defs/Sha256 pattern), got {got:?}")]
    InvalidSha256 {
        /// The offending input (truncated for log safety in callers).
        got: String,
    },

    /// String did not match the [`AssetPath`] pattern.
    #[error(
        "AssetPath must match `^assets/[0-9a-f]{{2}}/[0-9a-f]{{64}}\\.[a-z0-9]+$` (schema \
         $defs/AssetPath pattern), got {got:?}"
    )]
    InvalidAssetPath {
        /// The offending input.
        got: String,
    },

    /// String did not match the [`AssetRef`] pattern (neither a
    /// `UUIDv7` nor the nil UUID).
    #[error(
        "AssetRef must be either the nil UUID or a UUIDv7 (schema $defs/AssetRef pattern), \
         got {got:?}"
    )]
    InvalidAssetRef {
        /// The offending input.
        got: String,
    },
}

// ---------------------------------------------------------------------
// Sha256
// ---------------------------------------------------------------------

/// SHA-256 hex digest, lowercase, 64 chars. Matches schema
/// `$defs/Sha256`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Sha256(String);

fn sha256_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[0-9a-f]{64}$").expect("compile-time-correct regex"))
}

impl Sha256 {
    /// Construct from a string, validating against the schema pattern.
    ///
    /// # Errors
    /// Returns [`AssetNewtypeError::InvalidSha256`] if `s` is not
    /// 64 lowercase hex characters.
    pub fn new(s: String) -> Result<Self, AssetNewtypeError> {
        if sha256_re().is_match(&s) {
            Ok(Sha256(s))
        } else {
            Err(AssetNewtypeError::InvalidSha256 { got: s })
        }
    }

    /// Borrow the inner string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Sha256 {
    type Error = AssetNewtypeError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Sha256::new(value)
    }
}

impl From<Sha256> for String {
    fn from(value: Sha256) -> Self {
        value.0
    }
}

impl fmt::Display for Sha256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

// ---------------------------------------------------------------------
// AssetPath
// ---------------------------------------------------------------------

/// Content-addressed asset path: `assets/<aa>/<sha256>.<ext>`. Matches
/// schema `$defs/AssetPath`.
///
/// The "first 2 hex of `<aa>` equals first 2 hex of the `<sha256>`"
/// invariant is an engine-level rule (per the schema's prose) and is
/// **not** enforced at the type layer — it lives in the `asset.import`
/// verb. This newtype enforces the regex pattern only.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct AssetPath(String);

fn asset_path_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^assets/[0-9a-f]{2}/[0-9a-f]{64}\.[a-z0-9]+$")
            .expect("compile-time-correct regex")
    })
}

impl AssetPath {
    /// Construct from a string, validating against the schema pattern.
    ///
    /// # Errors
    /// Returns [`AssetNewtypeError::InvalidAssetPath`] if `s` does not
    /// match the layout `assets/<aa>/<sha256>.<ext>`.
    pub fn new(s: String) -> Result<Self, AssetNewtypeError> {
        if asset_path_re().is_match(&s) {
            Ok(AssetPath(s))
        } else {
            Err(AssetNewtypeError::InvalidAssetPath { got: s })
        }
    }

    /// Borrow the inner string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for AssetPath {
    type Error = AssetNewtypeError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        AssetPath::new(value)
    }
}

impl From<AssetPath> for String {
    fn from(value: AssetPath) -> Self {
        value.0
    }
}

impl fmt::Display for AssetPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

// ---------------------------------------------------------------------
// AssetRef
// ---------------------------------------------------------------------

/// The nil-UUID string per RFC 9562 §5.9. Used by [`AssetRef::Nil`].
const NIL_UUID_STR: &str = "00000000-0000-0000-0000-000000000000";

/// A reference to an asset by ID, OR the nil UUID for text clips that
/// carry no asset. Matches schema `$defs/AssetRef`.
///
/// On-wire shape is a single 36-char string (either the nil UUID or
/// a `UUIDv7`). The Rust enum is exposed via constructors —
/// [`AssetRef::nil`] and [`AssetRef::from_id`] — and accessors —
/// [`AssetRef::is_nil`] and [`AssetRef::id`]. The serde representation
/// is `try_from = "String"` / `into = "String"` so deserialize runs
/// the regex via the `TryFrom` impl, and serialize emits the string
/// form unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct AssetRef(AssetRefInner);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum AssetRefInner {
    Nil,
    Id(AssetId),
}

impl AssetRef {
    /// The nil-UUID `AssetRef`. Only valid on text clips per §0.13.
    #[must_use]
    pub fn nil() -> Self {
        AssetRef(AssetRefInner::Nil)
    }

    /// Wrap a concrete [`AssetId`] as an `AssetRef`.
    #[must_use]
    pub fn from_id(id: AssetId) -> Self {
        AssetRef(AssetRefInner::Id(id))
    }

    /// `true` if this is the nil-UUID sentinel.
    #[must_use]
    pub fn is_nil(&self) -> bool {
        matches!(self.0, AssetRefInner::Nil)
    }

    /// Borrow the wrapped [`AssetId`] if this is the non-nil variant.
    #[must_use]
    pub fn id(&self) -> Option<&AssetId> {
        match &self.0 {
            AssetRefInner::Nil => None,
            AssetRefInner::Id(id) => Some(id),
        }
    }
}

impl TryFrom<String> for AssetRef {
    type Error = AssetNewtypeError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value == NIL_UUID_STR {
            return Ok(AssetRef(AssetRefInner::Nil));
        }
        match value.parse::<UuidV7>() {
            Ok(u) => Ok(AssetRef(AssetRefInner::Id(AssetId::from_uuid_v7(u)))),
            Err(_) => Err(AssetNewtypeError::InvalidAssetRef { got: value }),
        }
    }
}

impl From<AssetRef> for String {
    fn from(value: AssetRef) -> Self {
        match value.0 {
            AssetRefInner::Nil => NIL_UUID_STR.to_string(),
            AssetRefInner::Id(id) => id.to_string(),
        }
    }
}

impl fmt::Display for AssetRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            AssetRefInner::Nil => f.write_str(NIL_UUID_STR),
            AssetRefInner::Id(id) => id.fmt(f),
        }
    }
}
