//! `CacheKey` — composition-cache lookup key + pinned `derive()` hash.
//!
//! Research 01 line 114 names `cache_hash` as a pure function of
//! `(node_id, args_hash, upstream_cache_hashes, tick)`. This module ships
//! the primitive without committing to the composition IR Node enum, which
//! depends on Research 01's render-pipeline design.
//!
//! Spec references:
//! - §0.2 — `Tick` (engine tick rate, `TICK_RATE_HZ`).
//! - §0.13 — content-addressed identity is part of the published contract.

use sha2::{Digest, Sha256};

use crate::node_id::IrNodeId;

/// Four §0.13-style inputs to a composition-cache lookup.
///
/// All fields are pre-hashed where appropriate — `args_hash` is the
/// SHA-256 of the canonicalized args feeding this node (produced upstream
/// by `verbreel-canon`), and each entry in `upstream_hashes` is the
/// SHA-256 result of an immediate parent's cache entry, in dependency
/// order. Order matters: reordering parents yields a different hash.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheKey {
    /// Graph node being keyed.
    pub node_id: IrNodeId,
    /// SHA-256 of the canonicalized args feeding `node_id`.
    pub args_hash: [u8; 32],
    /// SHA-256 of each immediate parent's cache result, in dependency order.
    pub upstream_hashes: Vec<[u8; 32]>,
    /// Engine tick at which the cache entry was produced (per §0.2 `TICK_RATE_HZ`).
    pub tick: u64,
}

impl CacheKey {
    /// Pure deterministic SHA-256 of all four inputs in a pinned byte layout.
    ///
    /// # Byte layout (v1 — PINNED CONTRACT)
    ///
    /// The hash input is the concatenation of, in this exact order:
    ///
    /// 1. `b"v1\0"` — 3-byte version marker. Future layout changes get a
    ///    new marker (`b"v2\0"` etc.) rather than silently reordering.
    /// 2. `node_id.as_uuid().as_bytes()` — 16 bytes.
    /// 3. `args_hash` — 32 bytes.
    /// 4. `(upstream_hashes.len() as u64).to_le_bytes()` — 8 bytes. The
    ///    count is hashed BEFORE the contents so an empty `Vec` is
    ///    distinguishable from a `Vec` of zero-filled entries.
    /// 5. `upstream_hashes` concatenated in declared order — `32 * N` bytes.
    /// 6. `tick.to_le_bytes()` — 8 bytes (little-endian per WebAssembly /
    ///    `bytemuck` convention used elsewhere in the engine).
    ///
    /// Changing any field above without bumping the version marker is a
    /// breaking change of the cache namespace and MUST be flagged in
    /// review.
    #[must_use]
    pub fn derive(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(b"v1\0");
        hasher.update(self.node_id.as_uuid().as_bytes());
        hasher.update(self.args_hash);
        let count = self.upstream_hashes.len() as u64;
        hasher.update(count.to_le_bytes());
        for h in &self.upstream_hashes {
            hasher.update(h);
        }
        hasher.update(self.tick.to_le_bytes());
        hasher.finalize().into()
    }
}
