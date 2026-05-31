//! `tools` module — `all_tools(&Engine)` catalog + schema passthrough,
//! and the `object_args` surface arg-shape guard. Pure unit-test surface
//! (no rmcp transport beyond the `Tool` model).

use serde_json::Value;
use tempfile::TempDir;
use verbreel_mcp::tools::{all_tools, object_args};
use verbreel_state::Engine;

/// Build an Engine over an isolated `TempDir` home, the same way the
/// server's `with_home` does.
fn engine() -> (TempDir, Engine) {
    let home = TempDir::new().expect("tempdir");
    let eng = Engine::new(home.path());
    (home, eng)
}

#[test]
fn all_tools_contains_project_list() {
    let (_home, eng) = engine();
    let tools = all_tools(&eng);
    assert!(
        tools.iter().any(|t| t.name.as_ref() == "project.list"),
        "full surface must advertise project.list"
    );
}

#[test]
fn project_list_input_schema_is_object() {
    let (_home, eng) = engine();
    let tools = all_tools(&eng);
    let tool = tools
        .iter()
        .find(|t| t.name.as_ref() == "project.list")
        .expect("project.list tool");
    let schema: &serde_json::Map<String, Value> = tool.input_schema.as_ref();
    assert_eq!(
        schema.get("type").and_then(Value::as_str),
        Some("object"),
        "schema from Engine::schema_for must be an object, got: {schema:?}"
    );
}

#[test]
fn all_tools_len_equals_verb_ids_and_is_full_surface() {
    let (_home, eng) = engine();
    let tools = all_tools(&eng);
    assert_eq!(
        tools.len(),
        eng.verb_ids().len(),
        "one tool per verb id — no whitelist subset"
    );
    assert!(
        tools.len() > 100,
        "full surface should expose the whole registry, got {}",
        tools.len()
    );
}

#[test]
fn tools_list_advertises_full_surface() {
    let (_home, eng) = engine();
    let tools = all_tools(&eng);
    assert_eq!(tools.len(), eng.verb_ids().len());
    assert!(
        tools.len() >= 120,
        "regression guard against a curated subset, got {}",
        tools.len()
    );
}

#[test]
fn tools_list_is_sorted_and_deterministic() {
    let (_home, eng) = engine();
    let names: Vec<String> = all_tools(&eng)
        .iter()
        .map(|t| t.name.as_ref().to_string())
        .collect();
    assert_eq!(
        names,
        eng.verb_ids(),
        "tool order must equal verb_ids order"
    );

    // Two independent builds over the same registry are byte-identical.
    let (_home2, eng2) = engine();
    let names2: Vec<String> = all_tools(&eng2)
        .iter()
        .map(|t| t.name.as_ref().to_string())
        .collect();
    assert_eq!(
        names, names2,
        "tools/list must be deterministic across builds"
    );
}

#[test]
fn tool_name_equals_verb_id_verbatim() {
    let (_home, eng) = engine();
    let names: Vec<String> = all_tools(&eng)
        .iter()
        .map(|t| t.name.as_ref().to_string())
        .collect();
    // Registry verb stays dotted.
    assert!(names.contains(&"project.list".to_string()));
    // Meta verbs stay flat — no synthetic `meta.` prefix.
    for flat in ["help", "schema", "list_capabilities"] {
        assert!(
            names.contains(&flat.to_string()),
            "meta verb {flat} must appear flat, not namespaced"
        );
        assert!(
            !names.iter().any(|n| n == &format!("meta.{flat}")),
            "must NOT add a synthetic meta.{flat}"
        );
    }
}

#[test]
fn input_schema_comes_from_engine_schema_for() {
    let (_home, eng) = engine();
    let tools = all_tools(&eng);
    // clip.list has a well-known schema; project.list too — pick one and
    // assert the tool's input_schema equals schema_for(id) coerced to a map.
    for id in ["project.list", "clip.list"] {
        let tool = tools
            .iter()
            .find(|t| t.name.as_ref() == id)
            .unwrap_or_else(|| panic!("{id} tool present"));
        let from_engine = match eng.schema_for(id) {
            Value::Object(m) => m,
            other => panic!("schema_for({id}) must be an object, got {other}"),
        };
        let on_tool: &serde_json::Map<String, Value> = tool.input_schema.as_ref();
        assert_eq!(
            on_tool, &from_engine,
            "{id} input_schema must equal Engine::schema_for output verbatim"
        );
    }
}

#[test]
fn object_args_coerces_null_to_empty_object() {
    let map = object_args(Value::Null).expect("null coerces to {}");
    assert!(map.is_empty());
}

#[test]
fn object_args_passes_object_through() {
    let map = object_args(serde_json::json!({ "a": 1 })).expect("object passes through");
    assert_eq!(map.get("a").and_then(Value::as_i64), Some(1));
}

#[test]
fn object_args_rejects_non_object() {
    let err = object_args(serde_json::json!([1, 2, 3])).expect_err("array must be rejected");
    assert!(
        err.to_string().contains("must be a JSON object"),
        "error should explain the shape requirement, got: {err}"
    );
}
