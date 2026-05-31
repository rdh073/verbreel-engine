//! Router integration tests — drive [`verbreel_http::router`] /
//! [`verbreel_http::router_with_state`] through
//! `tower::ServiceExt::oneshot`. No real TCP bind; every request goes
//! through the in-process service stack the same way axum dispatches.
//!
//! Surface under test (PR2): `POST /v1/{noun}/{verb}` dispatch over the
//! shared `Engine`, `GET /v1` discovery, the §0.1 envelope response
//! shape, and the held-engine open→mutate path.

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;
use verbreel_http::{AppState, router, router_with_state};

/// Send a JSON `POST` through the **default** router and return
/// `(status, body)`. Used for stateless project-less reads where no held
/// project context matters.
async fn post_json(uri: &str, body: Value) -> (StatusCode, Value) {
    post_json_with_router(router(), uri, body).await
}

/// Send a JSON `POST` through an explicit `router` and return
/// `(status, body)`. Lets a test pin a hermetic engine home.
async fn post_json_with_router(router: Router, uri: &str, body: Value) -> (StatusCode, Value) {
    let req = Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let res = router.oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    (status, value)
}

/// Send a `GET` through the **default** router and return
/// `(status, body)`.
async fn get_json(uri: &str) -> (StatusCode, Value) {
    get_json_with_router(router(), uri).await
}

/// Send a `GET` through an explicit `router` and return `(status, body)`.
async fn get_json_with_router(router: Router, uri: &str) -> (StatusCode, Value) {
    let req = Request::builder()
        .method(Method::GET)
        .uri(uri)
        .body(Body::empty())
        .unwrap();
    let res = router.oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    (status, value)
}

// --- liveness ------------------------------------------------------

#[tokio::test]
async fn healthz_returns_200_with_status_ok() {
    let (status, body) = get_json("/healthz").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({ "status": "ok" }));
}

// --- GET /v1 discovery --------------------------------------------

#[tokio::test]
async fn get_v1_lists_all_verb_ids() {
    use verbreel_state::Engine;

    let (status, body) = get_json("/v1").await;
    assert_eq!(status, StatusCode::OK);
    let verbs = body["verbs"]
        .as_array()
        .expect("response must carry a `verbs` array");
    let names: Vec<&str> = verbs.iter().filter_map(Value::as_str).collect();

    // Length matches the engine's id count exactly — no whitelist
    // truncation, no padding.
    let engine = Engine::new(std::env::temp_dir());
    assert_eq!(
        names.len(),
        engine.verb_ids().len(),
        "discovery must list every engine verb id, got: {names:?}"
    );

    // Sample across nouns plus the five meta ids. Meta verbs are
    // advertised in their **routable** dotted form `meta.<id>` (not the
    // bare flat id the engine registers) so a discovery client splitting
    // an id on `.` can reconstruct `/v1/meta/<verb>`.
    for expected in [
        "project.list",
        "project.create",
        "clip.add",
        "asset.import",
        "render.start",
        "meta.help",
        "meta.schema",
        "meta.describe",
        "meta.validate_command",
        "meta.list_capabilities",
    ] {
        assert!(
            names.contains(&expected),
            "discovery must contain `{expected}`, got: {names:?}"
        );
    }

    // The bare flat ids must NOT leak — every advertised id is routable
    // as `/v1/<noun>/<verb>`, and a bare `help` has no noun segment.
    for leaked in [
        "help",
        "schema",
        "describe",
        "validate_command",
        "list_capabilities",
    ] {
        assert!(
            !names.contains(&leaked),
            "discovery must advertise `meta.{leaked}`, not the bare flat id `{leaked}`: {names:?}"
        );
    }
}

/// Closes the discovery→dispatch loop: every meta id `GET /v1` advertises
/// in dotted form must be reachable at its `POST /v1/meta/<verb>` route.
/// `meta.help` advertised + `/v1/meta/help` dispatching proves a
/// discovery client can split the id on `.` and POST it back.
#[tokio::test]
async fn discovered_meta_id_is_routable() {
    let (_status, body) = get_json("/v1").await;
    let names: Vec<String> = body["verbs"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    assert!(
        names.iter().any(|n| n == "meta.help"),
        "discovery must advertise meta.help, got: {names:?}"
    );

    // Split the advertised id on `.` exactly as a discovery-driven client
    // would, then POST the reconstructed route.
    let (noun, verb) = "meta.help".split_once('.').unwrap();
    let (status, post_body) = post_json(&format!("/v1/{noun}/{verb}"), json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        post_body["ok"].as_bool(),
        Some(true),
        "POST /v1/meta/help (rebuilt from the advertised id) must dispatch: {post_body}"
    );
}

#[tokio::test]
async fn get_v1_discovery_is_sorted() {
    let (_status, body) = get_json("/v1").await;
    let names: Vec<&str> = body["verbs"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let mut expected = names.clone();
    expected.sort_unstable();
    assert_eq!(
        names, expected,
        "discovery must emit verb ids in sorted order (FROZEN verb_ids contract)"
    );
}

// --- POST dispatch: project-less read ------------------------------

#[tokio::test]
async fn post_project_list_returns_200_with_empty_array() {
    let (status, body) = post_json("/v1/project/list", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"].as_bool(), Some(true), "got body: {body}");
    let projects = body["data"]["projects"]
        .as_array()
        .expect("envelope must expose `data.projects` as a JSON array");
    assert!(
        projects.is_empty(),
        "v1 floor must return an empty projects list, got: {body}"
    );
}

#[tokio::test]
async fn post_project_list_returns_section_0_1_envelope() {
    let (_status, body) = post_json("/v1/project/list", json!({})).await;
    // The §0.1 Ok shape: ok + data + patch + warnings + event_id, all
    // present as top-level keys reached via Envelope::to_json.
    assert_eq!(body["ok"].as_bool(), Some(true));
    assert!(body.get("data").is_some(), "envelope must carry `data`");
    assert!(body.get("patch").is_some(), "envelope must carry `patch`");
    assert!(
        body.get("warnings").is_some(),
        "envelope must carry `warnings`"
    );
    assert!(
        body.get("event_id").is_some(),
        "envelope must carry `event_id`"
    );
    // No bespoke top-level `projects` — that would mean the envelope was
    // unwrapped / leaked instead of nested under `data`.
    assert!(
        body.get("projects").is_none(),
        "response must NOT expose `projects` at the top level: {body}"
    );
}

// --- POST dispatch: unknown verb comes from the engine -------------

#[tokio::test]
async fn post_unknown_verb_returns_e_unknown_verb_envelope() {
    // Engine returns E_UNKNOWN_VERB for the reconstructed key
    // `totally.fake`. Status is 200 (uniform-200 mapping); the failure is
    // carried in the body.
    let (status, body) = post_json("/v1/totally/fake", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"].as_bool(), Some(false), "got body: {body}");
    assert_eq!(body["code"].as_str(), Some("E_UNKNOWN_VERB"));
}

#[tokio::test]
async fn unknown_verb_404_comes_from_engine_not_whitelist() {
    // `clip.nonsense` is a well-formed two-segment path but not a
    // registered verb — the engine, not a hardcoded list, rejects it.
    let (status, body) = post_json("/v1/clip/nonsense", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"].as_bool(), Some(false), "got body: {body}");
    assert_eq!(body["code"].as_str(), Some("E_UNKNOWN_VERB"));
}

#[tokio::test]
async fn post_registered_verb_reaches_dispatch() {
    // `project.set_metadata` IS in the registry. With no whitelist it
    // reaches the engine, which rejects it for a project-arg reason
    // (missing/unopened project), NOT with E_UNKNOWN_VERB. The invariant
    // under test: a registered verb is reachable, not blocked.
    let (status, body) = post_json("/v1/project/set_metadata", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"].as_bool(), Some(false), "got body: {body}");
    assert_ne!(
        body["code"].as_str(),
        Some("E_UNKNOWN_VERB"),
        "a registered verb must reach dispatch, not 404 from a whitelist: {body}"
    );
}

// --- flat-meta route translation -----------------------------------

#[tokio::test]
async fn post_meta_help_routes_to_flat_id() {
    // POST /v1/meta/help → flat engine id `help` (NOT key `meta.help`,
    // which would be E_UNKNOWN_VERB). A true `ok` proves the
    // noun=="meta" translation fired.
    let (status, body) = post_json("/v1/meta/help", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"].as_bool(), Some(true), "got body: {body}");
}

#[tokio::test]
async fn post_meta_schema_routes_to_flat_id() {
    // Second flat-meta verb. `schema` needs a `target`; `project` is the
    // minimal valid arg. ok proves the translation holds for >1 verb.
    let (status, body) = post_json("/v1/meta/schema", json!({ "target": "project" })).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"].as_bool(), Some(true), "got body: {body}");
}

// --- held engine: open/create then mutate --------------------------

#[tokio::test]
async fn open_then_mutate_targets_held_project() -> anyhow::Result<()> {
    use tempfile::TempDir;

    let home = TempDir::new()?;
    let at = TempDir::new()?;
    // One router (one held engine) reused across both requests via
    // `clone()` — `oneshot` consumes the router, but cloning shares the
    // same `Arc<Mutex<Engine>>`, so the second request sees the project
    // the first one opened.
    let app = router_with_state(AppState::new(home.path().to_path_buf()));

    // 1. create populates the engine's open-project map.
    let (status, body) = post_json_with_router(
        app.clone(),
        "/v1/project/create",
        json!({
            "name": "held-engine-project",
            "canvas": "64x64",
            "at": at.path(),
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"].as_bool(), Some(true), "create failed: {body}");
    let project_id = body["data"]["project_id"]
        .as_str()
        .expect("create data must carry project_id")
        .to_string();

    // 2. a project-scoped read against the same engine instance succeeds
    //    ONLY because the engine is held across requests — a per-request
    //    engine would have an empty map and return E_PROJECT_NOT_FOUND.
    let (status, body) =
        post_json_with_router(app, "/v1/project/info", json!({ "project_id": project_id })).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["ok"].as_bool(),
        Some(true),
        "project.info must reach the held project: {body}"
    );
    Ok(())
}

// --- project_id is a body arg (§0.12) ------------------------------

#[tokio::test]
async fn caller_supplied_project_id_is_accepted() {
    // project.list is project-less (§0.12); a well-formed v7 project_id
    // body arg must not error the call.
    let pid = uuid::Uuid::now_v7();
    let (status, body) =
        post_json("/v1/project/list", json!({ "project_id": pid.to_string() })).await;
    assert_eq!(status, StatusCode::OK, "got body: {body}");
    assert_eq!(body["ok"].as_bool(), Some(true));
}

#[tokio::test]
async fn invalid_project_id_string_returns_error_envelope() {
    // A garbage project_id is now rejected by the engine's args-schema
    // validator (E_SCHEMA_VIOLATION), not the HTTP parse layer — the
    // HTTP layer passes the body straight through. We target a
    // project-scoped verb so the engine actually parses project_id.
    let (status, body) = post_json(
        "/v1/project/info",
        json!({ "project_id": "not-a-uuid-at-all" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"].as_bool(), Some(false), "got body: {body}");
    assert_eq!(body["code"].as_str(), Some("E_SCHEMA_VIOLATION"));
}

// --- transport-level 400 (pre-dispatch) ----------------------------

#[tokio::test]
async fn post_malformed_json_body_returns_400_with_bad_args() {
    let req = Request::builder()
        .method(Method::POST)
        .uri("/v1/project/list")
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
    // `JsonRejection::MissingJsonContentType`, mapped to the bad-args 400.
    let req = Request::builder()
        .method(Method::POST)
        .uri("/v1/project/list")
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
    // Valid JSON but not a JSON object — rejected before dispatch.
    let (status, body) = post_json("/v1/project/list", json!([1, 2, 3])).await;
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
    // `Value::Null` body coerces to `{}` so callers can omit the body for
    // a no-arg verb. project.list then returns the §0.1 Ok envelope.
    let (status, body) = post_json("/v1/project/list", Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"].as_bool(), Some(true), "got body: {body}");
}

// --- routing edge cases --------------------------------------------

#[tokio::test]
async fn get_on_post_only_route_returns_method_not_allowed() {
    let req = Request::builder()
        .method(Method::GET)
        .uri("/v1/project/list")
        .body(Body::empty())
        .unwrap();
    let res = router().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn unknown_route_returns_404() {
    // A genuinely unrouted path 404s at the router level — distinct from
    // the engine's E_UNKNOWN_VERB (which is a 200 envelope).
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
    // Two identical POSTs return equal `data`. With a held engine and no
    // synthetic-id synthesis at the HTTP layer, this is naturally stable.
    let (_a, body_a) = post_json("/v1/project/list", json!({})).await;
    let (_b, body_b) = post_json("/v1/project/list", json!({})).await;
    assert_eq!(
        body_a["data"], body_b["data"],
        "the §0.1 envelope `data` must be stable across identical requests"
    );
}

#[tokio::test]
async fn response_content_type_is_json() {
    let req = Request::builder()
        .method(Method::POST)
        .uri("/v1/project/list")
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

// --- native render behind /v1/render/start -------------------------

#[tokio::test]
#[cfg(feature = "native-render")]
async fn post_render_start_returns_error_envelope_with_code() -> anyhow::Result<()> {
    use tempfile::TempDir;
    use verbreel_runtime::RenderRuntimeConfig;
    use verbreel_state::{ProjectCreateArgs, project_create};

    let home = TempDir::new()?;
    let at = TempDir::new()?;
    let create_data = project_create(&ProjectCreateArgs {
        name: "http-render-project".to_string(),
        canvas: "64x64".to_string(),
        fps_num: Some(30),
        fps_den: Some(1),
        at: Some(at.path().to_path_buf()),
        activate: false,
        metadata: serde_json::Map::new(),
    })?;
    let app = router_with_state(AppState::with_render_runtime(
        home.path().to_path_buf(),
        RenderRuntimeConfig::new().with_project_root(&create_data.path),
    ));
    let (status, body) = post_json_with_router(
        app,
        "/v1/render/start",
        json!({
            "project_id": create_data.project_id,
            "preset": "definitely-not-a-preset",
            "out_path": "exports/out.mp4",
            "from_tk": 0,
            "to_tk": 8000,
            "overwrite": true,
        }),
    )
    .await;

    // §0.1 error envelope — NOT the old {"detail":…} shape. Code is the
    // structured field; the runtime message embeds it too.
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"].as_bool(), Some(false), "got body: {body}");
    assert_eq!(body["code"].as_str(), Some("E_RENDER_PRESET_UNKNOWN"));
    assert!(
        body["message"]
            .as_str()
            .is_some_and(|m| m.contains("E_RENDER_PRESET_UNKNOWN")),
        "envelope message must surface the runtime code, got: {body}"
    );
    Ok(())
}

/// §0.1 (#455): a successful native render records an event, so the §0.1
/// `Ok` envelope must carry that event's truthful id — never the
/// read-only/dry-run `""` placeholder — and the envelope `event_id` must
/// equal the `data.event_id` the runtime threaded through.
#[tokio::test]
#[cfg(feature = "native-render")]
async fn post_render_start_success_envelope_carries_recorded_event_id() -> anyhow::Result<()> {
    use std::fs;

    use tempfile::TempDir;
    use verbreel_runtime::RenderRuntimeConfig;
    use verbreel_state::{
        AssetImportData, MutateOutcome, ProjectCreateArgs, ProjectStore, default_fixtures,
        default_registry, project_create,
    };

    fn outcome_data(outcome: MutateOutcome) -> Value {
        match outcome {
            MutateOutcome::Applied { data, .. }
            | MutateOutcome::Replayed { data, .. }
            | MutateOutcome::NoOp { data, .. } => data,
        }
    }

    const W: u32 = 64;
    const H: u32 = 64;
    const DURATION_TK: i64 = 16_000;

    let home = TempDir::new()?;
    let at = TempDir::new()?;
    let source_path = at.path().join("source.ppm");
    let mut ppm = format!("P6\n{W} {H}\n255\n").into_bytes();
    for y in 0..H {
        for x in 0..W {
            ppm.push(u8::try_from(x * 255 / W).unwrap_or(u8::MAX));
            ppm.push(u8::try_from(y * 255 / H).unwrap_or(u8::MAX));
            ppm.push(160);
        }
    }
    fs::write(&source_path, ppm)?;

    let create_data = project_create(&ProjectCreateArgs {
        name: "http-render-success".to_string(),
        canvas: format!("{W}x{H}"),
        fps_num: Some(30),
        fps_den: Some(1),
        at: Some(at.path().to_path_buf()),
        activate: false,
        metadata: serde_json::Map::new(),
    })?;

    let registry = default_registry();
    let fixtures = default_fixtures();
    let mut store = ProjectStore::open_with_registry(&create_data.path, &registry, &fixtures)?;
    let import: AssetImportData = serde_json::from_value(outcome_data(store.mutate_via_verb(
        "asset.import",
        json!({
            "project_id": create_data.project_id,
            "paths": [source_path.display().to_string()],
        }),
        None,
    )?))?;
    let asset_id = import.assets[0]["id"]
        .as_str()
        .expect("asset id")
        .to_string();
    store.mutate_via_verb(
        "clip.add",
        json!({
            "project_id": create_data.project_id,
            "asset_id": asset_id,
            "track": create_data.seeded_track_ids.video.to_string(),
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": DURATION_TK,
            "name": "card",
        }),
        None,
    )?;
    store.save()?;
    // Release the exclusive events.jsonl flock before the runtime reopens
    // the project to render.
    drop(store);

    let app = router_with_state(AppState::with_render_runtime(
        home.path().to_path_buf(),
        RenderRuntimeConfig::new().with_project_root(&create_data.path),
    ));
    let (status, body) = post_json_with_router(
        app,
        "/v1/render/start",
        json!({
            "project_id": create_data.project_id,
            "preset": "deterministic",
            "out_path": "exports/out.mp4",
            "from_tk": 0,
            "to_tk": DURATION_TK,
            "deterministic": true,
            "overwrite": true,
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"].as_bool(), Some(true), "got body: {body}");
    let event_id = body["event_id"].as_str().expect("envelope event_id");
    assert!(
        !event_id.is_empty(),
        "recorded render must surface a non-empty event_id, got: {body}"
    );
    assert_eq!(
        body["event_id"], body["data"]["event_id"],
        "envelope event_id must equal data.event_id, got: {body}"
    );
    Ok(())
}
