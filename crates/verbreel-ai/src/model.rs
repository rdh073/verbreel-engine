//! Model identity + deterministic cache-key for AI inference results.
//!
//! An inference result (tracker trace, STT segments, audio analysis) is a
//! pure function of: which model produced it, against which source asset,
//! with which algorithm parameters, under which result-schema version.
//! Re-running the same `(model, asset, params, schema)` tuple MUST hit the
//! cache; changing any one of the four MUST miss. [`ModelCacheKey::derive`]
//! is that function.
//!
//! ## Why a pinned byte layout, not `serde_json::to_string`
//!
//! Cache identity is a §0.13-style content-addressed contract: the hash is
//! part of the on-disk cache namespace. JSON serialization is not a stable
//! hash input — key ordering, whitespace, and float formatting drift across
//! `serde_json` versions and would silently invalidate every cached result.
//! This module mirrors `verbreel_ir::cache_key::CacheKey::derive`: a SHA-256
//! over a versioned, length-prefixed byte concatenation.
//!
//! `verbreel-ai` does not depend on `verbreel-canon`, so it cannot canonical-
//! ize algorithm params itself. Instead it *consumes* a pre-computed
//! `params_hash: [u8; 32]` from the caller — the same "`args_hash` pre-hashed
//! upstream" pattern `verbreel-ir` uses. The composition layer that owns the
//! params (already linked against `verbreel-canon`) is responsible for
//! canonicalizing them and handing the digest down.

use sha2::{Digest, Sha256};
use verbreel_types::AssetHash;

/// Stable identifier for a model family / checkpoint.
///
/// The id is the spec algorithm/model literal the verb layer already uses
/// (e.g. `"mixformer_v2_s"`, `"whisper"`, `"onset"`). It is *not* a file
/// path or hash — two engine builds shipping the same logical model under
/// different on-disk paths must produce the same `ModelId`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModelId(String);

impl ModelId {
    /// Wrap a model-family literal.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Borrow the underlying literal.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Model checkpoint version.
///
/// Distinct from [`ModelId`]: the same family (`whisper`) ships multiple
/// checkpoints (`large-v3`, `tiny`) that produce different outputs and must
/// not share a cache entry. The version string is opaque — any change to it
/// changes the cache key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModelVersion(String);

impl ModelVersion {
    /// Wrap a checkpoint-version literal.
    #[must_use]
    pub fn new(version: impl Into<String>) -> Self {
        Self(version.into())
    }

    /// Borrow the underlying literal.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The four inputs that determine an AI-result cache entry.
///
/// All four are folded into [`ModelCacheKey::derive`]. The `params_hash` is
/// pre-computed by the caller (see module docs) — `verbreel-ai` never
/// canonicalizes params itself. `result_schema_version` namespaces the
/// cache by the *shape* of the cached result, so a future change to a data
/// struct (e.g. adding a field to `TrackerRunData`) cannot serve a stale
/// entry shaped for the old schema.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModelCacheKey {
    /// Model family / checkpoint id.
    pub model_id: ModelId,
    /// Model checkpoint version.
    pub model_version: ModelVersion,
    /// Content hash of the source asset the model ran against (§3.1).
    pub source_asset_hash: AssetHash,
    /// SHA-256 of the canonicalized algorithm params, pre-computed upstream
    /// (the caller owns `verbreel-canon`; `verbreel-ai` does not).
    pub params_hash: [u8; 32],
    /// Version of the result data struct shape this entry was produced for.
    pub result_schema_version: u32,
}

impl ModelCacheKey {
    /// Pure deterministic SHA-256 of all inputs in a pinned byte layout.
    ///
    /// # Byte layout (v1 — PINNED CONTRACT)
    ///
    /// The hash input is the concatenation of, in this exact order:
    ///
    /// 1. `b"vrai-mck-v1\0"` — 12-byte version marker. Future layout changes
    ///    get a new marker (`b"vrai-mck-v2\0"` etc.) rather than silently
    ///    reordering, which would corrupt the cache namespace.
    /// 2. `(model_id.len() as u64).to_le_bytes()` then the id bytes. The
    ///    length is hashed before the bytes so `("ab", "c")` and `("a",
    ///    "bc")` concatenations cannot collide.
    /// 3. `(model_version.len() as u64).to_le_bytes()` then the version
    ///    bytes.
    /// 4. `(source_asset_hash.len() as u64).to_le_bytes()` then the
    ///    asset-hash ASCII bytes.
    /// 5. `params_hash` — 32 bytes.
    /// 6. `result_schema_version.to_le_bytes()` — 4 bytes, little-endian.
    ///
    /// Changing any field above without bumping the version marker is a
    /// breaking change of the cache namespace and MUST be flagged in review.
    #[must_use]
    pub fn derive(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(b"vrai-mck-v1\0");
        write_len_prefixed(&mut hasher, self.model_id.as_str().as_bytes());
        write_len_prefixed(&mut hasher, self.model_version.as_str().as_bytes());
        write_len_prefixed(&mut hasher, self.source_asset_hash.as_str().as_bytes());
        hasher.update(self.params_hash);
        hasher.update(self.result_schema_version.to_le_bytes());
        hasher.finalize().into()
    }

    /// Lowercase-hex rendering of [`derive`](Self::derive), suitable for use
    /// as an on-disk cache filename stem.
    #[must_use]
    pub fn derive_hex(&self) -> String {
        let digest = self.derive();
        let mut out = String::with_capacity(64);
        for byte in digest {
            use std::fmt::Write;
            // Infallible: writing to a String never errors.
            let _ = write!(out, "{byte:02x}");
        }
        out
    }
}

/// Hash a length-prefixed byte slice: the `u64` LE length, then the bytes.
fn write_len_prefixed(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

#[cfg(test)]
mod tests {
    use super::*;

    const ASSET_A: &str = "aa11bb22cc33dd44ee55ff66001122334455667788990011223344556677889900";
    const ASSET_B: &str = "bb22cc33dd44ee55ff66001122334455667788990011223344556677889900aabb";

    fn key() -> ModelCacheKey {
        ModelCacheKey {
            model_id: ModelId::new("mixformer_v2_s"),
            model_version: ModelVersion::new("v2.0"),
            source_asset_hash: AssetHash::new(&ASSET_A[..64]).unwrap(),
            params_hash: [7u8; 32],
            result_schema_version: 1,
        }
    }

    #[test]
    fn stable_for_identical_inputs() {
        assert_eq!(key().derive(), key().derive());
        assert_eq!(key().derive_hex(), key().derive_hex());
    }

    #[test]
    fn changes_when_model_version_changes() {
        let mut other = key();
        other.model_version = ModelVersion::new("v2.1");
        assert_ne!(key().derive(), other.derive());
    }

    #[test]
    fn changes_when_model_id_changes() {
        let mut other = key();
        other.model_id = ModelId::new("yunet");
        assert_ne!(key().derive(), other.derive());
    }

    #[test]
    fn changes_when_source_asset_hash_changes() {
        let mut other = key();
        other.source_asset_hash = AssetHash::new(&ASSET_B[..64]).unwrap();
        assert_ne!(key().derive(), other.derive());
    }

    #[test]
    fn changes_when_params_hash_changes() {
        let mut other = key();
        other.params_hash = [9u8; 32];
        assert_ne!(key().derive(), other.derive());
    }

    #[test]
    fn changes_when_schema_version_changes() {
        let mut other = key();
        other.result_schema_version = 2;
        assert_ne!(key().derive(), other.derive());
    }

    #[test]
    fn length_prefix_prevents_field_boundary_collision() {
        // Without length-prefixing, ("ab","c") and ("a","bc") would hash
        // identically. The u64 length guards every variable-length field.
        let mut left = key();
        left.model_id = ModelId::new("ab");
        left.model_version = ModelVersion::new("c");
        let mut right = key();
        right.model_id = ModelId::new("a");
        right.model_version = ModelVersion::new("bc");
        assert_ne!(left.derive(), right.derive());
    }

    #[test]
    fn derive_hex_is_64_lowercase_hex_chars() {
        let hex = key().derive_hex();
        assert_eq!(hex.len(), 64);
        assert!(
            hex.bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        );
    }
}
