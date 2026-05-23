//! Content-addressed asset hash per spec §3.1.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use thiserror::Error;

/// SHA-256 hex string (lowercase, 64 chars) — per spec §3.1.
///
/// Asset files live at `assets/<aa>/<sha256>.<ext>` where `<aa>` is the first
/// two hex chars of `<sha256>`. The invariant `aa == sha256[0..2]` is enforced
/// by the storage layer, not here; this type validates only the hash format.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct AssetHash(String);

/// Error returned when constructing an [`AssetHash`] from an invalid string.
#[derive(Debug, Error)]
pub enum HashError {
    /// String is not the expected length (64 chars).
    #[error("expected 64 hex chars, got {0}")]
    BadLength(usize),

    /// String contains non-hex characters.
    #[error("non-hex character at position {position}")]
    NonHex {
        /// Byte offset of the first non-hex character.
        position: usize,
    },

    /// String contains uppercase hex; spec requires lowercase.
    #[error("uppercase hex not allowed at position {position}; spec §3.1 requires lowercase")]
    Uppercase {
        /// Byte offset of the first uppercase character.
        position: usize,
    },
}

impl AssetHash {
    /// Validate and wrap a 64-char lowercase-hex SHA-256 string.
    ///
    /// # Errors
    ///
    /// - [`HashError::BadLength`] if the string is not exactly 64 chars long.
    /// - [`HashError::Uppercase`] if the string contains uppercase hex.
    /// - [`HashError::NonHex`] if the string contains a non-hex character.
    pub fn new(s: impl Into<String>) -> Result<Self, HashError> {
        let s = s.into();
        if s.len() != 64 {
            return Err(HashError::BadLength(s.len()));
        }
        for (i, b) in s.bytes().enumerate() {
            if b.is_ascii_uppercase() {
                return Err(HashError::Uppercase { position: i });
            }
            if !b.is_ascii_hexdigit() {
                return Err(HashError::NonHex { position: i });
            }
        }
        Ok(AssetHash(s))
    }

    /// As a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// First two characters — used as the CAS directory prefix `assets/<aa>/`
    /// (spec §3.1, project-schema.json).
    #[must_use]
    pub fn prefix(&self) -> &str {
        &self.0[..2]
    }
}

impl FromStr for AssetHash {
    type Err = HashError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        AssetHash::new(s)
    }
}

impl fmt::Display for AssetHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for AssetHash {
    type Error = HashError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        AssetHash::new(value)
    }
}

impl From<AssetHash> for String {
    fn from(value: AssetHash) -> Self {
        value.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = "abc123def456000000000000000000000000000000000000000000000000feed";

    #[test]
    fn valid_hash_accepted() {
        let h = AssetHash::new(VALID).unwrap();
        assert_eq!(h.as_str(), VALID);
        assert_eq!(h.prefix(), "ab");
    }

    #[test]
    fn wrong_length_rejected() {
        assert!(matches!(
            AssetHash::new("abc"),
            Err(HashError::BadLength(3))
        ));
    }

    #[test]
    fn uppercase_rejected() {
        let mut s = String::from(VALID);
        s.replace_range(5..6, "F"); // capitalize position 5
        assert!(matches!(
            AssetHash::new(s),
            Err(HashError::Uppercase { position: 5 })
        ));
    }

    #[test]
    fn non_hex_rejected() {
        let mut s = String::from(VALID);
        s.replace_range(10..11, "z");
        assert!(matches!(
            AssetHash::new(s),
            Err(HashError::NonHex { position: 10 })
        ));
    }

    #[test]
    fn serde_round_trip() {
        let h = AssetHash::new(VALID).unwrap();
        let s = serde_json::to_string(&h).unwrap();
        assert_eq!(s, format!("\"{VALID}\""));
        let back: AssetHash = serde_json::from_str(&s).unwrap();
        assert_eq!(back, h);
    }
}
