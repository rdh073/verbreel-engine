//! verbreel-mcp — Model Context Protocol stdio server.
//!
//! Per the crate dependency rule in `CLAUDE.md`:
//!
//! ```text
//! verbreel-mcp → verbreel-state, verbreel-storage
//! ```
//!
//! ## Lib + bin split
//!
//! The crate ships a thin `main.rs` that delegates to [`run_stdio`].
//! Routing, verb dispatch, and the `ServerHandler` implementation live
//! here in the library so integration tests can exercise them directly
//! without spawning subprocesses or wiring stdin/stdout pipes.
//!
//! ## Protocol surface
//!
//! [`VerbreelServer`] implements [`rmcp::ServerHandler`] for three
//! request methods:
//!
//! | MCP method   | Behavior                                                          |
//! |--------------|-------------------------------------------------------------------|
//! | `initialize` | rmcp default — advertises `tools` capability via [`Self::get_info`] |
//! | `tools/list` | Returns the verbs registered in [`tools::SUPPORTED_VERBS`]        |
//! | `tools/call` | Routes to `verbreel_state::default_registry()` via [`tools::dispatch`] |
//!
//! Both `tools/list` and `tools/call` delegate to pure helpers
//! ([`VerbreelServer::tools_list`], [`VerbreelServer::call_tool_value`])
//! that take ordinary JSON inputs. Tests call those helpers — the
//! `ServerHandler` impl is a thin protocol wrapper around them.
//!
//! ## Logs go to stderr
//!
//! The MCP stdio transport owns stdout for JSON-RPC frames. Any log
//! output on stdout corrupts the protocol stream and breaks every
//! client. The binary (`main.rs`) initialises `tracing-subscriber` with
//! `with_writer(std::io::stderr)` — never `stdout`. If you change that,
//! tear down every test in this crate first.

#![deny(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]

use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt,
    model::{
        CallToolRequestParams, CallToolResult, Content, InitializeResult, ListToolsResult,
        PaginatedRequestParams, ServerCapabilities, ServerInfo,
    },
    service::RequestContext,
    transport::io::stdio,
};
use serde_json::Value;

pub mod tools;

/// MCP server exposing a curated subset of `verbreel-state` verbs.
///
/// Stateless — every `tools/call` synthesises a fresh prior project
/// inside [`tools::dispatch`]. Project lifecycle over MCP lands in a
/// later slice once the args registry is further along.
#[derive(Debug, Clone, Default)]
pub struct VerbreelServer;

impl VerbreelServer {
    /// Construct a new server. Equivalent to `Self::default()`.
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Pure helper backing the `tools/list` MCP request.
    ///
    /// Returns the verbs whitelisted in [`tools::SUPPORTED_VERBS`].
    /// Tests call this directly; the `ServerHandler::list_tools` impl
    /// is a thin protocol wrapper around it.
    #[must_use]
    pub fn tools_list(&self) -> ListToolsResult {
        ListToolsResult::with_all_items(tools::all_tools())
    }

    /// Pure helper backing `tools/call`.
    ///
    /// `arguments` is the raw JSON object the MCP client supplied (or
    /// `Value::Null` when omitted). Returns the verb's `data` envelope
    /// on success; the caller wraps it in a [`CallToolResult`].
    ///
    /// # Errors
    ///
    /// Bubbles up an `anyhow::Error` when [`tools::dispatch`] does.
    pub fn call_tool_value(&self, name: &str, arguments: Value) -> anyhow::Result<Value> {
        tools::dispatch(name, arguments)
    }
}

impl ServerHandler for VerbreelServer {
    fn get_info(&self) -> ServerInfo {
        let capabilities = ServerCapabilities::builder().enable_tools().build();
        InitializeResult::new(capabilities).with_instructions(
            "Verbreel MCP server — exposes selected verbs from \
             verbreel-state as MCP tools. v1 floor: project.list only.",
        )
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<rmcp::RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(self.tools_list())
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<rmcp::RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let name = request.name.as_ref();
        let arguments = request.arguments.map_or(Value::Null, Value::Object);

        match self.call_tool_value(name, arguments) {
            Ok(data) => Ok(CallToolResult::structured(data)),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                "verb '{name}' failed: {e}"
            ))])),
        }
    }
}

/// Run the MCP server over the stdio transport until the peer
/// disconnects.
///
/// Blocks the calling task. Intended to be invoked from a
/// `#[tokio::main]` binary entrypoint.
///
/// # Errors
///
/// Returns an error if the rmcp service fails to start (transport
/// creation, handshake) or the running service exits abnormally.
pub async fn run_stdio() -> anyhow::Result<()> {
    let server = VerbreelServer::new();
    let running = server.serve(stdio()).await?;
    running.waiting().await?;
    Ok(())
}
