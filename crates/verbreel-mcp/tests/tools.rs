//! `tools` module — `verb_as_tool`, `dispatch`, `SUPPORTED_VERBS`
//! coverage. Pure unit-test surface (no rmcp transport, no `tokio`).

use serde_json::{Value, json};
use verbreel_mcp::tools::{SUPPORTED_VERBS, all_tools, dispatch, is_supported, verb_as_tool};

#[test]
fn supported_verbs_contains_project_list() {
    assert!(
        SUPPORTED_VERBS.contains(&"project.list"),
        "v1 floor must whitelist project.list, got: {SUPPORTED_VERBS:?}"
    );
}

#[test]
fn is_supported_accepts_project_list() {
    assert!(is_supported("project.list"));
}

#[test]
fn is_supported_rejects_unknown_verb() {
    // Use a verb that exists in default_registry but is NOT whitelisted —
    // catches accidental "default-all" regressions.
    assert!(!is_supported("project.set_metadata"));
    assert!(!is_supported("totally.fake.verb"));
}

#[test]
fn verb_as_tool_returns_tool_for_whitelisted_verb() {
    let tool = verb_as_tool("project.list").expect("project.list must yield a Tool");
    assert_eq!(tool.name.as_ref(), "project.list");
    assert!(
        tool.description.as_ref().map(AsRef::as_ref)
            == Some(
                "List all known projects. v1 floor returns an empty array — the real catalog index lands in a later slice."
            ),
        "description must match the curated copy, got: {:?}",
        tool.description
    );
}

#[test]
fn verb_as_tool_returns_none_for_non_whitelisted_verb() {
    assert!(verb_as_tool("project.set_metadata").is_none());
    assert!(verb_as_tool("totally.fake.verb").is_none());
}

#[test]
fn verb_as_tool_input_schema_is_permissive_object() {
    let tool = verb_as_tool("project.list").unwrap();
    let schema: &serde_json::Map<String, Value> = tool.input_schema.as_ref();
    assert_eq!(
        schema.get("type").and_then(Value::as_str),
        Some("object"),
        "v1 floor must advertise an object schema, got: {schema:?}"
    );
}

#[test]
fn all_tools_yields_one_entry_at_v1_floor() {
    let tools = all_tools();
    assert_eq!(tools.len(), SUPPORTED_VERBS.len());
    assert_eq!(tools[0].name.as_ref(), "project.list");
}

#[test]
fn dispatch_project_list_returns_data_with_projects_key() {
    let data = dispatch("project.list", json!({})).expect("dispatch must succeed at v1 floor");
    assert!(
        data.get("projects").is_some(),
        "v1 envelope must expose `projects` key, got: {data}"
    );
}

#[test]
fn dispatch_project_list_returns_empty_array() {
    let data = dispatch("project.list", json!({})).unwrap();
    let arr = data
        .get("projects")
        .and_then(Value::as_array)
        .expect("`projects` must be an array");
    assert!(
        arr.is_empty(),
        "v1 floor must return empty projects list, got: {data}"
    );
}

#[test]
fn dispatch_accepts_null_arguments_as_empty_object() {
    let data = dispatch("project.list", Value::Null).expect("Null args must be coerced to {}");
    assert!(data.get("projects").is_some());
}

#[test]
fn dispatch_rejects_non_object_arguments() {
    let err =
        dispatch("project.list", json!([1, 2, 3])).expect_err("array arguments must be rejected");
    let msg = err.to_string();
    assert!(
        msg.contains("must be a JSON object"),
        "error should explain the shape requirement, got: {msg}"
    );
}

#[test]
fn dispatch_rejects_non_whitelisted_verb_even_if_in_registry() {
    // project.set_metadata IS in default_registry but NOT in SUPPORTED_VERBS;
    // the whitelist must short-circuit before the registry lookup.
    let err = dispatch("project.set_metadata", json!({}))
        .expect_err("non-whitelisted verbs must be rejected");
    let msg = err.to_string();
    assert!(
        msg.contains("not supported by this server"),
        "error should call out the whitelist, got: {msg}"
    );
}

#[test]
fn dispatch_rejects_unknown_verb() {
    let err = dispatch("totally.fake.verb", json!({})).expect_err("unknown verbs must be rejected");
    assert!(err.to_string().contains("not supported by this server"));
}

#[test]
fn dispatch_is_deterministic_across_invocations() {
    // The synthesised ProjectId varies, but project.list's data
    // envelope must NOT — otherwise the MCP tool would yield divergent
    // payloads on identical inputs.
    let a = dispatch("project.list", json!({})).unwrap();
    let b = dispatch("project.list", json!({})).unwrap();
    assert_eq!(a, b, "v1 envelope must be stable across runs");
}

#[test]
fn dispatch_caller_supplied_project_id_is_preserved() {
    // The caller-supplied project_id must not be overwritten by the
    // internal synthesised id — the contract is "only fill in if
    // missing". Use a real v7 UUID because the engine rejects the nil
    // UUID via §0.13 invariants.
    let pid = uuid::Uuid::now_v7();
    let data = dispatch("project.list", json!({ "project_id": pid }))
        .expect("dispatch must accept caller-supplied project_id");
    // Envelope itself is project-agnostic at v1, so just confirm the
    // call succeeded with the override.
    assert!(data.get("projects").is_some());
}
