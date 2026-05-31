//! `VerbreelServer` — protocol-surface coverage via the pure helpers
//! [`VerbreelServer::tools_list`] and [`VerbreelServer::call_tool_value`].
//!
//! The `ServerHandler` trait methods themselves need a `RequestContext`
//! that wraps a `Peer<RoleServer>`, and `Peer::new` is `pub(crate)` in
//! rmcp 1.7. Exercising the protocol layer end-to-end therefore
//! requires the rmcp `client` feature + `tokio::io::duplex` plumbing —
//! out of scope for this first slice. The pure helpers carry the same
//! logic; the `ServerHandler` impl is a thin wrapper around them.

use serde_json::{Value, json};
use verbreel_mcp::VerbreelServer;

#[test]
#[allow(
    clippy::default_constructed_unit_structs,
    reason = "this test exists to pin the `Default` impl on VerbreelServer — \
              `VerbreelServer` is the unit-struct form today, but adding state \
              later would silently drop Default. The explicit `::default()` \
              call is the contract we want to enforce."
)]
fn server_constructs_via_default() {
    let _server = VerbreelServer::default();
}

#[test]
fn server_constructs_via_new() {
    let _server = VerbreelServer::new();
}

#[test]
fn tools_list_includes_project_list() {
    let server = VerbreelServer::new();
    let result = server.tools_list();
    let names: Vec<&str> = result.tools.iter().map(|t| t.name.as_ref()).collect();
    assert!(
        names.contains(&"project.list"),
        "tools/list must advertise project.list, got: {names:?}"
    );
}

#[test]
#[cfg(feature = "native-render")]
fn tools_list_includes_render_start() {
    let server = VerbreelServer::new();
    let result = server.tools_list();
    let names: Vec<&str> = result.tools.iter().map(|t| t.name.as_ref()).collect();
    assert!(
        names.contains(&"render.start"),
        "tools/list must advertise render.start with native-render, got: {names:?}"
    );
}

#[test]
fn tools_list_has_no_next_cursor_at_v1_floor() {
    let server = VerbreelServer::new();
    let result = server.tools_list();
    assert!(
        result.next_cursor.is_none(),
        "v1 floor surfaces every tool in one page"
    );
}

#[test]
fn tools_list_count_matches_supported_whitelist() {
    let server = VerbreelServer::new();
    let result = server.tools_list();
    assert_eq!(
        result.tools.len(),
        verbreel_mcp::tools::SUPPORTED_VERBS.len(),
        "list_tools must reflect the SUPPORTED_VERBS whitelist exactly"
    );
}

#[test]
fn tools_list_entries_carry_descriptions() {
    let server = VerbreelServer::new();
    let result = server.tools_list();
    for tool in &result.tools {
        assert!(
            tool.description.is_some(),
            "every advertised tool must carry a description, got: {tool:?}"
        );
    }
}

#[test]
fn call_tool_value_project_list_returns_v1_envelope() {
    let server = VerbreelServer::new();
    let data = server
        .call_tool_value("project.list", json!({}))
        .expect("project.list must succeed at v1 floor");
    assert!(
        data.get("projects").and_then(Value::as_array).is_some(),
        "envelope must expose `projects` as a JSON array, got: {data}"
    );
}

#[test]
#[cfg(feature = "native-render")]
fn call_tool_value_render_start_delegates_to_runtime() -> anyhow::Result<()> {
    use tempfile::TempDir;
    use verbreel_runtime::RenderRuntimeConfig;
    use verbreel_state::{ProjectCreateArgs, project_create};

    let tmp = TempDir::new()?;
    let create_data = project_create(&ProjectCreateArgs {
        name: "mcp-render-project".to_string(),
        canvas: "64x64".to_string(),
        fps_num: Some(30),
        fps_den: Some(1),
        at: Some(tmp.path().to_path_buf()),
        activate: false,
        metadata: serde_json::Map::new(),
    })?;
    let server = VerbreelServer::with_runtime(
        RenderRuntimeConfig::new().with_project_root(&create_data.path),
    );
    let err = server
        .call_tool_value(
            "render.start",
            json!({
                "project_id": create_data.project_id,
                "preset": "definitely-not-a-preset",
                "out_path": "exports/out.mp4",
                "from_tk": 0,
                "to_tk": 8000,
                "overwrite": true,
            }),
        )
        .expect_err("runtime should reject preset");
    assert!(
        err.to_string().contains("E_RENDER_PRESET_UNKNOWN"),
        "MCP must surface the runtime error code, got: {err}"
    );
    Ok(())
}

#[test]
fn call_tool_value_project_list_is_empty_at_v1_floor() {
    let server = VerbreelServer::new();
    let data = server.call_tool_value("project.list", json!({})).unwrap();
    let arr = data.get("projects").and_then(Value::as_array).unwrap();
    assert!(arr.is_empty(), "v1 floor must report empty list");
}

#[test]
fn call_tool_value_unknown_verb_yields_error() {
    let server = VerbreelServer::new();
    let err = server
        .call_tool_value("totally.fake.verb", json!({}))
        .expect_err("unknown verbs must surface as errors");
    assert!(err.to_string().contains("not supported by this server"));
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
