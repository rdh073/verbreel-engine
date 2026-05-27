//! Tests for `compound.create` (§20.1) — v1 storage/schema floor.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::compound_create::{
    COMPOUND_CREATE_CLIPS_MAX, compute_patch, resolved_allow_gaps,
};
use verbreel_state::{
    CompoundCreateArgs, CompoundCreateData, CompoundCreateError, CompoundCreateVerb, Project, Verb,
    VerbError, VerbRegistry, default_fixtures, default_registry, validate_reconstructors,
};
use verbreel_types::ProjectId;

#[cfg(feature = "native")]
use verbreel_state::{LifecycleError, ProjectStore};

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const CLIP_ID_1: &str = "0190b8d3-15e3-7000-bd00-0000000bb910";
const CLIP_ID_2: &str = "0190b8d3-15e3-7000-bd00-0000000bb911";
const TRACK_ID: &str = "0190b8d3-15e3-7000-bd00-0000000aa910";
const ASSET_ID: &str = "0190b8d3-15e3-7000-bd00-0000000cc910";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture -> Project")
}

fn fixture_project_id() -> ProjectId {
    empty_project().id
}

fn args_default() -> CompoundCreateArgs {
    CompoundCreateArgs {
        project_id: fixture_project_id(),
        clips: vec![CLIP_ID_1.parse().expect("clip id parse")],
        name: None,
        allow_gaps: None,
    }
}

fn args_value_default() -> Value {
    serde_json::to_value(args_default()).expect("args serialize")
}

fn make_clip_id(index: usize) -> String {
    format!("0190b8d3-15e3-7000-bd00-{index:012x}")
}

fn make_clip_ids(count: usize) -> Vec<String> {
    (0..count).map(make_clip_id).collect()
}

fn custom_detail(err: VerbError) -> String {
    match err {
        VerbError::Custom(detail) => detail,
        other => panic!("expected Custom, got {other:?}"),
    }
}

fn bad_args_detail(err: VerbError) -> String {
    match err {
        VerbError::BadArgs { detail } => detail,
        other => panic!("expected BadArgs, got {other:?}"),
    }
}

#[test]
fn args_deserialize_minimal() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "clips": [CLIP_ID_1],
    });
    let parsed: CompoundCreateArgs = serde_json::from_value(raw).expect("minimal args parse");
    assert_eq!(parsed.project_id.to_string(), FIXTURE_PROJECT_ID);
    assert_eq!(parsed.clips.len(), 1);
    assert_eq!(parsed.clips[0].to_string(), CLIP_ID_1);
    assert_eq!(parsed.name, None);
    assert_eq!(parsed.allow_gaps, None);
}

#[test]
fn args_deserialize_all_optional_fields() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "clips": [CLIP_ID_1, CLIP_ID_2],
        "name": "My Compound",
        "allow_gaps": true,
    });
    let parsed: CompoundCreateArgs = serde_json::from_value(raw).expect("args parse");
    assert_eq!(parsed.clips.len(), 2);
    assert_eq!(parsed.name.as_deref(), Some("My Compound"));
    assert_eq!(parsed.allow_gaps, Some(true));
}

#[test]
fn args_unknown_field_rejected() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "clips": [CLIP_ID_1],
        "extra": true,
    });
    let err = serde_json::from_value::<CompoundCreateArgs>(raw).expect_err("unknown field rejects");
    assert!(err.to_string().contains("unknown field"));
}

#[test]
fn args_missing_project_id_fails_through_verb() {
    let prior = empty_project();
    let verb = CompoundCreateVerb;
    let err = verb
        .compute_patch(&prior, &json!({ "clips": [CLIP_ID_1] }))
        .expect_err("missing project_id should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_missing_clips_fails_through_verb() {
    let prior = empty_project();
    let verb = CompoundCreateVerb;
    let err = verb
        .compute_patch(&prior, &json!({ "project_id": FIXTURE_PROJECT_ID }))
        .expect_err("missing clips should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_non_array_clips_rejected() {
    let prior = empty_project();
    let verb = CompoundCreateVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "clips": CLIP_ID_1,
            }),
        )
        .expect_err("non-array clips should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_non_uuid_clip_entry_rejected() {
    let prior = empty_project();
    let verb = CompoundCreateVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "clips": ["not-a-uuid"],
            }),
        )
        .expect_err("malformed clip id should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_non_v7_clip_entry_rejected() {
    let prior = empty_project();
    let verb = CompoundCreateVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "clips": ["550e8400-e29b-41d4-a716-446655440000"],
            }),
        )
        .expect_err("non-v7 clip id should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_non_string_name_rejected() {
    let prior = empty_project();
    let verb = CompoundCreateVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "clips": [CLIP_ID_1],
                "name": 42,
            }),
        )
        .expect_err("non-string name should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_non_boolean_allow_gaps_rejected() {
    let prior = empty_project();
    let verb = CompoundCreateVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "clips": [CLIP_ID_1],
                "allow_gaps": "true",
            }),
        )
        .expect_err("non-boolean allow_gaps should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn omitted_allow_gaps_defaults_false() {
    let args = args_default();
    assert!(!resolved_allow_gaps(&args));
}

#[test]
fn explicit_allow_gaps_true_resolves_true() {
    let mut args = args_default();
    args.allow_gaps = Some(true);
    assert!(resolved_allow_gaps(&args));
}

#[test]
fn explicit_allow_gaps_false_resolves_false() {
    let mut args = args_default();
    args.allow_gaps = Some(false);
    assert!(!resolved_allow_gaps(&args));
}

#[test]
fn empty_clips_returns_compound_empty_as_custom() {
    let prior = empty_project();
    let verb = CompoundCreateVerb;
    let detail = custom_detail(
        verb.compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "clips": [],
            }),
        )
        .expect_err("empty clips should fail"),
    );
    assert!(detail.contains("E_COMPOUND_EMPTY"));
}

#[test]
fn clips_over_1000_returns_schema_violation_as_bad_args() {
    let prior = empty_project();
    let verb = CompoundCreateVerb;
    let detail = bad_args_detail(
        verb.compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "clips": make_clip_ids(COMPOUND_CREATE_CLIPS_MAX + 1),
            }),
        )
        .expect_err("maxItems should fail"),
    );
    assert!(detail.contains("E_SCHEMA_VIOLATION"));
    assert!(detail.contains("split the selection into smaller compounds"));
}

#[test]
fn clips_exactly_1000_is_accepted_then_hits_v1_floor_custom() {
    let prior = empty_project();
    let verb = CompoundCreateVerb;
    let detail = custom_detail(
        verb.compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "clips": make_clip_ids(COMPOUND_CREATE_CLIPS_MAX),
            }),
        )
        .expect_err("accepted non-empty should hit v1 floor"),
    );
    assert!(detail.contains("E_SCHEMA_VIOLATION"));
    assert!(detail.contains("schema/storage context"));
}

#[test]
fn accepted_non_empty_request_reaches_v1_floor_schema_violation_custom() {
    let prior = empty_project();
    let verb = CompoundCreateVerb;
    let detail = custom_detail(
        verb.compute_patch(&prior, &args_value_default())
            .expect_err("v1 floor should fail"),
    );
    assert!(detail.contains("E_SCHEMA_VIOLATION"));
}

#[test]
fn accepted_non_empty_with_allow_gaps_true_also_reaches_v1_floor_custom() {
    let prior = empty_project();
    let verb = CompoundCreateVerb;
    let detail = custom_detail(
        verb.compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "clips": [CLIP_ID_1, CLIP_ID_2],
                "allow_gaps": true,
            }),
        )
        .expect_err("v1 floor should fail"),
    );
    assert!(detail.contains("E_SCHEMA_VIOLATION"));
}

#[test]
fn future_data_serializes_exact_fields_and_omits_dedupe_when_none() {
    let data = CompoundCreateData {
        compound_clip_id: CLIP_ID_1.parse().expect("clip id"),
        compound_asset_id: ASSET_ID.parse().expect("asset id"),
        removed_clip_ids: vec![CLIP_ID_1.parse().expect("clip id")],
        track_id: TRACK_ID.parse().expect("track id"),
        track_position_tk: 120_000,
        duration_tk: 240_000,
        cleared_link_group_clip_ids: vec![CLIP_ID_2.parse().expect("clip id")],
        deduped_existing_asset: None,
    };

    let value = serde_json::to_value(&data).expect("data serializes");
    let obj = value.as_object().expect("data object");
    assert!(!obj.contains_key("deduped_existing_asset"));
    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "cleared_link_group_clip_ids",
            "compound_asset_id",
            "compound_clip_id",
            "duration_tk",
            "removed_clip_ids",
            "track_id",
            "track_position_tk",
        ]
    );
}

#[test]
fn future_data_includes_dedupe_when_present() {
    let data = CompoundCreateData {
        compound_clip_id: CLIP_ID_1.parse().expect("clip id"),
        compound_asset_id: ASSET_ID.parse().expect("asset id"),
        removed_clip_ids: vec![],
        track_id: TRACK_ID.parse().expect("track id"),
        track_position_tk: 0,
        duration_tk: 1,
        cleared_link_group_clip_ids: vec![],
        deduped_existing_asset: Some(true),
    };
    let value = serde_json::to_value(&data).expect("data serializes");
    assert_eq!(
        value.get("deduped_existing_asset").and_then(Value::as_bool),
        Some(true)
    );
}

#[test]
fn reserved_error_variants_display_codes_and_map_to_expected_verb_error() {
    let custom_cases = vec![
        CompoundCreateError::CompoundEmpty,
        CompoundCreateError::CompoundMixedTracks {
            member_track_ids: vec!["t1".to_string(), "t2".to_string()],
        },
        CompoundCreateError::CompoundNonContiguous {
            first_gap_after_clip_id: CLIP_ID_1.to_string(),
            first_gap_size_tk: 123,
        },
        CompoundCreateError::NotFound {
            failed_index: 2,
            failed_target: CLIP_ID_2.to_string(),
        },
        CompoundCreateError::Locked {
            failed_target: CLIP_ID_1.to_string(),
        },
        CompoundCreateError::StorageSchemaUnavailable {
            detail: "missing context".to_string(),
        },
    ];

    for error in custom_cases {
        let detail = error.to_string();
        assert!(detail.contains("E_"), "must carry spec code: {detail}");
        let mapped = VerbError::from(error);
        assert!(matches!(mapped, VerbError::Custom(_)));
    }

    let schema = CompoundCreateError::SchemaViolation {
        field: "clips",
        hint: "split the selection into smaller compounds",
        actual: 1001,
        max: 1000,
    };
    let detail = schema.to_string();
    assert!(detail.contains("E_SCHEMA_VIOLATION"));
    let mapped = VerbError::from(schema);
    assert!(matches!(mapped, VerbError::BadArgs { .. }));
}

#[test]
fn reconstruct_returns_null_for_well_formed_args() {
    let verb = CompoundCreateVerb;
    let prior = empty_project();
    let value = verb
        .reconstruct(&args_value_default(), &json!([]), &[], &prior)
        .expect("reconstruct should succeed");
    assert_eq!(value, Value::Null);
}

#[test]
fn reconstruct_rejects_malformed_args() {
    let verb = CompoundCreateVerb;
    let prior = empty_project();
    let err = verb
        .reconstruct(&json!({}), &json!([]), &[], &prior)
        .expect_err("malformed args should reject");
    assert!(err.to_string().contains("CompoundCreateArgs"));
}

#[test]
fn reconstruct_from_default_fixture_with_single_verb_registry() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "compound.create")
        .expect("fixture present");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(CompoundCreateVerb))
        .expect("register verb");

    let report = validate_reconstructors(&registry, &[fixture]).expect("validate fixture");
    assert_eq!(report.verbs_checked, vec!["compound.create"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn default_fixture_has_empty_patch_warnings_and_null_expected_data() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "compound.create")
        .expect("fixture present");
    assert_eq!(fixture.patch, json!([]));
    assert_eq!(fixture.warnings, Vec::<Value>::new());
    assert_eq!(fixture.expected_data, Value::Null);
}

#[test]
fn default_registry_contains_compound_create() {
    let registry = default_registry();
    assert!(
        registry.get("compound.create").is_some(),
        "compound.create should be registered"
    );
}

#[test]
fn verb_trait_lookup_via_default_registry_returns_v1_floor_schema_violation() {
    let registry = default_registry();
    let verb = registry
        .get("compound.create")
        .expect("compound.create in registry");
    let prior = empty_project();
    let detail = custom_detail(
        verb.compute_patch(&prior, &args_value_default())
            .expect_err("v1 floor should fail"),
    );
    assert!(detail.contains("E_SCHEMA_VIOLATION"));
}

#[cfg(feature = "native")]
#[test]
fn native_mutate_via_verb_returns_v1_floor_schema_violation_for_non_empty() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        empty_project(),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry");

    let outcome = store.mutate_via_verb(
        "compound.create",
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "clips": [CLIP_ID_1],
        }),
        None,
    );
    let Err(LifecycleError::VerbExecutionFailed { source, .. }) = outcome else {
        panic!("expected VerbExecutionFailed, got {outcome:?}");
    };
    let VerbError::Custom(detail) = source else {
        panic!("expected Custom, got {source:?}");
    };
    assert!(detail.contains("E_SCHEMA_VIOLATION"));
}

#[cfg(feature = "native")]
#[test]
fn native_mutate_via_verb_returns_compound_empty_for_empty_selection() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        empty_project(),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry");

    let outcome = store.mutate_via_verb(
        "compound.create",
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "clips": [],
        }),
        None,
    );
    let Err(LifecycleError::VerbExecutionFailed { source, .. }) = outcome else {
        panic!("expected VerbExecutionFailed, got {outcome:?}");
    };
    let VerbError::Custom(detail) = source else {
        panic!("expected Custom, got {source:?}");
    };
    assert!(detail.contains("E_COMPOUND_EMPTY"));
}

#[test]
fn compute_patch_helper_returns_storage_schema_floor_for_accepted_request() {
    let prior = empty_project();
    let args = args_default();
    let err = compute_patch(&prior, &args).expect_err("v1 floor should error");
    assert!(matches!(
        err,
        CompoundCreateError::StorageSchemaUnavailable { .. }
    ));
}

#[test]
fn compute_patch_helper_returns_schema_violation_for_oversized_clips() {
    let prior = empty_project();
    let args = CompoundCreateArgs {
        project_id: fixture_project_id(),
        clips: make_clip_ids(COMPOUND_CREATE_CLIPS_MAX + 1)
            .into_iter()
            .map(|id| id.parse().expect("clip id parse"))
            .collect(),
        name: None,
        allow_gaps: None,
    };
    let err = compute_patch(&prior, &args).expect_err("maxItems should fail");
    assert!(matches!(err, CompoundCreateError::SchemaViolation { .. }));
}
