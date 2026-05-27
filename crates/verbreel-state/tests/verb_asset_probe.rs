//! Tests for `asset.probe` (§3.3) — eighty-fifth production verb.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::asset_probe::compute_patch;
use verbreel_state::{
    AssetProbeArgs, AssetProbeData, AssetProbeError, AssetProbeVerb, MutateOutcome, Project, Verb,
    VerbError, VerbRegistry, default_fixtures, default_registry, validate_reconstructors,
};

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const A_PATH: &str = "/does/not/exist.mp4";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture → Project")
}

fn fixture_project_id() -> verbreel_types::ProjectId {
    empty_project().id
}

fn args_default() -> AssetProbeArgs {
    AssetProbeArgs {
        project_id: fixture_project_id(),
        path: A_PATH.to_string(),
    }
}

// --- args deserialization ---------------------------------------------------

#[test]
fn args_deserialize_ok() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "path": A_PATH,
    });
    let typed: AssetProbeArgs = serde_json::from_value(raw).expect("well-formed args parse");
    assert_eq!(typed.project_id.to_string(), FIXTURE_PROJECT_ID);
    assert_eq!(typed.path, A_PATH);
}

#[test]
fn args_missing_project_id_fails_through_verb() {
    let prior = empty_project();
    let verb = AssetProbeVerb;

    let err = verb
        .compute_patch(&prior, &json!({ "path": A_PATH }))
        .expect_err("missing project_id should map to BadArgs");

    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_missing_path_fails_through_verb() {
    let prior = empty_project();
    let verb = AssetProbeVerb;

    let err = verb
        .compute_patch(&prior, &json!({ "project_id": FIXTURE_PROJECT_ID }))
        .expect_err("missing path should map to BadArgs");

    assert!(matches!(err, VerbError::BadArgs { .. }));
}

// --- v1 floor: every well-formed call errors -------------------------------

#[test]
fn path_not_found_on_absurd_path() {
    let prior = empty_project();
    let args = args_default();
    let err = compute_patch(&prior, &args).expect_err("v1 floor: every path misses");
    let AssetProbeError::PathNotFound { path } = err;
    assert_eq!(path, A_PATH);
}

#[test]
fn path_not_found_on_empty_string() {
    let prior = empty_project();
    let args = AssetProbeArgs {
        project_id: fixture_project_id(),
        path: String::new(),
    };
    let err = compute_patch(&prior, &args).expect_err("empty path still misses");
    let AssetProbeError::PathNotFound { path } = err;
    assert_eq!(path, "");
}

#[test]
fn path_not_found_on_relative_path() {
    let prior = empty_project();
    let args = AssetProbeArgs {
        project_id: fixture_project_id(),
        path: "media/clip.mp4".to_string(),
    };
    let err = compute_patch(&prior, &args).expect_err("v1 floor: relative path misses too");
    let AssetProbeError::PathNotFound { path } = err;
    assert_eq!(path, "media/clip.mp4");
}

#[test]
fn error_maps_to_custom_not_bad_args() {
    let prior = empty_project();
    let verb = AssetProbeVerb;

    let err = verb
        .compute_patch(
            &prior,
            &json!({ "project_id": FIXTURE_PROJECT_ID, "path": A_PATH }),
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
    let verb = AssetProbeVerb;

    let err = verb
        .compute_patch(
            &prior,
            &json!({ "project_id": FIXTURE_PROJECT_ID, "path": A_PATH }),
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
fn error_detail_contains_queried_path() {
    let prior = empty_project();
    let weird_path = "/tmp/no-such-file-xyz-789.mp4";
    let args = AssetProbeArgs {
        project_id: fixture_project_id(),
        path: weird_path.to_string(),
    };
    let err = compute_patch(&prior, &args).expect_err("v1 floor errors");
    let msg = err.to_string();
    assert!(
        msg.contains(weird_path),
        "error message `{msg}` should mention path `{weird_path}`",
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
    let verb = AssetProbeVerb;
    let prior = empty_project();
    let args = serde_json::to_value(args_default()).expect("args → Value");
    let data = verb
        .reconstruct(&args, &json!([]), &[], &prior)
        .expect("reconstruct succeeds for well-formed args");
    assert_eq!(data, Value::Null);
}

#[test]
fn reconstruct_rejects_malformed_args() {
    let verb = AssetProbeVerb;
    let prior = empty_project();
    let err = verb
        .reconstruct(&json!({}), &json!([]), &[], &prior)
        .expect_err("malformed args should fail reconstruction");
    let s = err.to_string();
    assert!(
        s.contains("AssetProbeArgs") || s.contains("wrong type"),
        "unexpected reconstruct error: {s}",
    );
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "asset.probe")
        .expect("default_fixtures includes asset.probe");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(AssetProbeVerb))
        .expect("register asset.probe verb");

    let report = validate_reconstructors(&registry, &[fixture])
        .expect("asset.probe reconstruct from fixture");
    assert_eq!(report.verbs_checked, vec!["asset.probe"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn verb_trait_surface_lookup_via_default_registry() {
    let registry = default_registry();
    let verb = registry
        .get("asset.probe")
        .expect("asset.probe registered in default_registry");

    let prior = empty_project();
    let err = verb
        .compute_patch(
            &prior,
            &json!({ "project_id": FIXTURE_PROJECT_ID, "path": A_PATH }),
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
        "asset.probe",
        json!({ "project_id": FIXTURE_PROJECT_ID, "path": A_PATH }),
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
fn data_shape_carries_all_three_fields() {
    let data = AssetProbeData {
        kind: "video".to_string(),
        hash: "53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658".to_string(),
        metadata: json!({"width": 1920, "height": 1080}),
    };
    let v = serde_json::to_value(&data).expect("data serializes");
    let obj = v.as_object().expect("object");
    assert_eq!(obj.len(), 3, "expected 3 keys: kind, hash, metadata");
    assert_eq!(obj.get("kind"), Some(&json!("video")));
    assert_eq!(
        obj.get("hash"),
        Some(&json!(
            "53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658"
        )),
    );
    assert_eq!(
        obj.get("metadata"),
        Some(&json!({"width": 1920, "height": 1080})),
    );
}

#[test]
fn data_shape_round_trips_through_serde() {
    let original = AssetProbeData {
        kind: "audio".to_string(),
        hash: "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef".to_string(),
        metadata: json!({"channels": 2, "sample_rate_hz": 48_000}),
    };
    let v = serde_json::to_value(&original).expect("serialize");
    let back: AssetProbeData = serde_json::from_value(v).expect("deserialize");
    assert_eq!(original, back);
}

/// v1 floor happy-path deferral marker.
///
/// In v1 no file I/O happens in `compute_patch` (the pure-function
/// contract forbids it), so `asset.probe` cannot read the supplied
/// path and every well-formed call returns `E_ASSET_PATH_NOT_FOUND`.
/// The happy path — opening the file, computing SHA-256, deriving
/// `kind` from magic bytes / extension, filling `metadata` with
/// per-kind probe output, returning an `AssetProbeData` envelope —
/// lights up when the `VerbContext` / storage facade plumbs file I/O
/// into `compute_patch`. The `E_ASSET_UNREADABLE` and
/// `E_ASSET_UNSUPPORTED_KIND` error codes declared in §3.3 are
/// likewise unreachable in v1 (every call errors at
/// `E_ASSET_PATH_NOT_FOUND` first, before any read is attempted).
/// This test is intentionally a no-op so the deferral is named in
/// the test surface rather than in a hidden TODO.
#[test]
fn happy_path_unreachable_in_v1_floor() {
    // No assertions: this test exists to document the deferral.
}
