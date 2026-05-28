//! HTTP endpoint handlers.
//!
//! Three responsibilities, kept separate from the routing surface in
//! `lib.rs`:
//!
//! 1. [`healthz`] — flat liveness probe.
//! 2. [`list_tools`] — advertise the verbs whitelisted in
//!    [`SUPPORTED_VERBS`].
//! 3. [`call_tool`] — route a `POST /tools/{verb}` request through
//!    [`verbreel_state::default_registry`] and return the verb's `data`
//!    envelope under a top-level `data` key.
//!
//! ## Why a whitelist
//!
//! `verbreel-state` ships ~82 production verbs. Most need an on-disk
//! project, an event log, and a held lock — none of which this HTTP
//! server owns at v1 floor. The [`SUPPORTED_VERBS`] constant makes the
//! exposed surface explicit. Adding a verb later is additive: append
//! its name here and (if richer than the default description is
//! warranted) extend [`describe_verb`] with a matching arm. No edits
//! to [`call_tool`] required — the dispatch is registry-driven.
//!
//! ## Response shape (`POST /tools/{verb}`)
//!
//! | Status | Body                                                |
//! |--------|-----------------------------------------------------|
//! | 200    | `{"data": <verb-envelope>}`                          |
//! | 400    | `{"error": "bad args", "detail": "<reason>"}`        |
//! | 404    | `{"error": "unknown verb", "verb": "<name>"}`        |
//! | 500    | `{"error": "verb failure", "detail": "<reason>"}`    |
//!
//! 400 is reserved for malformed input — JSON syntax error, missing
//! `Content-Type`, non-object body. Verb-level rejections (bad arg
//! shape after parsing, invariant violation) surface as 500 with the
//! verb's own error message, mirroring the MCP slice contract.

use axum::{
    Json,
    extract::{Path, rejection::JsonRejection},
    http::StatusCode,
    response::IntoResponse,
};
use serde_json::{Value, json};
use verbreel_state::{ProjectId, default_registry, synthetic_empty_project};

/// Verbs exposed by this HTTP server at the current slice.
///
/// Adding a verb later is additive: append its name and (if needed)
/// extend [`describe_verb`].
pub const SUPPORTED_VERBS: &[&str] = &["project.list"];

/// Returns `true` when `verb` is exposed by this HTTP server.
#[must_use]
pub fn is_supported(verb: &str) -> bool {
    SUPPORTED_VERBS.contains(&verb)
}

/// `GET /healthz` — flat liveness probe. Always returns
/// `{"status": "ok"}` with a 200.
pub async fn healthz() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

/// `GET /tools` — return the verbs whitelisted in [`SUPPORTED_VERBS`].
///
/// Output is sorted by `name` so the response is deterministic across
/// invocations regardless of source-order changes to the whitelist.
pub async fn list_tools() -> Json<Value> {
    let mut tools: Vec<Value> = SUPPORTED_VERBS
        .iter()
        .map(|name| {
            json!({
                "name": name,
                "description": describe_verb(name),
            })
        })
        .collect();
    tools.sort_by(|a, b| {
        a["name"]
            .as_str()
            .unwrap_or_default()
            .cmp(b["name"].as_str().unwrap_or_default())
    });
    Json(json!({ "tools": tools }))
}

/// `POST /tools/{verb}` — dispatch a verb through
/// [`verbreel_state::default_registry`] and return its `data` envelope.
///
/// Response shape and error mapping are documented in the module-level
/// doc table above.
pub async fn call_tool(
    Path(verb): Path<String>,
    args: Result<Json<Value>, JsonRejection>,
) -> impl IntoResponse {
    if !is_supported(&verb) {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "unknown verb", "verb": verb })),
        );
    }

    let args = match args {
        Ok(Json(value)) => value,
        Err(rejection) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": "bad args",
                    "detail": rejection.body_text(),
                })),
            );
        }
    };

    // The Verb trait demands a JSON object — reject every other shape
    // up-front with the same 400 envelope so callers see a single
    // failure mode for "your body wasn't a JSON object".
    let mut args_obj = match args {
        Value::Object(map) => map,
        Value::Null => serde_json::Map::new(),
        other => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": "bad args",
                    "detail": format!(
                        "tool arguments must be a JSON object, got: {other}"
                    ),
                })),
            );
        }
    };

    // project.list is project-agnostic at v1 floor, but the Verb trait
    // still demands a `project_id` to clear its argument shape. When the
    // caller supplies one, the same id MUST flow into both `args` and
    // the synthetic `prior` — mismatched identities would let dispatch
    // see one project in its args and a different one in its prior
    // state, an invariant violation no current verb checks for but the
    // surrounding contract requires.
    let project_id: ProjectId = if let Some(existing) = args_obj.get("project_id") {
        match serde_json::from_value::<ProjectId>(existing.clone()) {
            Ok(parsed) => parsed,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({
                        "error": "bad args",
                        "detail": format!("invalid project_id: {e}"),
                    })),
                );
            }
        }
    } else {
        let synthesized = ProjectId::now();
        args_obj.insert("project_id".to_string(), json!(synthesized));
        synthesized
    };
    let args = Value::Object(args_obj);
    let prior = synthetic_empty_project(project_id);

    let registry = default_registry();
    let Some(verb_impl) = registry.get(&verb) else {
        // Whitelisted but missing from the registry — programmer bug,
        // not a user-facing 404. Surface it as 500 so the misalignment
        // is visible in logs rather than silently confusing the caller.
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": "verb failure",
                "detail": format!("verb '{verb}' whitelisted but not in default_registry"),
            })),
        );
    };

    match verb_impl.compute_patch(&prior, &args) {
        Ok((_patch, data, _warnings)) => (StatusCode::OK, Json(json!({ "data": data }))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": "verb failure",
                "detail": e.to_string(),
            })),
        ),
    }
}

/// Static descriptions for the verbs in this slice. New entries get
/// added when their verb is whitelisted; the default branch keeps the
/// `GET /tools` response well-formed even if a name is added to
/// [`SUPPORTED_VERBS`] before its description is filled in.
fn describe_verb(verb: &str) -> &'static str {
    match verb {
        "project.list" => {
            "List all known projects. v1 floor returns an empty array — \
             the real catalog index lands in a later slice."
        }
        _ => "Verbreel verb (no description provided).",
    }
}
