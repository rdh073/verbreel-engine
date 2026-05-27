//! Tests for `template.apply` (§16.3) — v1 template not-found floor.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::template_apply::compute_patch;
use verbreel_state::{
    Project, ReconstructError, TemplateApplyArgs, TemplateApplyData, TemplateApplyError,
    TemplateApplyVerb, TemplateTrackStrategy, Verb, VerbError, VerbRegistry, default_fixtures,
    default_registry, validate_reconstructors,
};

#[cfg(feature = "native")]
use verbreel_state::{LifecycleError, ProjectStore};

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const A_VALID_TEMPLATE_ID: &str = "0190b8d3-15e3-7000-bd00-0000feedbeef";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture -> Project")
}

fn fixture_project_id() -> verbreel_types::ProjectId {
    empty_project().id
}

fn args_default() -> TemplateApplyArgs {
    let mut slots = BTreeMap::new();
    slots.insert("slot_hero".to_string(), "asset_hero".to_string());

    TemplateApplyArgs {
        project_id: fixture_project_id(),
        template_id: A_VALID_TEMPLATE_ID.to_string(),
        slots,
        at_tk: None,
        track_strategy: TemplateTrackStrategy::CreateNew,
    }
}

#[test]
fn args_deserialize_ok_with_required_fields() {
    let typed: TemplateApplyArgs = serde_json::from_value(json!({
        "project_id": FIXTURE_PROJECT_ID,
        "template_id": A_VALID_TEMPLATE_ID,
        "slots": { "slot_hero": "asset_hero" },
    }))
    .expect("args parse");
    assert_eq!(typed.project_id.to_string(), FIXTURE_PROJECT_ID);
    assert_eq!(typed.template_id, A_VALID_TEMPLATE_ID);
    assert_eq!(
        typed.slots.get("slot_hero").map(String::as_str),
        Some("asset_hero")
    );
}

#[test]
fn omitted_track_strategy_defaults_to_create_new() {
    let typed: TemplateApplyArgs = serde_json::from_value(json!({
        "project_id": FIXTURE_PROJECT_ID,
        "template_id": A_VALID_TEMPLATE_ID,
        "slots": {},
    }))
    .expect("missing track_strategy should default");
    assert_eq!(typed.track_strategy, TemplateTrackStrategy::CreateNew);
}

#[test]
fn explicit_track_strategy_create_new_parses() {
    let typed: TemplateApplyArgs = serde_json::from_value(json!({
        "project_id": FIXTURE_PROJECT_ID,
        "template_id": A_VALID_TEMPLATE_ID,
        "slots": {},
        "track_strategy": "create_new",
    }))
    .expect("create_new should parse");
    assert_eq!(typed.track_strategy, TemplateTrackStrategy::CreateNew);
}

#[test]
fn explicit_track_strategy_use_existing_parses() {
    let typed: TemplateApplyArgs = serde_json::from_value(json!({
        "project_id": FIXTURE_PROJECT_ID,
        "template_id": A_VALID_TEMPLATE_ID,
        "slots": {},
        "track_strategy": "use_existing",
    }))
    .expect("use_existing should parse");
    assert_eq!(typed.track_strategy, TemplateTrackStrategy::UseExisting);
}

#[test]
fn invalid_track_strategy_maps_to_bad_args() {
    let prior = empty_project();
    let verb = TemplateApplyVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "template_id": A_VALID_TEMPLATE_ID,
                "slots": {},
                "track_strategy": "createNew",
            }),
        )
        .expect_err("invalid track_strategy should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn missing_required_fields_fail_through_verb() {
    let prior = empty_project();
    let verb = TemplateApplyVerb;
    let cases = [
        json!({
            "template_id": A_VALID_TEMPLATE_ID,
            "slots": {},
        }),
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "slots": {},
        }),
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "template_id": A_VALID_TEMPLATE_ID,
        }),
    ];

    for raw in cases {
        let err = verb
            .compute_patch(&prior, &raw)
            .expect_err("missing required field should map to BadArgs");
        assert!(matches!(err, VerbError::BadArgs { .. }));
    }
}

#[test]
fn non_string_project_id_fails_through_verb() {
    let prior = empty_project();
    let verb = TemplateApplyVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": 1234,
                "template_id": A_VALID_TEMPLATE_ID,
                "slots": {},
            }),
        )
        .expect_err("non-string project_id should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn non_string_template_id_fails_through_verb() {
    let prior = empty_project();
    let verb = TemplateApplyVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "template_id": 42,
                "slots": {},
            }),
        )
        .expect_err("non-string template_id should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn non_object_slots_fails_through_verb() {
    let prior = empty_project();
    let verb = TemplateApplyVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "template_id": A_VALID_TEMPLATE_ID,
                "slots": [],
            }),
        )
        .expect_err("non-object slots should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn non_string_slot_value_fails_through_verb() {
    let prior = empty_project();
    let verb = TemplateApplyVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "template_id": A_VALID_TEMPLATE_ID,
                "slots": { "slot_hero": 123 },
            }),
        )
        .expect_err("non-string slot values should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn unknown_field_rejected_through_verb() {
    let prior = empty_project();
    let verb = TemplateApplyVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "template_id": A_VALID_TEMPLATE_ID,
                "slots": {},
                "extra": true,
            }),
        )
        .expect_err("deny_unknown_fields rejects extras");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn empty_slots_are_shape_valid_and_hit_runtime_not_found() {
    let prior = empty_project();
    let err = compute_patch(
        &prior,
        &TemplateApplyArgs {
            slots: BTreeMap::new(),
            ..args_default()
        },
    )
    .expect_err("well-formed args should hit runtime floor");
    let TemplateApplyError::TemplateNotFound { template_id } = err else {
        panic!("expected TemplateNotFound, got {err:?}");
    };
    assert_eq!(template_id, A_VALID_TEMPLATE_ID);
}

#[test]
fn arbitrary_slot_ids_are_shape_valid_and_hit_runtime_not_found() {
    let prior = empty_project();
    let mut slots = BTreeMap::new();
    slots.insert(String::new(), "value-1".to_string());
    slots.insert("UPPER+mixed id".to_string(), "value-2".to_string());
    slots.insert("slot with spaces".to_string(), "value-3".to_string());

    let err = compute_patch(
        &prior,
        &TemplateApplyArgs {
            slots,
            ..args_default()
        },
    )
    .expect_err("v1 floor should miss any well-formed slot map");
    let TemplateApplyError::TemplateNotFound { .. } = err else {
        panic!("expected TemplateNotFound, got {err:?}");
    };
}

#[test]
fn empty_template_id_is_runtime_not_found() {
    let prior = empty_project();
    let err = compute_patch(
        &prior,
        &TemplateApplyArgs {
            template_id: String::new(),
            ..args_default()
        },
    )
    .expect_err("empty template_id is shape-valid");
    let TemplateApplyError::TemplateNotFound { template_id } = err else {
        panic!("expected TemplateNotFound, got {err:?}");
    };
    assert_eq!(template_id, "");
}

#[test]
fn negative_at_tk_returns_bad_time() {
    let prior = empty_project();
    let err = compute_patch(
        &prior,
        &TemplateApplyArgs {
            at_tk: Some(-1),
            ..args_default()
        },
    )
    .expect_err("negative at_tk should fail");
    let TemplateApplyError::BadTime { at_tk } = err else {
        panic!("expected BadTime, got {err:?}");
    };
    assert_eq!(at_tk, -1);
}

#[test]
fn negative_at_tk_maps_to_custom_through_verb() {
    let prior = empty_project();
    let verb = TemplateApplyVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "template_id": A_VALID_TEMPLATE_ID,
                "slots": {},
                "at_tk": -1,
            }),
        )
        .expect_err("negative at_tk should map to Custom runtime error");
    let VerbError::Custom(detail) = err else {
        panic!("expected Custom, got {err:?}");
    };
    assert!(detail.contains("E_BAD_TIME"));
}

#[test]
fn omitted_at_tk_maps_to_template_not_found_custom() {
    let prior = empty_project();
    let verb = TemplateApplyVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "template_id": A_VALID_TEMPLATE_ID,
                "slots": {},
            }),
        )
        .expect_err("well-formed args should hit runtime floor");
    let VerbError::Custom(detail) = err else {
        panic!("expected Custom, got {err:?}");
    };
    assert!(detail.contains("E_TEMPLATE_NOT_FOUND"));
}

#[test]
fn zero_at_tk_maps_to_template_not_found_custom() {
    let prior = empty_project();
    let verb = TemplateApplyVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "template_id": A_VALID_TEMPLATE_ID,
                "slots": {},
                "at_tk": 0,
            }),
        )
        .expect_err("well-formed args should hit runtime floor");
    let VerbError::Custom(detail) = err else {
        panic!("expected Custom, got {err:?}");
    };
    assert!(detail.contains("E_TEMPLATE_NOT_FOUND"));
}

#[test]
fn large_positive_at_tk_maps_to_template_not_found_custom() {
    let prior = empty_project();
    let verb = TemplateApplyVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "template_id": A_VALID_TEMPLATE_ID,
                "slots": {},
                "at_tk": i64::MAX / 2,
            }),
        )
        .expect_err("well-formed args should hit runtime floor");
    let VerbError::Custom(detail) = err else {
        panic!("expected Custom, got {err:?}");
    };
    assert!(detail.contains("E_TEMPLATE_NOT_FOUND"));
}

#[test]
fn track_strategy_wire_literals_are_snake_case() {
    assert_eq!(
        serde_json::to_value(TemplateTrackStrategy::CreateNew).expect("CreateNew -> Value"),
        json!("create_new")
    );
    assert_eq!(
        serde_json::to_value(TemplateTrackStrategy::UseExisting).expect("UseExisting -> Value"),
        json!("use_existing")
    );
}

#[test]
fn default_track_strategy_serializes_as_omitted() {
    let value = serde_json::to_value(args_default()).expect("args serialize");
    let obj = value.as_object().expect("args object");
    assert!(!obj.contains_key("track_strategy"));
}

#[test]
fn future_success_data_serializes_exact_spec_fields() {
    let data = TemplateApplyData {
        template_id: A_VALID_TEMPLATE_ID.to_string(),
        at_tk: 240_000,
        duration_tk: 120_000,
        created_track_ids: vec!["track-a".to_string()],
        reused_track_ids: vec!["track-b".to_string()],
        created_clip_ids: vec!["clip-a".to_string()],
        created_text_clip_ids: vec!["text-clip-a".to_string()],
        imported_asset_ids: vec!["asset-a".to_string()],
        substituted_slot_count: 2,
        defaulted_slot_ids: vec!["slot_headline".to_string()],
    };

    let value = serde_json::to_value(data).expect("TemplateApplyData -> Value");
    let obj = value.as_object().expect("TemplateApplyData is an object");
    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();
    let mut expected = vec![
        "at_tk",
        "created_clip_ids",
        "created_text_clip_ids",
        "created_track_ids",
        "defaulted_slot_ids",
        "duration_tk",
        "imported_asset_ids",
        "reused_track_ids",
        "substituted_slot_count",
        "template_id",
    ];
    expected.sort_unstable();
    assert_eq!(keys, expected);
}

#[test]
fn reserved_error_variants_display_expected_codes() {
    assert!(
        TemplateApplyError::TemplateNotFound {
            template_id: "t".to_string()
        }
        .to_string()
        .contains("E_TEMPLATE_NOT_FOUND")
    );
    assert!(
        TemplateApplyError::SlotMissing {
            missing_slot_ids: vec!["slot".to_string()]
        }
        .to_string()
        .contains("E_TEMPLATE_SLOT_MISSING")
    );
    assert!(
        TemplateApplyError::SlotKindMismatch {
            slot_id: "slot".to_string(),
            expected_kind: "video".to_string(),
            actual_value_type: "text".to_string()
        }
        .to_string()
        .contains("E_TEMPLATE_SLOT_KIND_MISMATCH")
    );
    assert!(
        TemplateApplyError::SlotConstraint {
            slot_id: "slot".to_string(),
            bound: "max_chars".to_string(),
            detail: "too long".to_string()
        }
        .to_string()
        .contains("E_TEMPLATE_SLOT_CONSTRAINT")
    );
    assert!(
        TemplateApplyError::SchemaViolation {
            detail: "declared=1.2.0".to_string()
        }
        .to_string()
        .contains("E_TEMPLATE_SCHEMA_VIOLATION")
    );
    assert!(
        TemplateApplyError::TrackKindMismatch {
            template_track_name: "vox".to_string(),
            template_track_kind: "audio".to_string(),
            target_track_kind: "text".to_string()
        }
        .to_string()
        .contains("E_TEMPLATE_TRACK_KIND_MISMATCH")
    );
    assert!(
        TemplateApplyError::ClipOverlap {
            failed_template_track_name: "vox".to_string(),
            colliding_clip_ids: vec!["clip-1".to_string()],
            hint: "use create_new".to_string()
        }
        .to_string()
        .contains("E_CLIP_OVERLAP")
    );
    assert!(
        TemplateApplyError::AssetNotFound {
            slot_id: "slot".to_string(),
            asset_id: "asset-1".to_string()
        }
        .to_string()
        .contains("E_ASSET_NOT_FOUND")
    );
    assert!(
        TemplateApplyError::BadTime { at_tk: -1 }
            .to_string()
            .contains("E_BAD_TIME")
    );
    assert!(
        TemplateApplyError::Locked {
            track_id: "track-1".to_string()
        }
        .to_string()
        .contains("E_LOCKED")
    );
    assert!(
        TemplateApplyError::Busy {
            detail: "preview running".to_string()
        }
        .to_string()
        .contains("E_BUSY")
    );
}

#[test]
fn all_template_apply_errors_map_to_custom() {
    let cases = vec![
        TemplateApplyError::TemplateNotFound {
            template_id: "t".to_string(),
        },
        TemplateApplyError::SlotMissing {
            missing_slot_ids: vec!["slot".to_string()],
        },
        TemplateApplyError::SlotKindMismatch {
            slot_id: "slot".to_string(),
            expected_kind: "video".to_string(),
            actual_value_type: "text".to_string(),
        },
        TemplateApplyError::SlotConstraint {
            slot_id: "slot".to_string(),
            bound: "max_chars".to_string(),
            detail: "too long".to_string(),
        },
        TemplateApplyError::SchemaViolation {
            detail: "declared=1.2.0".to_string(),
        },
        TemplateApplyError::TrackKindMismatch {
            template_track_name: "vox".to_string(),
            template_track_kind: "audio".to_string(),
            target_track_kind: "text".to_string(),
        },
        TemplateApplyError::ClipOverlap {
            failed_template_track_name: "vox".to_string(),
            colliding_clip_ids: vec!["clip-1".to_string()],
            hint: "use create_new".to_string(),
        },
        TemplateApplyError::AssetNotFound {
            slot_id: "slot".to_string(),
            asset_id: "asset-1".to_string(),
        },
        TemplateApplyError::BadTime { at_tk: -1 },
        TemplateApplyError::Locked {
            track_id: "track-1".to_string(),
        },
        TemplateApplyError::Busy {
            detail: "preview running".to_string(),
        },
    ];

    for case in cases {
        let mapped: VerbError = case.into();
        assert!(matches!(mapped, VerbError::Custom(_)));
    }
}

#[test]
fn reconstruct_returns_null_for_well_formed_args() {
    let verb = TemplateApplyVerb;
    let prior = empty_project();

    let data = verb
        .reconstruct(
            &serde_json::to_value(args_default()).expect("args serialize"),
            &json!([]),
            &[],
            &prior,
        )
        .expect("reconstruct succeeds");
    assert_eq!(data, Value::Null);
}

#[test]
fn reconstruct_rejects_malformed_args() {
    let verb = TemplateApplyVerb;
    let prior = empty_project();
    let err = verb
        .reconstruct(&json!({}), &json!([]), &[], &prior)
        .expect_err("malformed args should fail reconstruction");
    assert!(matches!(
        err,
        ReconstructError::TypeMismatch {
            expected: "TemplateApplyArgs",
            ..
        }
    ));
}

#[test]
fn default_fixture_validates_with_only_template_apply_verb() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "template.apply")
        .expect("default_fixtures includes template.apply");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(TemplateApplyVerb))
        .expect("register template.apply verb");

    let report = validate_reconstructors(&registry, &[fixture]).expect("fixture validates");
    assert_eq!(report.verbs_checked, vec!["template.apply"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn default_fixture_has_empty_patch_warnings_and_null_expected_data() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "template.apply")
        .expect("default_fixtures includes template.apply");
    assert_eq!(fixture.patch, json!([]));
    assert!(fixture.warnings.is_empty());
    assert_eq!(fixture.expected_data, Value::Null);
}

#[test]
fn default_registry_contains_template_apply() {
    let registry = default_registry();
    let verb = registry
        .get("template.apply")
        .expect("template.apply in default_registry");
    let prior = empty_project();
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "template_id": A_VALID_TEMPLATE_ID,
                "slots": {},
            }),
        )
        .expect_err("v1 floor always errors");
    let VerbError::Custom(detail) = err else {
        panic!("expected Custom, got {err:?}");
    };
    assert!(detail.contains("E_TEMPLATE_NOT_FOUND"));
}

#[cfg(feature = "native")]
#[test]
fn mutate_via_verb_returns_runtime_template_not_found_floor() {
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
        "template.apply",
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "template_id": A_VALID_TEMPLATE_ID,
            "slots": {},
        }),
        None,
    );
    let Err(LifecycleError::VerbExecutionFailed { source, .. }) = outcome else {
        panic!("expected VerbExecutionFailed, got {outcome:?}");
    };
    let VerbError::Custom(detail) = source else {
        panic!("expected Custom source, got {source:?}");
    };
    assert!(detail.contains("E_TEMPLATE_NOT_FOUND"));
}

#[cfg(feature = "native")]
#[test]
fn mutate_via_verb_negative_at_tk_returns_runtime_bad_time_floor() {
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
        "template.apply",
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "template_id": A_VALID_TEMPLATE_ID,
            "slots": {},
            "at_tk": -1,
        }),
        None,
    );
    let Err(LifecycleError::VerbExecutionFailed { source, .. }) = outcome else {
        panic!("expected VerbExecutionFailed, got {outcome:?}");
    };
    let VerbError::Custom(detail) = source else {
        panic!("expected Custom source, got {source:?}");
    };
    assert!(detail.contains("E_BAD_TIME"));
}
