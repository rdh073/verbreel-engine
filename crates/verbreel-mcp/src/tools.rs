//! Verb-to-MCP-tool bridge.
//!
//! Two concerns live here, kept separate from the protocol surface:
//!
//! 1. [`verb_as_tool`] — translate a verb name into the [`rmcp::model::Tool`]
//!    descriptor emitted by `tools/list`. Schemas are permissive at v1
//!    floor; future slices tighten via the `verbreel-args` framework.
//! 2. [`dispatch`] — route a `tools/call` payload through
//!    [`verbreel_state::default_registry`] and return the verb's `data`
//!    envelope as raw JSON.
//!
//! ## Why a whitelist
//!
//! `verbreel-state` ships ~82 production verbs; not all of them are
//! safe to expose over MCP at v1 (some require a real on-disk project,
//! event log, lock — none of which the stdio server owns yet). The
//! whitelist makes the surface explicit. Future slices append entries
//! to [`SUPPORTED_VERBS`] and keep the rest of this module untouched —
//! that's the DIP inversion called out in the slice brief.

use std::{borrow::Cow, sync::Arc};

use rmcp::model::{JsonObject, Tool};
use serde_json::{Value, json};
#[cfg(feature = "native-render")]
use verbreel_state::RenderStartArgs;
use verbreel_state::{ProjectId, default_registry, synthetic_empty_project};

/// Verbs exposed by this MCP server at the current slice.
///
/// Adding a new verb is purely additive: append its name here, and (if
/// it needs anything richer than the permissive default schema) extend
/// [`verb_as_tool`] with a matching arm.
#[cfg(not(feature = "native-render"))]
pub const SUPPORTED_VERBS: &[&str] = &["project.list"];

/// Verbs exposed by this MCP server when native render is enabled.
#[cfg(feature = "native-render")]
pub const SUPPORTED_VERBS: &[&str] = &["project.list", "render.start"];

/// Returns `true` when `verb` is exposed by this MCP server.
#[must_use]
pub fn is_supported(verb: &str) -> bool {
    SUPPORTED_VERBS.contains(&verb)
}

/// Build the `rmcp::model::Tool` descriptor returned in `tools/list`
/// for the named verb.
///
/// Returns `None` when `verb` is not in [`SUPPORTED_VERBS`] — callers
/// should treat that as "tool not found" rather than as a protocol error.
#[must_use]
pub fn verb_as_tool(verb: &str) -> Option<Tool> {
    // Resolve to the `&'static str` stored in SUPPORTED_VERBS so the
    // Tool's name carries a `'static` lifetime — Tool::name is
    // `Cow<'static, str>` and won't accept a borrow tied to the
    // caller-supplied `verb`.
    let name = SUPPORTED_VERBS.iter().copied().find(|n| *n == verb)?;
    Some(Tool::new(
        Cow::Borrowed(name),
        Cow::Borrowed(description_for(name)),
        Arc::new(permissive_input_schema()),
    ))
}

/// Iterate the full set of supported verbs as `rmcp::model::Tool`
/// entries — exactly the list returned by `tools/list`.
#[must_use]
pub fn all_tools() -> Vec<Tool> {
    SUPPORTED_VERBS
        .iter()
        .filter_map(|name| verb_as_tool(name))
        .collect()
}

/// Route a `tools/call` payload through `verbreel_state::default_registry`.
///
/// `args` is the raw JSON object the MCP client supplied. The verb's
/// `data` envelope is returned verbatim; warnings and the RFC 6902 patch
/// are dropped at v1 floor (the patch is meaningless without a backing
/// project store, and warnings have no MCP-side surface yet).
///
/// # Errors
///
/// Bubbles up an `anyhow::Error` when:
///
/// - `verb` is not in [`SUPPORTED_VERBS`];
/// - `verb` is not registered in `default_registry()` (programmer bug);
/// - the verb's `compute_patch` rejects the args (bad shape, invariant
///   violation, etc.).
pub fn dispatch(verb: &str, args: Value) -> anyhow::Result<Value> {
    #[cfg(feature = "native-render")]
    {
        dispatch_with_runtime(
            verb,
            args,
            &verbreel_runtime::RenderRuntimeConfig::from_env(),
        )
    }
    #[cfg(not(feature = "native-render"))]
    {
        dispatch_project_list_floor(verb, args)
    }
}

/// Route a `tools/call` payload with an injected native render runtime.
///
/// # Errors
///
/// Bubbles up an `anyhow::Error` when the verb is unsupported, arguments are
/// malformed, or the runtime rejects the request.
#[cfg(feature = "native-render")]
pub fn dispatch_with_runtime(
    verb: &str,
    args: Value,
    runtime: &verbreel_runtime::RenderRuntimeConfig,
) -> anyhow::Result<Value> {
    if !is_supported(verb) {
        return Err(anyhow::anyhow!("verb not supported by this server: {verb}"));
    }
    if verb == "render.start" {
        let args = object_args(args)?;
        let typed: RenderStartArgs = serde_json::from_value(Value::Object(args))
            .map_err(|err| anyhow::anyhow!("render.start: args deserialize failed: {err}"))?;
        let data = runtime.render_start(&typed)?;
        return serde_json::to_value(data).map_err(Into::into);
    }
    dispatch_project_list_floor(verb, args)
}

fn dispatch_project_list_floor(verb: &str, args: Value) -> anyhow::Result<Value> {
    if !is_supported(verb) {
        return Err(anyhow::anyhow!("verb not supported by this server: {verb}"));
    }
    let registry = default_registry();
    let verb_impl = registry
        .get(verb)
        .ok_or_else(|| anyhow::anyhow!("verb '{verb}' not in default_registry"))?;

    // The Verb trait demands a `project_id` in args; at v1 floor
    // project.list is project-agnostic, so we synthesise a fresh prior
    // and inject the same id on both sides. Matches the verbreel-cli
    // wrapper (#339).
    let project_id = ProjectId::now();
    let prior = synthetic_empty_project(project_id);

    let mut args_obj = object_args(args)?;
    args_obj
        .entry("project_id".to_string())
        .or_insert_with(|| json!(project_id));
    let args = Value::Object(args_obj);

    let (_patch, data, _warnings) = verb_impl.compute_patch(&prior, &args)?;
    Ok(data)
}

fn object_args(args: Value) -> anyhow::Result<serde_json::Map<String, Value>> {
    match args {
        Value::Object(map) => Ok(map),
        Value::Null => Ok(serde_json::Map::new()),
        other => Err(anyhow::anyhow!(
            "tool arguments must be a JSON object, got: {other}"
        )),
    }
}

/// Permissive `{"type": "object"}` schema used until the args-registry
/// surface (`verbreel-args`) carries real entries for every verb.
fn permissive_input_schema() -> JsonObject {
    let mut map = JsonObject::new();
    map.insert("type".to_string(), Value::String("object".to_string()));
    map.insert("additionalProperties".to_string(), Value::Bool(true));
    map
}

/// Static descriptions for the verbs in this slice. New entries get
/// added when their verb is whitelisted; the default branch keeps the
/// `tools/list` response well-formed even if a name is added to
/// [`SUPPORTED_VERBS`] before its description is filled in.
fn description_for(verb: &str) -> &'static str {
    match verb {
        "project.list" => {
            "List all known projects. v1 floor returns an empty array — \
             the real catalog index lands in a later slice."
        }
        "render.start" => "Start a native render for a project and write the encoded output path.",
        _ => "Verbreel verb (no description provided).",
    }
}
