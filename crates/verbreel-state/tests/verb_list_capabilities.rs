//! Tests for `list_capabilities` (§1.5) — sixty-third production verb.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::list_capabilities::{compute_patch, data_envelope_from_args};
use verbreel_state::{
    ListCapabilitiesArgs, ListCapabilitiesData, ListCapabilitiesVerb, MutateOutcome, Project, Verb,
    VerbError, VerbRegistry, default_fixtures, default_registry, validate_reconstructors,
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

fn args() -> ListCapabilitiesArgs {
    ListCapabilitiesArgs {
        project_id: fixture_project_id(),
    }
}

#[test]
fn args_deserialize_ok() {
    let raw = json!({ "project_id": FIXTURE_PROJECT_ID });
    let typed: ListCapabilitiesArgs =
        serde_json::from_value(raw).expect("project_id is the only required arg field");
    assert_eq!(typed.project_id.to_string(), FIXTURE_PROJECT_ID);
}

#[test]
fn args_missing_project_id_fails_through_verb() {
    let prior = empty_project();
    let verb = ListCapabilitiesVerb;

    let err = verb
        .compute_patch(&prior, &json!({}))
        .expect_err("missing project_id should map to BadArgs");

    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_wrong_type_fails_through_verb() {
    let prior = empty_project();
    let verb = ListCapabilitiesVerb;

    let err = verb
        .compute_patch(&prior, &json!({ "project_id": 12345 }))
        .expect_err("non-string project_id should map to BadArgs");

    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn happy_path_returns_all_thirteen_v1_fields() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");

    assert!(!data.engine_version.is_empty());
    assert_eq!(data.supported_schema_versions, ">=1.0.0 <2.0.0");
    assert_eq!(data.tick_rate_hz, 240_000);
    assert!(!data.verbs.is_empty());
    assert!(!data.effects.is_empty());
    assert_eq!(data.render_presets.len(), 8);
    assert!(data.caption_languages.is_empty());
    assert!(data.caption_engine.is_empty());
    assert!(data.caption_models.is_empty());
    assert!(data.caption_default_model.is_empty());
    assert!(data.linked_ffmpeg_version.is_empty());
    assert!(data.tested_ffmpeg_range.is_empty());
    assert!(data.bundled_fonts.is_empty());
}

#[test]
fn verbs_count_matches_registry() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");

    let registry_size = default_registry().verbs().len();
    assert_eq!(data.verbs.len(), registry_size);
}

#[test]
fn verbs_sorted_by_name_ascending() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");

    let names: Vec<String> = data.verbs.iter().map(|v| v.name.clone()).collect();
    let mut expected = names.clone();
    expected.sort();
    assert_eq!(names, expected);
}

#[test]
fn verbs_contains_list_capabilities_itself() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");

    assert!(
        data.verbs.iter().any(|v| v.name == "list_capabilities"),
        "verbs[] must contain list_capabilities (self-registration)",
    );
}

#[test]
fn effects_count_matches_bundled_effects() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");

    let bundled = verbreel_state::verbs::effect_list_available::bundled_effects().len();
    assert_eq!(data.effects.len(), bundled);
    assert_eq!(data.effects.len(), 16);
}

#[test]
fn effect_entry_has_exactly_three_fields() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");

    let first = data
        .effects
        .first()
        .expect("at least one bundled effect entry");
    let value = serde_json::to_value(first).expect("EffectEntry serializes");
    let obj = value.as_object().expect("EffectEntry is a JSON object");

    assert_eq!(obj.keys().count(), 3);
    assert!(obj.contains_key("kind"));
    assert!(obj.contains_key("category"));
    assert!(obj.contains_key("params_schema_id"));
    assert!(!obj.contains_key("summary"));
}

#[test]
fn tick_rate_hz_is_two_hundred_forty_thousand() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");

    assert_eq!(data.tick_rate_hz, 240_000);
}

#[test]
fn supported_schema_versions_is_v1_range() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");

    assert_eq!(data.supported_schema_versions, ">=1.0.0 <2.0.0");
}

#[test]
fn engine_version_is_non_empty() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");

    assert!(!data.engine_version.is_empty());
}

#[test]
fn render_presets_advertises_bundled_set() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    // Wired through to render.list_presets bundle (§11.4) so agents probing
    // capabilities see the available presets per §1.5 discovery contract.
    assert_eq!(data.render_presets.len(), 8);
    assert!(data.render_presets.contains(&"youtube-1080p".to_string()));
    assert!(data.render_presets.contains(&"prores-master".to_string()));
}

#[test]
fn caption_languages_is_empty() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    assert_eq!(data.caption_languages, Vec::<String>::new());
}

#[test]
fn caption_engine_is_empty_string() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    assert_eq!(data.caption_engine, "");
}

#[test]
fn caption_models_is_empty() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    assert_eq!(data.caption_models, Vec::<String>::new());
}

#[test]
fn linked_ffmpeg_version_is_empty_string() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    assert_eq!(data.linked_ffmpeg_version, "");
}

#[test]
fn bundled_fonts_is_empty() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");
    assert_eq!(data.bundled_fonts, Vec::<String>::new());
}

#[test]
fn verb_is_project_agnostic() {
    let prior_a = empty_project();
    let mut prior_b = empty_project();
    prior_b.name = "different-name".to_string();
    prior_b.duration_tk = verbreel_types::Tick::new(123_456);

    let (_, _, data_a) = compute_patch(&prior_a, &args()).expect("happy path a");
    let (_, _, data_b) = compute_patch(&prior_b, &args()).expect("happy path b");

    assert_eq!(data_a, data_b);
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
    let prior = empty_project();
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
        .find(|event| event.verb == "list_capabilities")
        .expect("default_fixtures includes list_capabilities");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(ListCapabilitiesVerb))
        .expect("register list_capabilities verb");

    let report = validate_reconstructors(&registry, &[fixture])
        .expect("list_capabilities reconstruct from fixture");
    assert_eq!(report.verbs_checked, vec!["list_capabilities"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn verb_trait_surface_lookup_via_default_registry() {
    let registry = default_registry();
    let verb = registry
        .get("list_capabilities")
        .expect("list_capabilities registered in default_registry");

    let prior = empty_project();
    let (patch, data, warnings) = verb
        .compute_patch(&prior, &json!({ "project_id": FIXTURE_PROJECT_ID }))
        .expect("default_registry invocation succeeds");

    assert!(patch.is_empty());
    assert!(warnings.is_empty());
    let typed: ListCapabilitiesData =
        serde_json::from_value(data).expect("envelope deserializes to ListCapabilitiesData");
    assert_eq!(typed.tick_rate_hz, 240_000);
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
            "list_capabilities",
            json!({"project_id": FIXTURE_PROJECT_ID}),
            None,
        )
        .expect("list_capabilities should route");

    let MutateOutcome::NoOp { data, warnings, .. } = outcome else {
        panic!("expected NoOp outcome from list_capabilities");
    };
    assert!(warnings.is_empty());

    let data: ListCapabilitiesData =
        serde_json::from_value(data).expect("list_capabilities data deserializes");
    assert_eq!(data.tick_rate_hz, 240_000);
    assert!(!data.verbs.is_empty());
}

#[test]
fn v1_1_fields_are_absent_from_envelope() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");

    let value = serde_json::to_value(&data).expect("envelope → Value");
    let obj = value.as_object().expect("envelope is a JSON object");

    let v1_1_fields = [
        "tracker_algorithms",
        "tracker_model_versions",
        "audio_analysis_algorithms",
        "audio_analysis_features",
        "stock_providers",
        "template_count",
        "preview_session_formats",
    ];
    for field in v1_1_fields {
        assert!(
            !obj.contains_key(field),
            "v1.0 floor must not expose v1.1+ field `{field}`",
        );
    }
    assert_eq!(obj.keys().count(), 13);
}

#[test]
fn data_envelope_keys_match_v1_floor_exactly() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args()).expect("happy path");

    let value: Value = serde_json::to_value(&data).expect("envelope → Value");
    let obj = value.as_object().expect("envelope is a JSON object");

    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();

    let mut expected = vec![
        "bundled_fonts",
        "caption_default_model",
        "caption_engine",
        "caption_languages",
        "caption_models",
        "effects",
        "engine_version",
        "linked_ffmpeg_version",
        "render_presets",
        "supported_schema_versions",
        "tested_ffmpeg_range",
        "tick_rate_hz",
        "verbs",
    ];
    expected.sort_unstable();

    assert_eq!(keys, expected);
}
