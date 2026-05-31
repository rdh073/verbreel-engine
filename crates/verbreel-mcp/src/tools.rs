//! Verb-to-MCP-tool bridge.
//!
//! One concern lives here, kept separate from the protocol surface:
//! translate the shared [`verbreel_state::Engine`]'s verb registry into
//! the [`rmcp::model::Tool`] descriptors emitted by `tools/list`.
//!
//! ## Engine-backed model
//!
//! The MCP surface advertises **every** verb the Engine knows. There is
//! no whitelist: [`all_tools`] iterates [`Engine::verb_ids`] (already
//! sorted → deterministic `tools/list`) and emits one [`Tool`] per id.
//! Tool name = verb id verbatim (registry verbs dotted like
//! `project.list`, flat meta verbs stay flat: `help`, `schema`,
//! `validate_command`, `list_capabilities`). `input_schema` comes from
//! [`Engine::schema_for`] — no `verbreel-args` dep, no local schema
//! table. Adding a verb to the Engine is zero MCP edits (DIP: the Engine
//! is the one dispatcher; the surface only iterates it).
//!
//! `tools/call` routing lives in `lib.rs` and goes straight through
//! [`Engine::dispatch`]; this module owns the catalog and the surface
//! arg-shape guard only.

use std::{borrow::Cow, sync::Arc};

use rmcp::model::{JsonObject, Tool};
use serde_json::Value;
use verbreel_state::Engine;

/// Build the full `tools/list` catalog: one [`Tool`] per verb id the
/// `engine` knows, in `Engine::verb_ids()` order (sorted → deterministic).
///
/// Tool name is the verb id verbatim; `input_schema` is the engine's
/// `schema_for(id)` coerced to a `JsonObject`. The description is a
/// generic one-liner — per-verb prose belongs in the spec, not in the
/// transport shell.
#[must_use]
pub fn all_tools(engine: &Engine) -> Vec<Tool> {
    engine
        .verb_ids()
        .into_iter()
        .map(|id| {
            let schema = json_object_from(engine.schema_for(&id));
            let description = Cow::Owned(format!("Verbreel verb {id}."));
            Tool::new(Cow::Owned(id), description, Arc::new(schema))
        })
        .collect()
}

/// Coerce a `tools/call` `arguments` payload to the JSON object the
/// Engine dispatch expects. `Null` (arguments omitted) becomes `{}`; a
/// non-object body (array, string, number) is a malformed MCP
/// `arguments` field, not a verb-level error, so it is rejected at the
/// surface before the Engine sees it.
///
/// # Errors
///
/// Returns [`ArgsShapeError`] when `args` is neither an object nor null.
pub fn object_args(args: Value) -> Result<serde_json::Map<String, Value>, ArgsShapeError> {
    match args {
        Value::Object(map) => Ok(map),
        Value::Null => Ok(serde_json::Map::new()),
        other => Err(ArgsShapeError {
            got: value_kind(&other),
        }),
    }
}

/// Surface-level rejection of a malformed `tools/call` `arguments` field.
///
/// This is distinct from a verb-level [`verbreel_state::Envelope::Err`]:
/// the MCP `arguments` member is required by the protocol to be an
/// object, so a non-object body never reaches the Engine. Keeping it a
/// typed surface error (rather than fabricating a fake §0.1 envelope)
/// preserves the boundary the brief calls out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArgsShapeError {
    got: &'static str,
}

impl std::fmt::Display for ArgsShapeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "tool arguments must be a JSON object, got: {}", self.got)
    }
}

impl std::error::Error for ArgsShapeError {}

/// Name the JSON kind for the [`ArgsShapeError`] message.
fn value_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Convert the `serde_json::Value` schema returned by
/// [`Engine::schema_for`] into the `rmcp::model::JsonObject`
/// (`serde_json::Map`) that [`Tool::new`] requires. `schema_for` always
/// returns a JSON object (well-known or permissive floor); the empty-map
/// fallback exists only to keep this total.
fn json_object_from(schema: Value) -> JsonObject {
    match schema {
        Value::Object(map) => map,
        _ => JsonObject::new(),
    }
}
