//! `VerbreelServer` — coverage via the pure helpers
//! [`VerbreelServer::tools_list`] and [`VerbreelServer::call_tool_value`].
//!
//! The `ServerHandler` trait methods need a `RequestContext` wrapping a
//! `Peer<RoleServer>`, and `Peer::new` is `pub(crate)` in rmcp 1.7, so
//! the protocol layer is exercised through these pure helpers the trait
//! impl delegates to.

use serde_json::json;
use verbreel_mcp::VerbreelServer;

#[test]
fn new_and_default_construct() {
    let _ = VerbreelServer::default();
    let _ = VerbreelServer::new();
}

#[test]
fn tools_list_advertises_the_full_surface() {
    let server = VerbreelServer::new();
    let result = server.tools_list();
    let names: Vec<&str> = result.tools.iter().map(|t| t.name.as_ref()).collect();
    assert!(names.contains(&"project.list"), "names: {names:?}");
    assert!(names.contains(&"clip.trim"));
    assert!(result.tools.len() >= 116, "got {}", result.tools.len());
}

#[test]
fn tools_list_has_no_next_cursor() {
    let server = VerbreelServer::new();
    let result = server.tools_list();
    assert!(result.next_cursor.is_none());
}

#[test]
fn unbound_call_tool_value_evaluates_statelessly() {
    let server = VerbreelServer::new();
    let data = server
        .call_tool_value("project.list", json!({}))
        .expect("project.list dispatches statelessly");
    assert!(data.get("projects").is_some(), "data: {data}");
}

#[test]
fn unbound_unknown_verb_yields_error() {
    let server = VerbreelServer::new();
    assert!(
        server
            .call_tool_value("totally.fake.verb", json!({}))
            .is_err()
    );
}

#[test]
fn project_bound_dispatch_persists_a_mutation() {
    // Create a real project, then bind a server to it.
    let ws = tempfile::tempdir().expect("tempdir");
    let root = {
        let session = verbreel_agent::Session::create(ws.path(), "demo", "1920x1080", None)
            .expect("create project");
        // Drop the session to release its events.jsonl flock before the
        // server opens the same project.
        session.root().to_path_buf()
    };
    let server = VerbreelServer::with_project(root);

    // Mutate through the bound server.
    server
        .call_tool_value("project.rename", json!({ "name": "Bound" }))
        .expect("rename dispatches");

    // A fresh call opens the project again and sees the persisted name.
    let info = server
        .call_tool_value("project.info", json!({}))
        .expect("project.info dispatches");
    assert_eq!(info["name"], "Bound", "info: {info}");
}

/// Under `native-render`, `render.start` routes to the runtime; an
/// unknown preset surfaces the runtime's typed error (lightweight —
/// fails at preset validation, no GPU/FFmpeg work).
#[test]
#[cfg(feature = "native-render")]
fn render_start_routes_to_runtime() {
    use verbreel_runtime::RenderRuntimeConfig;

    let ws = tempfile::tempdir().expect("tempdir");
    let (root, project_id) = {
        let session = verbreel_agent::Session::create(ws.path(), "rdr", "64x64", None)
            .expect("create project");
        (session.root().to_path_buf(), session.project_id())
    };
    let server = VerbreelServer::with_project_and_runtime(root, RenderRuntimeConfig::new());
    let err = server
        .call_tool_value(
            "render.start",
            json!({
                "project_id": project_id,
                "preset": "definitely-not-a-preset",
                "out_path": "exports/out.mp4",
                "from_tk": 0,
                "to_tk": 8000,
                "overwrite": true,
            }),
        )
        .expect_err("unknown preset must error");
    assert!(
        err.to_string().contains("E_RENDER_PRESET_UNKNOWN"),
        "expected runtime error code, got: {err}"
    );
}
