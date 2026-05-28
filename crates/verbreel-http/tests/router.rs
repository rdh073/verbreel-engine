//! Router integration tests — drive [`verbreel_http::router`] through
//! `tower::ServiceExt::oneshot`. No real TCP bind; every request goes
//! through the in-process service stack the same way axum dispatches.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;
use verbreel_http::router;

/// Send a JSON `POST` to the in-memory router and return
/// `(status, body)` for inspection. Keeps every test below to its
/// invariant assertions instead of repeating the wiring boilerplate.
async fn post_json(uri: &str, body: Value) -> (StatusCode, Value) {
    let req = Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let res = router().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    (status, value)
}

/// Send a `GET` request to the in-memory router and return
/// `(status, body)`.
async fn get_json(uri: &str) -> (StatusCode, Value) {
    let req = Request::builder()
        .method(Method::GET)
        .uri(uri)
        .body(Body::empty())
        .unwrap();
    let res = router().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    (status, value)
}

#[tokio::test]
async fn healthz_returns_200_with_status_ok() {
    let (status, body) = get_json("/healthz").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({ "status": "ok" }));
}

#[tokio::test]
async fn list_tools_returns_200_with_project_list_only() {
    let (status, body) = get_json("/tools").await;
    assert_eq!(status, StatusCode::OK);
    let tools = body["tools"]
        .as_array()
        .expect("response must carry `tools` array");
    assert_eq!(
        tools.len(),
        1,
        "v1 floor must advertise exactly one tool, got: {tools:?}"
    );
    assert_eq!(tools[0]["name"].as_str(), Some("project.list"));
}

#[tokio::test]
async fn list_tools_entries_carry_descriptions() {
    let (_status, body) = get_json("/tools").await;
    for tool in body["tools"].as_array().unwrap() {
        let description = tool["description"]
            .as_str()
            .expect("every advertised tool must carry a description");
        assert!(
            !description.is_empty(),
            "description must be non-empty, got: {tool}"
        );
    }
}

#[tokio::test]
async fn post_project_list_returns_200_with_empty_array() {
    let (status, body) = post_json("/tools/project.list", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    let projects = body["data"]["projects"]
        .as_array()
        .expect("envelope must expose `data.projects` as a JSON array");
    assert!(
        projects.is_empty(),
        "v1 floor must return empty projects list, got: {body}"
    );
}

#[tokio::test]
async fn post_project_list_envelope_is_wrapped_under_data_key() {
    let (_status, body) = post_json("/tools/project.list", json!({})).await;
    assert!(
        body.get("data").is_some(),
        "response must wrap the verb envelope under `data`, got: {body}"
    );
    // No top-level `projects` — that would mean the wrapper leaked the
    // envelope directly instead of nesting it.
    assert!(
        body.get("projects").is_none(),
        "response must NOT expose `projects` at the top level: {body}"
    );
}

#[tokio::test]
async fn post_unknown_verb_returns_404_with_error_envelope() {
    let (status, body) = post_json("/tools/totally.fake.verb", json!({})).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"].as_str(), Some("unknown verb"));
    assert_eq!(body["verb"].as_str(), Some("totally.fake.verb"));
}

#[tokio::test]
async fn post_non_whitelisted_verb_returns_404_even_if_in_registry() {
    // `project.set_metadata` IS in default_registry but NOT in
    // SUPPORTED_VERBS — the whitelist must short-circuit before
    // dispatch reaches the registry.
    let (status, body) = post_json("/tools/project.set_metadata", json!({})).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"].as_str(), Some("unknown verb"));
}

#[tokio::test]
async fn post_malformed_json_body_returns_400_with_bad_args() {
    let req = Request::builder()
        .method(Method::POST)
        .uri("/tools/project.list")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("{ not json"))
        .unwrap();
    let res = router().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["error"].as_str(), Some("bad args"));
    assert!(
        body["detail"].as_str().is_some_and(|s| !s.is_empty()),
        "detail must explain the parse failure, got: {body}"
    );
}

#[tokio::test]
async fn post_missing_content_type_returns_400_with_bad_args() {
    // axum's `Json` extractor demands a JSON content-type — surfaces as
    // a `JsonRejection::MissingJsonContentType`. Our handler maps every
    // `JsonRejection` flavor to the `{error: "bad args"}` envelope.
    let req = Request::builder()
        .method(Method::POST)
        .uri("/tools/project.list")
        .body(Body::from("{}"))
        .unwrap();
    let res = router().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["error"].as_str(), Some("bad args"));
}

#[tokio::test]
async fn post_array_body_returns_400_with_bad_args() {
    // Valid JSON but not a JSON object — verb dispatch refuses it.
    let (status, body) = post_json("/tools/project.list", json!([1, 2, 3])).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"].as_str(), Some("bad args"));
    assert!(
        body["detail"]
            .as_str()
            .is_some_and(|s| s.contains("JSON object")),
        "detail should call out the shape requirement, got: {body}"
    );
}

#[tokio::test]
async fn post_null_body_is_coerced_to_empty_object() {
    // `Value::Null` is a valid JSON body but the verb expects an object.
    // Coerce `null` -> `{}` so callers can omit the body when the verb
    // takes no args — same shape the MCP wrapper applies.
    let (status, body) = post_json("/tools/project.list", Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["data"]["projects"].is_array());
}

#[tokio::test]
async fn get_on_post_only_route_returns_method_not_allowed() {
    let req = Request::builder()
        .method(Method::GET)
        .uri("/tools/project.list")
        .body(Body::empty())
        .unwrap();
    let res = router().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn unknown_route_returns_404() {
    let req = Request::builder()
        .method(Method::GET)
        .uri("/no-such-path")
        .body(Body::empty())
        .unwrap();
    let res = router().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn project_list_response_is_deterministic_across_invocations() {
    // The synthesised ProjectId varies per request, but the verb's
    // `data` envelope must NOT — otherwise identical HTTP calls would
    // yield divergent payloads.
    let (_status_a, body_a) = post_json("/tools/project.list", json!({})).await;
    let (_status_b, body_b) = post_json("/tools/project.list", json!({})).await;
    assert_eq!(
        body_a["data"], body_b["data"],
        "v1 envelope must be stable across identical requests"
    );
}

#[tokio::test]
async fn caller_supplied_project_id_is_accepted() {
    // The dispatch layer fills in a synthesised project_id only when
    // the caller didn't supply one. Confirm a caller-supplied v7 UUID
    // is accepted (no 400, no 500) — the engine rejects nil UUIDs per
    // §0.13 invariants, so a fresh v7 is required.
    let pid = uuid::Uuid::now_v7();
    let (status, body) = post_json(
        "/tools/project.list",
        json!({ "project_id": pid.to_string() }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "got body: {body}");
    assert!(body["data"]["projects"].is_array());
}

#[tokio::test]
async fn caller_supplied_project_id_is_threaded_into_prior_state() {
    // The dispatch layer must use the caller's `project_id` for BOTH
    // the args going into the verb AND the synthetic prior Project,
    // otherwise args and prior refer to different projects and the
    // contract is violated. Spec §0.13 rejects nil UUIDs, so the
    // engine would return an error if the prior were silently built
    // from a fresh id while args carried the nil one — exploit that:
    // a nil-uuid caller id must surface as a 400 (parse rejection),
    // never a 500 from the verb seeing a mismatched pair.
    let nil = "00000000-0000-0000-0000-000000000000";
    let (status, body) = post_json("/tools/project.list", json!({ "project_id": nil })).await;
    // Either 400 (parse layer rejects nil) or 500 from the engine —
    // BOTH are acceptable disposition for an invalid id; what's NOT
    // acceptable is 200 with the nil silently overwritten by a fresh
    // synthetic id and the caller none the wiser.
    assert_ne!(
        status,
        StatusCode::OK,
        "nil project_id must not silently succeed; got body: {body}"
    );
}

#[tokio::test]
async fn invalid_project_id_string_returns_400() {
    // Garbage strings in `project_id` must be rejected at the parse
    // layer with a 400 — never silently overwritten or escalated to
    // a 500 from the verb.
    let (status, body) = post_json(
        "/tools/project.list",
        json!({ "project_id": "not-a-uuid-at-all" }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "garbage project_id must surface as 400; got body: {body}"
    );
    assert_eq!(body["error"], "bad args");
    assert!(
        body["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("project_id"),
        "detail must mention project_id; got: {body}"
    );
}

#[tokio::test]
async fn response_content_type_is_json() {
    let req = Request::builder()
        .method(Method::POST)
        .uri("/tools/project.list")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let res = router().oneshot(req).await.unwrap();
    let content_type = res
        .headers()
        .get(header::CONTENT_TYPE)
        .expect("response must carry a content-type header")
        .to_str()
        .unwrap();
    assert!(
        content_type.starts_with("application/json"),
        "expected JSON content-type, got: {content_type}"
    );
}
