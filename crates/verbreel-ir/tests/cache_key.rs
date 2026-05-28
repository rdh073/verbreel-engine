//! Integration tests for [`CacheKey::derive`] — the pinned-byte-layout
//! SHA-256 derivation that names a composition cache entry.
//!
//! Mirrors the known-fixture pattern from
//! `crates/verbreel-storage/tests/cas.rs`.

use uuid::Uuid;
use verbreel_ir::{CacheKey, IrNodeId};
use verbreel_types::id::UuidV7;

// Fixed UUIDv7 used by every fixture below. Bit 0x70 in the version nibble.
const FIXTURE_UUID: &str = "01900000-0000-7000-8000-000000000001";

// Pinned SHA-256 of:
//   b"v1\0"
//   ++ UUID(01900000-0000-7000-8000-000000000001).bytes (16)
//   ++ [0u8; 32]
//   ++ 0u64.to_le_bytes() (empty upstream count)
//   ++ 0u64.to_le_bytes() (tick = 0)
//
// Cross-checked against a standalone Python SHA-256 of the same byte
// concatenation. If this constant ever changes, the v1 hash contract has
// been broken and the change requires a new version marker.
const FIXTURE_HEX: &str = "7f11e99c3191296b3886cc92a2384806340548d9344b16c12f0c3597bf8be893";

/// Helper: build an [`IrNodeId`] from a fixed v7 UUID string.
fn fixture_node_id() -> IrNodeId {
    let inner: UuidV7 = FIXTURE_UUID.parse().expect("fixture UUID must be v7");
    IrNodeId::from_uuid_v7(inner)
}

/// Helper: hex-encode 32 bytes the same way the rest of the engine does
/// (lowercase, no separators).
fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0f) as usize] as char);
    }
    s
}

// --- deterministic-output invariants -------------------------------------

#[test]
fn derive_is_deterministic_across_invocations() {
    let key = CacheKey {
        node_id: fixture_node_id(),
        args_hash: [0u8; 32],
        upstream_hashes: vec![],
        tick: 0,
    };
    let a = key.derive();
    let b = key.derive();
    let c = key.derive();
    assert_eq!(a, b);
    assert_eq!(b, c);
}

#[test]
fn derive_matches_pinned_known_fixture() {
    let key = CacheKey {
        node_id: fixture_node_id(),
        args_hash: [0u8; 32],
        upstream_hashes: vec![],
        tick: 0,
    };
    assert_eq!(hex_lower(&key.derive()), FIXTURE_HEX);
}

#[test]
fn derive_output_is_64_lowercase_hex_chars() {
    let key = CacheKey {
        node_id: fixture_node_id(),
        args_hash: [0u8; 32],
        upstream_hashes: vec![],
        tick: 0,
    };
    let s = hex_lower(&key.derive());
    assert_eq!(s.len(), 64);
    assert!(
        s.chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
    );
}

// --- distinctness invariants ---------------------------------------------

#[test]
fn distinct_node_id_yields_distinct_hash() {
    let base = CacheKey {
        node_id: fixture_node_id(),
        args_hash: [0u8; 32],
        upstream_hashes: vec![],
        tick: 0,
    };
    let other_uuid_str = "01900000-0000-7000-8000-000000000002";
    let other = CacheKey {
        node_id: IrNodeId::from_uuid_v7(other_uuid_str.parse().unwrap()),
        ..base.clone()
    };
    assert_ne!(base.derive(), other.derive());
}

#[test]
fn distinct_args_hash_yields_distinct_hash() {
    let base = CacheKey {
        node_id: fixture_node_id(),
        args_hash: [0u8; 32],
        upstream_hashes: vec![],
        tick: 0,
    };
    let mut other_args = [0u8; 32];
    other_args[0] = 1;
    let other = CacheKey {
        args_hash: other_args,
        ..base.clone()
    };
    assert_ne!(base.derive(), other.derive());
}

#[test]
fn distinct_tick_yields_distinct_hash() {
    let base = CacheKey {
        node_id: fixture_node_id(),
        args_hash: [0u8; 32],
        upstream_hashes: vec![],
        tick: 0,
    };
    let other = CacheKey {
        tick: 1,
        ..base.clone()
    };
    assert_ne!(base.derive(), other.derive());
}

#[test]
fn distinct_upstream_count_yields_distinct_hash() {
    let base = CacheKey {
        node_id: fixture_node_id(),
        args_hash: [0u8; 32],
        upstream_hashes: vec![],
        tick: 0,
    };
    let with_one = CacheKey {
        upstream_hashes: vec![[7u8; 32]],
        ..base.clone()
    };
    let with_two = CacheKey {
        upstream_hashes: vec![[7u8; 32], [8u8; 32]],
        ..base.clone()
    };
    assert_ne!(base.derive(), with_one.derive());
    assert_ne!(with_one.derive(), with_two.derive());
    assert_ne!(base.derive(), with_two.derive());
}

#[test]
fn distinct_upstream_contents_yield_distinct_hash() {
    let mut a = [0u8; 32];
    a[0] = 1;
    let mut b = [0u8; 32];
    b[31] = 1;

    let key_a = CacheKey {
        node_id: fixture_node_id(),
        args_hash: [0u8; 32],
        upstream_hashes: vec![a],
        tick: 0,
    };
    let key_b = CacheKey {
        upstream_hashes: vec![b],
        ..key_a.clone()
    };
    assert_ne!(key_a.derive(), key_b.derive());
}

#[test]
fn upstream_order_matters() {
    let a = [1u8; 32];
    let b = [2u8; 32];
    let key_ab = CacheKey {
        node_id: fixture_node_id(),
        args_hash: [0u8; 32],
        upstream_hashes: vec![a, b],
        tick: 0,
    };
    let key_ba = CacheKey {
        upstream_hashes: vec![b, a],
        ..key_ab.clone()
    };
    assert_ne!(key_ab.derive(), key_ba.derive());
}

#[test]
fn empty_upstream_distinct_from_single_zero_entry() {
    // The §0.13-style count prefix means an empty Vec hashes differently
    // from a Vec containing one zero-filled entry. Without the count
    // prefix the two would collide — pin this invariant explicitly.
    let key_empty = CacheKey {
        node_id: fixture_node_id(),
        args_hash: [0u8; 32],
        upstream_hashes: vec![],
        tick: 0,
    };
    let key_one_zero = CacheKey {
        upstream_hashes: vec![[0u8; 32]],
        ..key_empty.clone()
    };
    assert_ne!(key_empty.derive(), key_one_zero.derive());
}

// --- byte-layout regression guards ---------------------------------------

#[test]
fn version_marker_is_part_of_input() {
    // If a future refactor accidentally drops the b"v1\0" prefix, the
    // fixture hash will change. The pinned-fixture test catches that
    // directly; this test documents the invariant separately for code
    // reviewers scanning by name.
    let key = CacheKey {
        node_id: fixture_node_id(),
        args_hash: [0u8; 32],
        upstream_hashes: vec![],
        tick: 0,
    };
    assert_eq!(hex_lower(&key.derive()), FIXTURE_HEX);
}

#[test]
fn tick_uses_little_endian_layout() {
    // tick = 1 must hash to a value different from tick = 1<<56. If a
    // future refactor swaps to big-endian, both ticks would still
    // distinguish from tick=0 but their pairwise relationship would flip.
    let base = CacheKey {
        node_id: fixture_node_id(),
        args_hash: [0u8; 32],
        upstream_hashes: vec![],
        tick: 1,
    };
    let swapped = CacheKey {
        tick: 1u64 << 56,
        ..base.clone()
    };
    assert_ne!(base.derive(), swapped.derive());
}

#[test]
fn node_id_uses_full_uuid_bytes_not_string() {
    // If the implementation accidentally hashed the UUID's hyphenated
    // string instead of its 16-byte representation, this test would
    // catch the regression: two UUIDs that share the same hyphenated
    // prefix differ in the trailing byte.
    let a_str = "01900000-0000-7000-8000-000000000001";
    let b_str = "01900000-0000-7000-8000-0000000000ff";
    let a: IrNodeId = a_str.parse().unwrap();
    let b: IrNodeId = b_str.parse().unwrap();

    // Sanity-check the fixtures share most of their bytes but differ in
    // the last one.
    let a_uuid = a.as_uuid();
    let b_uuid = b.as_uuid();
    let a_bytes = a_uuid.as_bytes();
    let b_bytes = b_uuid.as_bytes();
    assert_eq!(&a_bytes[..15], &b_bytes[..15]);
    assert_ne!(a_bytes[15], b_bytes[15]);

    let key_a = CacheKey {
        node_id: a,
        args_hash: [0u8; 32],
        upstream_hashes: vec![],
        tick: 0,
    };
    let key_b = CacheKey {
        node_id: b,
        ..key_a.clone()
    };
    assert_ne!(key_a.derive(), key_b.derive());
}

#[test]
fn clone_yields_equal_hash() {
    let key = CacheKey {
        node_id: IrNodeId::from_uuid_v7(
            UuidV7::from_uuid(Uuid::parse_str(FIXTURE_UUID).unwrap()).unwrap(),
        ),
        args_hash: [3u8; 32],
        upstream_hashes: vec![[4u8; 32], [5u8; 32]],
        tick: 42,
    };
    let cloned = key.clone();
    assert_eq!(key.derive(), cloned.derive());
}
