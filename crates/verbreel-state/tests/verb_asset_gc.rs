//! Tests for `asset.gc` (§3.6) — eighty-seventh production verb.

use std::sync::Arc;

use serde_json::json;
use verbreel_state::verbs::asset_gc::{compute_patch, data_envelope_from_args};
use verbreel_state::{
    ASSET_GC_NEITHER_HINT, AssetGcArgs, AssetGcData, AssetGcError, AssetGcVerb, MutateOutcome,
    Project, Verb, VerbError, VerbRegistry, default_fixtures, default_registry,
    validate_reconstructors,
};

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

// --- args deserialization ----------------------------------------------------

#[test]
fn args_deserialize_with_project_id_ok() {
    let args: AssetGcArgs = serde_json::from_value(json!({"project_id": FIXTURE_PROJECT_ID}))
        .expect("well-formed args deserialize");
    assert_eq!(
        args.project_id.expect("project_id present").to_string(),
        FIXTURE_PROJECT_ID
    );
    assert_eq!(args.global, None);
    assert_eq!(args.suppress_orphan_risk, None);
}

#[test]
fn args_deserialize_empty_object_ok() {
    // All three fields are optional at the deserialization layer; the
    // scope cross-validation happens in compute_patch.
    let args: AssetGcArgs =
        serde_json::from_value(json!({})).expect("empty args object should deserialize");
    assert!(args.project_id.is_none());
    assert!(args.global.is_none());
    assert!(args.suppress_orphan_risk.is_none());
}

#[test]
fn args_deserialize_all_three_fields() {
    let args: AssetGcArgs = serde_json::from_value(json!({
        "project_id": FIXTURE_PROJECT_ID,
        "global": false,
        "suppress_orphan_risk": true,
    }))
    .expect("all three fields should deserialize");
    assert!(args.project_id.is_some());
    assert_eq!(args.global, Some(false));
    assert_eq!(args.suppress_orphan_risk, Some(true));
}

#[test]
fn args_deny_unknown_fields() {
    let result: Result<AssetGcArgs, _> = serde_json::from_value(json!({
        "project_id": FIXTURE_PROJECT_ID,
        "stray_field": "rejected",
    }));
    assert!(result.is_err(), "stray field must be rejected");
}

#[test]
fn args_wrong_project_id_type_is_bad_args() {
    let prior = empty_project();
    let verb = AssetGcVerb;
    let err = verb
        .compute_patch(&prior, &json!({"project_id": 42}))
        .expect_err("non-string project_id should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

// --- validation matrix -------------------------------------------------------

#[test]
fn both_project_id_and_global_true_is_args_incompatible() {
    let prior = empty_project();
    let args = AssetGcArgs {
        project_id: Some(fixture_project_id()),
        global: Some(true),
        suppress_orphan_risk: None,
    };
    let err = compute_patch(&prior, &args).expect_err("mutually exclusive scope must error");
    assert!(matches!(err, AssetGcError::ArgsIncompatible { .. }));
}

#[test]
fn neither_scope_is_project_not_found() {
    let prior = empty_project();
    let args = AssetGcArgs::default();
    let err = compute_patch(&prior, &args).expect_err("no scope must error");
    match err {
        AssetGcError::ProjectNotFound { hint } => assert_eq!(hint, ASSET_GC_NEITHER_HINT),
        other => panic!("expected ProjectNotFound, got {other:?}"),
    }
}

#[test]
fn explicit_global_false_with_no_project_id_is_project_not_found() {
    let prior = empty_project();
    let args = AssetGcArgs {
        project_id: None,
        global: Some(false),
        suppress_orphan_risk: None,
    };
    let err = compute_patch(&prior, &args).expect_err("global: false with no project_id errors");
    assert!(matches!(err, AssetGcError::ProjectNotFound { .. }));
}

#[test]
fn global_true_only_is_gc_not_allowed() {
    let prior = empty_project();
    let args = AssetGcArgs {
        project_id: None,
        global: Some(true),
        suppress_orphan_risk: None,
    };
    let err = compute_patch(&prior, &args).expect_err("global gc must be refused in v1");
    assert!(matches!(err, AssetGcError::GcNotAllowed));
}

#[test]
fn global_true_with_suppress_is_still_gc_not_allowed() {
    // suppress_orphan_risk is a v1 no-op; refusal still triggers.
    let prior = empty_project();
    let args = AssetGcArgs {
        project_id: None,
        global: Some(true),
        suppress_orphan_risk: Some(true),
    };
    let err =
        compute_patch(&prior, &args).expect_err("global gc must be refused regardless of suppress");
    assert!(matches!(err, AssetGcError::GcNotAllowed));
}

// --- happy path --------------------------------------------------------------

#[test]
fn project_id_scope_returns_empty_data() {
    let prior = empty_project();
    let args = AssetGcArgs {
        project_id: Some(fixture_project_id()),
        global: None,
        suppress_orphan_risk: None,
    };
    let (_, _, data) = compute_patch(&prior, &args).expect("project_id scope is the v1 ok path");
    assert!(data.removed_paths.is_empty());
    assert_eq!(data.freed_bytes, 0);
}

#[test]
fn project_id_scope_with_suppress_returns_empty_data_no_warnings() {
    let prior = empty_project();
    let args = AssetGcArgs {
        project_id: Some(fixture_project_id()),
        global: None,
        suppress_orphan_risk: Some(true),
    };
    let (_, warnings, data) =
        compute_patch(&prior, &args).expect("project_id + suppress is the v1 ok path");
    assert!(warnings.is_empty());
    assert!(data.removed_paths.is_empty());
    assert_eq!(data.freed_bytes, 0);
}

#[test]
fn project_id_scope_with_explicit_global_false_returns_empty_data() {
    let prior = empty_project();
    let args = AssetGcArgs {
        project_id: Some(fixture_project_id()),
        global: Some(false),
        suppress_orphan_risk: None,
    };
    let (_, _, data) =
        compute_patch(&prior, &args).expect("project_id + global:false is the v1 ok path");
    assert!(data.removed_paths.is_empty());
    assert_eq!(data.freed_bytes, 0);
}

#[test]
fn compute_patch_returns_empty_patch() {
    let prior = empty_project();
    let args = AssetGcArgs {
        project_id: Some(fixture_project_id()),
        global: None,
        suppress_orphan_risk: None,
    };
    let (patch, _, _) = compute_patch(&prior, &args).expect("compute_patch should succeed");
    assert_eq!(patch, json!([]));
}

#[test]
fn compute_patch_warnings_always_empty_in_v1() {
    let prior = empty_project();
    let args = AssetGcArgs {
        project_id: Some(fixture_project_id()),
        global: None,
        suppress_orphan_risk: None,
    };
    let (_, warnings, _) = compute_patch(&prior, &args).expect("compute_patch should succeed");
    assert!(warnings.is_empty());
}

// --- data shape lock ---------------------------------------------------------

#[test]
fn data_shape_has_two_fields() {
    let prior = empty_project();
    let args = AssetGcArgs {
        project_id: Some(fixture_project_id()),
        global: None,
        suppress_orphan_risk: None,
    };
    let (_, _, data) = compute_patch(&prior, &args).expect("compute_patch should succeed");
    let value = serde_json::to_value(&data).expect("data serializes");
    let obj = value.as_object().expect("data is an object");
    assert_eq!(obj.len(), 2);
    assert!(obj.contains_key("removed_paths"));
    assert!(obj.contains_key("freed_bytes"));
}

#[test]
fn data_round_trip_through_serde() {
    let original = AssetGcData {
        removed_paths: Vec::new(),
        freed_bytes: 0,
    };
    let value = serde_json::to_value(&original).expect("serialize");
    let back: AssetGcData = serde_json::from_value(value).expect("deserialize");
    assert_eq!(back, original);
}

// --- verb trait surface ------------------------------------------------------

#[test]
fn verb_trait_surface_via_default_registry() {
    let registry = default_registry();
    let verb = registry
        .get("asset.gc")
        .expect("default_registry exposes asset.gc");
    assert_eq!(verb.verb(), "asset.gc");

    let prior = empty_project();
    let (patch, data, warnings) = verb
        .compute_patch(&prior, &json!({"project_id": FIXTURE_PROJECT_ID}))
        .expect("registered verb compute_patch should succeed");

    assert!(warnings.is_empty());
    let patch_value = serde_json::to_value(&patch).expect("patch → value");
    assert_eq!(patch_value, json!([]));
    let parsed: AssetGcData = serde_json::from_value(data).expect("data deserializes to envelope");
    assert!(parsed.removed_paths.is_empty());
    assert_eq!(parsed.freed_bytes, 0);
}

#[test]
fn verb_trait_routes_validation_errors_through_bad_args() {
    let registry = default_registry();
    let verb = registry.get("asset.gc").expect("verb registered");

    let prior = empty_project();
    let err = verb
        .compute_patch(
            &prior,
            &json!({"project_id": FIXTURE_PROJECT_ID, "global": true}),
        )
        .expect_err("mutually exclusive scope must error");
    assert!(matches!(err, VerbError::BadArgs { .. }));

    let err = verb
        .compute_patch(&prior, &json!({}))
        .expect_err("no scope must error");
    assert!(matches!(err, VerbError::BadArgs { .. }));

    let err = verb
        .compute_patch(&prior, &json!({"global": true}))
        .expect_err("global-only must error");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

// --- reconstructor / fixture -------------------------------------------------

#[test]
fn reconstructor_round_trip_byte_identical() {
    let prior = empty_project();
    let args = AssetGcArgs {
        project_id: Some(fixture_project_id()),
        global: None,
        suppress_orphan_risk: None,
    };
    let (patch_value, _, expected) =
        compute_patch(&prior, &args).expect("compute_patch should succeed");
    let patch: json_patch::Patch =
        serde_json::from_value(patch_value).expect("patch is valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("empty patch applies to empty project");

    let envelope = data_envelope_from_args(&args, &post_state)
        .expect("data_envelope_from_args should rebuild same data");
    assert_eq!(envelope, expected);

    let a = serde_json::to_value(&envelope).expect("envelope serializes");
    let b = serde_json::to_value(&expected).expect("expected serializes");
    assert_eq!(a, b);
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "asset.gc")
        .expect("default_fixtures includes asset.gc");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(AssetGcVerb))
        .expect("register asset.gc verb");

    let report =
        validate_reconstructors(&registry, &[fixture]).expect("asset.gc reconstruct from fixture");
    assert_eq!(report.verbs_checked, vec!["asset.gc"]);
    assert_eq!(report.fixtures_run, 1);
}

// --- native dispatcher route -------------------------------------------------

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
        .mutate_via_verb("asset.gc", json!({"project_id": FIXTURE_PROJECT_ID}), None)
        .expect("asset.gc should route");

    let MutateOutcome::NoOp { data, warnings, .. } = outcome else {
        panic!("expected NoOp outcome from asset.gc");
    };
    assert!(warnings.is_empty());

    let data: AssetGcData = serde_json::from_value(data).expect("asset.gc data deserializes");
    assert!(data.removed_paths.is_empty());
    assert_eq!(data.freed_bytes, 0);
}
