//! Tests for `effect.list_available` (§6.5) — twenty-seventh production verb.

use std::sync::Arc;

use serde_json::json;
use verbreel_state::verbs::effect_list_available::{compute_patch, data_envelope_from_args};
use verbreel_state::{
    EffectListAvailableArgs, EffectListAvailableData, EffectListAvailableVerb, MutateOutcome,
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

#[test]
fn compute_patch_without_filter_returns_all_effects() {
    let prior = empty_project();
    let args = EffectListAvailableArgs {
        project_id: fixture_project_id(),
        category: None,
    };

    let (patch, warnings, data) =
        compute_patch(&prior, &args).expect("effect.list_available with no filter should succeed");

    assert_eq!(patch, json!([]));
    assert!(warnings.is_empty());
    assert_eq!(data.effects.len(), 18);
}

#[test]
fn compute_patch_with_video_filter() {
    let prior = empty_project();
    let args = EffectListAvailableArgs {
        project_id: fixture_project_id(),
        category: Some(verbreel_state::verbs::effect_list_available::EffectCategory::Video),
    };

    let (_, _, data) =
        compute_patch(&prior, &args).expect("effect.list_available video filter should succeed");

    let kinds: Vec<&str> = data.effects.iter().map(|e| e.kind.as_str()).collect();
    assert_eq!(
        kinds,
        vec![
            "blur",
            "chroma_key",
            "color_correct",
            "crop",
            "curves",
            "hsl",
            "lut",
            "matte_refine",
            "relight"
        ]
    );
}

#[test]
fn compute_patch_with_audio_filter() {
    let prior = empty_project();
    let args = EffectListAvailableArgs {
        project_id: fixture_project_id(),
        category: Some(verbreel_state::verbs::effect_list_available::EffectCategory::Audio),
    };

    let (_, _, data) =
        compute_patch(&prior, &args).expect("effect.list_available audio filter should succeed");

    let kinds: Vec<&str> = data.effects.iter().map(|e| e.kind.as_str()).collect();
    assert_eq!(
        kinds,
        vec!["compressor", "denoise", "eq", "reverb", "time_stretch"]
    );
}

#[test]
fn compute_patch_with_transition_filter() {
    let prior = empty_project();
    let args = EffectListAvailableArgs {
        project_id: fixture_project_id(),
        category: Some(verbreel_state::verbs::effect_list_available::EffectCategory::Transition),
    };

    let (_, _, data) = compute_patch(&prior, &args)
        .expect("effect.list_available transition filter should succeed");

    let kinds: Vec<&str> = data.effects.iter().map(|e| e.kind.as_str()).collect();
    assert_eq!(
        kinds,
        vec![
            "transition.crossfade",
            "transition.dip_to_black",
            "transition.wipe"
        ],
    );
}

#[test]
fn compute_patch_with_text_filter() {
    let prior = empty_project();
    let args = EffectListAvailableArgs {
        project_id: fixture_project_id(),
        category: Some(verbreel_state::verbs::effect_list_available::EffectCategory::Text),
    };

    let (_, _, data) =
        compute_patch(&prior, &args).expect("effect.list_available text filter should succeed");

    let kinds: Vec<&str> = data.effects.iter().map(|e| e.kind.as_str()).collect();
    assert_eq!(kinds, vec!["burned_caption"]);
}

#[test]
fn compute_patch_sorted_by_kind_alphabetical() {
    let prior = empty_project();
    let args = EffectListAvailableArgs {
        project_id: fixture_project_id(),
        category: None,
    };

    let (_, _, data) =
        compute_patch(&prior, &args).expect("effect.list_available without filter should succeed");

    let sorted_kinds = data
        .effects
        .iter()
        .map(|e| e.kind.clone())
        .collect::<Vec<_>>();
    let mut expected = sorted_kinds.clone();
    expected.sort_unstable();

    assert_eq!(sorted_kinds, expected);
}

#[test]
fn each_effect_entry_has_non_empty_fields() {
    let prior = empty_project();
    let args = EffectListAvailableArgs {
        project_id: fixture_project_id(),
        category: None,
    };

    let (_, _, data) =
        compute_patch(&prior, &args).expect("effect.list_available without filter should succeed");

    for effect in &data.effects {
        assert!(!effect.kind.is_empty());
        assert!(!effect.category.is_empty());
        assert!(!effect.summary.is_empty());
        assert!(!effect.params_schema_id.is_empty());
    }
}

#[test]
fn invalid_category_string_maps_to_bad_args() {
    let prior = empty_project();
    let verb = EffectListAvailableVerb;

    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "category": "videoe",
            }),
        )
        .expect_err("invalid category should map to BadArgs");

    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn compute_patch_patch_always_empty() {
    let prior = empty_project();
    let args = EffectListAvailableArgs {
        project_id: fixture_project_id(),
        category: None,
    };

    let (patch, _, _) =
        compute_patch(&prior, &args).expect("effect.list_available without filter should succeed");

    assert_eq!(patch, json!([]));
}

#[test]
fn compute_patch_warnings_always_empty() {
    let prior = empty_project();
    let args = EffectListAvailableArgs {
        project_id: fixture_project_id(),
        category: None,
    };

    let (_, warnings, _) =
        compute_patch(&prior, &args).expect("effect.list_available without filter should succeed");

    assert!(warnings.is_empty());
}

#[test]
fn params_schema_id_is_effect_params_dot_kind_at_version() {
    let prior = empty_project();
    let args = EffectListAvailableArgs {
        project_id: fixture_project_id(),
        category: None,
    };

    let (_, _, data) =
        compute_patch(&prior, &args).expect("effect.list_available without filter should succeed");

    for effect in &data.effects {
        let (schema_prefix, tail) = effect
            .params_schema_id
            .split_once('.')
            .expect("params_schema_id format includes dot");
        assert_eq!(schema_prefix, "EffectParams");

        let (schema_kind, schema_version) = tail
            .split_once('@')
            .expect("params_schema_id format includes @version");
        assert_eq!(schema_kind, effect.kind);
        assert!(!schema_version.is_empty());
    }
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "effect.list_available")
        .expect("default_fixtures includes effect.list_available");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(EffectListAvailableVerb))
        .expect("register effect.list_available verb");

    let report = validate_reconstructors(&registry, &[fixture])
        .expect("effect.list_available reconstruct from fixture");
    assert_eq!(report.verbs_checked, vec!["effect.list_available"]);
    assert_eq!(report.fixtures_run, 1);
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
            "effect.list_available",
            json!({"project_id": FIXTURE_PROJECT_ID}),
            None,
        )
        .expect("effect.list_available should route");

    let MutateOutcome::NoOp { data, warnings, .. } = outcome else {
        panic!("expected NoOp outcome from effect.list_available");
    };
    assert!(warnings.is_empty());

    let data: EffectListAvailableData =
        serde_json::from_value(data).expect("effect.list_available data deserializes");
    assert_eq!(data.effects.len(), 18);
}

#[test]
fn data_envelope_from_args_rebuilds_from_post_state() {
    let prior = empty_project();
    let args = EffectListAvailableArgs {
        project_id: fixture_project_id(),
        category: None,
    };

    let (patch, _, expected) = compute_patch(&prior, &args).expect("compute patch");
    let patch: json_patch::Patch = serde_json::from_value(patch).expect("valid patch type");
    let post_state = prior
        .apply(&patch)
        .expect("applying empty patch to empty project");

    let envelope = data_envelope_from_args(&args, &post_state)
        .expect("data envelope from post state should rebuild same data");
    assert_eq!(envelope, expected);
}
