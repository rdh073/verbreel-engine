//! `tools` module — `verb_as_tool`, `all_tools`, `dispatch` coverage.
//! Pure unit-test surface (no rmcp transport, no `tokio`).

use serde_json::{Value, json};
use verbreel_mcp::tools::{all_tools, dispatch, verb_as_tool};

#[test]
fn all_tools_exposes_the_full_surface() {
    let tools = all_tools();
    assert!(tools.len() >= 116, "got {} tools", tools.len());
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    assert!(names.contains(&"project.list"));
    assert!(names.contains(&"clip.trim"));
    assert!(names.contains(&"render.queue.add"));
}

#[test]
fn verb_as_tool_resolves_a_known_verb() {
    let tool = verb_as_tool("clip.trim").expect("clip.trim must yield a Tool");
    assert_eq!(tool.name.as_ref(), "clip.trim");
}

#[test]
fn verb_as_tool_rejects_an_unknown_verb() {
    assert!(verb_as_tool("totally.fake.verb").is_none());
}

#[test]
fn well_known_verb_tool_carries_an_object_schema() {
    // clip.list has a hand-curated schema in verbreel-args.
    let tool = verb_as_tool("clip.list").unwrap();
    assert_eq!(
        tool.input_schema.get("type").and_then(Value::as_str),
        Some("object")
    );
}

#[test]
fn dispatch_project_list_returns_projects_key() {
    let data = dispatch("project.list", json!({})).expect("project.list dispatches");
    assert!(data.get("projects").is_some(), "data: {data}");
}

#[test]
fn dispatch_accepts_null_arguments_as_empty_object() {
    let data = dispatch("project.list", Value::Null).expect("null coerces to {}");
    assert!(data.get("projects").is_some());
}

#[test]
fn dispatch_rejects_non_object_arguments() {
    assert!(dispatch("project.list", json!([1, 2, 3])).is_err());
}

#[test]
fn dispatch_rejects_unknown_verb() {
    assert!(dispatch("totally.fake.verb", json!({})).is_err());
}
