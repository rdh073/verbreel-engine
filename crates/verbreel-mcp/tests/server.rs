//! `VerbreelServer` — protocol-surface coverage via the async pure
//! helpers [`VerbreelServer::tools_list`] and
//! [`VerbreelServer::call_tool_value`].
//!
//! The `ServerHandler` trait methods themselves need a `RequestContext`
//! that wraps a `Peer<RoleServer>`, and `Peer::new` is `pub(crate)` in
//! rmcp 1.7. Exercising the protocol layer end-to-end therefore requires
//! the rmcp `client` feature + `tokio::io::duplex` plumbing — out of
//! scope here. The pure helpers carry the same logic; the `ServerHandler`
//! impl is a thin wrapper around them.

use serde_json::{Value, json};
use tempfile::TempDir;
use verbreel_mcp::VerbreelServer;
use verbreel_state::Envelope;

/// Build a server over an isolated `TempDir` home.
fn server() -> (TempDir, VerbreelServer) {
    let home = TempDir::new().expect("tempdir");
    let srv = VerbreelServer::with_home(home.path());
    (home, srv)
}

#[test]
fn server_constructs_via_new() {
    let _server = VerbreelServer::new();
}

#[tokio::test]
async fn tools_list_includes_project_list() {
    let (_home, server) = server();
    let result = server.tools_list().await;
    let names: Vec<&str> = result.tools.iter().map(|t| t.name.as_ref()).collect();
    assert!(
        names.contains(&"project.list"),
        "tools/list must advertise project.list, got first few: {:?}",
        &names[..names.len().min(5)]
    );
}

#[tokio::test]
async fn tools_list_includes_render_start() {
    // render.start is now a plain registry verb served by Engine::dispatch
    // — the native-render apparatus is gone; it must still be advertised.
    let (_home, server) = server();
    let result = server.tools_list().await;
    let names: Vec<&str> = result.tools.iter().map(|t| t.name.as_ref()).collect();
    assert!(
        names.contains(&"render.start"),
        "render.start must appear as a plain verb id"
    );
}

#[tokio::test]
async fn tools_list_has_no_next_cursor_at_v1_floor() {
    let (_home, server) = server();
    let result = server.tools_list().await;
    assert!(
        result.next_cursor.is_none(),
        "with_all_items surfaces every tool in one page"
    );
}

#[tokio::test]
async fn tools_list_count_matches_verb_ids() {
    let (_home, server) = server();
    let result = server.tools_list().await;
    assert!(
        result.tools.len() > 100,
        "list_tools must reflect the full verb surface, got {}",
        result.tools.len()
    );
}

#[tokio::test]
async fn tools_list_entries_carry_descriptions() {
    let (_home, server) = server();
    let result = server.tools_list().await;
    for tool in &result.tools {
        assert!(
            tool.description.is_some(),
            "every advertised tool must carry a description, got: {tool:?}"
        );
    }
}

#[tokio::test]
async fn call_tool_value_project_list_returns_v1_envelope() {
    let (_home, server) = server();
    let env = server
        .call_tool_value("project.list", json!({}))
        .await
        .expect("project.list args are an object");
    let Envelope::Ok { data, .. } = &env else {
        panic!("project.list must be ok:true, got {env:?}");
    };
    assert!(
        data.get("projects").and_then(Value::as_array).is_some(),
        "envelope data must expose `projects` as a JSON array, got: {data}"
    );
}

#[tokio::test]
async fn call_tool_value_project_list_is_empty_at_v1_floor() {
    let (_home, server) = server();
    let env = server
        .call_tool_value("project.list", json!({}))
        .await
        .unwrap();
    let Envelope::Ok { data, .. } = &env else {
        panic!("expected ok envelope, got {env:?}");
    };
    let arr = data.get("projects").and_then(Value::as_array).unwrap();
    assert!(arr.is_empty(), "no projects registered → empty list");
}

#[tokio::test]
async fn call_tool_value_unknown_verb_yields_e_unknown_verb() {
    let (_home, server) = server();
    let env = server
        .call_tool_value("totally.fake.verb", json!({}))
        .await
        .expect("args shape is fine; the verb-level failure is in the envelope");
    let Envelope::Err { code, .. } = &env else {
        panic!("unknown verb must be an err envelope, got {env:?}");
    };
    assert_eq!(
        code, "E_UNKNOWN_VERB",
        "unknown verb resolves at the Engine, not a surface 'not supported' reject"
    );
}

#[tokio::test]
async fn call_tool_value_render_start_surfaces_engine_error() {
    // render flows through Engine::dispatch now. With no open project the
    // Engine rejects render.start with E_PROJECT_NOT_FOUND — proving the
    // verb reaches the Engine rather than a deleted runtime path.
    let (_home, server) = server();
    let env = server
        .call_tool_value(
            "render.start",
            json!({
                "project_id": "0192f000-0000-7000-8000-000000000000",
                "preset": "definitely-not-a-preset",
                "out_path": "exports/out.mp4",
            }),
        )
        .await
        .expect("args are an object");
    assert!(
        matches!(env, Envelope::Err { .. }),
        "render.start with no open project must be an err envelope, got {env:?}"
    );
}

#[tokio::test]
async fn call_tool_value_mutating_verb_without_open_project_is_e_project_not_found() {
    // track.add with a random project_id and no open project → the Engine
    // returns E_PROJECT_NOT_FOUND, proving the surface passes project_id
    // through (§0.12) and synthesises nothing.
    let (_home, server) = server();
    let env = server
        .call_tool_value(
            "track.add",
            json!({ "project_id": "0192f000-0000-7000-8000-000000000000", "kind": "video" }),
        )
        .await
        .expect("args are an object");
    let Envelope::Err { code, .. } = &env else {
        panic!("expected err envelope, got {env:?}");
    };
    assert_eq!(code, "E_PROJECT_NOT_FOUND");
}

#[tokio::test]
async fn call_tool_value_project_less_read_needs_no_project_id() {
    // §0.12: the surface injects nothing. help / schema / list_capabilities
    // succeed with no project_id in args — the Engine routes the
    // project-less read against its own synthetic prior. `schema` still
    // needs its own required `target` arg (a verb-level requirement, not a
    // project-context one); the point is that none of them needs project_id.
    let (_home, server) = server();
    let cases = [
        ("help", json!({})),
        ("schema", json!({ "target": "project" })),
        ("list_capabilities", json!({})),
    ];
    for (id, args) in cases {
        let env = server
            .call_tool_value(id, args)
            .await
            .unwrap_or_else(|e| panic!("{id} args are an object: {e}"));
        assert!(
            env.is_ok(),
            "{id} must succeed with no project_id, got {env:?}"
        );
    }
}

#[tokio::test]
async fn call_tool_value_rejects_non_object_arguments() {
    let (_home, server) = server();
    let err = server
        .call_tool_value("project.list", json!([1, 2, 3]))
        .await
        .expect_err("array arguments must be a surface reject");
    assert!(
        err.to_string().contains("must be a JSON object"),
        "surface guard survives, got: {err}"
    );
}

#[tokio::test]
async fn call_tool_value_accepts_null_arguments_as_empty_object() {
    let (_home, server) = server();
    let env = server
        .call_tool_value("project.list", Value::Null)
        .await
        .expect("Null args coerce to {}");
    assert!(env.is_ok(), "project.list over coerced {{}} must succeed");
}

#[tokio::test]
async fn call_tool_value_project_list_is_deterministic() {
    let (_home, server) = server();
    let a = server
        .call_tool_value("project.list", json!({}))
        .await
        .unwrap();
    let b = server
        .call_tool_value("project.list", json!({}))
        .await
        .unwrap();
    assert_eq!(
        a.to_json(),
        b.to_json(),
        "project.list envelope must be stable across calls"
    );
}

#[test]
fn get_info_advertises_tools_capability() {
    use rmcp::ServerHandler;
    let server = VerbreelServer::new();
    let info = server.get_info();
    assert!(
        info.capabilities.tools.is_some(),
        "server must advertise the tools capability so clients call tools/list"
    );
}

#[test]
fn get_info_includes_instructions() {
    use rmcp::ServerHandler;
    let server = VerbreelServer::new();
    let info = server.get_info();
    let instructions = info
        .instructions
        .as_ref()
        .expect("server must carry human-facing instructions");
    assert!(
        instructions.contains("Verbreel"),
        "instructions should name the project, got: {instructions}"
    );
}

#[test]
fn get_info_uses_default_protocol_version() {
    use rmcp::ServerHandler;
    use rmcp::model::ProtocolVersion;
    let server = VerbreelServer::new();
    let info = server.get_info();
    assert_eq!(info.protocol_version, ProtocolVersion::default());
}
