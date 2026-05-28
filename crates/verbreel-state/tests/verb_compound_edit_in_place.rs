//! Tests for `compound.edit_in_place` (§20.4) — v1 compound-session floor.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::compound_edit_in_place::compute_patch;
use verbreel_state::{
    Canvas, CompoundEditInPlaceArgs, CompoundEditInPlaceData, CompoundEditInPlaceError,
    CompoundEditInPlaceVerb, Project, Verb, VerbError, VerbRegistry, default_fixtures,
    default_registry, validate_reconstructors,
};
use verbreel_types::ProjectId;

#[cfg(feature = "native")]
use verbreel_state::{LifecycleError, ProjectStore};

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const CLIP_ID_1: &str = "0190b8d3-15e3-7000-bd00-0000000bb910";
const ASSET_ID: &str = "0190b8d3-15e3-7000-bd00-0000000cc910";
const TRACK_ID: &str = "0190b8d3-15e3-7000-bd00-0000000aa910";
const CHILD_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-0000000dd910";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture -> Project")
}

fn fixture_project_id() -> ProjectId {
    empty_project().id
}

fn args_default() -> CompoundEditInPlaceArgs {
    CompoundEditInPlaceArgs {
        project_id: fixture_project_id(),
        clip: CLIP_ID_1.to_string(),
    }
}

fn args_value_default() -> Value {
    serde_json::to_value(args_default()).expect("args serialize")
}

fn custom_detail(err: VerbError) -> String {
    match err {
        VerbError::Custom(detail) => detail,
        other => panic!("expected Custom, got {other:?}"),
    }
}

#[test]
fn args_deserialize_minimal() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "clip": CLIP_ID_1,
    });
    let parsed: CompoundEditInPlaceArgs = serde_json::from_value(raw).expect("minimal args parse");
    assert_eq!(parsed.project_id.to_string(), FIXTURE_PROJECT_ID);
    assert_eq!(parsed.clip, CLIP_ID_1);
}

#[test]
fn args_unknown_field_rejected() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "clip": CLIP_ID_1,
        "extra": true,
    });
    let err =
        serde_json::from_value::<CompoundEditInPlaceArgs>(raw).expect_err("unknown field rejects");
    assert!(err.to_string().contains("unknown field"));
}

#[test]
fn args_missing_project_id_fails_through_verb() {
    let prior = empty_project();
    let verb = CompoundEditInPlaceVerb;
    let err = verb
        .compute_patch(&prior, &json!({ "clip": CLIP_ID_1 }))
        .expect_err("missing project_id should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_missing_clip_fails_through_verb() {
    let prior = empty_project();
    let verb = CompoundEditInPlaceVerb;
    let err = verb
        .compute_patch(&prior, &json!({ "project_id": FIXTURE_PROJECT_ID }))
        .expect_err("missing clip should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_non_string_clip_rejected() {
    let prior = empty_project();
    let verb = CompoundEditInPlaceVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "clip": 42,
            }),
        )
        .expect_err("non-string clip should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn bare_uuid_selector_reaches_compound_not_a_compound_custom() {
    let prior = empty_project();
    let verb = CompoundEditInPlaceVerb;
    let detail = custom_detail(
        verb.compute_patch(&prior, &args_value_default())
            .expect_err("bare UUID should hit v1 floor"),
    );
    assert!(detail.contains("E_COMPOUND_NOT_A_COMPOUND"));
}

#[test]
fn clip_qualified_selector_reaches_compound_not_a_compound_custom() {
    let prior = empty_project();
    let verb = CompoundEditInPlaceVerb;
    let detail = custom_detail(
        verb.compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "clip": format!("clip:{CLIP_ID_1}"),
            }),
        )
        .expect_err("clip:UUID should hit v1 floor"),
    );
    assert!(detail.contains("E_COMPOUND_NOT_A_COMPOUND"));
}

#[test]
fn malformed_bare_uuid_returns_bad_selector_custom() {
    let prior = empty_project();
    let verb = CompoundEditInPlaceVerb;
    let detail = custom_detail(
        verb.compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "clip": "not-a-uuid",
            }),
        )
        .expect_err("malformed UUID should fail"),
    );
    assert!(detail.contains("E_BAD_SELECTOR"));
}

#[test]
fn malformed_clip_body_returns_bad_selector_custom() {
    let prior = empty_project();
    let verb = CompoundEditInPlaceVerb;
    let detail = custom_detail(
        verb.compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "clip": "clip:not-a-uuid",
            }),
        )
        .expect_err("malformed clip body should fail"),
    );
    assert!(detail.contains("E_BAD_SELECTOR"));
}

#[test]
fn qualified_asset_prefix_returns_selector_kind_mismatch_custom() {
    let prior = empty_project();
    let verb = CompoundEditInPlaceVerb;
    let detail = custom_detail(
        verb.compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "clip": format!("asset:{ASSET_ID}"),
            }),
        )
        .expect_err("asset prefix should fail"),
    );
    assert!(detail.contains("E_SELECTOR_KIND_MISMATCH"));
}

#[test]
fn qualified_track_prefix_returns_selector_kind_mismatch_custom() {
    let prior = empty_project();
    let verb = CompoundEditInPlaceVerb;
    let detail = custom_detail(
        verb.compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "clip": format!("track:{TRACK_ID}"),
            }),
        )
        .expect_err("track prefix should fail"),
    );
    assert!(detail.contains("E_SELECTOR_KIND_MISMATCH"));
}

#[test]
fn empty_selector_returns_bad_selector_custom() {
    let prior = empty_project();
    let verb = CompoundEditInPlaceVerb;
    let detail = custom_detail(
        verb.compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "clip": "",
            }),
        )
        .expect_err("empty selector should fail"),
    );
    assert!(detail.contains("E_BAD_SELECTOR"));
}

#[test]
fn future_data_serializes_exact_fields() {
    let data = CompoundEditInPlaceData {
        edit_session_id: "0190b8d3-15e3-7000-bd00-0000000ee910".to_string(),
        child_project_id: CHILD_PROJECT_ID.parse().expect("project id"),
        compound_asset_id: ASSET_ID.parse().expect("asset id"),
        child_duration_tk: 240_000,
        child_canvas: Canvas {
            width: 1080,
            height: 1920,
            background: "#000000ff".to_string(),
            pixel_aspect_num: 1,
            pixel_aspect_den: 1,
        },
        child_fps_num: 30,
        child_fps_den: 1,
    };

    let value = serde_json::to_value(&data).expect("data serializes");
    let obj = value.as_object().expect("data object");
    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "child_canvas",
            "child_duration_tk",
            "child_fps_den",
            "child_fps_num",
            "child_project_id",
            "compound_asset_id",
            "edit_session_id",
        ]
    );
}

#[test]
fn reserved_error_variants_display_codes_and_map_to_custom() {
    let cases = vec![
        CompoundEditInPlaceError::NotFound {
            clip: CLIP_ID_1.to_string(),
        },
        CompoundEditInPlaceError::NoMatch {
            selector: "clip:audio[name=\"missing\"][0]".to_string(),
        },
        CompoundEditInPlaceError::BadSelector {
            detail: "bad selector".to_string(),
        },
        CompoundEditInPlaceError::SelectorKindMismatch {
            actual_kind: "asset".to_string(),
        },
        CompoundEditInPlaceError::CompoundNotACompound {
            clip_id: CLIP_ID_1.to_string(),
            actual_kind: "video".to_string(),
        },
        CompoundEditInPlaceError::Locked {
            failed_target: CLIP_ID_1.to_string(),
        },
        CompoundEditInPlaceError::CompoundSessionLimit {
            project_id: FIXTURE_PROJECT_ID.to_string(),
            cap: 8,
        },
    ];

    for error in cases {
        let detail = error.to_string();
        assert!(
            detail.contains("E_"),
            "detail must include spec code: {detail}"
        );
        let mapped = VerbError::from(error);
        assert!(matches!(mapped, VerbError::Custom(_)));
    }
}

#[test]
fn reconstruct_returns_null_for_well_formed_args() {
    let verb = CompoundEditInPlaceVerb;
    let prior = empty_project();
    let value = verb
        .reconstruct(&args_value_default(), &json!([]), &[], &prior)
        .expect("reconstruct should succeed");
    assert_eq!(value, Value::Null);
}

#[test]
fn reconstruct_rejects_malformed_args() {
    let verb = CompoundEditInPlaceVerb;
    let prior = empty_project();
    let err = verb
        .reconstruct(&json!({}), &json!([]), &[], &prior)
        .expect_err("malformed args should reject");
    assert!(err.to_string().contains("CompoundEditInPlaceArgs"));
}

#[test]
fn reconstruct_from_default_fixture_with_single_verb_registry() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "compound.edit_in_place")
        .expect("fixture present");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(CompoundEditInPlaceVerb))
        .expect("register verb");

    let report = validate_reconstructors(&registry, &[fixture]).expect("validate fixture");
    assert_eq!(report.verbs_checked, vec!["compound.edit_in_place"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn default_fixture_has_empty_patch_warnings_and_null_expected_data() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "compound.edit_in_place")
        .expect("fixture present");
    assert_eq!(fixture.patch, json!([]));
    assert_eq!(fixture.warnings, Vec::<Value>::new());
    assert_eq!(fixture.expected_data, Value::Null);
}

#[test]
fn default_registry_contains_compound_edit_in_place() {
    let registry = default_registry();
    assert!(
        registry.get("compound.edit_in_place").is_some(),
        "compound.edit_in_place should be registered"
    );
}

#[test]
fn verb_trait_lookup_via_default_registry_returns_compound_not_a_compound() {
    let registry = default_registry();
    let verb = registry
        .get("compound.edit_in_place")
        .expect("compound.edit_in_place in registry");
    let prior = empty_project();
    let detail = custom_detail(
        verb.compute_patch(&prior, &args_value_default())
            .expect_err("v1 floor should fail"),
    );
    assert!(detail.contains("E_COMPOUND_NOT_A_COMPOUND"));
}

#[cfg(feature = "native")]
#[test]
fn native_mutate_via_verb_returns_compound_not_a_compound_for_accepted_selectors() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        empty_project(),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry");

    for selector in [CLIP_ID_1.to_string(), format!("clip:{CLIP_ID_1}")] {
        let outcome = store.mutate_via_verb(
            "compound.edit_in_place",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "clip": selector,
            }),
            None,
        );
        let Err(LifecycleError::VerbExecutionFailed { source, .. }) = outcome else {
            panic!("expected VerbExecutionFailed, got {outcome:?}");
        };
        let VerbError::Custom(detail) = source else {
            panic!("expected Custom, got {source:?}");
        };
        assert!(detail.contains("E_COMPOUND_NOT_A_COMPOUND"));
    }
}

#[cfg(feature = "native")]
#[test]
fn native_mutate_via_verb_returns_bad_selector_for_bad_selectors() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        empty_project(),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry");

    for selector in ["not-a-uuid", "clip:not-a-uuid"] {
        let outcome = store.mutate_via_verb(
            "compound.edit_in_place",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "clip": selector,
            }),
            None,
        );
        let Err(LifecycleError::VerbExecutionFailed { source, .. }) = outcome else {
            panic!("expected VerbExecutionFailed, got {outcome:?}");
        };
        let VerbError::Custom(detail) = source else {
            panic!("expected Custom, got {source:?}");
        };
        assert!(detail.contains("E_BAD_SELECTOR"));
    }
}

#[test]
fn compute_patch_helper_returns_compound_not_a_compound_for_accepted_selector() {
    let prior = empty_project();
    let args = args_default();
    let err = compute_patch(&prior, &args).expect_err("v1 floor should error");
    assert!(matches!(
        err,
        CompoundEditInPlaceError::CompoundNotACompound { .. }
    ));
}
