//! Tests for `effect.remove` (§6.2) — forty-fourth production verb.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::effect_remove::{
    W_KEYFRAMES_REMOVED_CODE, compute_patch, data_envelope_from_args_warnings,
};
use verbreel_state::{
    EffectRemoveArgs, EffectRemoveData, EffectRemoveError, EffectRemoveVerb, MutateOutcome,
    Project, RecordedEvent, Track, Verb, VerbError, VerbRegistry, default_fixtures,
    default_registry, validate_reconstructors,
};
use verbreel_types::ProjectId;

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const TRACK_TEXT_A: &str = "01900000-0000-7000-8000-0000000aa501";
const CLIP_TEXT_A: &str = "01900000-0000-7000-8000-0000000bb501";
const EFFECT_A: &str = "01900000-0000-7000-8000-0000000cc501";
const EFFECT_B: &str = "01900000-0000-7000-8000-0000000cc502";
const EFFECT_C: &str = "01900000-0000-7000-8000-0000000cc503";
const KEYFRAME_A: &str = "01900000-0000-7000-8000-0000000dd501";
const KEYFRAME_B: &str = "01900000-0000-7000-8000-0000000dd502";
const KEYFRAME_C: &str = "01900000-0000-7000-8000-0000000dd503";
const KEYFRAME_D: &str = "01900000-0000-7000-8000-0000000dd504";
const MISSING_EFFECT: &str = "01900000-0000-7000-8000-0000000ee501";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture -> Project")
}

fn fixture_project_id() -> ProjectId {
    empty_project().id
}

fn text_effect(id: &str, kind: &str) -> Value {
    json!({
        "id": id,
        "kind": kind,
        "enabled": true,
        "params": { "amount": 1 },
    })
}

fn keyframe(id: &str, property: &str, time_tk: u64, value: Value) -> Value {
    json!({
        "id": id,
        "property": property,
        "time_tk": time_tk,
        "value": value,
        "easing": "linear",
    })
}

fn target_property(effect: &str, key: &str) -> String {
    format!("effects[{effect}].params.{key}")
}

fn text_clip(locked: bool, effects: Vec<Value>, keyframes: Vec<Value>) -> Value {
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
        "keyframes": keyframes,
    })
}

fn text_track(
    track_locked: bool,
    clip_locked: bool,
    effects: Vec<Value>,
    keyframes: Vec<Value>,
) -> Track {
    serde_json::from_value(json!({
        "id": TRACK_TEXT_A,
        "kind": "text",
        "name": "Text 1",
        "locked": track_locked,
        "clips": [text_clip(clip_locked, effects, keyframes)],
    }))
    .expect("text track fixture parses")
}

fn project_with_track(
    track_locked: bool,
    clip_locked: bool,
    effects: Vec<Value>,
    keyframes: Vec<Value>,
) -> Project {
    let mut project = empty_project();
    project.tracks = vec![text_track(track_locked, clip_locked, effects, keyframes)];
    project.duration_tk = verbreel_types::Tick::new(480_000);
    project
}

fn project_with_effect(kind: &str, keyframes: Vec<Value>) -> Project {
    project_with_track(false, false, vec![text_effect(EFFECT_A, kind)], keyframes)
}

fn three_effect_project(keyframes: Vec<Value>) -> Project {
    project_with_track(
        false,
        false,
        vec![
            text_effect(EFFECT_A, "blur"),
            text_effect(EFFECT_B, "sharpen"),
            text_effect(EFFECT_C, "glow"),
        ],
        keyframes,
    )
}

fn args(effect: &str) -> EffectRemoveArgs {
    EffectRemoveArgs {
        project_id: fixture_project_id(),
        effect: effect.to_string(),
    }
}

fn apply_patch(prior: &Project, patch: Value) -> Project {
    let patch: json_patch::Patch = serde_json::from_value(patch).expect("patch parses");
    prior.apply(&patch).expect("patch applies")
}

fn effect_order(project: &Project) -> Vec<String> {
    project.tracks[0].clips[0]
        .effects
        .iter()
        .map(|effect| effect.id.to_string())
        .collect()
}

fn keyframe_ids(project: &Project) -> Vec<String> {
    project.tracks[0].clips[0]
        .keyframes
        .iter()
        .map(|keyframe| keyframe.id.to_string())
        .collect()
}

fn warning_removed_ids(warning: &Value) -> Vec<String> {
    warning["details"]["removed_keyframe_ids"]
        .as_array()
        .expect("removed ids array")
        .iter()
        .map(|value| value.as_str().expect("id string").to_string())
        .collect()
}

#[test]
fn compute_patch_removes_effect_with_no_keyframes() {
    let prior = three_effect_project(Vec::new());

    let (patch, warnings, data) = compute_patch(&prior, &args(EFFECT_B)).expect("happy path");

    assert!(warnings.is_empty());
    assert_eq!(data.removed_effect_id.to_string(), EFFECT_B);
    assert!(data.removed_keyframe_ids.is_empty());
    assert_eq!(patch.as_array().expect("patch is array").len(), 1);

    let post = apply_patch(&prior, patch);
    assert_eq!(effect_order(&post), vec![EFFECT_A, EFFECT_C]);
    assert!(keyframe_ids(&post).is_empty());
}

#[test]
fn compute_patch_removes_effect_and_two_targeting_keyframes() {
    let prior = three_effect_project(vec![
        keyframe(
            KEYFRAME_A,
            &target_property(EFFECT_B, "amount"),
            10,
            json!(0.1),
        ),
        keyframe(
            KEYFRAME_B,
            &target_property(EFFECT_B, "mix"),
            20,
            json!(0.2),
        ),
    ]);

    let (patch, warnings, data) = compute_patch(&prior, &args(EFFECT_B)).expect("happy path");

    assert_eq!(data.removed_effect_id.to_string(), EFFECT_B);
    assert_eq!(
        stringify_data_ids(&data),
        vec![KEYFRAME_A.to_string(), KEYFRAME_B.to_string()]
    );
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_KEYFRAMES_REMOVED_CODE);
    assert_eq!(warnings[0]["details"]["clip_id"], CLIP_TEXT_A);
    assert_eq!(
        warning_removed_ids(&warnings[0]),
        vec![KEYFRAME_A.to_string(), KEYFRAME_B.to_string()]
    );
    assert_eq!(patch.as_array().expect("patch is array").len(), 2);

    let post = apply_patch(&prior, patch);
    assert_eq!(effect_order(&post), vec![EFFECT_A, EFFECT_C]);
    assert!(keyframe_ids(&post).is_empty());
}

#[test]
fn keyframes_targeting_other_effects_or_clip_properties_survive() {
    let prior = three_effect_project(vec![
        keyframe(
            KEYFRAME_A,
            &target_property(EFFECT_B, "amount"),
            10,
            json!(0.1),
        ),
        keyframe(
            KEYFRAME_B,
            &target_property(EFFECT_A, "amount"),
            20,
            json!(0.2),
        ),
        keyframe(KEYFRAME_C, "opacity", 30, json!(0.5)),
        keyframe(KEYFRAME_D, "transform.x", 40, json!(12)),
    ]);

    let (patch, warnings, data) = compute_patch(&prior, &args(EFFECT_B)).expect("happy path");
    let post = apply_patch(&prior, patch);

    assert_eq!(stringify_data_ids(&data), vec![KEYFRAME_A.to_string()]);
    assert_eq!(
        warning_removed_ids(&warnings[0]),
        vec![KEYFRAME_A.to_string()]
    );
    assert_eq!(
        keyframe_ids(&post),
        vec![
            KEYFRAME_B.to_string(),
            KEYFRAME_C.to_string(),
            KEYFRAME_D.to_string()
        ]
    );
}

#[test]
fn compute_patch_unknown_effect_returns_not_found() {
    let prior = three_effect_project(Vec::new());

    let err = compute_patch(&prior, &args(MISSING_EFFECT)).expect_err("missing effect rejects");

    match err {
        EffectRemoveError::EffectNotFound { effect_id } => assert_eq!(effect_id, MISSING_EFFECT),
        other => panic!("expected EffectNotFound, got {other:?}"),
    }
}

#[test]
fn compute_patch_malformed_effect_uuid_returns_bad_selector() {
    let prior = three_effect_project(Vec::new());

    let err = compute_patch(&prior, &args("not-a-uuid")).expect_err("bad selector rejects");

    match err {
        EffectRemoveError::BadSelector { detail } => assert!(detail.contains("UUID")),
        other => panic!("expected BadSelector, got {other:?}"),
    }
}

#[test]
fn compute_patch_locked_clip_returns_locked() {
    let prior = project_with_track(false, true, vec![text_effect(EFFECT_A, "blur")], Vec::new());

    let err = compute_patch(&prior, &args(EFFECT_A)).expect_err("locked clip rejects");

    match err {
        EffectRemoveError::Locked { kind, id, .. } => {
            assert_eq!(kind, "clip");
            assert_eq!(id, CLIP_TEXT_A);
        }
        other => panic!("expected Locked, got {other:?}"),
    }
}

#[test]
fn compute_patch_locked_track_returns_locked() {
    let prior = project_with_track(true, false, vec![text_effect(EFFECT_A, "blur")], Vec::new());

    let err = compute_patch(&prior, &args(EFFECT_A)).expect_err("locked track rejects");

    match err {
        EffectRemoveError::Locked { kind, id, .. } => {
            assert_eq!(kind, "track");
            assert_eq!(id, TRACK_TEXT_A);
        }
        other => panic!("expected Locked, got {other:?}"),
    }
}

#[test]
fn compute_patch_managed_time_stretch_returns_managed_error() {
    let prior = project_with_effect("time_stretch", Vec::new());

    let err = compute_patch(&prior, &args(EFFECT_A)).expect_err("managed effect rejects");

    match err {
        EffectRemoveError::ManagedEffect {
            kind,
            managing_verb,
            hint,
        } => {
            assert_eq!(kind, "time_stretch");
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
    let prior = project_with_effect("burned_caption", Vec::new());

    let err = compute_patch(&prior, &args(EFFECT_A)).expect_err("managed effect rejects");

    match err {
        EffectRemoveError::ManagedEffect {
            kind,
            managing_verb,
            hint,
        } => {
            assert_eq!(kind, "burned_caption");
            assert_eq!(managing_verb, "caption.burn_in");
            assert_eq!(
                hint,
                "call caption.burn_off to remove the effect while keeping the source text track, or track.remove on the source text track to cascade-remove both, or effect.toggle --enabled false to disable without removing — see §10.5"
            );
        }
        other => panic!("expected ManagedEffect, got {other:?}"),
    }
}

#[test]
fn compute_patch_managed_denoise_returns_managed_error() {
    let prior = project_with_effect("denoise", Vec::new());

    let err = compute_patch(&prior, &args(EFFECT_A)).expect_err("managed effect rejects");

    match err {
        EffectRemoveError::ManagedEffect {
            kind,
            managing_verb,
            hint,
        } => {
            assert_eq!(kind, "denoise");
            assert_eq!(managing_verb, "audio.denoise");
            assert_eq!(
                hint,
                "call audio.denoise --strength 0 on the target to remove, or effect.toggle --enabled false to disable without removing"
            );
        }
        other => panic!("expected ManagedEffect, got {other:?}"),
    }
}

#[test]
fn effects_order_is_preserved_for_non_removed_siblings() {
    let prior = three_effect_project(Vec::new());

    let (patch, _warnings, _data) = compute_patch(&prior, &args(EFFECT_B)).expect("happy path");
    let post = apply_patch(&prior, patch);

    assert_eq!(effect_order(&post), vec![EFFECT_A, EFFECT_C]);
}

#[test]
fn data_envelope_contains_removed_effect_and_keyframes() {
    let prior = three_effect_project(vec![keyframe(
        KEYFRAME_A,
        &target_property(EFFECT_B, "amount"),
        10,
        json!(0.1),
    )]);

    let (_patch, _warnings, data) = compute_patch(&prior, &args(EFFECT_B)).expect("happy path");

    assert_eq!(data.removed_effect_id.to_string(), EFFECT_B);
    assert_eq!(stringify_data_ids(&data), vec![KEYFRAME_A.to_string()]);
}

#[test]
fn reconstructor_round_trip_with_cascade_warning() {
    let prior = three_effect_project(vec![
        keyframe(
            KEYFRAME_A,
            &target_property(EFFECT_B, "amount"),
            10,
            json!(0.1),
        ),
        keyframe(
            KEYFRAME_B,
            &target_property(EFFECT_B, "mix"),
            20,
            json!(0.2),
        ),
    ]);
    let args = args(EFFECT_B);

    let (patch, warnings, recorded_data) = compute_patch(&prior, &args).expect("compute patch");
    let typed_patch: json_patch::Patch =
        serde_json::from_value(patch.clone()).expect("patch parses to typed JSON patch");
    let post_state = prior.apply(&typed_patch).expect("patch applies");
    let reconstructed =
        data_envelope_from_args_warnings(&args, &warnings).expect("reconstructs from warnings");
    assert_eq!(recorded_data, reconstructed);

    let expected_data = serde_json::to_value(reconstructed).expect("envelope serializes");
    let recorded = RecordedEvent {
        verb: "effect.remove".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch,
        warnings,
        post_state,
        expected_data,
    };

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(EffectRemoveVerb))
        .expect("register effect.remove verb");

    let report = validate_reconstructors(&registry, &[recorded]).expect("validation passes");
    assert_eq!(report.verbs_checked, vec!["effect.remove"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn reconstructor_round_trip_with_no_cascade_returns_empty_keyframes() {
    let prior = three_effect_project(Vec::new());
    let args = args(EFFECT_B);

    let (patch, warnings, recorded_data) = compute_patch(&prior, &args).expect("compute patch");
    let typed_patch: json_patch::Patch =
        serde_json::from_value(patch).expect("patch parses to typed JSON patch");
    let post_state = prior.apply(&typed_patch).expect("patch applies");
    let verb = EffectRemoveVerb;

    let reconstructed = verb
        .reconstruct(
            &serde_json::to_value(&args).expect("args serialize"),
            &json!([]),
            &warnings,
            &post_state,
        )
        .expect("verb reconstructs");
    let reconstructed: EffectRemoveData =
        serde_json::from_value(reconstructed).expect("effect.remove data parses");

    assert_eq!(recorded_data, reconstructed);
    assert!(reconstructed.removed_keyframe_ids.is_empty());
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "effect.remove")
        .expect("default_fixtures includes effect.remove");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(EffectRemoveVerb))
        .expect("register effect.remove verb");

    let report =
        validate_reconstructors(&registry, &[fixture]).expect("reconstruction from fixture");
    assert_eq!(report.verbs_checked, vec!["effect.remove"]);
}

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let base_project = three_effect_project(vec![keyframe(
        KEYFRAME_A,
        &target_property(EFFECT_B, "amount"),
        10,
        json!(0.1),
    )]);
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        base_project,
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry clears the gate");

    let outcome = store
        .mutate_via_verb(
            "effect.remove",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "effect": EFFECT_B,
            }),
            None,
        )
        .expect("mutate_via_verb happy path");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("happy path must return Applied, got {outcome:?}");
    };

    let data: EffectRemoveData = serde_json::from_value(data).expect("effect.remove data parses");
    assert_eq!(data.removed_effect_id.to_string(), EFFECT_B);
    assert_eq!(stringify_data_ids(&data), vec![KEYFRAME_A.to_string()]);
    assert_eq!(warnings[0]["code"], W_KEYFRAMES_REMOVED_CODE);
    assert_eq!(effect_order(store.project()), vec![EFFECT_A, EFFECT_C]);
    assert!(keyframe_ids(store.project()).is_empty());
}

#[test]
fn verb_boundary_rejects_missing_effect_arg_as_bad_args() {
    let prior = three_effect_project(Vec::new());
    let verb = EffectRemoveVerb;

    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
            }),
        )
        .expect_err("missing effect rejects during args deserialize");

    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn compute_patch_error_variants_map_to_bad_args() {
    let prior = project_with_effect("time_stretch", Vec::new());
    let verb = EffectRemoveVerb;

    for args in [
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "effect": "not-a-uuid",
        }),
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "effect": MISSING_EFFECT,
        }),
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "effect": EFFECT_A,
        }),
    ] {
        let err = verb
            .compute_patch(&prior, &args)
            .expect_err("error variant maps to bad args");
        assert!(matches!(err, VerbError::BadArgs { .. }));
    }
}

#[test]
fn data_envelope_without_warning_has_empty_removed_keyframe_ids() {
    let data = data_envelope_from_args_warnings(&args(EFFECT_A), &[])
        .expect("envelope reconstructs without warnings");

    assert_eq!(data.removed_effect_id.to_string(), EFFECT_A);
    assert!(data.removed_keyframe_ids.is_empty());
}

#[test]
fn data_envelope_reads_removed_keyframe_ids_from_warning() {
    let warning = json!({
        "code": W_KEYFRAMES_REMOVED_CODE,
        "message": "effect keyframes targeting the removed effect were removed",
        "details": {
            "clip_id": CLIP_TEXT_A,
            "removed_keyframe_ids": [KEYFRAME_B, KEYFRAME_A],
        }
    });

    let data = data_envelope_from_args_warnings(&args(EFFECT_A), &[warning])
        .expect("envelope reconstructs from warning");

    assert_eq!(
        stringify_data_ids(&data),
        vec![KEYFRAME_A.to_string(), KEYFRAME_B.to_string()]
    );
}

#[test]
fn data_envelope_rejects_malformed_warning_removed_keyframe_id() {
    let warning = json!({
        "code": W_KEYFRAMES_REMOVED_CODE,
        "message": "effect keyframes targeting the removed effect were removed",
        "details": {
            "clip_id": CLIP_TEXT_A,
            "removed_keyframe_ids": ["not-a-uuid"],
        }
    });

    let err = data_envelope_from_args_warnings(&args(EFFECT_A), &[warning])
        .expect_err("malformed warning rejects");

    assert!(err.to_string().contains("removed_keyframe_ids"));
}

#[test]
fn patch_replaces_effects_and_keyframes_arrays_atomically_when_cascade_fires() {
    let prior = three_effect_project(vec![keyframe(
        KEYFRAME_A,
        &target_property(EFFECT_B, "amount"),
        10,
        json!(0.1),
    )]);

    let (patch, _warnings, _data) = compute_patch(&prior, &args(EFFECT_B)).expect("happy path");
    let ops = patch.as_array().expect("patch is array");

    assert_eq!(ops.len(), 2);
    assert_eq!(ops[0]["op"], "replace");
    assert_eq!(ops[0]["path"], "/tracks/0/clips/0/effects");
    assert_eq!(ops[1]["op"], "replace");
    assert_eq!(ops[1]["path"], "/tracks/0/clips/0/keyframes");
}

fn stringify_data_ids(data: &EffectRemoveData) -> Vec<String> {
    data.removed_keyframe_ids
        .iter()
        .map(ToString::to_string)
        .collect()
}
