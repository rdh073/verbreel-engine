//! Tests for `effect.reorder` (§6.6) — forty-second production verb.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::effect_reorder::{
    W_EFFECT_REORDER_ENVELOPE_CODE, W_NOOP_CODE, compute_patch,
    data_envelope_from_patch_warnings_post_state,
};
use verbreel_state::{
    EffectReorderArgs, EffectReorderData, EffectReorderError, EffectReorderVerb, MutateOutcome,
    Project, RecordedEvent, ToIndex, Track, Verb, VerbError, VerbRegistry, default_fixtures,
    default_registry, validate_reconstructors,
};
use verbreel_types::ProjectId;

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const TRACK_TEXT_A: &str = "01900000-0000-7000-8000-0000000aa301";
const CLIP_TEXT_A: &str = "01900000-0000-7000-8000-0000000bb301";
const EFFECT_A: &str = "01900000-0000-7000-8000-0000000cc301";
const EFFECT_B: &str = "01900000-0000-7000-8000-0000000cc302";
const EFFECT_C: &str = "01900000-0000-7000-8000-0000000cc303";
const MISSING_EFFECT: &str = "01900000-0000-7000-8000-0000000dd301";

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

fn text_clip(id: &str, locked: bool, effects: Vec<Value>) -> Value {
    json!({
        "id": id,
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
        "clips": [text_clip(CLIP_TEXT_A, clip_locked, effects)],
    }))
    .expect("text track fixture parses")
}

fn project_with_track(track_locked: bool, clip_locked: bool, effects: Vec<Value>) -> Project {
    let mut project = empty_project();
    project.tracks = vec![text_track(track_locked, clip_locked, effects)];
    project.duration_tk = verbreel_types::Tick::new(480_000);
    project
}

fn three_effect_project() -> Project {
    project_with_track(
        false,
        false,
        vec![
            text_effect(EFFECT_A, "blur"),
            text_effect(EFFECT_B, "sharpen"),
            text_effect(EFFECT_C, "glow"),
        ],
    )
}

fn args(effect: &str, to_index: ToIndex) -> EffectReorderArgs {
    EffectReorderArgs {
        project_id: fixture_project_id(),
        effect: effect.to_string(),
        to_index,
    }
}

fn effect_order(project: &Project) -> Vec<String> {
    project.tracks[0].clips[0]
        .effects
        .iter()
        .map(|effect| effect.id.to_string())
        .collect()
}

fn apply_patch(prior: &Project, patch: Value) -> Project {
    let patch: json_patch::Patch = serde_json::from_value(patch).expect("patch parses");
    prior.apply(&patch).expect("patch applies")
}

fn move_paths(patch: &Value) -> (&str, &str) {
    let ops = patch.as_array().expect("patch is array");
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0]["op"], "move");
    (
        ops[0]["from"].as_str().expect("from path"),
        ops[0]["path"].as_str().expect("path path"),
    )
}

#[test]
fn compute_patch_moves_first_to_last() {
    let prior = three_effect_project();
    let args = args(EFFECT_A, ToIndex::Integer(2));

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("happy path");
    assert_eq!(
        move_paths(&patch),
        ("/tracks/0/clips/0/effects/0", "/tracks/0/clips/0/effects/2")
    );
    assert_eq!(warnings[0]["code"], W_EFFECT_REORDER_ENVELOPE_CODE);
    assert_eq!(data.effect_id.to_string(), EFFECT_A);
    assert_eq!(data.parent_kind, "clip");
    assert_eq!(data.parent_id.to_string(), CLIP_TEXT_A);
    assert_eq!(data.from_index, 0);
    assert_eq!(data.to_index, 2);

    let post = apply_patch(&prior, patch);
    assert_eq!(effect_order(&post), vec![EFFECT_B, EFFECT_C, EFFECT_A]);
}

#[test]
fn compute_patch_moves_last_to_first() {
    let prior = three_effect_project();
    let args = args(EFFECT_C, ToIndex::Integer(0));

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("happy path");
    assert_eq!(
        move_paths(&patch),
        ("/tracks/0/clips/0/effects/2", "/tracks/0/clips/0/effects/0")
    );
    assert_eq!(warnings[0]["code"], W_EFFECT_REORDER_ENVELOPE_CODE);
    assert_eq!(data.from_index, 2);
    assert_eq!(data.to_index, 0);

    let post = apply_patch(&prior, patch);
    assert_eq!(effect_order(&post), vec![EFFECT_C, EFFECT_A, EFFECT_B]);
}

#[test]
fn compute_patch_moves_middle_to_end() {
    let prior = three_effect_project();
    let args = args(EFFECT_B, ToIndex::End("end".to_string()));

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("happy path");
    assert_eq!(
        move_paths(&patch),
        ("/tracks/0/clips/0/effects/1", "/tracks/0/clips/0/effects/2")
    );
    assert_eq!(warnings[0]["code"], W_EFFECT_REORDER_ENVELOPE_CODE);
    assert_eq!(data.from_index, 1);
    assert_eq!(data.to_index, 2);

    let post = apply_patch(&prior, patch);
    assert_eq!(effect_order(&post), vec![EFFECT_A, EFFECT_C, EFFECT_B]);
}

#[test]
fn end_sentinel_deserializes_and_resolves_to_last_index() {
    let prior = three_effect_project();
    let typed: EffectReorderArgs = serde_json::from_value(json!({
        "project_id": FIXTURE_PROJECT_ID,
        "effect": EFFECT_A,
        "to_index": "end",
    }))
    .expect("end sentinel deserializes");

    let (_patch, _warnings, data) = compute_patch(&prior, &typed).expect("end resolves");
    assert_eq!(data.to_index, 2);
}

#[test]
fn compute_patch_noop_same_integer_index_emits_w_noop() {
    let prior = three_effect_project();
    let args = args(EFFECT_B, ToIndex::Integer(1));

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("no-op");
    assert!(patch.as_array().expect("patch is array").is_empty());
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_NOOP_CODE);
    assert_eq!(warnings[0]["details"]["from_index"], 1);
    assert_eq!(warnings[0]["details"]["to_index"], 1);
    assert_eq!(
        warnings[0]["details"]["message"],
        "effect already at requested index"
    );
    assert_eq!(data.from_index, 1);
    assert_eq!(data.to_index, 1);
}

#[test]
fn compute_patch_noop_end_when_effect_is_already_last() {
    let prior = three_effect_project();
    let args = args(EFFECT_C, ToIndex::End("end".to_string()));

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("end no-op");
    assert!(patch.as_array().expect("patch is array").is_empty());
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_NOOP_CODE);
    assert_eq!(data.from_index, 2);
    assert_eq!(data.to_index, 2);
}

#[test]
fn compute_patch_unknown_effect_returns_not_found() {
    let prior = three_effect_project();

    let err = compute_patch(&prior, &args(MISSING_EFFECT, ToIndex::Integer(0)))
        .expect_err("missing effect rejects");

    match err {
        EffectReorderError::EffectNotFound { effect_id } => assert_eq!(effect_id, MISSING_EFFECT),
        other => panic!("expected EffectNotFound, got {other:?}"),
    }
}

#[test]
fn compute_patch_malformed_effect_uuid_returns_bad_selector() {
    let prior = three_effect_project();

    let err = compute_patch(&prior, &args("not-a-uuid", ToIndex::Integer(0)))
        .expect_err("bad selector rejects");

    match err {
        EffectReorderError::BadSelector { detail } => assert!(detail.contains("UUID")),
        other => panic!("expected BadSelector, got {other:?}"),
    }
}

#[test]
fn compute_patch_locked_clip_returns_locked() {
    let prior = project_with_track(false, true, vec![text_effect(EFFECT_A, "blur")]);

    let err = compute_patch(&prior, &args(EFFECT_A, ToIndex::Integer(0)))
        .expect_err("locked clip rejects");

    match err {
        EffectReorderError::Locked { kind, id, .. } => {
            assert_eq!(kind, "clip");
            assert_eq!(id, CLIP_TEXT_A);
        }
        other => panic!("expected Locked, got {other:?}"),
    }
}

#[test]
fn compute_patch_locked_track_returns_locked() {
    let prior = project_with_track(true, false, vec![text_effect(EFFECT_A, "blur")]);

    let err = compute_patch(&prior, &args(EFFECT_A, ToIndex::Integer(0)))
        .expect_err("locked track rejects");

    match err {
        EffectReorderError::Locked { kind, id, .. } => {
            assert_eq!(kind, "track");
            assert_eq!(id, TRACK_TEXT_A);
        }
        other => panic!("expected Locked, got {other:?}"),
    }
}

#[test]
fn compute_patch_negative_index_returns_bad_range() {
    let prior = three_effect_project();

    let err = compute_patch(&prior, &args(EFFECT_A, ToIndex::Integer(-1)))
        .expect_err("negative index rejects");

    match err {
        EffectReorderError::BadRange {
            to_index,
            effects_len,
        } => {
            assert_eq!(to_index, -1);
            assert_eq!(effects_len, 3);
        }
        other => panic!("expected BadRange, got {other:?}"),
    }
}

#[test]
fn compute_patch_too_large_index_returns_bad_range() {
    let prior = three_effect_project();

    let err = compute_patch(&prior, &args(EFFECT_A, ToIndex::Integer(3)))
        .expect_err("too-large index rejects");

    match err {
        EffectReorderError::BadRange {
            to_index,
            effects_len,
        } => {
            assert_eq!(to_index, 3);
            assert_eq!(effects_len, 3);
        }
        other => panic!("expected BadRange, got {other:?}"),
    }
}

#[test]
fn non_end_string_returns_schema_violation_at_verb_boundary() {
    let prior = three_effect_project();
    let verb = EffectReorderVerb;

    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "effect": EFFECT_A,
                "to_index": "head",
            }),
        )
        .expect_err("non-end string rejects");

    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn reconstructor_round_trip() {
    let prior = three_effect_project();
    let args = args(EFFECT_A, ToIndex::Integer(2));

    let (patch, warnings, recorded) = compute_patch(&prior, &args).expect("compute patch");
    let typed_patch: json_patch::Patch =
        serde_json::from_value(patch.clone()).expect("patch parses to typed JSON patch");
    let post_state = prior.apply(&typed_patch).expect("patch applies");
    let reconstructed =
        data_envelope_from_patch_warnings_post_state(&args, &patch, &warnings, &post_state)
            .expect("reconstructs");
    assert_eq!(recorded, reconstructed);

    let expected_data = serde_json::to_value(reconstructed).expect("envelope serializes");
    let recorded = RecordedEvent {
        verb: "effect.reorder".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch,
        warnings,
        post_state,
        expected_data,
    };

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(EffectReorderVerb))
        .expect("register effect.reorder verb");

    let report = validate_reconstructors(&registry, &[recorded]).expect("validation passes");
    assert_eq!(report.verbs_checked, vec!["effect.reorder"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "effect.reorder")
        .expect("default_fixtures includes effect.reorder");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(EffectReorderVerb))
        .expect("register effect.reorder verb");

    let report =
        validate_reconstructors(&registry, &[fixture]).expect("reconstruction from fixture");
    assert_eq!(report.verbs_checked, vec!["effect.reorder"]);
}

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let base_project = three_effect_project();
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        base_project,
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry clears the gate");

    let outcome = store
        .mutate_via_verb(
            "effect.reorder",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "effect": EFFECT_A,
                "to_index": 2,
            }),
            None,
        )
        .expect("mutate_via_verb happy path");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("happy path must return Applied, got {outcome:?}");
    };

    let data: EffectReorderData = serde_json::from_value(data).expect("effect.reorder data parses");
    assert_eq!(data.effect_id.to_string(), EFFECT_A);
    assert_eq!(data.from_index, 0);
    assert_eq!(data.to_index, 2);
    assert_eq!(warnings[0]["code"], W_EFFECT_REORDER_ENVELOPE_CODE);
    assert_eq!(
        effect_order(store.project()),
        vec![EFFECT_B, EFFECT_C, EFFECT_A]
    );
}

#[test]
fn non_moved_sibling_order_is_preserved() {
    let prior = three_effect_project();
    let args = args(EFFECT_B, ToIndex::Integer(0));

    let (patch, _warnings, _data) = compute_patch(&prior, &args).expect("happy path");
    let post = apply_patch(&prior, patch);

    assert_eq!(effect_order(&post), vec![EFFECT_B, EFFECT_A, EFFECT_C]);
}

#[test]
fn compute_patch_error_variants_map_to_bad_args() {
    let prior = three_effect_project();
    let verb = EffectReorderVerb;

    for args in [
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "effect": "not-a-uuid",
            "to_index": 0,
        }),
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "effect": MISSING_EFFECT,
            "to_index": 0,
        }),
        json!({
            "project_id": FIXTURE_PROJECT_ID,
            "effect": EFFECT_A,
            "to_index": 99,
        }),
    ] {
        let err = verb
            .compute_patch(&prior, &args)
            .expect_err("error maps to VerbError::BadArgs");
        assert!(matches!(err, VerbError::BadArgs { .. }));
    }
}
