//! Router integration tests — drive [`verbreel_http::router`] through
//! `tower::ServiceExt::oneshot`. No real TCP bind; every request goes
//! through the in-process service stack the same way axum dispatches.
//!
//! Each test gets its own temp workspace so project state never leaks
//! across cases.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;
use verbreel_http::{AppState, router};

/// A fresh temp workspace bound into an [`AppState`]. The returned
/// `TempDir` must be held for the test's duration.
fn workspace() -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = AppState::new(dir.path().to_path_buf());
    (dir, state)
}

/// Send a request to a fresh router built from `state` (cloned, so the
/// caller can send many requests against the same workspace). Returns
/// `(status, json_body)`.
async fn send(
    state: &AppState,
    method: Method,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let builder = Request::builder().method(method).uri(uri);
    let req = match body {
        Some(v) => builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(v.to_string()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    };
    let res = router(state.clone()).oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

#[tokio::test]
async fn healthz_returns_ok() {
    let (_dir, state) = workspace();
    let (status, body) = send(&state, Method::GET, "/healthz", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ok");
}

#[tokio::test]
async fn capabilities_exposes_the_full_verb_catalog() {
    let (_dir, state) = workspace();
    let (status, body) = send(&state, Method::GET, "/capabilities", None).await;
    assert_eq!(status, StatusCode::OK);
    let verbs = body["verbs"].as_array().expect("verbs array");
    assert!(verbs.len() >= 116, "got {} verbs", verbs.len());
    assert!(verbs.iter().any(|v| v["id"] == "clip.trim"));
    assert_eq!(body["tick_rate_hz"].as_u64().unwrap(), 240_000);
}

#[tokio::test]
async fn tools_lists_every_verb() {
    let (_dir, state) = workspace();
    let (status, body) = send(&state, Method::GET, "/tools", None).await;
    assert_eq!(status, StatusCode::OK);
    let count = body["count"].as_u64().expect("count");
    assert!(count >= 116, "got {count} tools");
    let tools = body["tools"].as_array().expect("tools array");
    assert!(tools.iter().any(|t| t["name"] == "render.queue.add"));
}

#[tokio::test]
async fn create_then_dispatch_then_observe_persists() {
    let (_dir, state) = workspace();

    // create
    let (status, body) = send(
        &state,
        Method::POST,
        "/projects",
        Some(json!({ "name": "demo", "canvas": "1920x1080" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create body: {body}");
    assert!(body["project_id"].is_string());

    // mutate: rename
    let (status, body) = send(
        &state,
        Method::POST,
        "/projects/demo/tools/project.rename",
        Some(json!({ "name": "Renamed" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "rename body: {body}");
    assert_eq!(body["mutated"], true);
    assert!(body["event_id"].is_string());

    // read: project.info reflects the rename (persisted across the
    // open/close each dispatch performs).
    let (status, body) = send(
        &state,
        Method::POST,
        "/projects/demo/tools/project.info",
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "info body: {body}");
    assert_eq!(body["mutated"], false);
    assert_eq!(body["data"]["name"], "Renamed");

    // list
    let (status, body) = send(&state, Method::GET, "/projects", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["projects"], json!(["demo"]));
}

#[tokio::test]
async fn dispatch_on_missing_project_is_404() {
    let (_dir, state) = workspace();
    let (status, _body) = send(
        &state,
        Method::POST,
        "/projects/ghost/tools/project.info",
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn unknown_verb_is_404() {
    let (_dir, state) = workspace();
    send(
        &state,
        Method::POST,
        "/projects",
        Some(json!({ "name": "demo" })),
    )
    .await;
    let (status, _body) = send(
        &state,
        Method::POST,
        "/projects/demo/tools/clip.teleport",
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn create_rejects_path_traversal_name() {
    let (_dir, state) = workspace();
    let (status, body) = send(
        &state,
        Method::POST,
        "/projects",
        Some(json!({ "name": "../escape" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
}

/// Under `native-render`, `render.start` routes to the runtime; an
/// unknown preset must surface the runtime's typed error code through
/// HTTP (lightweight — fails at preset validation, no GPU/FFmpeg work).
#[tokio::test]
#[cfg(feature = "native-render")]
async fn render_start_delegates_to_runtime() -> anyhow::Result<()> {
    use verbreel_runtime::RenderRuntimeConfig;
    use verbreel_state::{ProjectCreateArgs, project_create};

    let dir = tempfile::tempdir()?;
    let created = project_create(&ProjectCreateArgs {
        name: "http-render".to_string(),
        canvas: "64x64".to_string(),
        fps_num: Some(30),
        fps_den: Some(1),
        at: Some(dir.path().to_path_buf()),
        activate: false,
        metadata: serde_json::Map::new(),
    })?;
    let state = AppState::with_render_runtime(dir.path().to_path_buf(), RenderRuntimeConfig::new());
    let (status, body) = send(
        &state,
        Method::POST,
        "/projects/http-render/tools/render.start",
        Some(json!({
            "project_id": created.project_id,
            "preset": "definitely-not-a-preset",
            "out_path": "exports/out.mp4",
            "from_tk": 0,
            "to_tk": 8000,
            "overwrite": true,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        body["detail"]
            .as_str()
            .is_some_and(|detail| detail.contains("E_RENDER_PRESET_UNKNOWN")),
        "HTTP must surface the runtime error code, got: {body}"
    );
    Ok(())
}
