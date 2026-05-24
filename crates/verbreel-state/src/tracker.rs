//! [`Tracker`] — motion / object tracker record. **Placeholder** until
//! §18 work lands. Mirrors `$defs/Tracker` from
//! `spec/project-schema.json` only at the round-trip-shape level —
//! every algorithm-specific field stays as a [`serde_json::Value`].
//!
//! Phase 2 first-slice (#19) carries this through round-trip so empty
//! `trackers: []` projects don't lose the field; full typing will come
//! with §18 work.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// A tracker record. **Placeholder shape** — every field is
/// untyped JSON until §18.
///
/// The top-level type is a thin newtype around
/// `serde_json::Map<String, serde_json::Value>` so the round-trip
/// preserves key ordering in the test fixture. Replace with a proper
/// struct in the §18 typing slice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Tracker(
    /// Inner key/value bag. Matches the shape of `$defs/Tracker` per
    /// schema, but with no field-level typing yet.
    ///
    // TODO(#19 follow-up, §18): replace with a typed struct (`id:
    // TrackerId`, `source_clip_id: ClipId`, `algorithm: String`,
    // `params: serde_json::Value`, `cache_hash: String`,
    // `sample_count: i64`, `applied_to_clip_ids: Vec<ClipId>`,
    // `created_at: String`, `last_run_at: Option<String>`).
    pub  Map<String, Value>,
);
