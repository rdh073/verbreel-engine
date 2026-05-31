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
//! [`VerbreelServer`] is a thin shell over the one shared
//! [`verbreel_state::Engine`] and implements [`rmcp::ServerHandler`] for
//! three request methods:
//!
//! | MCP method   | Behavior                                                          |
//! |--------------|-------------------------------------------------------------------|
//! | `initialize` | rmcp default — advertises `tools` capability via [`VerbreelServer::get_info`] |
//! | `tools/list` | Advertises every verb from [`Engine::verb_ids`]                   |
//! | `tools/call` | Routes every verb through [`Engine::dispatch`]                    |
//!
//! Both `tools/list` and `tools/call` delegate to async helpers
//! ([`VerbreelServer::tools_list`], [`VerbreelServer::call_tool_value`])
//! that take ordinary JSON inputs and lock the shared Engine across the
//! call. Tests call those helpers — the `ServerHandler` impl is a thin
//! protocol wrapper around them.
//!
//! ## §0.12 project context
//!
//! The MCP server NEVER infers project context from process state. A
//! mutating verb's `project_id` is a client arg in the `tools/call`
//! body; the surface passes it straight to [`Engine::dispatch`] and
//! fabricates nothing. The Engine owns routing the three project-less
//! reads (`help` / `schema` / `list_capabilities`) against its own
//! synthetic prior — the surface injects no id for any verb.
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

use std::path::PathBuf;
use std::sync::Arc;

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
use tokio::sync::Mutex;
use verbreel_state::{Engine, Envelope};

pub mod tools;

/// MCP server exposing the full Verbreel verb surface via the shared
/// [`Engine`].
///
/// Holds the Engine behind `Arc<Mutex<…>>`: [`Engine::dispatch`] takes
/// `&mut self` while rmcp's [`ServerHandler`] methods take `&self`, and
/// the `Arc<Mutex<…>>` keeps the handler `Clone` for the rmcp service.
/// The lock is held only across a single `dispatch` / `verb_ids` call.
#[derive(Clone)]
pub struct VerbreelServer {
    engine: Arc<Mutex<Engine>>,
}

impl std::fmt::Debug for VerbreelServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VerbreelServer").finish_non_exhaustive()
    }
}

impl VerbreelServer {
    /// Construct a server over a fresh [`Engine`] rooted at the process
    /// home. Home is resolved from `VERBREEL_HOME`, else `HOME`, else the
    /// current directory — the same precedence the runtime uses to locate
    /// the user-wide `.verbreel/projects-index` (§0.12).
    #[must_use]
    pub fn new() -> Self {
        Self::with_home(default_home())
    }

    /// Construct a server over an [`Engine`] rooted at `home`. Tests use
    /// this with a `TempDir` so each server gets an isolated projects
    /// index.
    #[must_use]
    pub fn with_home(home: impl Into<PathBuf>) -> Self {
        Self {
            engine: Arc::new(Mutex::new(Engine::new(home))),
        }
    }

    /// Pure async helper backing the `tools/list` MCP request.
    ///
    /// Locks the Engine and builds one tool per [`Engine::verb_ids`]
    /// entry (sorted → deterministic, full surface). Tests call this
    /// directly; the `ServerHandler::list_tools` impl is a thin protocol
    /// wrapper around it.
    pub async fn tools_list(&self) -> ListToolsResult {
        let engine = self.engine.lock().await;
        ListToolsResult::with_all_items(tools::all_tools(&engine))
    }

    /// Async helper backing `tools/call`.
    ///
    /// `arguments` is the raw JSON the MCP client supplied (or
    /// `Value::Null` when omitted). Coerces it to a JSON object at the
    /// surface (§0.12 — the surface injects no `project_id`), then routes
    /// `name` + args through [`Engine::dispatch`] and returns the §0.1
    /// [`Envelope`]. An unknown verb is no longer a surface reject: it
    /// flows to the Engine and comes back as an `E_UNKNOWN_VERB` envelope.
    ///
    /// # Errors
    ///
    /// Returns [`tools::ArgsShapeError`] only when `arguments` is neither
    /// a JSON object nor null — a malformed MCP `arguments` field, which
    /// never reaches the Engine. Every verb-level failure is carried in
    /// the returned `Ok(Envelope::Err { .. })`, not this `Err`.
    pub async fn call_tool_value(
        &self,
        name: &str,
        arguments: Value,
    ) -> Result<Envelope, tools::ArgsShapeError> {
        let args = tools::object_args(arguments)?;
        // §0.12: MCP passes project_id straight from the client body; the
        // Engine routes project-less reads (help/schema/list_capabilities)
        // against a synthetic prior — the surface never injects an id.
        let mut engine = self.engine.lock().await;
        Ok(engine.dispatch(name, Value::Object(args)))
    }
}

impl Default for VerbreelServer {
    fn default() -> Self {
        Self::new()
    }
}

impl ServerHandler for VerbreelServer {
    fn get_info(&self) -> ServerInfo {
        let capabilities = ServerCapabilities::builder().enable_tools().build();
        InitializeResult::new(capabilities).with_instructions(
            "Verbreel MCP server — exposes the full Verbreel verb surface \
             via the shared engine.",
        )
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<rmcp::RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(self.tools_list().await)
    }

    /// Route `tools/call` through the shared Engine and map the §0.1
    /// envelope to a [`CallToolResult`].
    ///
    /// An `ok:true` envelope → [`CallToolResult::structured`] (the full
    /// envelope, so clients see `patch` / `event_id` / `warnings`, not
    /// just `data`). An `ok:false` envelope →
    /// [`CallToolResult::structured_error`], which carries the same
    /// envelope in `structured_content` (and mirrored as text) with
    /// `is_error = true` — that shape lets a client read the §0.7 `code`
    /// (`E_UNKNOWN_VERB`, `E_PROJECT_NOT_FOUND`, …). A malformed
    /// `arguments` field (not an object) is the one surface-level reject
    /// and surfaces as an unstructured [`CallToolResult::error`].
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<rmcp::RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let name = request.name.as_ref();
        let arguments = request.arguments.map_or(Value::Null, Value::Object);

        match self.call_tool_value(name, arguments).await {
            Ok(envelope) if envelope.is_ok() => Ok(CallToolResult::structured(envelope.to_json())),
            Ok(envelope) => Ok(CallToolResult::structured_error(envelope.to_json())),
            Err(shape_err) => Ok(CallToolResult::error(vec![Content::text(
                shape_err.to_string(),
            )])),
        }
    }
}

/// Resolve the process home dir for the Engine's `.verbreel/projects-index`
/// (§0.12): `VERBREEL_HOME`, else `HOME`, else the current directory.
/// Mirrors `verbreel_runtime::RenderRuntimeConfig::from_env` precedence.
fn default_home() -> PathBuf {
    if let Some(home) = std::env::var_os("VERBREEL_HOME").or_else(|| std::env::var_os("HOME")) {
        return PathBuf::from(home);
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
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
