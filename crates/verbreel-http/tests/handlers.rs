//! Handler-level shape tests — exercise the `SUPPORTED_VERBS`
//! whitelist contract directly. The router-level coverage in
//! `tests/router.rs` proves the wiring; these tests pin the
//! invariants that hold regardless of how callers reach the
//! handlers.

use verbreel_http::handlers::{SUPPORTED_VERBS, is_supported};

#[test]
fn supported_verbs_contains_project_list() {
    assert!(
        SUPPORTED_VERBS.contains(&"project.list"),
        "v1 floor must whitelist project.list, got: {SUPPORTED_VERBS:?}"
    );
}

#[test]
fn supported_verbs_holds_exactly_one_entry_at_v1_floor() {
    // The slice contract: exactly one verb at v1 floor. Future slices
    // append additively — when that happens, update this number and the
    // related router-level test in lockstep.
    assert_eq!(
        SUPPORTED_VERBS.len(),
        1,
        "expected exactly one whitelisted verb at v1 floor, got: {SUPPORTED_VERBS:?}"
    );
}

#[test]
fn is_supported_accepts_project_list() {
    assert!(is_supported("project.list"));
}

#[test]
fn is_supported_rejects_unknown_verb() {
    assert!(!is_supported("totally.fake.verb"));
}

#[test]
fn is_supported_rejects_registered_but_non_whitelisted_verb() {
    // `project.set_metadata` IS in default_registry but NOT in
    // SUPPORTED_VERBS — catches accidental "default-all" regressions.
    assert!(!is_supported("project.set_metadata"));
}

#[test]
fn is_supported_rejects_empty_string() {
    assert!(!is_supported(""));
}

#[tokio::test]
async fn healthz_returns_status_ok_payload() {
    use axum::Json;
    use serde_json::Value;

    let Json(body): Json<Value> = verbreel_http::handlers::healthz().await;
    assert_eq!(body["status"].as_str(), Some("ok"));
}

#[tokio::test]
async fn list_tools_advertises_every_whitelisted_verb() {
    use axum::Json;
    use serde_json::Value;

    let Json(body): Json<Value> = verbreel_http::handlers::list_tools().await;
    let tools = body["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    for verb in SUPPORTED_VERBS {
        assert!(
            names.contains(verb),
            "list_tools must advertise '{verb}', got names: {names:?}"
        );
    }
    assert_eq!(
        tools.len(),
        SUPPORTED_VERBS.len(),
        "list_tools must reflect SUPPORTED_VERBS exactly: {names:?} vs {SUPPORTED_VERBS:?}"
    );
}

#[tokio::test]
async fn list_tools_output_is_sorted_by_name() {
    use axum::Json;
    use serde_json::Value;

    let Json(body): Json<Value> = verbreel_http::handlers::list_tools().await;
    let tools = body["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    let mut expected = names.clone();
    expected.sort_unstable();
    assert_eq!(
        names, expected,
        "list_tools must emit verbs in sorted order so the response is deterministic"
    );
}
