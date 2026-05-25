//! Tests for `effect.toggle` (§6.4) — twenty-sixth production verb.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::effect_toggle::{
    DEFAULT_ENABLED, W_NOOP_CODE, compute_patch, data_envelope_from_post_state,
};
use verbreel_state::{
    EffectToggleArgs, EffectToggleData, EffectToggleError, EffectToggleVerb, MutateOutcome,
    Project, RecordedEvent, Track, Verb, VerbError, VerbRegistry, default_fixtures,
    default_registry, validate_reconstructors,
};
use verbreel_types::ProjectId;

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const TRACK_TEXT_A: &str = "01900000-0000-7000-8000-0000000aa201";
const TRACK_TEXT_B: &str = "01900000-0000-7000-8000-0000000aa202";
const CLIP_TEXT_A: &str = "01900000-0000-7000-8000-0000000bb201";
const CLIP_TEXT_B: &str = "01900000-0000-7000-8000-0000000bb202";
const CLIP_TEXT_C: &str = "01900000-0000-7000-8000-0000000bb203";
const CLIP_TEXT_D: &str = "01900000-0000-7000-8000-0000000bb204";
const EFFECT_A: &str = "01900000-0000-7000-8000-0000000cc201";
const EFFECT_B: &str = "01900000-0000-7000-8000-0000000cc202";
const EFFECT_C: &str = "01900000-0000-7000-8000-0000000cc203";
const EFFECT_D: &str = "01900000-0000-7000-8000-0000000cc204";
const MISSING_EFFECT: &str = "01900000-0000-7000-8000-0000000dd901";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture → Project")
}

fn fixture_project_id() -> ProjectId {
    empty_project().id
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

fn text_effect(id: &str, enabled: bool) -> Value {
    json!({
        "id": id,
        "kind": "blur",
        "enabled": enabled,
        "params": { "radius": 5 },
    })
}

fn text_track(
    id: &str,
    name: &str,
    track_locked: bool,
    clip_id: &str,
    clip_locked: bool,
    effects: Vec<Value>,
) -> Track {
    serde_json::from_value(json!({
        "id": id,
        "kind": "text",
        "name": name,
        "locked": track_locked,
        "clips": [text_clip(clip_id, clip_locked, effects)],
    }))
    .expect("text track fixture parses")
}

fn text_track_with_clips(id: &str, name: &str, track_locked: bool, clips: Vec<Value>) -> Track {
    serde_json::from_value(json!({
        "id": id,
        "kind": "text",
        "name": name,
        "locked": track_locked,
        "clips": clips,
    }))
    .expect("text track with clips parses")
}

fn project_with_tracks(tracks: Vec<Track>) -> Project {
    let mut project = empty_project();
    project.tracks = tracks;
    project.duration_tk = verbreel_types::Tick::new(480_000);
    project
}

fn patch_enabled_value(patch: &Value) -> bool {
    let arr = patch.as_array().expect("patch is an array");
    let op = arr[0].as_object().expect("patch op is object");
    assert_eq!(op.get("op").and_then(Value::as_str), Some("replace"));
    op.get("value")
        .and_then(Value::as_bool)
        .expect("replace value is bool")
}

fn patch_path(patch: &Value) -> &str {
    patch.as_array().expect("patch is array")[0]
        .get("path")
        .and_then(Value::as_str)
        .expect("patch op path is str")
}

#[test]
fn compute_patch_disabled_effect_enables() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
        vec![text_effect(EFFECT_A, false)],
    )]);

    let args = EffectToggleArgs {
        project_id: fixture_project_id(),
        effect: EFFECT_A.to_string(),
        enabled: Some(true),
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("happy path enables effect");
    assert!(patch_enabled_value(&patch));
    assert!(warnings.is_empty());
    assert_eq!(data.effect_id.to_string(), EFFECT_A);
    assert!(data.enabled);
    assert_eq!(patch_path(&patch), "/tracks/0/clips/0/effects/0/enabled",);
}

#[test]
fn compute_patch_enabled_effect_disables() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
        vec![text_effect(EFFECT_A, true)],
    )]);

    let args = EffectToggleArgs {
        project_id: fixture_project_id(),
        effect: EFFECT_A.to_string(),
        enabled: Some(false),
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("happy path disables effect");
    assert!(!patch_enabled_value(&patch));
    assert!(warnings.is_empty());
    assert_eq!(data.effect_id.to_string(), EFFECT_A);
    assert!(!data.enabled);
    assert_eq!(patch_path(&patch), "/tracks/0/clips/0/effects/0/enabled",);
}

#[test]
fn compute_patch_defaulted_to_true() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
        vec![text_effect(EFFECT_A, false)],
    )]);

    let args = EffectToggleArgs {
        project_id: fixture_project_id(),
        effect: EFFECT_A.to_string(),
        enabled: None,
    };

    let (patch, warnings, data) =
        compute_patch(&prior, &args).expect("omitted enabled defaults true");
    assert_eq!(patch_enabled_value(&patch), DEFAULT_ENABLED);
    assert!(warnings.is_empty());
    assert_eq!(data.effect_id.to_string(), EFFECT_A);
    assert!(data.enabled);
}

#[test]
fn compute_patch_idempotent_already_enabled_emits_w_noop() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
        vec![text_effect(EFFECT_A, true)],
    )]);

    let args = EffectToggleArgs {
        project_id: fixture_project_id(),
        effect: EFFECT_A.to_string(),
        enabled: Some(true),
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("idempotent enable is no-op");
    assert!(patch.as_array().expect("patch is array").is_empty());
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_NOOP_CODE);
    assert_eq!(warnings[0]["message"], "effect enabled state unchanged");
    assert_eq!(warnings[0]["details"]["effect_id"], EFFECT_A);
    assert_eq!(warnings[0]["details"]["enabled"], true);
    assert_eq!(data.effect_id.to_string(), EFFECT_A);
    assert!(data.enabled);
}

#[test]
fn compute_patch_idempotent_already_disabled_emits_w_noop() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
        vec![text_effect(EFFECT_A, false)],
    )]);

    let args = EffectToggleArgs {
        project_id: fixture_project_id(),
        effect: EFFECT_A.to_string(),
        enabled: Some(false),
    };

    let (patch, warnings, data) =
        compute_patch(&prior, &args).expect("idempotent disable is no-op");
    assert!(patch.as_array().expect("patch is array").is_empty());
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_NOOP_CODE);
    assert_eq!(warnings[0]["message"], "effect enabled state unchanged");
    assert_eq!(warnings[0]["details"]["effect_id"], EFFECT_A);
    assert_eq!(warnings[0]["details"]["enabled"], false);
    assert_eq!(data.effect_id.to_string(), EFFECT_A);
    assert!(!data.enabled);
}

#[test]
fn compute_patch_none_on_already_disabled_still_patches() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
        vec![text_effect(EFFECT_A, false)],
    )]);

    let args = EffectToggleArgs {
        project_id: fixture_project_id(),
        effect: EFFECT_A.to_string(),
        enabled: None,
    };

    let (patch, warnings, data) =
        compute_patch(&prior, &args).expect("defaulted true should mutate");
    assert!(patch_enabled_value(&patch));
    assert!(warnings.is_empty());
    assert_eq!(data.effect_id.to_string(), EFFECT_A);
    assert!(data.enabled);
}

#[test]
fn compute_patch_locked_parent_clip_returns_locked_error() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        true,
        vec![text_effect(EFFECT_A, false)],
    )]);

    let err = compute_patch(
        &prior,
        &EffectToggleArgs {
            project_id: fixture_project_id(),
            effect: EFFECT_A.to_string(),
            enabled: Some(true),
        },
    )
    .expect_err("locked parent clip must reject");

    match err {
        EffectToggleError::Locked { clip_id } => assert_eq!(clip_id, CLIP_TEXT_A),
        other => panic!("expected Locked, got {other:?}"),
    }
}

#[test]
fn compute_patch_track_lock_does_not_block_clip_effect_toggle() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        true,
        CLIP_TEXT_A,
        false,
        vec![text_effect(EFFECT_A, false)],
    )]);

    let args = EffectToggleArgs {
        project_id: fixture_project_id(),
        effect: EFFECT_A.to_string(),
        enabled: Some(true),
    };

    let (patch, warnings, data) =
        compute_patch(&prior, &args).expect("track lock should not block");
    assert!(patch_enabled_value(&patch));
    assert!(warnings.is_empty());
    assert_eq!(data.effect_id.to_string(), EFFECT_A);
    assert!(data.enabled);
}

#[test]
fn compute_patch_bad_uuid_errors() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
        vec![text_effect(EFFECT_A, false)],
    )]);

    let err = compute_patch(
        &prior,
        &EffectToggleArgs {
            project_id: fixture_project_id(),
            effect: "not-a-uuid".to_string(),
            enabled: Some(true),
        },
    )
    .expect_err("bad effect selector must reject");

    match err {
        EffectToggleError::BadSelector { detail } => assert!(detail.contains("UUID"), "{detail}"),
        other => panic!("expected BadSelector, got {other:?}"),
    }
}

#[test]
fn compute_patch_effect_not_found_errors() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
        vec![text_effect(EFFECT_A, false)],
    )]);

    let err = compute_patch(
        &prior,
        &EffectToggleArgs {
            project_id: fixture_project_id(),
            effect: MISSING_EFFECT.to_string(),
            enabled: Some(true),
        },
    )
    .expect_err("missing effect must reject");

    match err {
        EffectToggleError::EffectNotFound { effect_id } => assert_eq!(effect_id, MISSING_EFFECT),
        other => panic!("expected EffectNotFound, got {other:?}"),
    }
}

#[test]
fn data_envelope_returns_post_state_enabled() {
    let mut post_state = empty_project();
    post_state.tracks = vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
        vec![text_effect(EFFECT_A, true)],
    )];

    let args = EffectToggleArgs {
        project_id: fixture_project_id(),
        effect: EFFECT_A.to_string(),
        enabled: Some(false),
    };

    let data = data_envelope_from_post_state(&args, &post_state)
        .expect("post-state envelope should read effect state");
    assert_eq!(data.effect_id.to_string(), EFFECT_A);
    assert!(data.enabled);
}

#[test]
fn reconstructor_round_trip() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
        vec![text_effect(EFFECT_A, false)],
    )]);
    let args = EffectToggleArgs {
        project_id: fixture_project_id(),
        effect: EFFECT_A.to_string(),
        enabled: Some(true),
    };

    let (patch, _warnings, _data) = compute_patch(&prior, &args).expect("compute_patch ok");
    let typed_patch: json_patch::Patch =
        serde_json::from_value(patch.clone()).expect("patch parses to typed JSON patch");
    let post_state = prior
        .apply(&typed_patch)
        .expect("effect.toggle patch should apply cleanly");

    let expected_data =
        serde_json::to_value(data_envelope_from_post_state(&args, &post_state).expect("envelope"))
            .expect("envelope serializes");

    let recorded = RecordedEvent {
        verb: "effect.toggle".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch,
        warnings: vec![],
        post_state,
        expected_data,
    };

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(EffectToggleVerb))
        .expect("register effect.toggle verb");

    let report = validate_reconstructors(&registry, std::slice::from_ref(&recorded))
        .expect("reconstructor validation should pass");
    assert_eq!(report.verbs_checked, vec!["effect.toggle"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn compute_patch_error_variants_map_to_bad_args() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
        vec![text_effect(EFFECT_A, false)],
    )]);
    let verb = EffectToggleVerb;

    let bad_selector = serde_json::to_value(EffectToggleArgs {
        project_id: fixture_project_id(),
        effect: "not-a-uuid".to_string(),
        enabled: Some(true),
    })
    .expect("bad selector args serialize");
    let err = verb
        .compute_patch(&prior, &bad_selector)
        .expect_err("bad selector maps");
    assert!(matches!(err, VerbError::BadArgs { .. }));

    let missing = serde_json::to_value(EffectToggleArgs {
        project_id: fixture_project_id(),
        effect: MISSING_EFFECT.to_string(),
        enabled: Some(true),
    })
    .expect("missing effect args serialize");
    let err = verb
        .compute_patch(&prior, &missing)
        .expect_err("missing effect maps");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "effect.toggle")
        .expect("default_fixtures includes effect.toggle");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(EffectToggleVerb))
        .expect("register effect.toggle verb");

    let report = validate_reconstructors(&registry, &[fixture])
        .expect("effect.toggle reconstruct from fixture");
    assert_eq!(report.verbs_checked, vec!["effect.toggle"]);
    assert_eq!(report.fixtures_run, 1);
}

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let base_project = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
        vec![text_effect(EFFECT_A, false)],
    )]);
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        base_project,
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry clears the gate and writes project.json");

    let args = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "effect": EFFECT_A,
        "enabled": true,
    });

    let outcome = store
        .mutate_via_verb("effect.toggle", args, None)
        .expect("mutate_via_verb happy path");
    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("happy path must return Applied, got {outcome:?}");
    };

    let data: EffectToggleData =
        serde_json::from_value(data).expect("effect.toggle data is EffectToggleData");
    assert_eq!(data.effect_id.to_string(), EFFECT_A);
    assert!(data.enabled);
    assert!(store.project().tracks[0].clips[0].effects[0].enabled);
    assert_eq!(warnings, Vec::<Value>::new());
}

#[test]
fn compute_patch_multi_effect_clip_targets_correct_index() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        "Text 1",
        false,
        CLIP_TEXT_A,
        false,
        vec![
            text_effect(EFFECT_A, true),
            text_effect(EFFECT_B, false),
            text_effect(EFFECT_C, true),
        ],
    )]);

    let args = EffectToggleArgs {
        project_id: fixture_project_id(),
        effect: EFFECT_B.to_string(),
        enabled: Some(true),
    };

    let (patch, warnings, data) =
        compute_patch(&prior, &args).expect("happy path targets middle effect");
    assert_eq!(patch_path(&patch), "/tracks/0/clips/0/effects/1/enabled");
    assert!(patch_enabled_value(&patch));
    assert!(warnings.is_empty());
    assert_eq!(data.effect_id.to_string(), EFFECT_B);
    assert!(data.enabled);
}

#[test]
fn compute_patch_multi_track_clip_targets_correct_indexes() {
    let prior = project_with_tracks(vec![
        text_track_with_clips(
            TRACK_TEXT_A,
            "Text 1",
            false,
            vec![text_clip(
                CLIP_TEXT_A,
                false,
                vec![text_effect(EFFECT_A, false)],
            )],
        ),
        text_track_with_clips(
            TRACK_TEXT_B,
            "Text 2",
            false,
            vec![
                text_clip(CLIP_TEXT_B, false, vec![text_effect(EFFECT_A, false)]),
                text_clip(CLIP_TEXT_C, false, vec![text_effect(EFFECT_C, false)]),
                text_clip(
                    CLIP_TEXT_D,
                    false,
                    vec![
                        text_effect(EFFECT_A, false),
                        text_effect(EFFECT_B, false),
                        text_effect(EFFECT_C, false),
                        text_effect(EFFECT_D, false),
                    ],
                ),
            ],
        ),
    ]);

    let args = EffectToggleArgs {
        project_id: fixture_project_id(),
        effect: EFFECT_D.to_string(),
        enabled: Some(true),
    };

    let (patch, warnings, data) =
        compute_patch(&prior, &args).expect("happy path targets deep effect");
    assert_eq!(patch_path(&patch), "/tracks/1/clips/2/effects/3/enabled");
    assert!(patch_enabled_value(&patch));
    assert!(warnings.is_empty());
    assert_eq!(data.effect_id.to_string(), EFFECT_D);
    assert!(data.enabled);
}
