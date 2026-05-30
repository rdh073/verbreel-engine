//! Tests for `asset.import` (§3.1) — eighty-fourth production verb.
//!
//! The pure verb surface remains a v1 floor (no root filesystem
//! context), while the native kernel route wires non-empty imports
//! through CAS. These tests lock both surfaces, plus reconstructor
//! behavior through default fixtures.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::asset_import::{compute_patch, data_envelope_from_args, import};
use verbreel_state::{
    ASSET_IMPORT_SCHEMA_VIOLATION_HINT, AssetImportArgs, AssetImportData, AssetImportError,
    AssetImportVerb, ImportMode, PATHS_MAX_BATCH, Project, Verb, VerbError, VerbRegistry,
    default_fixtures, default_registry, validate_reconstructors,
};

#[cfg(feature = "native")]
use verbreel_state::MutateOutcome;
#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture → Project")
}

fn fixture_project_id() -> verbreel_types::ProjectId {
    empty_project().id
}

fn empty_paths_args() -> AssetImportArgs {
    AssetImportArgs {
        project_id: fixture_project_id(),
        paths: Vec::new(),
        mode: None,
        soft: None,
    }
}

fn single_path_args(path: &str) -> AssetImportArgs {
    AssetImportArgs {
        project_id: fixture_project_id(),
        paths: vec![path.to_string()],
        mode: None,
        soft: None,
    }
}

// --- args deserialization ---------------------------------------------------

#[test]
fn args_deserialize_happy_path() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "paths": ["/tmp/a.mp4", "/tmp/b.mp4"],
        "mode": "copy",
        "soft": true,
    });
    let typed: AssetImportArgs = serde_json::from_value(raw).expect("well-formed args deserialize");
    assert_eq!(typed.project_id.to_string(), FIXTURE_PROJECT_ID);
    assert_eq!(typed.paths.len(), 2);
    assert_eq!(typed.mode, Some(ImportMode::Copy));
    assert_eq!(typed.soft, Some(true));
}

#[test]
fn args_reject_unknown_fields() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "paths": [],
        "totally_made_up_field": true,
    });
    let err = serde_json::from_value::<AssetImportArgs>(raw)
        .expect_err("deny_unknown_fields must reject extras");
    assert!(
        err.to_string().contains("totally_made_up_field"),
        "error should name the offending field, got: {err}"
    );
}

#[test]
fn args_accept_missing_optional_mode_and_soft() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "paths": ["/tmp/a.mp4"],
    });
    let typed: AssetImportArgs =
        serde_json::from_value(raw).expect("mode/soft optional and may be omitted");
    assert!(typed.mode.is_none());
    assert!(typed.soft.is_none());
}

#[test]
fn args_mode_link_round_trips() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "paths": [],
        "mode": "link",
    });
    let typed: AssetImportArgs =
        serde_json::from_value(raw).expect("mode `link` should deserialize");
    assert_eq!(typed.mode, Some(ImportMode::Link));
    let back = serde_json::to_value(&typed).expect("serialize");
    assert_eq!(back.get("mode").and_then(Value::as_str), Some("link"));
}

// --- empty-paths → spec-documented no-op ------------------------------------

#[test]
fn empty_paths_returns_empty_assets() {
    let data = import(&empty_paths_args()).expect("empty paths is a successful no-op (§3.1)");
    assert!(data.assets.is_empty());
}

#[test]
fn empty_paths_returns_empty_modes_used() {
    let data = import(&empty_paths_args()).expect("empty paths is a successful no-op");
    assert!(data.modes_used.is_empty());
}

#[test]
fn empty_paths_returns_empty_missing_paths() {
    let data = import(&empty_paths_args()).expect("empty paths is a successful no-op");
    assert!(data.missing_paths.is_empty());
}

#[test]
fn empty_paths_returns_empty_skipped_input_indices() {
    let data = import(&empty_paths_args()).expect("empty paths is a successful no-op");
    assert!(data.skipped_input_indices.is_empty());
}

// --- >1000 paths → E_SCHEMA_VIOLATION ---------------------------------------

#[test]
fn paths_over_cap_fires_schema_violation() {
    let mut args = empty_paths_args();
    args.paths = (0..=PATHS_MAX_BATCH).map(|i| format!("/p{i}")).collect();
    assert_eq!(args.paths.len(), PATHS_MAX_BATCH + 1);
    let err = import(&args).expect_err("`maxItems: 1000` cap should fire");
    assert!(
        matches!(err, AssetImportError::SchemaViolation { .. }),
        "expected SchemaViolation, got {err:?}"
    );
}

#[test]
fn schema_violation_names_paths_field_and_hint() {
    let mut args = empty_paths_args();
    args.paths = vec!["/x".to_string(); PATHS_MAX_BATCH + 1];
    let err = import(&args).expect_err("cap fires");
    match err {
        AssetImportError::SchemaViolation { field, hint, .. } => {
            assert_eq!(field, "paths");
            assert_eq!(hint, ASSET_IMPORT_SCHEMA_VIOLATION_HINT);
        }
        other => panic!("expected SchemaViolation, got {other:?}"),
    }
}

#[test]
fn schema_violation_carries_actual_and_max_counts() {
    let mut args = empty_paths_args();
    args.paths = vec!["/x".to_string(); PATHS_MAX_BATCH + 1];
    let err = import(&args).expect_err("cap fires");
    match err {
        AssetImportError::SchemaViolation {
            actual_count,
            max_count,
            ..
        } => {
            assert_eq!(actual_count, PATHS_MAX_BATCH + 1);
            assert_eq!(max_count, PATHS_MAX_BATCH);
        }
        other => panic!("expected SchemaViolation, got {other:?}"),
    }
}

#[test]
fn paths_exactly_at_cap_is_not_a_schema_violation() {
    // At the cap, the cap itself is satisfied (>, not >=). The next
    // rejection step is the v1 floor's AssetPathNotFound.
    let mut args = empty_paths_args();
    args.paths = vec!["/x".to_string(); PATHS_MAX_BATCH];
    let err = import(&args).expect_err("v1 floor: non-empty → AssetPathNotFound");
    assert!(
        matches!(err, AssetImportError::AssetPathNotFound { .. }),
        "expected AssetPathNotFound at the cap, got {err:?}"
    );
}

// --- >=1 path → v1 floor E_ASSET_PATH_NOT_FOUND ----------------------------

#[test]
fn single_path_returns_asset_path_not_found() {
    let args = single_path_args("/tmp/does-not-exist.mp4");
    let err = import(&args).expect_err("v1 floor: non-empty → AssetPathNotFound");
    assert!(matches!(err, AssetImportError::AssetPathNotFound { .. }));
}

#[test]
fn multiple_paths_report_first_path_in_error() {
    let mut args = empty_paths_args();
    args.paths = vec![
        "/tmp/first.mp4".to_string(),
        "/tmp/second.mp4".to_string(),
        "/tmp/third.mp4".to_string(),
    ];
    let err = import(&args).expect_err("v1 floor: non-empty → AssetPathNotFound");
    match err {
        AssetImportError::AssetPathNotFound { path } => {
            assert_eq!(path, "/tmp/first.mp4", "must report the first path");
        }
        other => panic!("expected AssetPathNotFound, got {other:?}"),
    }
}

#[test]
fn asset_path_not_found_carries_input_path_verbatim() {
    let args = single_path_args("/some/weird path with spaces.mp4");
    let err = import(&args).expect_err("v1 floor: non-empty → AssetPathNotFound");
    match err {
        AssetImportError::AssetPathNotFound { path } => {
            assert_eq!(path, "/some/weird path with spaces.mp4");
        }
        other => panic!("expected AssetPathNotFound, got {other:?}"),
    }
}

// --- mode / soft defaults ---------------------------------------------------

#[test]
fn mode_omitted_round_trips_as_none() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "paths": [],
    });
    let typed: AssetImportArgs = serde_json::from_value(raw).expect("omitted mode is allowed");
    assert!(typed.mode.is_none());
}

#[test]
fn soft_omitted_round_trips_as_none() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "paths": [],
    });
    let typed: AssetImportArgs = serde_json::from_value(raw).expect("omitted soft is allowed");
    assert!(typed.soft.is_none());
}

// --- verb-trait surface -----------------------------------------------------

#[test]
fn verb_trait_compute_patch_empty_paths_returns_empty_patch_and_envelope() {
    let prior = empty_project();
    let verb = AssetImportVerb;
    let args = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "paths": [],
    });
    let (patch, data, warnings) = verb
        .compute_patch(&prior, &args)
        .expect("empty paths → spec-documented no-op success");
    assert!(warnings.is_empty());
    let patch_value = serde_json::to_value(&patch).expect("patch → value");
    assert_eq!(patch_value, json!([]));
    let parsed: AssetImportData = serde_json::from_value(data).expect("data deserializes");
    assert!(parsed.assets.is_empty());
    assert!(parsed.modes_used.is_empty());
    assert!(parsed.missing_paths.is_empty());
    assert!(parsed.skipped_input_indices.is_empty());
}

#[test]
fn verb_trait_args_missing_project_id_is_bad_args() {
    let prior = empty_project();
    let verb = AssetImportVerb;
    let err = verb
        .compute_patch(&prior, &json!({"paths": []}))
        .expect_err("missing project_id should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn verb_trait_schema_violation_is_bad_args() {
    let prior = empty_project();
    let verb = AssetImportVerb;
    let paths: Vec<String> = (0..=PATHS_MAX_BATCH).map(|i| format!("/p{i}")).collect();
    let err = verb
        .compute_patch(
            &prior,
            &json!({"project_id": FIXTURE_PROJECT_ID, "paths": paths}),
        )
        .expect_err("schema violation maps to BadArgs (arg-shape rejection)");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn verb_trait_asset_path_not_found_is_custom() {
    let prior = empty_project();
    let verb = AssetImportVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({"project_id": FIXTURE_PROJECT_ID, "paths": ["/x"]}),
        )
        .expect_err("AssetPathNotFound maps to Custom (runtime-class rejection)");
    assert!(matches!(err, VerbError::Custom(_)));
}

#[test]
fn verb_registered_in_default_registry() {
    let registry = default_registry();
    let verb = registry
        .get("asset.import")
        .expect("default_registry exposes asset.import");
    assert_eq!(verb.verb(), "asset.import");
}

// --- native route ----------------------------------------------------------

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb_for_empty_paths() {
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
            "asset.import",
            json!({"project_id": FIXTURE_PROJECT_ID, "paths": []}),
            None,
        )
        .expect("asset.import should route");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("expected Applied outcome for empty-paths asset.import");
    };
    assert!(warnings.is_empty());

    let data: AssetImportData =
        serde_json::from_value(data).expect("asset.import data deserializes");
    assert!(data.assets.is_empty());
    assert!(data.modes_used.is_empty());
    assert!(data.missing_paths.is_empty());
    assert!(data.skipped_input_indices.is_empty());
}

#[cfg(feature = "native")]
#[test]
fn mutate_via_verb_non_empty_import_writes_cas_and_records_canonical_hash_path() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let source = dir.path().join("sample.SRT");
    std::fs::write(&source, b"1\n00:00:00,000 --> 00:00:00,500\nhi\n").expect("write source file");

    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        empty_project(),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry succeeds");

    let outcome = store
        .mutate_via_verb(
            "asset.import",
            json!({"project_id": FIXTURE_PROJECT_ID, "paths": [source.to_string_lossy()]}),
            None,
        )
        .expect("asset.import non-empty should succeed on native route");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("expected Applied outcome for non-empty asset.import");
    };
    assert!(warnings.is_empty());

    let data: AssetImportData =
        serde_json::from_value(data).expect("asset.import data deserializes");
    assert_eq!(data.assets.len(), 1);
    assert_eq!(data.modes_used.len(), 1);
    assert!(data.missing_paths.is_empty());
    assert!(data.skipped_input_indices.is_empty());

    let imported = data.assets[0]
        .as_object()
        .expect("data.assets[0] is an object");
    let hash = imported
        .get("hash")
        .and_then(Value::as_str)
        .expect("imported asset has hash");
    let path = imported
        .get("path")
        .and_then(Value::as_str)
        .expect("imported asset has path");
    assert_eq!(hash.len(), 64);
    assert_eq!(path, format!("assets/{}/{}.srt", &hash[..2], hash));

    let cas_target = dir.path().join(path);
    assert!(
        cas_target.exists(),
        "CAS target must exist: {}",
        cas_target.display()
    );
}

#[cfg(feature = "native")]
#[test]
fn mutate_via_verb_ppm_import_records_image_asset() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let source = dir.path().join("frame.PPM");
    std::fs::write(
        &source,
        b"P6\n# generated fixture\n2 1\n255\n\x00\x00\x00\xff\xff\xff",
    )
    .expect("write source file");

    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        empty_project(),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry succeeds");

    let outcome = store
        .mutate_via_verb(
            "asset.import",
            json!({"project_id": FIXTURE_PROJECT_ID, "paths": [source.to_string_lossy()]}),
            None,
        )
        .expect("asset.import non-empty should succeed on native route");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("expected Applied outcome for non-empty asset.import");
    };
    assert!(warnings.is_empty());

    let data: AssetImportData =
        serde_json::from_value(data).expect("asset.import data deserializes");
    let imported = data.assets[0]
        .as_object()
        .expect("data.assets[0] is an object");
    assert_eq!(imported.get("kind").and_then(Value::as_str), Some("image"));
    assert_eq!(
        imported
            .get("metadata")
            .and_then(|v| v.get("width"))
            .and_then(Value::as_u64),
        Some(2)
    );
    assert_eq!(
        imported
            .get("metadata")
            .and_then(|v| v.get("height"))
            .and_then(Value::as_u64),
        Some(1)
    );
    let hash = imported
        .get("hash")
        .and_then(Value::as_str)
        .expect("imported asset has hash");
    let path = imported
        .get("path")
        .and_then(Value::as_str)
        .expect("imported asset has path");
    assert_eq!(path, format!("assets/{}/{}.ppm", &hash[..2], hash));
}

#[cfg(feature = "native")]
#[test]
fn mutate_via_verb_truncated_ppm_does_not_record_image_asset() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let source = dir.path().join("truncated.ppm");
    std::fs::write(&source, b"P6\n2 1\n255\n\x00\x00\x00").expect("write source file");

    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        empty_project(),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry succeeds");

    let outcome = store
        .mutate_via_verb(
            "asset.import",
            json!({"project_id": FIXTURE_PROJECT_ID, "paths": [source.to_string_lossy()]}),
            None,
        )
        .expect("asset.import non-empty should succeed on native route");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("expected Applied outcome for non-empty asset.import");
    };
    assert!(warnings.is_empty());

    let data: AssetImportData =
        serde_json::from_value(data).expect("asset.import data deserializes");
    let imported = data.assets[0]
        .as_object()
        .expect("data.assets[0] is an object");
    assert_eq!(
        imported.get("kind").and_then(Value::as_str),
        Some("subtitle")
    );
}

#[cfg(feature = "native")]
#[test]
fn mutate_via_verb_rejects_corrupt_existing_cas_object() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let source = dir.path().join("sample.srt");
    std::fs::write(&source, b"1\n00:00:00,000 --> 00:00:00,500\nhi\n").expect("write source file");

    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        empty_project(),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry succeeds");

    let args = json!({"project_id": FIXTURE_PROJECT_ID, "paths": [source.to_string_lossy()]});
    let outcome = store
        .mutate_via_verb("asset.import", args.clone(), None)
        .expect("initial import writes CAS object");
    let MutateOutcome::Applied { data, .. } = outcome else {
        panic!("expected Applied outcome for initial asset.import");
    };
    let data: AssetImportData =
        serde_json::from_value(data).expect("asset.import data deserializes");
    let path = data.assets[0]
        .get("path")
        .and_then(Value::as_str)
        .expect("imported asset has path");
    std::fs::write(dir.path().join(path), b"corrupt bytes").expect("corrupt CAS object");

    let err = store
        .mutate_via_verb("asset.import", args, None)
        .expect_err("corrupt CAS object must be rejected");
    assert!(
        err.to_string()
            .contains("existing CAS object contents do not match expected hash"),
        "unexpected error: {err}"
    );
}

// --- data shape lock --------------------------------------------------------

#[test]
fn empty_envelope_serializes_to_four_keys() {
    let data = import(&empty_paths_args()).expect("empty paths");
    let value = serde_json::to_value(&data).expect("envelope serializes");
    let obj = value.as_object().expect("envelope is an object");
    assert_eq!(obj.len(), 4, "envelope must have exactly four keys");
    for key in [
        "assets",
        "modes_used",
        "missing_paths",
        "skipped_input_indices",
    ] {
        assert!(obj.contains_key(key), "envelope must serialize `{key}`");
    }
}

#[test]
fn empty_envelope_all_values_are_empty_arrays() {
    let data = import(&empty_paths_args()).expect("empty paths");
    let value = serde_json::to_value(&data).expect("envelope serializes");
    let obj = value.as_object().expect("envelope is an object");
    for key in [
        "assets",
        "modes_used",
        "missing_paths",
        "skipped_input_indices",
    ] {
        let arr = obj
            .get(key)
            .and_then(Value::as_array)
            .unwrap_or_else(|| panic!("`{key}` must be an array"));
        assert!(arr.is_empty(), "`{key}` must be empty at v1");
    }
}

#[test]
fn envelope_round_trip_byte_identical() {
    let data = import(&empty_paths_args()).expect("empty paths");
    let serialized = serde_json::to_value(&data).expect("serialize");
    let back: AssetImportData = serde_json::from_value(serialized).expect("deserialize");
    assert_eq!(data, back);
}

// --- reconstructor / fixture ------------------------------------------------

#[test]
fn reconstructor_round_trip_byte_identical() {
    let args = empty_paths_args();
    let prior = empty_project();
    let (patch_value, _, expected) =
        compute_patch(&prior, &args).expect("compute_patch succeeds for empty paths");
    let patch: json_patch::Patch =
        serde_json::from_value(patch_value).expect("patch is valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("empty patch applies to empty project");

    let envelope = data_envelope_from_args(&args, &post_state)
        .expect("data_envelope_from_args should rebuild same data");
    assert_eq!(envelope, expected);
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "asset.import")
        .expect("default_fixtures includes asset.import");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(AssetImportVerb))
        .expect("register asset.import verb");

    let report = validate_reconstructors(&registry, &[fixture])
        .expect("asset.import reconstruct from fixture");
    assert_eq!(report.verbs_checked, vec!["asset.import"]);
    assert_eq!(report.fixtures_run, 1);
}
