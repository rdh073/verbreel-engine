//! Hand-curated JSON Schemas for the simplest verb argument shapes the
//! engine ships, plus a [`default_registry`] factory that assembles
//! them.
//!
//! ## What "well-known" means
//!
//! The schemas here cover the verbs whose `*Args` types are
//! genuinely minimal — `project_id` plus, at most, a handful of simple
//! scalar / enum fields:
//!
//! - `help` — `{ project_id: uuid, topic?: string|null }`
//! - `project.list` — `{ project_id: uuid }`
//! - `list_capabilities` — `{ project_id: uuid }`
//! - `font.list` — `{ project_id: uuid }`
//! - `asset.list` — `{ project_id: uuid, kind?: enum|null }`
//! - `tracker.list` — `{ project_id: uuid }`
//! - `compound.flatten` — `{ project_id: uuid, clip: string }`
//! - `timeline.undo` — `{ project_id: uuid, steps?: int>=1|null }`
//! - `timeline.history` — `{ project_id: uuid, limit?: int|null,
//!   since?: string|null, include_undone?: bool|null }`
//! - `keyframe.list` — `{ project_id: uuid, clip: string,
//!   property?: string|null }`
//! - `clip.list` — `{ project_id: uuid, track?: string|null,
//!   at_tk?: int|null }`
//!
//! Verbs with richer args (clip composition, render queue ops, asset
//! import) land in follow-up slices once their schema shapes settle.
//!
//! ## Why hand-curated, not derived
//!
//! The first slice of [`crate`] proves the validator + registry
//! framework with explicit schema strings. A future slice introduces
//! `schemars`-derive (or equivalent) so the same shapes can be
//! generated from the typed `*Args` structs in `verbreel-state`. This
//! module is the bridge — it gives downstream callers
//! ([`verbreel-mcp`], [`verbreel-http`]) a single function call to
//! get a usable registry today.
//!
//! ## Schema invariants
//!
//! Every schema in this module:
//!
//! - is `"type": "object"`,
//! - sets `"additionalProperties": false` (so callers cannot smuggle
//!   extra keys past the validator — matches the
//!   `#[serde(deny_unknown_fields)]` discipline on the corresponding
//!   `*Args` structs),
//! - requires `project_id` and constrains it to `"type": "string"`
//!   with `"format": "uuid"`.

use crate::registry::ArgsRegistry;
use crate::schema::Schema;

/// JSON Schema for `help` arguments.
///
/// The optional `topic` field mirrors `HelpArgs.topic: Option<String>`
/// in the state crate — `None` lists nouns, a single noun lists verbs
/// under it, a full verb id returns that verb's doc. Omitting `topic`
/// from this schema would reject the documented multi-mode call shape.
pub const HELP_SCHEMA: &str = r#"{
  "type": "object",
  "additionalProperties": false,
  "required": ["project_id"],
  "properties": {
    "project_id": { "type": "string", "format": "uuid" },
    "topic": { "type": ["string", "null"] }
  }
}"#;

/// JSON Schema for `project.list` arguments.
pub const PROJECT_LIST_SCHEMA: &str = r#"{
  "type": "object",
  "additionalProperties": false,
  "required": ["project_id"],
  "properties": {
    "project_id": { "type": "string", "format": "uuid" }
  }
}"#;

/// JSON Schema for `list_capabilities` arguments.
pub const LIST_CAPABILITIES_SCHEMA: &str = r#"{
  "type": "object",
  "additionalProperties": false,
  "required": ["project_id"],
  "properties": {
    "project_id": { "type": "string", "format": "uuid" }
  }
}"#;

/// JSON Schema for `font.list` arguments.
pub const FONT_LIST_SCHEMA: &str = r#"{
  "type": "object",
  "additionalProperties": false,
  "required": ["project_id"],
  "properties": {
    "project_id": { "type": "string", "format": "uuid" }
  }
}"#;

/// JSON Schema for `asset.list` arguments.
///
/// Adds an optional `kind` discriminator over the four
/// [`verbreel_state::AssetKindFilter`](../../verbreel_state/enum.AssetKindFilter.html)
/// variants. The state crate's `AssetListArgs` uses
/// `#[serde(rename_all = "lowercase")]`, so the enum strings here are
/// lower-case.
pub const ASSET_LIST_SCHEMA: &str = r#"{
  "type": "object",
  "additionalProperties": false,
  "required": ["project_id"],
  "properties": {
    "project_id": { "type": "string", "format": "uuid" },
    "kind": {
      "anyOf": [
        { "type": "string", "enum": ["video", "audio", "image", "subtitle"] },
        { "type": "null" }
      ]
    }
  }
}"#;

/// JSON Schema for `tracker.list` arguments.
///
/// `TrackerListArgs` is `{ project_id }` only — the minimal mirror of
/// `project.list`, with no optional fields.
pub const TRACKER_LIST_SCHEMA: &str = r#"{
  "type": "object",
  "additionalProperties": false,
  "required": ["project_id"],
  "properties": {
    "project_id": { "type": "string", "format": "uuid" }
  }
}"#;

/// JSON Schema for `compound.flatten` arguments.
///
/// `CompoundFlattenArgs.clip` is a required selector string
/// (`<UUIDv7>` or `clip:<UUIDv7>`); the schema constrains its JSON type
/// only — selector parsing happens verb-side.
pub const COMPOUND_FLATTEN_SCHEMA: &str = r#"{
  "type": "object",
  "additionalProperties": false,
  "required": ["project_id", "clip"],
  "properties": {
    "project_id": { "type": "string", "format": "uuid" },
    "clip": { "type": "string" }
  }
}"#;

/// JSON Schema for `timeline.undo` arguments.
///
/// `TimelineUndoArgs.steps: Option<i64>` defaults to `1` when omitted
/// and the verb's local schema requires `steps >= 1`. Encoding the
/// `minimum: 1` here moves that `E_SCHEMA_VIOLATION` to the args
/// boundary; the integer type matches `i64`.
pub const TIMELINE_UNDO_SCHEMA: &str = r#"{
  "type": "object",
  "additionalProperties": false,
  "required": ["project_id"],
  "properties": {
    "project_id": { "type": "string", "format": "uuid" },
    "steps": {
      "anyOf": [
        { "type": "integer", "minimum": 1 },
        { "type": "null" }
      ]
    }
  }
}"#;

/// JSON Schema for `timeline.history` arguments.
///
/// Mirrors `TimelineHistoryArgs`: `limit: Option<i64>`,
/// `since: Option<String>`, `include_undone: Option<bool>`. Each is
/// nullable so the omitted-vs-`null` parity that `Option<T>` gives the
/// typed struct is preserved at the schema boundary.
pub const TIMELINE_HISTORY_SCHEMA: &str = r#"{
  "type": "object",
  "additionalProperties": false,
  "required": ["project_id"],
  "properties": {
    "project_id": { "type": "string", "format": "uuid" },
    "limit": { "type": ["integer", "null"] },
    "since": { "type": ["string", "null"] },
    "include_undone": { "type": ["boolean", "null"] }
  }
}"#;

/// JSON Schema for `keyframe.list` arguments.
///
/// `KeyframeListArgs.clip` is a required bare-`UUIDv7` selector string;
/// `property` is an optional dotted-path filter. Both are constrained
/// to JSON `string` only — selector / path semantics are verb-side.
pub const KEYFRAME_LIST_SCHEMA: &str = r#"{
  "type": "object",
  "additionalProperties": false,
  "required": ["project_id", "clip"],
  "properties": {
    "project_id": { "type": "string", "format": "uuid" },
    "clip": { "type": "string" },
    "property": { "type": ["string", "null"] }
  }
}"#;

/// JSON Schema for `clip.list` arguments.
///
/// Mirrors `ClipListArgs`: `track: Option<String>` (a `UUIDv7`
/// selector) and `at_tk: Option<i64>` (a timeline tick filter). Both
/// are nullable; `at_tk` is an integer to match `i64`.
pub const CLIP_LIST_SCHEMA: &str = r#"{
  "type": "object",
  "additionalProperties": false,
  "required": ["project_id"],
  "properties": {
    "project_id": { "type": "string", "format": "uuid" },
    "track": { "type": ["string", "null"] },
    "at_tk": { "type": ["integer", "null"] }
  }
}"#;

/// Assemble an [`ArgsRegistry`] pre-loaded with every schema in this
/// module.
///
/// # Panics
///
/// Panics only if a schema literal in this module fails the
/// `jsonschema::Validator` compile check — an upstream bug in the
/// const strings, not a runtime concern. Test `every_well_known_schema_compiles`
/// in `tests/well_known.rs` pins this contract.
#[must_use]
pub fn default_registry() -> ArgsRegistry {
    let mut registry = ArgsRegistry::new();
    registry.register("help", schema_from_str(HELP_SCHEMA));
    registry.register("project.list", schema_from_str(PROJECT_LIST_SCHEMA));
    registry.register(
        "list_capabilities",
        schema_from_str(LIST_CAPABILITIES_SCHEMA),
    );
    registry.register("font.list", schema_from_str(FONT_LIST_SCHEMA));
    registry.register("asset.list", schema_from_str(ASSET_LIST_SCHEMA));
    registry.register("tracker.list", schema_from_str(TRACKER_LIST_SCHEMA));
    registry.register("compound.flatten", schema_from_str(COMPOUND_FLATTEN_SCHEMA));
    registry.register("timeline.undo", schema_from_str(TIMELINE_UNDO_SCHEMA));
    registry.register("timeline.history", schema_from_str(TIMELINE_HISTORY_SCHEMA));
    registry.register("keyframe.list", schema_from_str(KEYFRAME_LIST_SCHEMA));
    registry.register("clip.list", schema_from_str(CLIP_LIST_SCHEMA));
    registry
}

fn schema_from_str(raw: &str) -> Schema {
    let value: serde_json::Value =
        serde_json::from_str(raw).expect("well-known schema literal is valid JSON");
    Schema::from_value(value).expect("well-known schema literal is a valid JSON Schema")
}
