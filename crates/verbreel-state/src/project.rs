//! [`Project`] — root of the project graph. Mirrors the top-level
//! object in `spec/project-schema.json`.
//!
//! 16 root fields per schema: 12 required + 4 optional with defaults.
//! See `spec/project-schema.json` (root, not `$defs`) for the full
//! contract.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};
use verbreel_types::{EventId, ProjectId, TICK_RATE_HZ, Tick};

use crate::asset::Asset;
use crate::canvas::Canvas;
use crate::marker::Marker;
use crate::track::Track;
use crate::tracker::Tracker;

/// The schema version this engine writes. Spec: `SemVer 2.0.0` string
/// matching the schema's `^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)…$`
/// pattern. The Phase 2 first-slice ships `1.0.0`.
pub const SCHEMA_VERSION: &str = "1.0.0";

/// The root of a Verbreel project graph.
///
/// Schema reference: `spec/project-schema.json` (top-level object —
/// the schema is the project type, not under `$defs`).
///
/// `tick_rate_hz` is constrained to `const: 240000` by the schema; the
/// typed shape enforces this at deserialize time via a custom
/// validator (see [`deserialize_tick_rate_hz`]) and at serialize time
/// by re-emitting [`TICK_RATE_HZ`] from `verbreel-types`. The field is
/// kept as a plain `u32` rather than a unit type so the round-trip JSON
/// shape stays `"tick_rate_hz": 240000` (a unit type would serialize
/// to `null`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    /// Project ID — strongly-typed [`ProjectId`] (`UuidV7`).
    pub id: ProjectId,

    /// `SemVer` string for the schema this document was written
    /// against. First slice emits [`SCHEMA_VERSION`] (`"1.0.0"`).
    pub schema_version: String,

    /// Fixed tick rate at 240,000 Hz. Schema enforces `const: 240000`;
    /// the typed deserializer rejects any other value via
    /// [`deserialize_tick_rate_hz`].
    #[serde(
        serialize_with = "serialize_tick_rate_hz",
        deserialize_with = "deserialize_tick_rate_hz"
    )]
    pub tick_rate_hz: u32,

    /// Human display name.
    pub name: String,

    /// RFC 3339 timestamp of project creation.
    ///
    // TODO(#19 follow-up): replace `String` with a `chrono::DateTime`
    // or `time::OffsetDateTime` newtype once the time crate is wired
    // into the workspace.
    pub created_at: String,

    /// RFC 3339 timestamp of the last in-memory mutation. **Stripped
    /// from `project_hash`** per the `verbreel-canon` projection rule
    /// — two projects identical apart from `updated_at` hash equally.
    pub updated_at: String,

    /// Canvas geometry and background.
    pub canvas: Canvas,

    /// Frame-rate numerator.
    pub fps_num: u32,

    /// Frame-rate denominator.
    pub fps_den: u32,

    /// Total project duration in ticks at [`TICK_RATE_HZ`].
    pub duration_tk: Tick,

    /// Ordered list of tracks (rendering bottom-to-top in the UI's
    /// natural sense; the spec leaves visual layering to the engine
    /// and §0.13 will codify it).
    pub tracks: Vec<Track>,

    /// Project-level asset registry. See [`Asset`] for the tagged-union
    /// variant shape.
    pub assets: Vec<Asset>,

    /// Project-level time markers. Optional in schema; defaults to
    /// `[]` if absent. See [`Marker`].
    #[serde(default)]
    pub markers: Vec<Marker>,

    /// Engine / agent metadata. Optional in schema; defaults to `{}`.
    /// Uses [`serde_json::Map`] to preserve insertion order (matters
    /// for canonicalization of round-trip fixtures).
    #[serde(default)]
    pub metadata: Map<String, Value>,

    /// The most recent event id this project was saved against, or
    /// `None` for a project that has not been saved yet. **Stripped
    /// from `project_hash`** per the canon projection rule.
    ///
    /// Schema shape is `anyOf: [Uuid, null]`. The Rust idiomatic form
    /// `Option<EventId>` round-trips cleanly: `null` ↔ `None`,
    /// UUID string ↔ `Some(EventId)`.
    #[serde(default)]
    pub last_saved_event_id: Option<EventId>,

    /// Tracker records. **Placeholder** until §18 work lands. Optional
    /// in schema; defaults to `[]`.
    #[serde(default)]
    pub trackers: Vec<Tracker>,
}

/// Serializer for `tick_rate_hz` — always emits [`TICK_RATE_HZ`] so
/// the on-disk JSON has the schema-mandated `240000` regardless of
/// the in-memory value (which is itself guarded by the deserializer).
///
/// The `&u32` signature is mandated by serde's `serialize_with` —
/// clippy's `trivially_copy_pass_by_ref` lint would prefer `u32` by
/// value but serde's calling convention takes a reference.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn serialize_tick_rate_hz<S>(_v: &u32, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_u32(TICK_RATE_HZ)
}

/// Deserializer for `tick_rate_hz` — rejects any value other than
/// [`TICK_RATE_HZ`] (`240000`). The schema's `const: 240000` constraint
/// is reproduced here so the typed shape catches the violation at the
/// serde boundary, not only at the schema-validation layer.
fn deserialize_tick_rate_hz<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::Error;
    let v = u32::deserialize(deserializer)?;
    if v == TICK_RATE_HZ {
        Ok(v)
    } else {
        Err(D::Error::custom(format!(
            "tick_rate_hz must be {TICK_RATE_HZ} (spec §0.2 fixed-rate invariant), got {v}"
        )))
    }
}
