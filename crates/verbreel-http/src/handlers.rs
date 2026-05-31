//! HTTP endpoint handlers — the Agentic-Experience surface.
//!
//! The server is bound to a *workspace* directory ([`AppState`]).
//! Projects live at `<workspace>/<name>`; mutating verbs route through a
//! real [`verbreel_agent::Session`] (the §0.8 write-ordering kernel), so
//! edits persist. Three responsibility groups, kept separate from the
//! routing surface in `lib.rs`:
//!
//! 1. **Discovery** — [`healthz`], [`capabilities`] (the full verb
//!    catalog + per-verb args schemas), [`list_tools`].
//! 2. **Project lifecycle** — [`list_projects`], [`create_project`].
//! 3. **Dispatch** — [`call_tool`] applies one verb to a named project;
//!    [`agent_plan`] (feature `claude`) turns an intent into a plan and
//!    applies it.
//!
//! Under the `native-render` feature, `call_tool` routes `render.start`
//! to [`verbreel_runtime`] for an actual pixel encode against
//! `<workspace>/<name>`; every other verb takes the Session path.
//!
//! ## Path-traversal guard
//!
//! Project names arrive in the URL path and are joined onto the
//! workspace root. [`reject_unsafe_name`] turns any name that is not a
//! single, plain path component (no separators, no `.`/`..`, no NUL) into
//! a 400, so a request can never escape the workspace.

use std::path::PathBuf;

use axum::{
    Json,
    extract::{Path, State, rejection::JsonRejection},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};
use verbreel_agent::{AgentError, Capabilities, RunOutcome, Session};

use crate::AppState;

/// `GET /healthz` — flat liveness probe.
pub async fn healthz() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

/// `GET /capabilities` — the full engine capability catalog (every verb,
/// grouped by domain, with per-verb JSON args schemas). The surface an
/// agent reads to plan.
pub async fn capabilities() -> Json<Capabilities> {
    Json(Capabilities::current())
}

/// `GET /tools` — every registered verb as a tool descriptor
/// (`{ name, domain, args_schema? }`). The full surface, not a whitelist.
pub async fn list_tools() -> Json<Value> {
    let caps = Capabilities::current();
    let tools: Vec<Value> = caps
        .verbs
        .iter()
        .map(|v| {
            json!({
                "name": v.id,
                "domain": v.domain,
                "args_schema": v.args_schema,
            })
        })
        .collect();
    Json(json!({ "tools": tools, "count": tools.len() }))
}

/// `GET /projects` — names of every project under the workspace.
pub async fn list_projects(State(state): State<AppState>) -> Response {
    let mut names: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&state.workspace) {
        for entry in entries.flatten() {
            if entry.path().join("project.json").is_file()
                && let Some(name) = entry.file_name().to_str()
            {
                names.push(name.to_string());
            }
        }
    }
    names.sort();
    (StatusCode::OK, Json(json!({ "projects": names }))).into_response()
}

/// Body for `POST /projects`.
#[derive(Debug, Deserialize)]
pub struct CreateProjectBody {
    /// Project display name (also the workspace directory name).
    pub name: String,
    /// Canvas `<W>x<H>`; defaults to `1920x1080`.
    #[serde(default)]
    pub canvas: Option<String>,
    /// Frame rate `<num>/<den>`; defaults to 30/1.
    #[serde(default)]
    pub fps: Option<String>,
}

/// `POST /projects` — create a project under the workspace.
pub async fn create_project(
    State(state): State<AppState>,
    body: Result<Json<CreateProjectBody>, JsonRejection>,
) -> Response {
    let Json(body) = match body {
        Ok(b) => b,
        Err(rejection) => return bad_request(&rejection.body_text()),
    };
    if let Some(resp) = reject_unsafe_name(&body.name) {
        return resp;
    }
    let canvas = body.canvas.as_deref().unwrap_or("1920x1080");
    if let Err(detail) = validate_canvas_shape(canvas) {
        return bad_request(&detail);
    }
    let fps = match parse_fps_opt(body.fps.as_deref()) {
        Ok(f) => f,
        Err(detail) => return bad_request(&detail),
    };
    match Session::create(&state.workspace, &body.name, canvas, fps) {
        Ok(session) => (
            StatusCode::CREATED,
            Json(json!({
                "project_id": session.project_id().to_string(),
                "name": body.name,
                "root": session.root().display().to_string(),
            })),
        )
            .into_response(),
        Err(e) => agent_error_response(&e),
    }
}

/// `POST /projects/{name}/tools/{verb}` — apply one verb to a project.
///
/// Opens `<workspace>/<name>`, runs `verb` through the §0.8 forward path,
/// saves on a mutation, and returns the verb's `data` envelope plus
/// metadata. Read-only verbs return their data with no event written.
/// Under `native-render`, `render.start` is routed to the render runtime
/// for an actual pixel encode instead of the pure registry verb.
pub async fn call_tool(
    State(state): State<AppState>,
    Path((name, verb)): Path<(String, String)>,
    args: Result<Json<Value>, JsonRejection>,
) -> Response {
    if let Some(resp) = reject_unsafe_name(&name) {
        return resp;
    }
    let args = match args {
        Ok(Json(value)) => value,
        Err(rejection) => return bad_request(&rejection.body_text()),
    };
    let root = state.workspace.join(&name);

    #[cfg(feature = "native-render")]
    if verb == "render.start" {
        return call_render_start(&state, &root, args);
    }

    let mut session = match Session::open(&root) {
        Ok(s) => s,
        Err(e) => return agent_error_response(&e),
    };
    match session.run(&verb, args, None) {
        Ok(outcome) => {
            if outcome.mutated()
                && let Err(e) = session.save()
            {
                return agent_error_response(&e);
            }
            (StatusCode::OK, Json(outcome_json(&outcome))).into_response()
        }
        Err(e) => agent_error_response(&e),
    }
}

/// Route `render.start` to [`verbreel_runtime`] for an actual pixel
/// encode against the project at `root` (feature `native-render`).
#[cfg(feature = "native-render")]
fn call_render_start(state: &AppState, root: &std::path::Path, args: Value) -> Response {
    let typed: verbreel_state::RenderStartArgs = match serde_json::from_value(args) {
        Ok(typed) => typed,
        Err(err) => return bad_request(&format!("render.start: args deserialize failed: {err}")),
    };
    let runtime = state.render_runtime().clone().with_project_root(root);
    match runtime.render_start(&typed) {
        Ok(data) => match serde_json::to_value(&data) {
            Ok(value) => (
                StatusCode::OK,
                Json(json!({ "kind": "rendered", "mutated": true, "data": value })),
            )
                .into_response(),
            Err(err) => internal_error(&err.to_string()),
        },
        Err(err) => internal_error(&err.to_string()),
    }
}

/// Body for `POST /projects/{name}/agent`.
#[derive(Debug, Deserialize)]
pub struct AgentBody {
    /// Natural-language editing intent.
    pub intent: String,
    /// When true, apply the plan; when false (default), only return it.
    #[serde(default)]
    pub apply: bool,
}

/// `POST /projects/{name}/agent` — plan an intent (feature `claude`) and
/// optionally apply it.
#[cfg(feature = "claude")]
pub async fn agent_plan(
    State(state): State<AppState>,
    Path(name): Path<String>,
    body: Result<Json<AgentBody>, JsonRejection>,
) -> Response {
    if let Some(resp) = reject_unsafe_name(&name) {
        return resp;
    }
    let Json(body) = match body {
        Ok(b) => b,
        Err(rejection) => return bad_request(&rejection.body_text()),
    };

    // The Claude planner crosses the network with a *blocking* HTTP client
    // and the Session apply does blocking file IO. Running either directly
    // on a Tokio worker would park the thread for the whole multi-second
    // round-trip (and `reqwest::blocking` panics when entered from inside a
    // runtime), so the entire blocking unit is offloaded to `spawn_blocking`.
    let workspace = state.workspace.clone();
    let task = tokio::task::spawn_blocking(move || -> Result<Value, AgentError> {
        use verbreel_agent::{AnthropicClient, LlmPlanner, Planner};
        let caps = Capabilities::current();
        let client = AnthropicClient::from_env().map_err(AgentError::Planner)?;
        let plan = LlmPlanner::new(client)
            .plan(&body.intent, &caps)
            .map_err(AgentError::Planner)?;
        let plan_json = serde_json::to_value(&plan).unwrap_or(Value::Null);
        if !body.apply {
            return Ok(json!({ "plan": plan_json, "applied": false }));
        }
        let mut session = Session::open(workspace.join(&name))?;
        let results = session.apply_plan(&plan, &caps)?;
        let steps: Vec<Value> = results
            .iter()
            .map(|r| json!({ "verb": r.verb, "outcome": outcome_json(&r.outcome) }))
            .collect();
        Ok(json!({ "plan": plan_json, "applied": true, "steps": steps }))
    });

    match task.await {
        Ok(Ok(body)) => (StatusCode::OK, Json(body)).into_response(),
        Ok(Err(e)) => agent_error_response(&e),
        Err(join) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "planner task failed", "detail": join.to_string() })),
        )
            .into_response(),
    }
}

/// Feature-off `POST /projects/{name}/agent` — planning needs `claude`.
#[cfg(not(feature = "claude"))]
pub async fn agent_plan(
    State(_state): State<AppState>,
    Path(_name): Path<String>,
    _body: Result<Json<AgentBody>, JsonRejection>,
) -> Response {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({
            "error": "planner unavailable",
            "detail": "rebuild verbreel-http with `--features claude` and set ANTHROPIC_API_KEY"
        })),
    )
        .into_response()
}

/// Shape one [`RunOutcome`] as the JSON body of a `call_tool` response.
fn outcome_json(outcome: &RunOutcome) -> Value {
    match outcome {
        RunOutcome::Query { data, warnings } => json!({
            "kind": "query",
            "mutated": false,
            "data": data,
            "warnings": warnings,
        }),
        RunOutcome::Mutated {
            event_id,
            data,
            warnings,
        } => json!({
            "kind": "mutated",
            "mutated": true,
            "event_id": event_id,
            "data": data,
            "warnings": warnings,
        }),
        RunOutcome::Replayed {
            event_id,
            data,
            warnings,
        } => json!({
            "kind": "replayed",
            "mutated": false,
            "event_id": event_id,
            "data": data,
            "warnings": warnings,
        }),
    }
}

/// Map an [`AgentError`] onto an HTTP status + JSON error body.
fn agent_error_response(err: &AgentError) -> Response {
    let (status, kind) = match err {
        AgentError::UnknownVerb { .. } => (StatusCode::NOT_FOUND, "unknown verb"),
        AgentError::VerbExecution { .. } | AgentError::InvalidPlan { .. } => {
            (StatusCode::BAD_REQUEST, "bad request")
        }
        AgentError::ProjectCreate(_) => (StatusCode::CONFLICT, "project create failed"),
        AgentError::Lifecycle(lifecycle) => lifecycle_status(lifecycle),
        AgentError::Planner(planner) => planner_status(planner),
    };
    (
        status,
        Json(json!({ "error": kind, "detail": err.to_string() })),
    )
        .into_response()
}

/// HTTP status for a kernel lifecycle error.
fn lifecycle_status(err: &verbreel_state::LifecycleError) -> (StatusCode, &'static str) {
    use verbreel_state::LifecycleError as L;
    match err {
        L::NoProjectJson => (StatusCode::NOT_FOUND, "no such project"),
        L::ProjectAlreadyExists => (StatusCode::CONFLICT, "project exists"),
        L::LockHeldByAnotherProcess { .. } => (StatusCode::CONFLICT, "project locked"),
        L::UnknownVerb { .. } => (StatusCode::NOT_FOUND, "unknown verb"),
        L::VerbExecutionFailed { .. }
        | L::IdempotencyConflict { .. }
        | L::IdempotencyBusy { .. } => (StatusCode::BAD_REQUEST, "verb rejected"),
        _ => (StatusCode::INTERNAL_SERVER_ERROR, "engine failure"),
    }
}

/// HTTP status for a planner error.
fn planner_status(err: &verbreel_agent::PlannerError) -> (StatusCode, &'static str) {
    use verbreel_agent::PlannerError as P;
    match err {
        P::NotConfigured { .. } => (StatusCode::NOT_IMPLEMENTED, "planner not configured"),
        P::Transport { .. } => (StatusCode::BAD_GATEWAY, "planner transport"),
        P::BadReply { .. } => (StatusCode::BAD_GATEWAY, "planner reply"),
    }
}

/// 400 with a `{ error, detail }` body.
fn bad_request(detail: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "error": "bad request", "detail": detail })),
    )
        .into_response()
}

/// 500 with a `{ error, detail }` body.
#[cfg(feature = "native-render")]
fn internal_error(detail: &str) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": "verb failure", "detail": detail })),
    )
        .into_response()
}

/// Path-traversal guard for `<workspace>/<name>`: returns a 400 response
/// when `name` is not a single, plain path component, else `None`.
///
/// Returns `Option<Response>` (not `Result<(), Response>`) because
/// `axum::response::Response` is large — a `Result` Err of that size
/// trips `clippy::result_large_err`, and the "rejection or nothing"
/// shape is exactly an `Option` anyway.
fn reject_unsafe_name(name: &str) -> Option<Response> {
    let bad = name.is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || name.contains('\0');
    bad.then(|| {
        bad_request("project name must be a single path component (no '/', '\\', '..', or NUL)")
    })
}

/// Validate the `<W>x<H>` canvas shape up front so a malformed canvas is
/// a 400 (bad input) rather than flowing into `Session::create` and
/// surfacing as a 409 (which reads as "already exists"). The kernel still
/// range-checks the dimensions; this only guards the wire shape.
fn validate_canvas_shape(canvas: &str) -> Result<(), String> {
    let (w, h) = canvas
        .split_once('x')
        .ok_or_else(|| format!("canvas must be <W>x<H>, got {canvas:?}"))?;
    w.trim()
        .parse::<u32>()
        .map_err(|_| format!("canvas width is not a number in {canvas:?}"))?;
    h.trim()
        .parse::<u32>()
        .map_err(|_| format!("canvas height is not a number in {canvas:?}"))?;
    Ok(())
}

/// Parse an optional `<num>/<den>` fps literal.
fn parse_fps_opt(fps: Option<&str>) -> Result<Option<(u32, u32)>, String> {
    let Some(s) = fps else { return Ok(None) };
    let (num, den) = s
        .split_once('/')
        .ok_or_else(|| "fps must be <num>/<den>, e.g. 30/1".to_string())?;
    let num = num
        .trim()
        .parse()
        .map_err(|_| "fps numerator invalid".to_string())?;
    let den = den
        .trim()
        .parse()
        .map_err(|_| "fps denominator invalid".to_string())?;
    Ok(Some((num, den)))
}

/// Resolve the workspace directory for the server from the environment.
///
/// `VERBREEL_WORKSPACE` pins a stable directory; unset falls back to
/// `<temp>/verbreel-workspace` so the server always starts. The directory
/// is created if absent.
#[must_use]
pub fn workspace_from_env() -> PathBuf {
    let dir = std::env::var_os("VERBREEL_WORKSPACE").map_or_else(
        || std::env::temp_dir().join("verbreel-workspace"),
        PathBuf::from,
    );
    let _ = std::fs::create_dir_all(&dir);
    dir
}
