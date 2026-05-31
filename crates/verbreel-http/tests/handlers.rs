//! Handler-level unit tests — call the handlers directly, no router.
//!
//! The whitelist contract these tests used to pin is gone (PR2 deletes
//! `SUPPORTED_VERBS` / `is_supported`). What remains here is the flat
//! liveness probe; the `/v1` discovery surface and the dispatch surface
//! are exercised through the router in `tests/router.rs`, which is the
//! realistic call path.

#[tokio::test]
async fn healthz_returns_status_ok_payload() {
    use axum::Json;
    use serde_json::Value;

    let Json(body): Json<Value> = verbreel_http::handlers::healthz().await;
    assert_eq!(body["status"].as_str(), Some("ok"));
}
