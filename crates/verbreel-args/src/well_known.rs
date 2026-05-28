//! Hand-curated JSON Schemas for the simplest verb argument shapes the
//! engine ships, plus a [`default_registry`] factory that assembles
//! them.
//!
//! ## What "well-known" means
//!
//! The five schemas here cover the verbs whose `*Args` types are
//! genuinely minimal:
//!
//! - `help` — `{ project_id: uuid }`
//! - `project.list` — `{ project_id: uuid }`
//! - `list_capabilities` — `{ project_id: uuid }`
//! - `font.list` — `{ project_id: uuid }`
//! - `asset.list` — `{ project_id: uuid, kind?: enum }`
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
    registry
}

fn schema_from_str(raw: &str) -> Schema {
    let value: serde_json::Value =
        serde_json::from_str(raw).expect("well-known schema literal is valid JSON");
    Schema::from_value(value).expect("well-known schema literal is a valid JSON Schema")
}
