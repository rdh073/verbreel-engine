//! Verb-to-MCP-tool bridge.
//!
//! Two concerns, kept separate from the protocol surface in `lib.rs`:
//!
//! 1. [`all_tools`] / [`verb_as_tool`] — project the engine capability
//!    catalog ([`verbreel_agent::Capabilities`]) into the
//!    [`rmcp::model::Tool`] descriptors `tools/list` emits. Every
//!    registered verb is exposed, each carrying its real JSON args schema
//!    where one is registered (else a permissive `{"type":"object"}`).
//!    This is the full surface — no whitelist.
//! 2. [`dispatch`] — the *stateless* evaluation path: route a verb
//!    through [`verbreel_state::default_registry`] against a synthetic
//!    empty project and return its `data` envelope. Used when the server
//!    is not bound to a project; the project-backed path
//!    ([`crate::VerbreelServer`] with a workspace project) lives in
//!    `lib.rs` and persists through a [`verbreel_agent::Session`].

use std::{borrow::Cow, sync::Arc};

use rmcp::model::{JsonObject, Tool};
use serde_json::{Value, json};
use verbreel_agent::Capabilities;
use verbreel_state::{ProjectId, default_registry, synthetic_empty_project};

/// Build the `rmcp::model::Tool` descriptor for one verb id.
///
/// Returns `None` when `verb` is not a registered engine verb.
#[must_use]
pub fn verb_as_tool(verb: &str) -> Option<Tool> {
    let caps = Capabilities::current();
    let info = caps.get(verb)?;
    Some(tool_from(&info.id, &info.domain, info.args_schema.as_ref()))
}

/// Every registered verb as a `tools/list` entry, with real args schemas
/// where known.
#[must_use]
pub fn all_tools() -> Vec<Tool> {
    Capabilities::current()
        .verbs
        .iter()
        .map(|v| tool_from(&v.id, &v.domain, v.args_schema.as_ref()))
        .collect()
}

/// Construct one [`Tool`] from a verb id, its domain, and optional schema.
fn tool_from(verb: &str, domain: &str, schema: Option<&Value>) -> Tool {
    let input_schema = schema
        .and_then(Value::as_object)
        .map_or_else(permissive_input_schema, Clone::clone);
    Tool::new(
        Cow::Owned(verb.to_string()),
        Cow::Owned(format!("Verbreel `{verb}` verb (domain `{domain}`).")),
        Arc::new(input_schema),
    )
}

/// Stateless evaluation: route `verb` through `default_registry` against
/// a synthetic empty project and return its `data` envelope.
///
/// Read-only verbs (e.g. `project.list`, `list_capabilities`) return real
/// answers; editing verbs that need existing project state will surface
/// the verb's own error (no project bound means no clips/tracks to act
/// on). For persisted edits, bind the server to a project — see
/// [`crate::VerbreelServer`].
///
/// # Errors
///
/// - `verb` is not registered in `default_registry`;
/// - `args` is neither a JSON object nor `null`;
/// - the verb's `compute_patch` rejects the args.
pub fn dispatch(verb: &str, args: Value) -> anyhow::Result<Value> {
    let registry = default_registry();
    let verb_impl = registry
        .get(verb)
        .ok_or_else(|| anyhow::anyhow!("verb not registered: {verb}"))?;

    let project_id = ProjectId::now();
    let prior = synthetic_empty_project(project_id);

    let mut args_obj = match args {
        Value::Object(map) => map,
        Value::Null => serde_json::Map::new(),
        other => {
            return Err(anyhow::anyhow!(
                "tool arguments must be a JSON object, got: {other}"
            ));
        }
    };
    args_obj
        .entry("project_id".to_string())
        .or_insert_with(|| json!(project_id));

    let (_patch, data, _warnings) = verb_impl.compute_patch(&prior, &Value::Object(args_obj))?;
    Ok(data)
}

/// Permissive `{"type": "object"}` schema for verbs whose typed schema
/// has not yet migrated into `verbreel-args`.
fn permissive_input_schema() -> JsonObject {
    let mut map = JsonObject::new();
    map.insert("type".to_string(), Value::String("object".to_string()));
    map.insert("additionalProperties".to_string(), Value::Bool(true));
    map
}
