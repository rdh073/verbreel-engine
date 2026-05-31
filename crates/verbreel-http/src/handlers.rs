//! HTTP endpoint handlers — a thin shell over [`verbreel_state::Engine`].
//!
//! Three responsibilities, kept separate from the routing surface in
//! `lib.rs`:
//!
//! 1. [`healthz`] — flat liveness probe.
//! 2. [`list_verbs`] — `GET /v1` discovery: the sorted id list the held
//!    [`Engine::verb_ids`] reports.
//! 3. [`call_verb`] — route a `POST /v1/{noun}/{verb}` request through
//!    [`Engine::dispatch`] and return the §0.1 envelope produced by
//!    [`verbreel_state::Envelope::to_json`].
//!
//! ## No whitelist
//!
//! Every verb the engine knows about is reachable. The HTTP layer parses
//! the path, reads the JSON body, calls `dispatch`, serializes the
//! [`verbreel_state::Envelope`], and picks an HTTP status — it adds zero
//! verb logic. Which verbs need an open project, what `E_*` code fires,
//! snap / idempotency: all of that already lives in the engine. An
//! unknown verb is rejected by the engine (`E_UNKNOWN_VERB`), not by a
//! hardcoded list here.
//!
//! ## Status mapping (§0.1 envelope is self-describing)
//!
//! Every dispatched verb — success or `E_*` failure — returns **`200`**;
//! the `ok` flag in the body carries success. The §0.1 envelope is
//! self-describing, GUI/remote clients branch on `body.ok`, and a
//! uniform 200 avoids leaking a partial HTTP-status taxonomy that does
//! not map cleanly onto the `E_*` codes (the spec never pins one). See
//! [`http_status_for`] — flipping to an `E_*`→status taxonomy is a
//! one-function edit. **`400` is reserved for transport-level malformed
//! input** (non-object body, wrong content-type, JSON syntax error) and
//! is returned by [`bad_args`] **before** dispatch — those failures
//! never reach the engine or the envelope.

use axum::{
    Json,
    extract::{Path, State, rejection::JsonRejection},
    http::StatusCode,
    response::IntoResponse,
};
use serde_json::{Value, json};
use verbreel_state::Envelope;
#[cfg(feature = "native-render")]
use verbreel_state::RenderStartArgs;

use crate::AppState;

/// `GET /healthz` — flat liveness probe. Always returns
/// `{"status": "ok"}` with a 200.
pub async fn healthz() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

/// `GET /v1` — discovery: the verb-id list the held engine reports via
/// [`Engine::verb_ids`], projected to the **routable** form.
///
/// Shape: `{ "verbs": ["asset.import", "clip.add", "meta.help", …] }`.
///
/// The five meta verbs (`help`, `schema`, `describe`,
/// `validate_command`, `list_capabilities`) are registered with **flat
/// ids** (no noun dot), but they are only reachable at
/// `POST /v1/meta/<verb>` — [`call_verb`] strips the `meta.` segment
/// back to the flat id before dispatch. Advertising the bare flat id
/// here would break a discovery-driven client that splits an id on `.`
/// to reconstruct `/v1/<noun>/<verb>`: it would get a single segment
/// and have no noun to build the route. So discovery emits the **dotted
/// routable form** `meta.<id>` for every flat id — the exact inverse of
/// the [`call_verb`] flat-meta translation — keeping every advertised id
/// mechanically mappable to its POST route. A no-dot id is, by
/// construction, a meta verb (verified: only the five meta verbs
/// register a flat id), so the projection is registration-driven, not a
/// hardcoded list that can drift.
///
/// `verb_ids()` already returns a sorted `Vec<String>` (FROZEN PR1
/// contract); prefixing `meta.` preserves that order (the flat ids sort
/// into the same relative positions among themselves, and all gain the
/// same `meta.` prefix), but we re-sort defensively so the output stays
/// sorted regardless of how the projection reshuffles ids relative to
/// the dotted general verbs.
///
/// [`Engine::verb_ids`]: verbreel_state::Engine::verb_ids
pub async fn list_verbs(State(state): State<AppState>) -> Json<Value> {
    let mut verbs: Vec<String> = {
        let engine = state.engine.lock().await;
        engine.verb_ids()
    }
    .into_iter()
    .map(|id| {
        if id.contains('.') {
            id
        } else {
            format!("meta.{id}")
        }
    })
    .collect();
    verbs.sort_unstable();
    Json(json!({ "verbs": verbs }))
}

/// `POST /v1/{noun}/{verb}` — dispatch a verb through the held
/// [`Engine`] and return the §0.1 envelope.
///
/// The two-segment path is reconstructed into the registry key the
/// engine dispatches on:
///
/// - **General verbs** → `<noun>.<verb>` (e.g. `clip.add`,
///   `project.list`).
/// - **Meta verbs** → flat id. The five meta verbs (`help`, `schema`,
///   `describe`, `validate_command`, `list_capabilities`) are
///   registered in `verbreel-state` with **flat ids** (no noun dot),
///   but the spec lists them under noun `Meta` and the public URL is
///   `POST /v1/meta/<verb>` (`commands.md:7`, §1). So when
///   `noun == "meta"` the engine verb id is `verb` alone (`help`, not
///   `meta.help`). This is a pure surface translation — no engine
///   change — keeping the URL spec-shaped while feeding the engine the
///   flat id it registers. (`commands/meta.md` pins no alternate HTTP
///   path; if it ever does, this is the one line to revisit.)
///
/// The JSON body becomes the args object passed straight to `dispatch`
/// (an omitted / `null` body coerces to `{}` so no-arg verbs need no
/// body). The engine routes project-less reads vs project-scoped
/// mutations internally and resolves `project_id` (a body arg, §0.12)
/// against its own open-project map.
///
/// Status mapping lives entirely in [`http_status_for`]; see the
/// module-level doc for the 200-always rationale.
///
/// [`Engine`]: verbreel_state::Engine
pub async fn call_verb(
    State(state): State<AppState>,
    Path((noun, verb)): Path<(String, String)>,
    body: Result<Json<Value>, JsonRejection>,
) -> impl IntoResponse {
    let verb_id = if noun == "meta" {
        verb
    } else {
        format!("{noun}.{verb}")
    };

    let args = match body {
        Ok(Json(Value::Object(map))) => Value::Object(map),
        // An omitted or explicitly-null body means "no args" — coerce to
        // an empty object so verbs that take no args need no body.
        Ok(Json(Value::Null)) => json!({}),
        Ok(Json(other)) => {
            return bad_args(format!("request body must be a JSON object, got: {other}"));
        }
        Err(rejection) => return bad_args(rejection.body_text()),
    };

    // render.start is a §11 streaming verb whose native execution lives
    // in `verbreel-runtime` (injected at this composition root), not in
    // the shared Engine — the Engine routes the in-graph registry
    // `render.start` verb but does not own the ffmpeg runtime. Under
    // `native-render` the HTTP layer therefore calls the injected
    // runtime directly and wraps its result in the §0.1 envelope.
    #[cfg(feature = "native-render")]
    if verb_id == "render.start" {
        return call_render_start(&args, &state);
    }

    let envelope = {
        let mut engine = state.engine.lock().await;
        // TODO: `dispatch` does synchronous blocking fs I/O (event-log
        // append, project.json write) while holding the async mutex on
        // the tokio worker thread. Move it onto `tokio::task::spawn_blocking`
        // (the engine handle is already `Arc<Mutex<…>>`, so a clone can
        // cross the spawn boundary) so a slow disk does not stall the
        // reactor. Deferred: needs the `Engine` to be `Send + 'static`
        // across the spawn and is a perf concern, not a correctness one.
        engine.dispatch(&verb_id, args)
    };
    let status = http_status_for(&envelope);
    (status, Json(envelope.to_json()))
}

/// Dispatch the native `render.start` through the injected runtime and
/// wrap the result in the §0.1 envelope.
///
/// Success → an `ok:true` envelope carrying the runtime's `data` (no
/// patch / `event_id` — the native render path records its own event in
/// `verbreel-runtime`). Failure → an `ok:false` envelope whose `message`
/// carries the runtime's error string (which embeds the `E_*` code, e.g.
/// `E_RENDER_PRESET_UNKNOWN`).
#[cfg(feature = "native-render")]
fn call_render_start(args: &Value, state: &AppState) -> (StatusCode, Json<Value>) {
    let typed: RenderStartArgs = match serde_json::from_value(args.clone()) {
        Ok(typed) => typed,
        Err(err) => {
            return error_envelope(
                "E_SCHEMA_VIOLATION",
                &format!("render.start: args deserialize failed: {err}"),
            );
        }
    };
    match state.render_runtime().render_start(&typed) {
        Ok(data) => {
            let envelope = Envelope::Ok {
                data: json!(data),
                patch: json!([]),
                warnings: vec![],
                // TODO(#455): thread the truthful event_id recorded by the
                // native render path in verbreel-runtime through here; `""`
                // is the §0.1 read-only/dry-run placeholder until then.
                event_id: String::new(),
            };
            (StatusCode::OK, Json(envelope.to_json()))
        }
        // The runtime error Display embeds its `E_*` code; surface it as
        // the envelope `code` so clients read a stable code, not just the
        // human message.
        Err(err) => error_envelope(err.code(), &err.to_string()),
    }
}

/// Build an `ok:false` §0.1 envelope response with a 200 status (status
/// mapping is uniform — see [`http_status_for`]).
///
/// Synthesizes the wire shape through [`Envelope::Err`] + [`Envelope::to_json`]
/// (rather than a hand-rolled `json!`) so it stays mechanically in lockstep
/// with the engine's §0.1 serialization.
#[cfg(feature = "native-render")]
fn error_envelope(code: &str, message: &str) -> (StatusCode, Json<Value>) {
    let envelope = Envelope::Err {
        code: code.to_string(),
        message: message.to_string(),
        hint: None,
        details: None,
    };
    (StatusCode::OK, Json(envelope.to_json()))
}

/// Map an [`Envelope`] to an HTTP status. **Uniform `200`** — the §0.1
/// envelope is self-describing (`ok` + `code`), so success and every
/// `E_*` failure both return 200 and clients branch on `body.ok`.
///
/// This is the single point to flip the policy. The reviewer-preferred
/// `E_*`→status taxonomy (unknown/not-found → 404, schema/bad-arg → 400,
/// busy/locked → 409, other-err → 500, ok → 200) would replace the body
/// of this one function — every status decision routes through here, so
/// no handler edits are needed to switch.
#[must_use]
pub fn http_status_for(envelope: &Envelope) -> StatusCode {
    // Uniform 200 for both arms today. The `match` (rather than an
    // unconditional return) is the seam: swapping these arms for the
    // `E_*`→status taxonomy is a body-only edit, no signature/caller
    // churn, because `envelope` is already destructured here.
    match envelope {
        Envelope::Ok { .. } | Envelope::Err { .. } => StatusCode::OK,
    }
}

/// `400 Bad Request` with a `{"error":"bad args","detail":…}` body.
///
/// Reserved for **transport-level** malformed input — a non-object JSON
/// body, a wrong/missing content-type, or a JSON syntax error — caught
/// **before** dispatch. Every verb-level rejection (unknown verb, bad
/// arg shape, schema violation, project-not-open) flows through the
/// engine into the §0.1 error envelope instead, never here.
fn bad_args(detail: impl Into<String>) -> (StatusCode, Json<Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "error": "bad args", "detail": detail.into() })),
    )
}
