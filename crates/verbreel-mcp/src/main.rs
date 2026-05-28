//! Verbreel — MCP stdio server binary entrypoint.
//!
//! All routing lives in `verbreel_mcp::run_stdio`. This binary is a thin
//! wrapper so integration tests can call the library function directly
//! without spawning subprocesses.
//!
//! ## Logs go to stderr
//!
//! The MCP stdio transport owns stdout for JSON-RPC frames. Any log
//! output on stdout corrupts the protocol stream. `tracing-subscriber`
//! is configured with `with_writer(io::stderr)` — do not change that
//! without re-reading the lib-level note.

use std::io;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_writer(io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    verbreel_mcp::run_stdio().await
}
