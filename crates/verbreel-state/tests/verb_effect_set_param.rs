//! Tests for `effect.set_param` (§6.3) — forty-third production verb.

use std::sync::Arc;

use serde_json::{Map, Value, json};
use verbreel_state::verbs::effect_set_param::{
    W_NOOP_CODE, compute_patch, data_envelope_from_post_state,
};
use verbreel_state::{
    EFFECT_PARAMS_MAX_BYTES, EFFECT_PARAMS_MAX_KEYS, EffectSetParamArgs, EffectSetParamData,
    EffectSetParamError, EffectSetParamVerb, MutateOutcome, Project, RecordedEvent, Track, Verb,
    VerbError, VerbRegistry, default_fixtures, default_registry, validate_reconstructors,
};
use verbreel_types::ProjectId;

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const TRACK_TEXT_A: &str = "01900000-0000-7000-8000-0000000aa401";
const CLIP_TEXT_A: &str = "01900000-0000-7000-8000-0000000bb401";
const EFFECT_A: &str = "01900000-0000-7000-8000-0000000cc401";
const MISSING_EFFECT: &str = "01900000-0000-7000-8000-0000000dd401";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture -> Project")
}

fn fixture_project_id() -> ProjectId {
    empty_project().id
}

fn text_effect(id: &str, kind: &str, params: Map<String, Value>) -> Value {
    json!({
        "id": id,
        "kind": kind,
        "enabled": true,
        "params": Value::Object(params),
    })
}

fn text_clip(locked: bool, effects: Vec<Value>) -> Value {
    json!({
        "id": CLIP_TEXT_A,
        "name": "Clip",
        "asset_id": "00000000-0000-0000-0000-000000000000",
        "track_position_tk": 0,
        "source_in_tk": 0,
        "source_out_tk": 480_000,
        "locked": locked,
        "text": {
            "content": "Hello",
            "font_family": "Arial",
            "font_size_px": 24,
        },
        "effects": effects,
    })
}

fn text_track(track_locked: bool, clip_locked: bool, effects: Vec<Value>) -> Track {
    serde_json::from_value(json!({
        "id": TRACK_TEXT_A,
        "kind": "text",
        "name": "Text 1",
        "locked": track_locked,
        "clips": [text_clip(clip_locked, effects)],
    }))
    .expect("text track fixture parses")
}

fn project_with_track(track_locked: bool, clip_locked: bool, effects: Vec<Value>) -> Project {
    let mut project = empty_project();
    project.tracks = vec![text_track(track_locked, clip_locked, effects)];
    project.duration_tk = verbreel_types::Tick::new(480_000);
    project
}

fn project_with_effect(kind: &str, params: Map<String, Value>) -> Project {
    project_with_track(false, false, vec![text_effect(EFFECT_A, kind, params)])
}

fn args_with(params: Map<String, Value>) -> EffectSetParamArgs {
    EffectSetParamArgs {
        project_id: fixture_project_id(),
        effect: EFFECT_A.to_string(),
        params,
    }
}

fn map(entries: &[(&str, Value)]) -> Map<String, Value> {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_string(), value.clone()))
        .collect()
}

fn apply_patch(prior: &Project, patch: Value) -> Project {
    let patch: json_patch::Patch = serde_json::from_value(patch).expect("patch parses");
    prior.apply(&patch).expect("patch applies")
}

fn patched_params(patch: &Value) -> &Map<String, Value> {
    let ops = patch.as_array().expect("patch is array");
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0]["op"], "replace");
    assert_eq!(ops[0]["path"], "/tracks/0/clips/0/effects/0/params");
    ops[0]["value"].as_object().expect("params value is object")
}

fn effect_params(project: &Project) -> &Map<String, Value> {
    &project.tracks[0].clips[0].effects[0].params
}

#[test]
fn compute_patch_adds_new_param_to_empty_params() {
    let prior = project_with_effect("color_correct", Map::new());
    let args = args_with(map(&[("brightness", json!(0.2))]));

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("happy path");

    assert!(warnings.is_empty());
    assert_eq!(data.effect_id.to_string(), EFFECT_A);
    assert_eq!(data.params, map(&[("brightness", json!(0.2))]));
    assert_eq!(patched_params(&patch), &data.params);

    let post = apply_patch(&prior, patch);
    assert_eq!(effect_params(&post), &data.params);
}

#[test]
fn compute_patch_updates_existing_key_and_preserves_others() {
    let prior = project_with_effect(
        "eq",
        map(&[("gain", json!(1.0)), ("frequency", json!(440.0))]),
    );
    let args = args_with(map(&[("gain", json!(1.5))]));

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("happy path");

    assert!(warnings.is_empty());
    assert_eq!(
        data.params,
        map(&[("gain", json!(1.5)), ("frequency", json!(440.0))])
    );
    assert_eq!(patched_params(&patch), &data.params);
}

#[test]
fn compute_patch_merges_multi_key_params_object() {
    let prior = project_with_effect("color_correct", map(&[("brightness", json!(0.1))]));
    let args = args_with(map(&[("contrast", json!(1.2)), ("saturation", json!(0.8))]));

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("happy path");

    assert!(warnings.is_empty());
    assert_eq!(
        data.params,
        map(&[
            ("brightness", json!(0.1)),
            ("contrast", json!(1.2)),
            ("saturation", json!(0.8)),
        ])
    );
    assert_eq!(patched_params(&patch), &data.params);
}

#[test]
fn compute_patch_setting_one_key_does_not_touch_other_keys() {
    let prior = project_with_effect(
        "color_correct",
        map(&[
            ("lift", json!(0.1)),
            ("gamma", json!(1.0)),
            ("gain", json!(1.2)),
        ]),
    );
    let args = args_with(map(&[("gamma", json!(0.95))]));

    let (patch, _warnings, data) = compute_patch(&prior, &args).expect("happy path");
    let post = apply_patch(&prior, patch);

    assert_eq!(effect_params(&post), &data.params);
    assert_eq!(effect_params(&post)["lift"], json!(0.1));
    assert_eq!(effect_params(&post)["gamma"], json!(0.95));
    assert_eq!(effect_params(&post)["gain"], json!(1.2));
}

#[test]
fn compute_patch_missing_effect_returns_not_found() {
    let prior = project_with_effect("eq", Map::new());
    let mut args = args_with(map(&[("gain", json!(2.0))]));
    args.effect = MISSING_EFFECT.to_string();

    let err = compute_patch(&prior, &args).expect_err("missing effect rejects");

    match err {
        EffectSetParamError::EffectNotFound { effect_id } => assert_eq!(effect_id, MISSING_EFFECT),
        other => panic!("expected EffectNotFound, got {other:?}"),
    }
}

#[test]
fn compute_patch_malformed_effect_uuid_returns_bad_selector() {
    let prior = project_with_effect("eq", Map::new());
    let mut args = args_with(map(&[("gain", json!(2.0))]));
    args.effect = "not-a-uuid".to_string();

    let err = compute_patch(&prior, &args).expect_err("bad selector rejects");

    match err {
        EffectSetParamError::BadSelector { detail } => assert!(detail.contains("UUID")),
        other => panic!("expected BadSelector, got {other:?}"),
    }
}

#[test]
fn compute_patch_locked_clip_returns_locked() {
    let prior = project_with_track(
        false,
        true,
        vec![text_effect(EFFECT_A, "eq", map(&[("gain", json!(1.0))]))],
    );

    let err = compute_patch(&prior, &args_with(map(&[("gain", json!(2.0))])))
        .expect_err("locked clip rejects");

    match err {
        EffectSetParamError::Locked { kind, id, .. } => {
            assert_eq!(kind, "clip");
            assert_eq!(id, CLIP_TEXT_A);
        }
        other => panic!("expected Locked, got {other:?}"),
    }
}

#[test]
fn compute_patch_locked_track_returns_locked() {
    let prior = project_with_track(
        true,
        false,
        vec![text_effect(EFFECT_A, "eq", map(&[("gain", json!(1.0))]))],
    );

    let err = compute_patch(&prior, &args_with(map(&[("gain", json!(2.0))])))
        .expect_err("locked track rejects");

    match err {
        EffectSetParamError::Locked { kind, id, .. } => {
            assert_eq!(kind, "track");
            assert_eq!(id, TRACK_TEXT_A);
        }
        other => panic!("expected Locked, got {other:?}"),
    }
}

#[test]
fn compute_patch_managed_time_stretch_returns_managed_error() {
    let prior = project_with_effect("time_stretch", map(&[("factor", json!(1.25))]));

    let err = compute_patch(&prior, &args_with(map(&[("factor", json!(1.5))])))
        .expect_err("managed effect rejects");

    match err {
        EffectSetParamError::ManagedEffect {
            managing_verb,
            hint,
            ..
        } => {
            assert_eq!(managing_verb, "clip.set_speed");
            assert_eq!(
                hint,
                "call clip.set_speed --preserve_pitch false on the parent clip"
            );
        }
        other => panic!("expected ManagedEffect, got {other:?}"),
    }
}

#[test]
fn compute_patch_managed_burned_caption_returns_managed_error() {
    let prior = project_with_effect("burned_caption", map(&[("opacity", json!(1.0))]));

    let err = compute_patch(&prior, &args_with(map(&[("opacity", json!(0.5))])))
        .expect_err("managed effect rejects");

    match err {
        EffectSetParamError::ManagedEffect { managing_verb, .. } => {
            assert_eq!(managing_verb, "caption.burn_in")
        }
        other => panic!("expected ManagedEffect, got {other:?}"),
    }
}

#[test]
fn compute_patch_managed_denoise_returns_managed_error() {
    let prior = project_with_effect("denoise", map(&[("strength", json!(0.8))]));

    let err = compute_patch(&prior, &args_with(map(&[("strength", json!(0.3))])))
        .expect_err("managed effect rejects");

    match err {
        EffectSetParamError::ManagedEffect { managing_verb, .. } => {
            assert_eq!(managing_verb, "audio.denoise")
        }
        other => panic!("expected ManagedEffect, got {other:?}"),
    }
}

#[test]
fn compute_patch_post_merge_over_64_keys_returns_bad_params() {
    let prior_params: Map<String, Value> = (0..EFFECT_PARAMS_MAX_KEYS)
        .map(|idx| (format!("k{idx}"), json!(idx)))
        .collect();
    let prior = project_with_effect("eq", prior_params);

    let err = compute_patch(&prior, &args_with(map(&[("overflow", json!(true))])))
        .expect_err("key cap rejects");

    match err {
        EffectSetParamError::BadParams {
            field,
            bound,
            value,
        } => {
            assert_eq!(field, "params.keys");
            assert_eq!(bound, EFFECT_PARAMS_MAX_KEYS);
            assert_eq!(value, EFFECT_PARAMS_MAX_KEYS + 1);
        }
        other => panic!("expected BadParams, got {other:?}"),
    }
}

#[test]
fn compute_patch_post_merge_over_16384_bytes_returns_bad_params() {
    let prior = project_with_effect("eq", Map::new());
    let args = args_with(map(&[("blob", json!("x".repeat(EFFECT_PARAMS_MAX_BYTES)))]));

    let err = compute_patch(&prior, &args).expect_err("byte cap rejects");

    match err {
        EffectSetParamError::BadParams {
            field,
            bound,
            value,
        } => {
            assert_eq!(field, "params.bytes");
            assert_eq!(bound, EFFECT_PARAMS_MAX_BYTES);
            assert!(value > EFFECT_PARAMS_MAX_BYTES, "{value}");
        }
        other => panic!("expected BadParams, got {other:?}"),
    }
}

#[test]
fn compute_patch_same_params_emits_w_noop() {
    let prior = project_with_effect("eq", map(&[("gain", json!(1.0))]));

    let (patch, warnings, data) =
        compute_patch(&prior, &args_with(map(&[("gain", json!(1.0))]))).expect("no-op");

    assert!(patch.as_array().expect("patch is array").is_empty());
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_NOOP_CODE);
    assert_eq!(warnings[0]["details"]["effect_id"], EFFECT_A);
    assert_eq!(warnings[0]["details"]["message"], "effect.set_param no-op");
    assert_eq!(data.params, map(&[("gain", json!(1.0))]));
}

#[test]
fn compute_patch_empty_merge_emits_w_noop() {
    let prior = project_with_effect("eq", map(&[("gain", json!(1.0))]));

    let (patch, warnings, data) = compute_patch(&prior, &args_with(Map::new())).expect("no-op");

    assert!(patch.as_array().expect("patch is array").is_empty());
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_NOOP_CODE);
    assert_eq!(data.params, map(&[("gain", json!(1.0))]));
}

#[test]
fn data_envelope_reads_post_state_params() {
    let post_state = project_with_effect(
        "eq",
        map(&[("gain", json!(1.5)), ("frequency", json!(440.0))]),
    );
    let args = args_with(map(&[("gain", json!(1.5))]));

    let data = data_envelope_from_post_state(&args, &post_state)
        .expect("post-state envelope should read effect params");

    assert_eq!(data.effect_id.to_string(), EFFECT_A);
    assert_eq!(
        data.params,
        map(&[("gain", json!(1.5)), ("frequency", json!(440.0))])
    );
}

#[test]
fn reconstructor_round_trip() {
    let prior = project_with_effect("eq", map(&[("gain", json!(1.0))]));
    let args = args_with(map(&[("gain", json!(1.5)), ("frequency", json!(440.0))]));

    let (patch, warnings, recorded) = compute_patch(&prior, &args).expect("compute patch");
    let typed_patch: json_patch::Patch =
        serde_json::from_value(patch.clone()).expect("patch parses to typed JSON patch");
    let post_state = prior.apply(&typed_patch).expect("patch applies");
    let reconstructed =
        data_envelope_from_post_state(&args, &post_state).expect("reconstructs from post-state");
    assert_eq!(recorded, reconstructed);

    let expected_data = serde_json::to_value(reconstructed).expect("envelope serializes");
    let recorded = RecordedEvent {
        verb: "effect.set_param".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch,
        warnings,
        post_state,
        expected_data,
    };

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(EffectSetParamVerb))
        .expect("register effect.set_param verb");

    let report = validate_reconstructors(&registry, &[recorded]).expect("validation passes");
    assert_eq!(report.verbs_checked, vec!["effect.set_param"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "effect.set_param")
        .expect("default_fixtures includes effect.set_param");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(EffectSetParamVerb))
        .expect("register effect.set_param verb");

    let report =
        validate_reconstructors(&registry, &[fixture]).expect("reconstruction from fixture");
    assert_eq!(report.verbs_checked, vec!["effect.set_param"]);
}

#[test]
fn verb_boundary_rejects_scalar_params_as_bad_args() {
    let prior = project_with_effect("eq", Map::new());
    let verb = EffectSetParamVerb;

    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "effect": EFFECT_A,
                "params": 1,
            }),
        )
        .expect_err("scalar params reject during args deserialize");

    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn compute_patch_error_variants_map_to_bad_args() {
    let prior = project_with_effect("time_stretch", map(&[("factor", json!(1.0))]));
    let verb = EffectSetParamVerb;

    for args in [
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "effect": "not-a-uuid",
            "params": { "factor": 1.5 },
        }),
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "effect": MISSING_EFFECT,
            "params": { "factor": 1.5 },
        }),
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "effect": EFFECT_A,
            "params": { "factor": 1.5 },
        }),
    ] {
        let err = verb
            .compute_patch(&prior, &args)
            .expect_err("error variant maps to bad args");
        assert!(matches!(err, VerbError::BadArgs { .. }));
    }
}

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let base_project = project_with_effect("eq", map(&[("gain", json!(1.0))]));
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        base_project,
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry clears the gate");

    let outcome = store
        .mutate_via_verb(
            "effect.set_param",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "effect": EFFECT_A,
                "params": { "gain": 1.5, "frequency": 440.0 },
            }),
            None,
        )
        .expect("mutate_via_verb happy path");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("happy path must return Applied, got {outcome:?}");
    };

    let data: EffectSetParamData =
        serde_json::from_value(data).expect("effect.set_param data parses");
    assert_eq!(data.effect_id.to_string(), EFFECT_A);
    assert_eq!(data.params["gain"], json!(1.5));
    assert_eq!(data.params["frequency"], json!(440.0));
    assert_eq!(effect_params(store.project()), &data.params);
    assert_eq!(warnings, Vec::<Value>::new());
}
