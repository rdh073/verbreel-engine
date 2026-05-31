//! verbreel-http — axum 0.8 HTTP server for the Agentic-Experience layer.
//!
//! Per the crate dependency rule in `CLAUDE.md`:
//!
//! ```text
//! verbreel-http → verbreel-agent, verbreel-state, verbreel-storage
//! ```
//!
//! ## Lib + bin split
//!
//! The crate ships a thin `main.rs` that delegates to [`serve`]. Routing
//! and the request handlers live here in the library so integration
//! tests exercise them via `tower::ServiceExt::oneshot` without binding a
//! real TCP socket.
//!
//! ## Surface
//!
//! [`router`] assembles the AX endpoints over a workspace-bound
//! [`AppState`]:
//!
//! | Method | Path                                | Handler                       |
//! |--------|-------------------------------------|-------------------------------|
//! | GET    | `/healthz`                          | [`handlers::healthz`]         |
//! | GET    | `/capabilities`                     | [`handlers::capabilities`]    |
//! | GET    | `/tools`                            | [`handlers::list_tools`]      |
//! | GET    | `/projects`                         | [`handlers::list_projects`]   |
//! | POST   | `/projects`                         | [`handlers::create_project`]  |
//! | POST   | `/projects/{name}/tools/{verb}`     | [`handlers::call_tool`]       |
//! | POST   | `/projects/{name}/agent`            | [`handlers::agent_plan`]      |
//!
//! Every verb in the engine registry is reachable through `call_tool` —
//! there is no whitelist. Mutations persist through a real
//! [`verbreel_agent::Session`] rooted at `<workspace>/<name>`.
//!
//! ## Logs go to stderr
//!
//! The binary configures `tracing-subscriber` with `with_writer(stderr)`
//! so stdout stays free for piped output.

#![deny(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]

use std::net::SocketAddr;
use std::path::PathBuf;

use axum::{
    Router,
    routing::{get, post},
};

pub mod handlers;

/// Shared server state: the workspace directory projects live under
/// (`<workspace>/<name>`).
#[derive(Debug, Clone)]
pub struct AppState {
    /// Directory holding the server's projects.
    pub workspace: PathBuf,
}

/// Assemble the axum router over `state`.
///
/// Pure — no I/O, no socket — so tests construct a fresh router per case
/// (with a temp workspace) and drive it through
/// `tower::ServiceExt::oneshot`. (`Router` is itself `#[must_use]`, so no
/// attribute here.)
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(handlers::healthz))
        .route("/capabilities", get(handlers::capabilities))
        .route("/tools", get(handlers::list_tools))
        .route(
            "/projects",
            get(handlers::list_projects).post(handlers::create_project),
        )
        .route("/projects/{name}/tools/{verb}", post(handlers::call_tool))
        .route("/projects/{name}/agent", post(handlers::agent_plan))
        .with_state(state)
}

/// Bind `addr` and serve the [`router`] until the listener errors out.
///
/// The workspace is resolved from `VERBREEL_WORKSPACE`
/// ([`handlers::workspace_from_env`]).
///
/// # Errors
///
/// Returns an error if the TCP bind fails (port in use, insufficient
/// permissions, invalid address) or if `axum::serve` exits abnormally.
pub async fn serve(addr: SocketAddr) -> anyhow::Result<()> {
    let workspace = handlers::workspace_from_env();
    tracing::info!("verbreel-http workspace: {}", workspace.display());
    let state = AppState { workspace };
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("verbreel-http listening on {addr}");
    axum::serve(listener, router(state)).await?;
    Ok(())
}
