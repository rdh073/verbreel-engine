//! verbreel-http — axum 0.8 HTTP server.
//!
//! Per the crate dependency rule in `CLAUDE.md`:
//!
//! ```text
//! verbreel-http → verbreel-state, verbreel-storage
//! verbreel-http/native-render → verbreel-runtime
//! ```
//!
//! ## Lib + bin split
//!
//! The crate ships a thin `main.rs` that delegates to [`serve`]. Routing
//! and the request handlers live here in the library so integration
//! tests can exercise them via `tower::ServiceExt::oneshot` without
//! binding a real TCP socket.
//!
//! ## Surface (§commands.md:7 — `POST /v1/<noun>/<verb>`)
//!
//! [`router`] assembles a thin shell over the shared
//! [`verbreel_state::Engine`]:
//!
//! | Method | Path                  | Handler                  |
//! |--------|-----------------------|--------------------------|
//! | GET    | `/healthz`            | [`handlers::healthz`]    |
//! | GET    | `/v1`                 | [`handlers::list_verbs`] |
//! | POST   | `/v1/{noun}/{verb}`   | [`handlers::call_verb`]  |
//!
//! Every verb [`verbreel_state::Engine::verb_ids`] knows about is
//! reachable — there is **no whitelist**. The two-segment path is
//! reconstructed into the `<noun>.<verb>` registry key the engine
//! dispatches on (with the flat-meta translation documented on
//! [`handlers::call_verb`]). An unknown verb is rejected by the
//! **engine** returning `E_UNKNOWN_VERB` in the §0.1 envelope, never by
//! a hardcoded list at the HTTP layer. Responses are the §0.1 envelope
//! produced by [`verbreel_state::Envelope::to_json`].
//!
//! ## Held engine (§0.12 project context)
//!
//! [`AppState`] holds one [`verbreel_state::Engine`] behind an
//! `Arc<tokio::sync::Mutex<…>>` shared across requests, so a
//! `project.create` / `project.open` POST populates the engine's
//! open-project map and a later POST naming the same `project_id` in its
//! body targets it. Project context is **always** a client-supplied body
//! arg — never inferred from the URL path or process state (§0.12).
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
use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    Router,
    routing::{get, post},
};
use tokio::sync::Mutex;
use verbreel_state::Engine;

pub mod handlers;

/// Axum application state.
///
/// Holds the shared [`Engine`] across requests so the open-project map
/// (populated by `project.create` / `project.open`) survives between
/// POSTs (§0.12). `dispatch` takes `&mut self` and may do blocking fs
/// I/O for the persisted engine, so the handle is an async
/// `tokio::sync::Mutex` held across the dispatch call. This serializes
/// mutating requests per server instance — a strictly-stronger (and
/// simpler) serialization than the §0.8 per-project write mutex the
/// engine already requires. A per-project lock map is a documented
/// follow-up, not a spec requirement.
#[derive(Clone)]
pub struct AppState {
    /// Shared verb-dispatch engine, mutated under an async lock.
    pub engine: Arc<Mutex<Engine>>,
    #[cfg(feature = "native-render")]
    render_runtime: verbreel_runtime::RenderRuntimeConfig,
}

impl AppState {
    /// Build application state for an engine rooted at `home`.
    ///
    /// `home` is the user directory whose `.verbreel/projects-index`
    /// resolves a `project_id` to a root (§0.12). Under `native-render`
    /// the render runtime is resolved from the environment
    /// ([`verbreel_runtime::RenderRuntimeConfig::from_env`]).
    #[must_use]
    pub fn new(home: PathBuf) -> Self {
        Self {
            engine: Arc::new(Mutex::new(Engine::new(home))),
            #[cfg(feature = "native-render")]
            render_runtime: verbreel_runtime::RenderRuntimeConfig::from_env(),
        }
    }

    /// Build application state with an injected render runtime.
    ///
    /// The engine is rooted at `home`; the render runtime is the one the
    /// caller supplies (tests point it at a fixture project root).
    #[cfg(feature = "native-render")]
    #[must_use]
    pub fn with_render_runtime(
        home: PathBuf,
        render_runtime: verbreel_runtime::RenderRuntimeConfig,
    ) -> Self {
        Self {
            engine: Arc::new(Mutex::new(Engine::new(home))),
            render_runtime,
        }
    }

    #[cfg(feature = "native-render")]
    pub(crate) fn render_runtime(&self) -> &verbreel_runtime::RenderRuntimeConfig {
        &self.render_runtime
    }
}

/// Resolve the `.verbreel` home dir for the engine's projects index.
///
/// `VERBREEL_HOME` wins when set; otherwise `$HOME/.verbreel` (the same
/// location `verbreel-storage` writes its `projects-index` into). There
/// is no hardcoded absolute fallback — if neither `VERBREEL_HOME` nor
/// `HOME` is set the engine is rooted at the current directory, which
/// keeps `serve` infallible without inventing a path.
fn default_home() -> PathBuf {
    if let Ok(home) = std::env::var("VERBREEL_HOME") {
        return PathBuf::from(home);
    }
    if let Ok(user_home) = std::env::var("HOME") {
        return PathBuf::from(user_home).join(".verbreel");
    }
    PathBuf::from(".")
}

/// Assemble the axum router with default home-resolved state.
///
/// Resolves the engine home via [`default_home`] and builds a fresh
/// [`AppState`]. Tests that need a hermetic home use
/// [`router_with_state`] with `AppState::new(tmp_home)` instead.
pub fn router() -> Router {
    router_with_state(AppState::new(default_home()))
}

/// Assemble the axum router with explicit app state.
pub fn router_with_state(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(handlers::healthz))
        .route("/v1", get(handlers::list_verbs))
        .route("/v1/{noun}/{verb}", post(handlers::call_verb))
        .with_state(state)
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
