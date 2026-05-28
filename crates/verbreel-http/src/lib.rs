//! verbreel-http — axum 0.8 HTTP server.
//!
//! Per the crate dependency rule in `CLAUDE.md`:
//!
//! ```text
//! verbreel-http → verbreel-state, verbreel-storage
//! ```
//!
//! ## Lib + bin split
//!
//! The crate ships a thin `main.rs` that delegates to [`serve`]. Routing
//! and the request handlers live here in the library so integration
//! tests can exercise them via `tower::ServiceExt::oneshot` without
//! binding a real TCP socket.
//!
//! ## Surface
//!
//! [`router`] assembles three endpoints:
//!
//! | Method | Path             | Handler                |
//! |--------|------------------|------------------------|
//! | GET    | `/healthz`       | [`handlers::healthz`]  |
//! | GET    | `/tools`         | [`handlers::list_tools`] |
//! | POST   | `/tools/{verb}`  | [`handlers::call_tool`]  |
//!
//! Only verbs in [`handlers::SUPPORTED_VERBS`] are reachable through
//! `POST /tools/{verb}`. Future slices append to the whitelist; the
//! dispatcher itself is untouched (DIP — high-level dispatch depends on
//! `verbreel_state::default_registry`, not on concrete verb modules).
//!
//! ## Logs go to stderr
//!
//! The binary configures `tracing-subscriber` with
//! `with_writer(io::stderr)`. Server logs on stderr is the conventional
//! split that keeps stdout free for piped JSON output.

#![deny(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]

use std::net::SocketAddr;

use axum::{
    Router,
    routing::{get, post},
};

pub mod handlers;

/// Assemble the axum router with every endpoint this crate exposes.
///
/// Pure — no I/O, no async — so unit tests can construct a fresh router
/// per case and drive it through `tower::ServiceExt::oneshot`.
pub fn router() -> Router {
    Router::new()
        .route("/healthz", get(handlers::healthz))
        .route("/tools", get(handlers::list_tools))
        .route("/tools/{verb}", post(handlers::call_tool))
}

/// Bind `addr` and serve the [`router`] until the listener errors out.
///
/// # Errors
///
/// Returns an error if the TCP bind fails (port in use, insufficient
/// permissions, invalid address) or if `axum::serve` exits abnormally.
pub async fn serve(addr: SocketAddr) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("verbreel-http listening on {addr}");
    axum::serve(listener, router()).await?;
    Ok(())
}
