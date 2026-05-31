//! End-to-end `ServerHandler` trait coverage over an in-process rmcp
//! duplex pipe.
//!
//! `tests/server.rs` exercises the async pure helpers
//! ([`VerbreelServer::tools_list`] / [`VerbreelServer::call_tool_value`])
//! directly. Those helpers never touch the `ServerHandler` trait methods,
//! whose `list_tools` / `call_tool` impls each need a `RequestContext`
//! wrapping a `Peer<RoleServer>` — and `Peer::new` is `pub(crate)` in rmcp
//! 1.7. The only way to build that context is to actually run the service:
//! `VerbreelServer::with_home(tempdir).serve()` on one half of a
//! `tokio::io::duplex` pair, a real rmcp client on the other, completing
//! the initialize handshake and then driving `tools/list` + `tools/call`
//! over the wire. That routes through `ServerHandler::list_tools` and
//! `ServerHandler::call_tool` for real (§0.1 envelope; §0.7 `E_*`).

use rmcp::{
    ServiceExt,
    model::{
        CallToolRequestParams, ClientJsonRpcMessage, ServerJsonRpcMessage, ServerResult,
    },
    transport::{IntoTransport, Transport},
};
use serde_json::{Value, json};
use tempfile::TempDir;
use verbreel_mcp::VerbreelServer;

/// Spawn `VerbreelServer::with_home(tempdir)` on the server half of a
/// duplex pair and return a connected, fully-initialized rmcp client
/// (`()` is the minimal `ClientHandler`). The `TempDir` is returned so the
/// caller keeps it alive — dropping it would unlink the isolated
/// `.verbreel/projects-index` mid-test.
async fn connect() -> (
    TempDir,
    rmcp::service::RunningService<rmcp::RoleClient, ()>,
) {
    let home = TempDir::new().expect("tempdir");
    let (server_transport, client_transport) = tokio::io::duplex(4096);

    let server = VerbreelServer::with_home(home.path());
    tokio::spawn(async move {
        // The handshake + request loop runs until the client disconnects;
        // a failure here surfaces as a client-side transport error, which
        // the assertions below catch.
        if let Ok(running) = server.serve(server_transport).await {
            let _ = running.waiting().await;
        }
    });

    let client = ()
        .serve(client_transport)
        .await
        .expect("client completes the initialize handshake over duplex");
    (home, client)
}

/// (a) `tools/list` over the wire drives `ServerHandler::list_tools` and
/// returns the full verb surface (>100 tools), including the two anchor
/// verbs the brief names.
#[tokio::test]
async fn list_tools_over_duplex_advertises_full_surface() {
    let (_home, client) = connect().await;
    let tools = client
        .list_all_tools()
        .await
        .expect("tools/list round-trips through ServerHandler::list_tools");

    assert!(
        tools.len() > 100,
        "list_tools must advertise the full verb surface, got {}",
        tools.len()
    );
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    assert!(
        names.contains(&"project.list"),
        "full surface must include project.list"
    );
    assert!(
        names.contains(&"render.start"),
        "full surface must include render.start"
    );

    client.cancel().await.expect("client shuts down cleanly");
}

/// (b) `tools/call project.list {}` drives `ServerHandler::call_tool` down
/// the ok arm: a structured, non-error result whose §0.1 envelope carries
/// `data.projects` as a JSON array.
#[tokio::test]
async fn call_tool_project_list_over_duplex_is_structured_ok() {
    let (_home, client) = connect().await;
    let result = client
        .call_tool(CallToolRequestParams::new("project.list").with_arguments(json!({})
            .as_object()
            .expect("empty object")
            .clone()))
        .await
        .expect("project.list round-trips through ServerHandler::call_tool");

    assert_eq!(
        result.is_error,
        Some(false),
        "ok envelope maps to a non-error CallToolResult"
    );
    let envelope = result
        .structured_content
        .as_ref()
        .expect("ok arm uses CallToolResult::structured → structured_content present");
    let projects = envelope
        .get("data")
        .and_then(|d| d.get("projects"))
        .and_then(Value::as_array);
    assert!(
        projects.is_some(),
        "ok envelope data must expose `projects` as an array, got: {envelope}"
    );

    client.cancel().await.expect("client shuts down cleanly");
}

/// (c) `tools/call totally.fake.verb {}` drives `ServerHandler::call_tool`
/// down the err arm: an unknown verb is no surface reject — it flows to the
/// Engine and comes back as a structured error result whose §0.7 envelope
/// code is `E_UNKNOWN_VERB`.
#[tokio::test]
async fn call_tool_unknown_verb_over_duplex_is_structured_e_unknown_verb() {
    let (_home, client) = connect().await;
    let result = client
        .call_tool(CallToolRequestParams::new("totally.fake.verb").with_arguments(
            json!({}).as_object().expect("empty object").clone(),
        ))
        .await
        .expect("unknown verb still round-trips; the failure is in the envelope");

    assert_eq!(
        result.is_error,
        Some(true),
        "err envelope maps to is_error:true"
    );
    let envelope = result
        .structured_content
        .as_ref()
        .expect("err arm uses CallToolResult::structured_error → structured_content present");
    assert_eq!(
        envelope.get("code").and_then(Value::as_str),
        Some("E_UNKNOWN_VERB"),
        "unknown verb resolves at the Engine, not a surface reject, got: {envelope}"
    );

    client.cancel().await.expect("client shuts down cleanly");
}

/// (d) The args-shape arm — non-object `arguments` → unstructured error.
///
/// `ServerHandler::call_tool` maps a `tools::ArgsShapeError` to
/// `CallToolResult::error` (unstructured: `is_error:true`, no
/// `structured_content`). The typed rmcp client cannot reach that arm:
/// `CallToolRequestParams::arguments` is `Option<JsonObject>`, so a
/// non-object cannot be expressed through `call_tool`. Drive it with a
/// raw transport instead — a hand-built JSON-RPC `tools/call` frame whose
/// `arguments` is an array — and assert the server rejects it without
/// emitting an ok result. rmcp deserializes the incoming frame into
/// `CallToolRequestParams` before `call_tool` runs, so a non-object
/// `arguments` is refused at that protocol-deserialize boundary (a
/// JSON-RPC error response), strictly upstream of the Engine — which is
/// the same "non-object never reaches the Engine" invariant the
/// `ArgsShapeError` surface guard documents, enforced one layer earlier on
/// the wire.
#[tokio::test]
async fn call_tool_non_object_args_over_duplex_is_refused() {
    let home = TempDir::new().expect("tempdir");
    let (server_transport, client_transport) = tokio::io::duplex(4096);

    let server = VerbreelServer::with_home(home.path());
    tokio::spawn(async move {
        if let Ok(running) = server.serve(server_transport).await {
            let _ = running.waiting().await;
        }
    });

    let mut client =
        IntoTransport::<rmcp::RoleClient, _, _>::into_transport(client_transport);

    let init: ClientJsonRpcMessage = serde_json::from_str(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "protocolVersion":"2025-11-25",
            "capabilities":{},
            "clientInfo":{"name":"e2e-raw","version":"0.0.1"}}}"#,
    )
    .expect("init frame parses");
    client.send(init).await.expect("send initialize");
    let _init_resp = client.receive().await.expect("receive initialize result");

    let initialized: ClientJsonRpcMessage =
        serde_json::from_str(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
            .expect("initialized notification parses");
    client
        .send(initialized)
        .await
        .expect("send initialized notification");

    // arguments is an array — illegal per CallToolRequestParams. The
    // server's request deserialize rejects it before call_tool runs.
    let bad_call: ClientJsonRpcMessage = serde_json::from_str(
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
            "name":"project.list","arguments":[1,2,3]}}"#,
    )
    .expect("malformed tools/call frame parses as a client message");
    client.send(bad_call).await.expect("send malformed tools/call");

    let response = loop {
        let msg = client.receive().await.expect("server responds");
        if matches!(
            msg,
            ServerJsonRpcMessage::Response(_) | ServerJsonRpcMessage::Error(_)
        ) {
            break msg;
        }
    };

    match response {
        // The server's request deserialize rejects a non-object
        // `arguments` field before `call_tool` runs: a JSON-RPC error
        // (method-not-found / invalid params), strictly upstream of the
        // Engine. This is the wire-level enforcement of the same invariant
        // the `ArgsShapeError` surface guard documents.
        ServerJsonRpcMessage::Error(_) => {}
        ServerJsonRpcMessage::Response(r) => match r.result {
            // If a future rmcp ever forwarded the non-object through to
            // call_tool, the surface ArgsShapeError arm must still produce
            // an unstructured error result — never an ok envelope.
            ServerResult::CallToolResult(call) => {
                assert_eq!(
                    call.is_error,
                    Some(true),
                    "non-object args must surface as an error result, never ok"
                );
                assert!(
                    call.structured_content.is_none(),
                    "the args-shape reject is unstructured, got: {:?}",
                    call.structured_content
                );
            }
            other => panic!("non-object args must not yield an ok envelope, got: {other:?}"),
        },
        // The receive loop only breaks on Response / Error.
        other => panic!("expected a server response, got: {other:?}"),
    }
}
