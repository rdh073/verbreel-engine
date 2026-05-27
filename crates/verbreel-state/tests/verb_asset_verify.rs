//! Tests for `asset.verify` (§3.7) — eighty-eighth production verb.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::asset_verify::{compute_patch, data_envelope_from_args};
use verbreel_state::{
    AssetVerifyArgs, AssetVerifyData, AssetVerifyMode, AssetVerifyVerb, Project, Verb, VerbError,
    VerbRegistry, default_fixtures, default_registry, validate_reconstructors,
};

#[cfg(feature = "native")]
use verbreel_state::{MutateOutcome, ProjectStore};

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture → Project")
}

fn fixture_project_id() -> verbreel_types::ProjectId {
    empty_project().id
}

fn args() -> AssetVerifyArgs {
    AssetVerifyArgs {
        project_id: fixture_project_id(),
        strict: None,
    }
}

fn args_strict(strict: bool) -> AssetVerifyArgs {
    AssetVerifyArgs {
        project_id: fixture_project_id(),
        strict: Some(strict),
    }
}

fn project_with_n_assets(n: usize) -> Project {
    let mut prior = empty_project();
    for i in 0..n {
        // Synthetic asset records — content is opaque to asset.verify
        // (the v1 floor only reads `prior.assets.len()`).
        let asset = json!({
            "id": format!("01900000-0000-7000-8000-000000a0{:04x}", i),
            "kind": "video",
            "hash": "36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da",
            "path": "assets/36/36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da.mp4",
            "original_filename": format!("asset-{i}.mp4"),
            "imported_at": "2026-05-24T00:00:00Z",
            "metadata": {
                "duration_tk": 240_000,
                "width": 1920,
                "height": 1080,
                "fps_num": 30,
                "fps_den": 1,
                "video_codec": "h264",
                "container": "mp4",
                "fingerprint": {
                    "mtime_ms": 1_700_000_000_000_i64,
                    "size_bytes": 1024,
                }
            }
        });
        prior
            .assets
            .push(serde_json::from_value(asset).expect("synthetic asset parses"));
    }
    prior
}

#[test]
fn args_deserialize_ok() {
    let raw = json!({ "project_id": FIXTURE_PROJECT_ID, "strict": true });
    let typed: AssetVerifyArgs = serde_json::from_value(raw).expect("happy-path args deserialize");
    assert_eq!(typed.project_id.to_string(), FIXTURE_PROJECT_ID);
    assert_eq!(typed.strict, Some(true));
}

#[test]
fn args_strict_omitted_deserializes_to_none() {
    let raw = json!({ "project_id": FIXTURE_PROJECT_ID });
    let typed: AssetVerifyArgs = serde_json::from_value(raw).expect("strict is optional");
    assert!(typed.strict.is_none());
}

#[test]
fn args_non_bool_strict_fails_through_verb() {
    let prior = empty_project();
    let verb = AssetVerifyVerb;

    let err = verb
        .compute_patch(
            &prior,
            &json!({ "project_id": FIXTURE_PROJECT_ID, "strict": "yes" }),
        )
        .expect_err("non-bool strict should map to BadArgs");

    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn empty_project_yields_zero_checked_and_empty_unverified() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    assert_eq!(data.checked_count, 0);
    assert!(data.unverified_asset_ids.is_empty());
}

#[test]
fn one_asset_yields_checked_count_one() {
    let prior = project_with_n_assets(1);
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    assert_eq!(data.checked_count, 1);
    assert!(data.unverified_asset_ids.is_empty());
}

#[test]
fn three_assets_yields_checked_count_three() {
    let prior = project_with_n_assets(3);
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    assert_eq!(data.checked_count, 3);
    assert!(data.unverified_asset_ids.is_empty());
}

#[test]
fn default_mode_is_fast() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    assert_eq!(data.mode, AssetVerifyMode::Fast);
}

#[test]
fn strict_false_is_fast_mode() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args_strict(false)).expect("happy path");
    assert_eq!(data.mode, AssetVerifyMode::Fast);
}

#[test]
fn strict_true_is_strict_mode() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args_strict(true)).expect("happy path");
    assert_eq!(data.mode, AssetVerifyMode::Strict);
}

#[test]
fn mode_fast_serializes_lowercase() {
    let value = serde_json::to_value(AssetVerifyMode::Fast).expect("mode → Value");
    assert_eq!(value, json!("fast"));
}

#[test]
fn mode_strict_serializes_lowercase() {
    let value = serde_json::to_value(AssetVerifyMode::Strict).expect("mode → Value");
    assert_eq!(value, json!("strict"));
}

#[test]
fn data_envelope_has_exactly_three_keys() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    let value: Value = serde_json::to_value(&data).expect("envelope → Value");
    let obj = value.as_object().expect("envelope is a JSON object");

    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();
    let mut expected = vec!["checked_count", "mode", "unverified_asset_ids"];
    expected.sort_unstable();
    assert_eq!(keys, expected);
    assert_eq!(obj.keys().count(), 3);
}

#[test]
fn patch_is_empty() {
    let prior = empty_project();
    let (patch, _, _) = compute_patch(&prior, &args()).expect("happy path");
    assert_eq!(patch, json!([]));
}

#[test]
fn warnings_are_empty() {
    let prior = empty_project();
    let (_, warnings, _) = compute_patch(&prior, &args()).expect("happy path");
    assert!(warnings.is_empty());
}

#[test]
fn reconstruct_byte_identical_to_compute_patch() {
    let prior = project_with_n_assets(2);
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");

    let envelope = data_envelope_from_args(&args(), &prior).expect("envelope rebuilds");

    let lhs = serde_json::to_vec(&data).expect("forward data serializes");
    let rhs = serde_json::to_vec(&envelope).expect("reconstructed envelope serializes");
    assert_eq!(lhs, rhs);
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "asset.verify")
        .expect("default_fixtures includes asset.verify");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(AssetVerifyVerb))
        .expect("register asset.verify verb");

    let report = validate_reconstructors(&registry, &[fixture])
        .expect("asset.verify reconstruct from fixture");
    assert_eq!(report.verbs_checked, vec!["asset.verify"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn verb_trait_surface_lookup_via_default_registry() {
    let registry = default_registry();
    let verb = registry
        .get("asset.verify")
        .expect("asset.verify registered in default_registry");

    let prior = empty_project();
    let (patch, data, warnings) = verb
        .compute_patch(
            &prior,
            &json!({ "project_id": FIXTURE_PROJECT_ID, "strict": true }),
        )
        .expect("default_registry invocation succeeds");

    assert!(patch.is_empty());
    assert!(warnings.is_empty());
    let typed: AssetVerifyData =
        serde_json::from_value(data).expect("envelope deserializes to AssetVerifyData");
    assert_eq!(typed.checked_count, 0);
    assert!(typed.unverified_asset_ids.is_empty());
    assert_eq!(typed.mode, AssetVerifyMode::Strict);
}

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        empty_project(),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry succeeds");

    let outcome = store
        .mutate_via_verb(
            "asset.verify",
            json!({"project_id": FIXTURE_PROJECT_ID}),
            None,
        )
        .expect("asset.verify should route");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("expected Applied outcome from asset.verify");
    };
    assert!(warnings.is_empty());

    let data: AssetVerifyData =
        serde_json::from_value(data).expect("asset.verify data deserializes");
    assert_eq!(data.checked_count, 0);
    assert!(data.unverified_asset_ids.is_empty());
    assert_eq!(data.mode, AssetVerifyMode::Fast);
}
