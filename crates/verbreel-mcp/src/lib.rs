//! verbreel-mcp — Model Context Protocol stdio server.
//!
//! Per the crate dependency rule in `CLAUDE.md`:
//!
//! ```text
//! verbreel-mcp → verbreel-agent, verbreel-state, verbreel-storage
//! verbreel-mcp/native-render → verbreel-runtime
//! ```
//!
//! ## What an agent gets
//!
//! This is the agent-native surface: `tools/list` advertises **every**
//! engine verb with its real JSON args schema (full discovery), and
//! `tools/call` dispatches them. A connected agent (e.g. Claude) reads
//! the catalog and plans directly — the MCP client *is* the planner, so
//! no built-in NL planner is needed here.
//!
//! ## Bound vs. stateless
//!
//! The server is optionally bound to a project root
//! (`VERBREEL_PROJECT`). When bound, `tools/call` opens a real
//! [`verbreel_agent::Session`] and routes mutations through the §0.8
//! write-ordering kernel, so edits persist. When unbound, `tools/call`
//! evaluates verbs statelessly against a synthetic empty project (read-
//! only verbs return real answers; editing verbs surface their own
//! "nothing to act on" errors). Discovery (`tools/list`) works in both
//! modes.
//!
//! Under the `native-render` feature, `render.start` is routed to
//! [`verbreel_runtime`] for an actual pixel encode (scoped to the bound
//! project when one is set).
//!
//! ## Lib + bin split
//!
//! The crate ships a thin `main.rs` that delegates to [`run_stdio`].
//! Routing, dispatch, and the `ServerHandler` impl live here so
//! integration tests exercise them via the pure helpers
//! ([`VerbreelServer::tools_list`], [`VerbreelServer::call_tool_value`])
//! without spawning subprocesses.
//!
//! ## Logs go to stderr
//!
//! The MCP stdio transport owns stdout for JSON-RPC frames. Any log on
//! stdout corrupts the protocol stream. The binary initialises
//! `tracing-subscriber` with `with_writer(std::io::stderr)` — never
//! stdout.

#![deny(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]

use std::path::PathBuf;

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
use verbreel_agent::Session;

pub mod tools;

/// MCP server exposing the full `verbreel-state` verb surface.
///
/// Optionally bound to a project root: when `project_root` is `Some`,
/// `tools/call` mutations persist through a [`Session`]; when `None`, the
/// server evaluates verbs statelessly. Under `native-render` it also
/// carries the render runtime for `render.start` pixel encode.
#[derive(Debug, Clone)]
#[cfg_attr(not(feature = "native-render"), derive(Default))]
pub struct VerbreelServer {
    project_root: Option<PathBuf>,
    #[cfg(feature = "native-render")]
    render_runtime: verbreel_runtime::RenderRuntimeConfig,
}

#[cfg(feature = "native-render")]
impl Default for VerbreelServer {
    fn default() -> Self {
        Self {
            project_root: None,
            render_runtime: verbreel_runtime::RenderRuntimeConfig::from_env(),
        }
    }
}

impl VerbreelServer {
    /// Construct an unbound (stateless) server.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct a server bound to the project rooted at `root`.
    #[must_use]
    pub fn with_project(root: PathBuf) -> Self {
        Self {
            project_root: Some(root),
            #[cfg(feature = "native-render")]
            render_runtime: verbreel_runtime::RenderRuntimeConfig::from_env(),
        }
    }

    /// Construct a server bound to `root` with an explicit render runtime
    /// (tests / embedders).
    #[cfg(feature = "native-render")]
    #[must_use]
    pub fn with_project_and_runtime(
        root: PathBuf,
        render_runtime: verbreel_runtime::RenderRuntimeConfig,
    ) -> Self {
        Self {
            project_root: Some(root),
            render_runtime,
        }
    }

    /// Construct from the environment: bound to `VERBREEL_PROJECT` when
    /// set, otherwise unbound (stateless).
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            project_root: std::env::var_os("VERBREEL_PROJECT").map(PathBuf::from),
            #[cfg(feature = "native-render")]
            render_runtime: verbreel_runtime::RenderRuntimeConfig::from_env(),
        }
    }

    /// Pure helper backing the `tools/list` MCP request — every
    /// registered verb with its schema.
    #[must_use]
    pub fn tools_list(&self) -> ListToolsResult {
        ListToolsResult::with_all_items(tools::all_tools())
    }

    /// Pure helper backing `tools/call`.
    ///
    /// Bound: open the project, run the verb through the §0.8 forward
    /// path, save on a mutation, return the verb's `data`. Unbound:
    /// stateless evaluation via [`tools::dispatch`]. Under
    /// `native-render`, `render.start` is routed to the render runtime.
    ///
    /// # Errors
    ///
    /// Bubbles up an `anyhow::Error` from the kernel (unknown verb, bad
    /// args, lifecycle / IO failure) or the render runtime.
    pub fn call_tool_value(&self, name: &str, arguments: Value) -> anyhow::Result<Value> {
        #[cfg(feature = "native-render")]
        if name == "render.start" {
            return self.call_render_start(arguments);
        }
        match &self.project_root {
            Some(root) => {
                let mut session = Session::open(root)?;
                let outcome = session.run(name, arguments, None)?;
                if outcome.mutated() {
                    session.save()?;
                }
                Ok(outcome.data().clone())
            }
            None => tools::dispatch(name, arguments),
        }
    }

    /// Route `render.start` to [`verbreel_runtime`] for an actual pixel
    /// encode, scoped to the bound project when one is set.
    #[cfg(feature = "native-render")]
    fn call_render_start(&self, arguments: Value) -> anyhow::Result<Value> {
        let typed: verbreel_state::RenderStartArgs = serde_json::from_value(arguments)
            .map_err(|err| anyhow::anyhow!("render.start: args deserialize failed: {err}"))?;
        let runtime = match &self.project_root {
            Some(root) => self.render_runtime.clone().with_project_root(root),
            None => self.render_runtime.clone(),
        };
        let data = runtime.render_start(&typed)?;
        serde_json::to_value(data).map_err(Into::into)
    }
}

impl ServerHandler for VerbreelServer {
    fn get_info(&self) -> ServerInfo {
        let capabilities = ServerCapabilities::builder().enable_tools().build();
        InitializeResult::new(capabilities).with_instructions(
            "Verbreel MCP server — exposes every verbreel-state verb as an MCP tool with its \
             JSON args schema. Call `list_capabilities` or read tools/list for the full surface. \
             Bind a project with VERBREEL_PROJECT to persist edits; unbound, calls evaluate \
             statelessly.",
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
/// disconnects. The server is configured from the environment
/// ([`VerbreelServer::from_env`]).
///
/// Blocks the calling task. Intended to be invoked from a
/// `#[tokio::main]` binary entrypoint.
///
/// # Errors
///
/// Returns an error if the rmcp service fails to start (transport
/// creation, handshake) or the running service exits abnormally.
pub async fn run_stdio() -> anyhow::Result<()> {
    let server = VerbreelServer::from_env();
    let running = server.serve(stdio()).await?;
    running.waiting().await?;
    Ok(())
}
