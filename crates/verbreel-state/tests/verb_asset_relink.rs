//! Tests for `asset.relink` (§3.5) — eighty-sixth production verb.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::asset_relink::compute_patch;
use verbreel_state::{
    AssetMode, AssetRelinkArgs, AssetRelinkData, AssetRelinkError, AssetRelinkFingerprint,
    AssetRelinkVerb, MutateOutcome, Project, Verb, VerbError, VerbRegistry, default_fixtures,
    default_registry, validate_reconstructors,
};

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const A_ASSET_ID: &str = "01900000-0000-7000-8000-00000000aa01";
const A_SOURCE_PATH: &str = "/does/not/exist.mp4";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture → Project")
}

fn fixture_project_id() -> verbreel_types::ProjectId {
    empty_project().id
}

fn args_default() -> AssetRelinkArgs {
    AssetRelinkArgs {
        project_id: fixture_project_id(),
        asset_id: A_ASSET_ID.to_string(),
        source_path: A_SOURCE_PATH.to_string(),
        mode: None,
    }
}

// --- args deserialization ---------------------------------------------------

#[test]
fn args_deserialize_ok() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "asset_id": A_ASSET_ID,
        "source_path": A_SOURCE_PATH,
    });
    let typed: AssetRelinkArgs = serde_json::from_value(raw).expect("well-formed args parse");
    assert_eq!(typed.project_id.to_string(), FIXTURE_PROJECT_ID);
    assert_eq!(typed.asset_id, A_ASSET_ID);
    assert_eq!(typed.source_path, A_SOURCE_PATH);
    assert!(typed.mode.is_none(), "omitted mode → None at args layer");
}

#[test]
fn args_deserialize_mode_copy_ok() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "asset_id": A_ASSET_ID,
        "source_path": A_SOURCE_PATH,
        "mode": "copy",
    });
    let typed: AssetRelinkArgs = serde_json::from_value(raw).expect("mode=copy parses");
    assert_eq!(typed.mode, Some(AssetMode::Copy));
}

#[test]
fn args_deserialize_mode_link_ok() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "asset_id": A_ASSET_ID,
        "source_path": A_SOURCE_PATH,
        "mode": "link",
    });
    let typed: AssetRelinkArgs = serde_json::from_value(raw).expect("mode=link parses");
    assert_eq!(typed.mode, Some(AssetMode::Link));
}

#[test]
fn args_missing_project_id_fails_through_verb() {
    let prior = empty_project();
    let verb = AssetRelinkVerb;

    let err = verb
        .compute_patch(
            &prior,
            &json!({ "asset_id": A_ASSET_ID, "source_path": A_SOURCE_PATH }),
        )
        .expect_err("missing project_id should map to BadArgs");

    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_missing_asset_id_fails_through_verb() {
    let prior = empty_project();
    let verb = AssetRelinkVerb;

    let err = verb
        .compute_patch(
            &prior,
            &json!({ "project_id": FIXTURE_PROJECT_ID, "source_path": A_SOURCE_PATH }),
        )
        .expect_err("missing asset_id should map to BadArgs");

    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_missing_source_path_fails_through_verb() {
    let prior = empty_project();
    let verb = AssetRelinkVerb;

    let err = verb
        .compute_patch(
            &prior,
            &json!({ "project_id": FIXTURE_PROJECT_ID, "asset_id": A_ASSET_ID }),
        )
        .expect_err("missing source_path should map to BadArgs");

    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_invalid_mode_fails_through_verb() {
    let prior = empty_project();
    let verb = AssetRelinkVerb;

    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "asset_id": A_ASSET_ID,
                "source_path": A_SOURCE_PATH,
                "mode": "symlink",
            }),
        )
        .expect_err("non-spec mode value should map to BadArgs");

    assert!(matches!(err, VerbError::BadArgs { .. }));
}

// --- v1 floor: every well-formed call errors -------------------------------

#[test]
fn path_not_found_on_absurd_path() {
    let prior = empty_project();
    let args = args_default();
    let err = compute_patch(&prior, &args).expect_err("v1 floor: every path misses");
    let AssetRelinkError::PathNotFound { source_path } = err;
    assert_eq!(source_path, A_SOURCE_PATH);
}

#[test]
fn path_not_found_on_empty_string() {
    let prior = empty_project();
    let args = AssetRelinkArgs {
        project_id: fixture_project_id(),
        asset_id: A_ASSET_ID.to_string(),
        source_path: String::new(),
        mode: None,
    };
    let err = compute_patch(&prior, &args).expect_err("empty path still misses");
    let AssetRelinkError::PathNotFound { source_path } = err;
    assert_eq!(source_path, "");
}

#[test]
fn path_not_found_on_relative_path() {
    let prior = empty_project();
    let args = AssetRelinkArgs {
        project_id: fixture_project_id(),
        asset_id: A_ASSET_ID.to_string(),
        source_path: "media/clip.mp4".to_string(),
        mode: Some(AssetMode::Link),
    };
    let err = compute_patch(&prior, &args).expect_err("v1 floor: relative path misses too");
    let AssetRelinkError::PathNotFound { source_path } = err;
    assert_eq!(source_path, "media/clip.mp4");
}

#[test]
fn error_maps_to_custom_not_bad_args() {
    let prior = empty_project();
    let verb = AssetRelinkVerb;

    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "asset_id": A_ASSET_ID,
                "source_path": A_SOURCE_PATH,
            }),
        )
        .expect_err("v1 floor: verb always errors");

    // Runtime-state error (path miss on disk) must surface as Custom — not
    // BadArgs. BadArgs is reserved for arg-shape failures
    // (validate_command §1.4 relies on this distinction to avoid
    // mis-reporting well-formed args as invalid).
    assert!(
        matches!(err, VerbError::Custom(_)),
        "expected VerbError::Custom, got {err:?}",
    );
}

#[test]
fn error_detail_contains_e_asset_path_not_found_code() {
    let prior = empty_project();
    let verb = AssetRelinkVerb;

    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "asset_id": A_ASSET_ID,
                "source_path": A_SOURCE_PATH,
            }),
        )
        .expect_err("v1 floor: verb always errors");

    let VerbError::Custom(detail) = err else {
        panic!("expected Custom, got {err:?}");
    };
    assert!(
        detail.contains("E_ASSET_PATH_NOT_FOUND"),
        "detail `{detail}` should mention E_ASSET_PATH_NOT_FOUND",
    );
}

#[test]
fn error_detail_contains_queried_source_path() {
    let prior = empty_project();
    let weird_path = "/tmp/no-such-relink-xyz-789.mp4";
    let args = AssetRelinkArgs {
        project_id: fixture_project_id(),
        asset_id: A_ASSET_ID.to_string(),
        source_path: weird_path.to_string(),
        mode: None,
    };
    let err = compute_patch(&prior, &args).expect_err("v1 floor errors");
    let msg = err.to_string();
    assert!(
        msg.contains(weird_path),
        "error message `{msg}` should mention source_path `{weird_path}`",
    );
}

#[test]
fn verb_is_project_agnostic_on_error_path() {
    let prior_a = empty_project();
    let mut prior_b = empty_project();
    prior_b.name = "different-name".to_string();
    prior_b.duration_tk = verbreel_types::Tick::new(987_654);

    let args = args_default();
    let err_a = compute_patch(&prior_a, &args).expect_err("a errors");
    let err_b = compute_patch(&prior_b, &args).expect_err("b errors");

    assert_eq!(err_a, err_b);
}

// --- reconstructor / fixture --------------------------------------------

#[test]
fn reconstruct_returns_null_for_args_deserialize_round_trip() {
    let verb = AssetRelinkVerb;
    let prior = empty_project();
    let args = serde_json::to_value(args_default()).expect("args → Value");
    let data = verb
        .reconstruct(&args, &json!([]), &[], &prior)
        .expect("reconstruct succeeds for well-formed args");
    assert_eq!(data, Value::Null);
}

#[test]
fn reconstruct_rejects_malformed_args() {
    let verb = AssetRelinkVerb;
    let prior = empty_project();
    let err = verb
        .reconstruct(&json!({}), &json!([]), &[], &prior)
        .expect_err("malformed args should fail reconstruction");
    let s = err.to_string();
    assert!(
        s.contains("AssetRelinkArgs") || s.contains("wrong type"),
        "unexpected reconstruct error: {s}",
    );
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "asset.relink")
        .expect("default_fixtures includes asset.relink");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(AssetRelinkVerb))
        .expect("register asset.relink verb");

    let report = validate_reconstructors(&registry, &[fixture])
        .expect("asset.relink reconstruct from fixture");
    assert_eq!(report.verbs_checked, vec!["asset.relink"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn verb_trait_surface_lookup_via_default_registry() {
    let registry = default_registry();
    let verb = registry
        .get("asset.relink")
        .expect("asset.relink registered in default_registry");

    let prior = empty_project();
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "asset_id": A_ASSET_ID,
                "source_path": A_SOURCE_PATH,
            }),
        )
        .expect_err("v1 floor: always errors");

    let VerbError::Custom(detail) = err else {
        panic!("expected Custom, got {err:?}");
    };
    assert!(detail.contains("E_ASSET_PATH_NOT_FOUND"));
}

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb_and_errors() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        empty_project(),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry succeeds");

    let outcome = store.mutate_via_verb(
        "asset.relink",
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "asset_id": A_ASSET_ID,
            "source_path": A_SOURCE_PATH,
        }),
        None,
    );

    match outcome {
        Err(_) => {}
        Ok(MutateOutcome::Applied { .. }) => {
            panic!("expected mutate_via_verb to error in v1 floor, got Applied")
        }
        Ok(other) => panic!("expected Err for v1 floor, got Ok({other:?})"),
    }
}

// --- data shape sanity -----------------------------------------------------

#[test]
fn data_shape_carries_all_six_fields() {
    let data = AssetRelinkData {
        asset_id: A_ASSET_ID.to_string(),
        old_resolved_path:
            "assets/53/53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.mp4"
                .to_string(),
        new_resolved_path:
            "assets/53/53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.mp4"
                .to_string(),
        new_fingerprint: AssetRelinkFingerprint {
            mtime_ms: 1_700_000_000_000_i64,
            size_bytes: 1024,
        },
        mode_used: AssetMode::Copy,
        fallback_reason: Some("cross_filesystem".to_string()),
    };
    let v = serde_json::to_value(&data).expect("data serializes");
    let obj = v.as_object().expect("object");
    // Six keys with fallback_reason present.
    assert_eq!(obj.len(), 6, "expected 6 keys when fallback_reason is Some");
    for key in [
        "asset_id",
        "old_resolved_path",
        "new_resolved_path",
        "new_fingerprint",
        "mode_used",
        "fallback_reason",
    ] {
        assert!(obj.contains_key(key), "data must serialize `{key}`");
    }
    assert_eq!(obj.get("mode_used"), Some(&json!("copy")));
}

#[test]
fn data_shape_skips_fallback_reason_when_none() {
    let data = AssetRelinkData {
        asset_id: A_ASSET_ID.to_string(),
        old_resolved_path:
            "assets/53/53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.mp4"
                .to_string(),
        new_resolved_path:
            "assets/53/53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.mp4"
                .to_string(),
        new_fingerprint: AssetRelinkFingerprint {
            mtime_ms: 1_700_000_000_000_i64,
            size_bytes: 1024,
        },
        mode_used: AssetMode::Link,
        fallback_reason: None,
    };
    let v = serde_json::to_value(&data).expect("data serializes");
    let obj = v.as_object().expect("object");
    assert_eq!(
        obj.len(),
        5,
        "fallback_reason: None must skip serialization (matches spec `?`)"
    );
    assert!(!obj.contains_key("fallback_reason"));
    assert_eq!(obj.get("mode_used"), Some(&json!("link")));
}

#[test]
fn data_shape_round_trips_through_serde() {
    let original = AssetRelinkData {
        asset_id: A_ASSET_ID.to_string(),
        old_resolved_path:
            "assets/53/53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.mp4"
                .to_string(),
        new_resolved_path:
            "assets/de/deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef.mp4"
                .to_string(),
        new_fingerprint: AssetRelinkFingerprint {
            mtime_ms: 1_700_000_000_000_i64,
            size_bytes: 4096,
        },
        mode_used: AssetMode::Link,
        fallback_reason: None,
    };
    let v = serde_json::to_value(&original).expect("serialize");
    let back: AssetRelinkData = serde_json::from_value(v).expect("deserialize");
    assert_eq!(original, back);
}

#[test]
fn asset_mode_round_trips_through_serde() {
    for mode in [AssetMode::Copy, AssetMode::Link] {
        let v = serde_json::to_value(mode).expect("mode serializes");
        let back: AssetMode = serde_json::from_value(v).expect("mode deserializes");
        assert_eq!(mode, back);
    }
}

/// v1 floor happy-path deferral marker.
///
/// In v1 no file I/O happens in `compute_patch` (the pure-function
/// contract forbids it), so `asset.relink` cannot read the supplied
/// `source_path` and every well-formed call returns
/// `E_ASSET_PATH_NOT_FOUND`. The happy path — opening the source,
/// computing SHA-256, comparing against the recorded `Asset.hash`,
/// stat-ing for the new fingerprint, optionally hard-linking into the
/// content-addressed store, emitting a patch that rewrites
/// `Asset.path` and `Asset.metadata.fingerprint`, and returning an
/// `AssetRelinkData` envelope — lights up when the `VerbContext` /
/// storage facade plumbs file I/O into `compute_patch`. The other
/// four declared error codes (`E_ASSET_NOT_FOUND`,
/// `E_ASSET_HASH_MISMATCH`, `E_ASSET_UNREADABLE`, `E_IO`) and the
/// `W_ASSET_MODE_FALLBACK` warning are likewise unreachable in v1
/// (every call errors at `E_ASSET_PATH_NOT_FOUND` first, before any
/// read is attempted). This test is intentionally a no-op so the
/// deferral is named in the test surface rather than in a hidden
/// TODO.
#[test]
fn happy_path_unreachable_in_v1_floor() {
    // No assertions: this test exists to document the deferral.
}
