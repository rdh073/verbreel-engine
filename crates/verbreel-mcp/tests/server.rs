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
    // render.start is a registry verb, so it is always advertised in
    // tools/list (the native-render interception happens at tools/call,
    // not in the catalog).
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
async fn call_tool_value_render_start_surfaces_error_for_unknown_project() {
    // render.start with an unresolvable project_id must be an err envelope
    // on EITHER surface path: under native-render the injected runtime
    // returns E_PROJECT_NOT_FOUND before touching ffmpeg; without the
    // feature it falls through to the Engine's E_RENDER_FAIL floor. Both
    // are `Envelope::Err`, so this base-gate test is feature-agnostic.
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
        "render.start with no resolvable project must be an err envelope, got {env:?}"
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

/// Under `native-render`, a `render.start` call is intercepted by the
/// injected runtime (not the Engine floor). With an empty runtime config
/// rooted at a hermetic home (no projects index), the project cannot be
/// resolved, so the runtime returns `RuntimeError::ProjectNotFound` BEFORE
/// any ffmpeg/codec work — exercising the render-error → `E_*` mapping
/// without a real render. Proves the §0.1 envelope carries the mapped
/// `E_PROJECT_NOT_FOUND` code, mirroring `verbreel-http`.
#[cfg(feature = "native-render")]
#[tokio::test]
async fn call_tool_value_render_start_maps_runtime_error_to_e_code() {
    let home = TempDir::new().expect("tempdir");
    // Empty runtime: no project roots, a hermetic home with no index.
    // `from_env` would leak the developer's cwd/HOME, so build explicitly.
    let runtime = verbreel_runtime::RenderRuntimeConfig::new().with_home(home.path());
    let server = VerbreelServer::with_render_runtime(home.path(), runtime);

    let env = server
        .call_tool_value(
            "render.start",
            json!({
                "project_id": "0192f000-0000-7000-8000-000000000000",
                "preset": "youtube-1080p",
                "out_path": "exports/out.mp4",
            }),
        )
        .await
        .expect("args are an object");

    let Envelope::Err { code, message, .. } = &env else {
        panic!("unresolvable project must be an err envelope, got {env:?}");
    };
    assert_eq!(
        code, "E_PROJECT_NOT_FOUND",
        "runtime error must map to the §0.7 E_* code, got message: {message}"
    );
}

/// Under `native-render`, malformed `render.start` args (missing required
/// `out_path`) fail the `RenderStartArgs` deserialize at the surface and
/// map to `E_SCHEMA_VIOLATION` — not a runtime call. Mirrors HTTP's
/// `call_render_start` deserialize arm.
#[cfg(feature = "native-render")]
#[tokio::test]
async fn call_tool_value_render_start_bad_args_maps_to_e_schema_violation() {
    let home = TempDir::new().expect("tempdir");
    let runtime = verbreel_runtime::RenderRuntimeConfig::new().with_home(home.path());
    let server = VerbreelServer::with_render_runtime(home.path(), runtime);

    let env = server
        .call_tool_value(
            "render.start",
            json!({ "project_id": "0192f000-0000-7000-8000-000000000000" }),
        )
        .await
        .expect("args are an object (the verb-level deserialize is the failure)");

    let Envelope::Err { code, .. } = &env else {
        panic!("missing required args must be an err envelope, got {env:?}");
    };
    assert_eq!(code, "E_SCHEMA_VIOLATION");
}

/// Under `native-render`, `with_home` must point the render runtime at the
/// same `home` the Engine uses (`from_env().with_home(home)`), not at the
/// developer's process `HOME` alone. A project registered ONLY in the
/// injected temp home must resolve through the render path: a bad preset
/// then surfaces `E_RENDER_PRESET_UNKNOWN` (resolution succeeded). Before
/// the fix, `with_home` ignored `home` so the runtime never saw the temp
/// index and the same call surfaced `E_PROJECT_NOT_FOUND`.
#[cfg(feature = "native-render")]
#[tokio::test]
async fn with_home_injects_home_into_render_runtime() {
    let home = TempDir::new().expect("tempdir");
    let create = verbreel_state::project_create(&verbreel_state::ProjectCreateArgs {
        name: "mcp-render-home".to_string(),
        canvas: "64x64".to_string(),
        fps_num: Some(30),
        fps_den: Some(1),
        at: Some(home.path().join("projects")),
        activate: false,
        metadata: serde_json::Map::new(),
    })
    .expect("project create");
    let project_id = create.project_id.to_string();
    verbreel_storage::layout::register_project(
        home.path(),
        &project_id,
        "mcp-render-home",
        &create.path,
        "2025-01-01T00:00:00Z",
    )
    .expect("register project in temp home index");

    let server = VerbreelServer::with_home(home.path());
    let env = server
        .call_tool_value(
            "render.start",
            json!({
                "project_id": project_id,
                "preset": "definitely-not-a-preset",
                "out_path": "exports/out.mp4",
            }),
        )
        .await
        .expect("args are an object");

    let Envelope::Err { code, .. } = &env else {
        panic!("bad preset must be an err envelope, got {env:?}");
    };
    assert_eq!(
        code, "E_RENDER_PRESET_UNKNOWN",
        "with_home must inject home so the runtime resolves the temp-home \
         project (then rejects the preset); E_PROJECT_NOT_FOUND means home \
         was ignored, got: {env:?}"
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
