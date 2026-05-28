//! Verbreel — HTTP server binary entrypoint.
//!
//! All routing lives in `verbreel_http::serve` / `verbreel_http::router`.
//! This binary is a thin wrapper so integration tests can call the
//! library directly without spawning a subprocess or binding a real
//! TCP listener.
//!
//! ## Logs go to stderr
//!
//! `tracing-subscriber` is configured with `with_writer(io::stderr)`.
//! HTTP servers conventionally split structured logs (stderr) from
//! protocol/application output (stdout).

use std::io;
use std::net::SocketAddr;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_writer(io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // Default bind is `127.0.0.1:0` — the OS picks an ephemeral port,
    // so the binary never fails when port 8080 is occupied and never
    // collides with another instance on the same host. Operators set
    // `VERBREEL_HTTP_ADDR` to pin a stable port when they need one.
    let addr: SocketAddr = std::env::var("VERBREEL_HTTP_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:0".to_string())
        .parse()?;

    verbreel_http::serve(addr).await
}
