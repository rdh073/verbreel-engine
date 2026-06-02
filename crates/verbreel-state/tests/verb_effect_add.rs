//! Tests for `effect.add` (§6.1) -- fiftieth production verb.

use std::sync::Arc;

use serde_json::{Map, Value, json};
use verbreel_state::verbs::effect_add::{
    W_EFFECT_ADD_ENVELOPE_CODE, W_TIME_SNAPPED_CODE, compute_patch,
    data_envelope_from_args_warnings,
};
use verbreel_state::{
    EffectAddArgs, EffectAddData, EffectAddError, EffectAddVerb, MutateOutcome, Project,
    RecordedEvent, Track, VerbRegistry, default_fixtures, default_registry,
    validate_reconstructors,
};
use verbreel_types::{EffectId, ProjectId, Tick};

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const TRACK_TEXT_A: &str = "01900000-0000-7000-8000-0000000aa501";
const TRACK_TEXT_B: &str = "01900000-0000-7000-8000-0000000aa502";
const CLIP_TEXT_A: &str = "01900000-0000-7000-8000-0000000bb501";
const CLIP_TEXT_B: &str = "01900000-0000-7000-8000-0000000bb502";
const EFFECT_A: &str = "01900000-0000-7000-8000-0000000cc501";
const MISSING_CLIP: &str = "01900000-0000-7000-8000-0000000dd501";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture -> Project")
}

fn fixture_project_id() -> ProjectId {
    empty_project().id
}

fn map(entries: &[(&str, Value)]) -> Map<String, Value> {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_string(), value.clone()))
        .collect()
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

fn text_effect(id: &str, kind: &str) -> Value {
    json!({
        "id": id,
        "kind": kind,
        "enabled": true,
        "params": { "radius_px": 5 },
    })
}

fn text_track(
    id: &str,
    track_locked: bool,
    clip_id: &str,
    clip_locked: bool,
    effects: Vec<Value>,
) -> Track {
    serde_json::from_value(json!({
        "id": id,
        "kind": "text",
        "name": "Text",
        "locked": track_locked,
        "clips": [text_clip(clip_id, clip_locked, effects)],
    }))
    .expect("text track fixture parses")
}

fn project_with_tracks(tracks: Vec<Track>) -> Project {
    let mut project = empty_project();
    project.tracks = tracks;
    project.duration_tk = Tick::new(480_000);
    project
}

fn project_with_text_clip(track_locked: bool, clip_locked: bool, effects: Vec<Value>) -> Project {
    project_with_tracks(vec![text_track(
        TRACK_TEXT_A,
        track_locked,
        CLIP_TEXT_A,
        clip_locked,
        effects,
    )])
}

fn args(kind: &str) -> EffectAddArgs {
    EffectAddArgs {
        project_id: fixture_project_id(),
        target: format!("clip:{CLIP_TEXT_A}"),
        kind: kind.to_string(),
        params: None,
        in_tk: None,
        out_tk: None,
    }
}

fn args_with_window(kind: &str, in_tk: i64, out_tk: i64) -> EffectAddArgs {
    EffectAddArgs {
        in_tk: Some(in_tk),
        out_tk: Some(out_tk),
        ..args(kind)
    }
}

fn apply_patch(prior: &Project, patch: Value) -> Project {
    let patch: json_patch::Patch = serde_json::from_value(patch).expect("patch parses");
    prior.apply(&patch).expect("patch applies")
}

fn visible_warning_codes(warnings: &[Value]) -> Vec<String> {
    warnings
        .iter()
        .filter_map(|warning| warning["code"].as_str())
        .filter(|code| !code.ends_with("_ENVELOPE"))
        .map(ToString::to_string)
        .collect()
}

#[test]
fn compute_patch_adds_blur_without_window_or_params() {
    let prior = project_with_text_clip(false, false, Vec::new());
    let args = args("blur");

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("happy path");
    let post = apply_patch(&prior, patch);
    let effect = post.tracks[0].clips[0]
        .effects
        .last()
        .expect("effect added");

    assert_eq!(post.tracks[0].clips[0].effects.len(), 1);
    assert_eq!(effect.id, data.effect_id);
    assert_eq!(effect.kind.to_string(), "blur");
    assert!(effect.enabled);
    assert!(effect.params.is_empty());
    assert_eq!(effect.window, None);
    assert_eq!(data.target_kind, "clip");
    assert_eq!(data.target_id.to_string(), CLIP_TEXT_A);
    assert_eq!(
        warnings.last().and_then(|warning| warning["code"].as_str()),
        Some(W_EFFECT_ADD_ENVELOPE_CODE)
    );
}

#[test]
fn compute_patch_adds_color_correct_with_params() {
    let prior = project_with_text_clip(false, false, Vec::new());
    let args = EffectAddArgs {
        params: Some(map(&[("brightness", json!(0.1)), ("contrast", json!(1.2))])),
        ..args("color_correct")
    };

    let (patch, _warnings, data) = compute_patch(&prior, &args).expect("params accepted");
    let post = apply_patch(&prior, patch);
    let effect = &post.tracks[0].clips[0].effects[0];

    assert_eq!(effect.id, data.effect_id);
    assert_eq!(effect.params["brightness"], json!(0.1));
    assert_eq!(effect.params["contrast"], json!(1.2));
}

#[test]
fn compute_patch_adds_window_when_pair_supplied() {
    let prior = project_with_text_clip(false, false, Vec::new());
    let args = args_with_window("blur", 8_000, 16_000);

    let (patch, warnings, _data) = compute_patch(&prior, &args).expect("window accepted");
    let post = apply_patch(&prior, patch);
    let window = post.tracks[0].clips[0].effects[0]
        .window
        .expect("window set");

    assert_eq!(window.in_tk.get(), 8_000);
    assert_eq!(window.out_tk.get(), 16_000);
    assert_eq!(visible_warning_codes(&warnings), Vec::<String>::new());
}

#[test]
fn compute_patch_omitted_window_stores_none() {
    let prior = project_with_text_clip(false, false, Vec::new());
    let (patch, _warnings, _data) = compute_patch(&prior, &args("blur")).expect("happy path");
    let post = apply_patch(&prior, patch);

    assert_eq!(post.tracks[0].clips[0].effects[0].window, None);
}

#[test]
fn compute_patch_rejects_in_tk_without_out_tk() {
    let prior = project_with_text_clip(false, false, Vec::new());
    let err = compute_patch(
        &prior,
        &EffectAddArgs {
            in_tk: Some(8_000),
            out_tk: None,
            ..args("blur")
        },
    )
    .expect_err("pair required");

    match err {
        EffectAddError::SchemaViolation { field, message } => {
            assert_eq!(field, "in_tk_out_tk_pair");
            assert_eq!(message, "supply both or neither");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compute_patch_rejects_out_tk_without_in_tk() {
    let prior = project_with_text_clip(false, false, Vec::new());
    let err = compute_patch(
        &prior,
        &EffectAddArgs {
            in_tk: None,
            out_tk: Some(16_000),
            ..args("blur")
        },
    )
    .expect_err("pair required");

    assert!(matches!(err, EffectAddError::SchemaViolation { .. }));
}

#[test]
fn compute_patch_rejects_bare_uuid_target() {
    let prior = project_with_text_clip(false, false, Vec::new());
    let err = compute_patch(
        &prior,
        &EffectAddArgs {
            target: CLIP_TEXT_A.to_string(),
            ..args("blur")
        },
    )
    .expect_err("bare target rejected");

    match err {
        EffectAddError::BadSelector { hint, .. } => {
            assert_eq!(hint, "target must be qualified form, e.g. clip:<uuid>");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compute_patch_rejects_malformed_clip_target() {
    let prior = project_with_text_clip(false, false, Vec::new());
    let err = compute_patch(
        &prior,
        &EffectAddArgs {
            target: "clip:not-a-uuid".to_string(),
            ..args("blur")
        },
    )
    .expect_err("malformed uuid rejected");

    assert!(matches!(err, EffectAddError::BadSelector { .. }));
}

#[test]
fn compute_patch_rejects_track_target_kind_mismatch() {
    let prior = project_with_text_clip(false, false, Vec::new());
    let err = compute_patch(
        &prior,
        &EffectAddArgs {
            target: format!("track:{TRACK_TEXT_A}"),
            ..args("blur")
        },
    )
    .expect_err("track target rejected");

    match err {
        EffectAddError::SelectorKindMismatch {
            actual_prefix,
            hint,
        } => {
            assert_eq!(actual_prefix, "track");
            assert_eq!(
                hint,
                "track-attached effects deferred; track.effects is not yet typed"
            );
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compute_patch_unknown_clip_returns_not_found() {
    let prior = project_with_text_clip(false, false, Vec::new());
    let err = compute_patch(
        &prior,
        &EffectAddArgs {
            target: format!("clip:{MISSING_CLIP}"),
            ..args("blur")
        },
    )
    .expect_err("missing clip rejected");

    match err {
        EffectAddError::ClipNotFound { clip_id } => assert_eq!(clip_id, MISSING_CLIP),
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compute_patch_rejects_locked_clip() {
    let prior = project_with_text_clip(false, true, Vec::new());
    let err = compute_patch(&prior, &args("blur")).expect_err("locked clip rejected");

    match err {
        EffectAddError::Locked { failed_clip } => assert_eq!(failed_clip, CLIP_TEXT_A),
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compute_patch_rejects_locked_parent_track() {
    let prior = project_with_text_clip(true, false, Vec::new());
    let err = compute_patch(&prior, &args("blur")).expect_err("locked track rejected");

    match err {
        EffectAddError::Locked { failed_clip } => assert_eq!(failed_clip, CLIP_TEXT_A),
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compute_patch_rejects_unknown_kind() {
    let prior = project_with_text_clip(false, false, Vec::new());
    let err = compute_patch(&prior, &args("bogus_effect")).expect_err("unknown kind");

    match err {
        EffectAddError::UnknownKind { kind, allowed } => {
            assert_eq!(kind, "bogus_effect");
            assert!(allowed.contains(&"blur"));
            assert!(allowed.contains(&"transition.wipe"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compute_patch_rejects_managed_time_stretch() {
    assert_managed_kind("time_stretch", "clip.set_speed");
}

#[test]
fn compute_patch_rejects_managed_burned_caption() {
    assert_managed_kind("burned_caption", "caption.burn_in");
}

#[test]
fn compute_patch_rejects_managed_denoise() {
    assert_managed_kind("denoise", "audio.denoise");
}

fn assert_managed_kind(kind: &str, expected_verb: &str) {
    let prior = project_with_text_clip(false, false, Vec::new());
    let err = compute_patch(&prior, &args(kind)).expect_err("managed kind rejected");

    match err {
        EffectAddError::ManagedEffect { managing_verb, .. } => {
            assert_eq!(managing_verb, expected_verb);
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compute_patch_rejects_negative_in_tk() {
    let prior = project_with_text_clip(false, false, Vec::new());
    let err = compute_patch(&prior, &args_with_window("blur", -1, 16_000))
        .expect_err("negative in_tk rejected");

    assert!(matches!(
        err,
        EffectAddError::BadTime {
            field: "in_tk",
            value: -1,
            ..
        }
    ));
}

#[test]
fn compute_patch_rejects_in_tk_greater_than_or_equal_out_tk() {
    let prior = project_with_text_clip(false, false, Vec::new());
    let err = compute_patch(&prior, &args_with_window("blur", 16_000, 16_000))
        .expect_err("degenerate window rejected");

    assert!(matches!(
        err,
        EffectAddError::BadTime {
            field: "out_tk",
            value: 16_000,
            ..
        }
    ));
}

#[test]
fn compute_patch_rejects_out_tk_beyond_parent_duration() {
    let prior = project_with_text_clip(false, false, Vec::new());
    let err = compute_patch(&prior, &args_with_window("blur", 0, 480_001))
        .expect_err("out of bounds rejected");

    match err {
        EffectAddError::BadTime {
            field,
            value,
            bound,
        } => {
            assert_eq!(field, "out_tk");
            assert_eq!(value, 480_001);
            assert_eq!(bound, [0, 480_000]);
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compute_patch_rejects_params_with_more_than_64_keys() {
    let prior = project_with_text_clip(false, false, Vec::new());
    let params = (0..65)
        .map(|index| (format!("k{index}"), json!(index)))
        .collect();
    let err = compute_patch(
        &prior,
        &EffectAddArgs {
            params: Some(params),
            ..args("blur")
        },
    )
    .expect_err("key cap rejected");

    match err {
        EffectAddError::BadParams {
            field,
            bound,
            value,
        } => {
            assert_eq!(field, "params.keys");
            assert_eq!(bound, 64);
            assert_eq!(value, 65);
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compute_patch_rejects_params_over_byte_cap() {
    let prior = project_with_text_clip(false, false, Vec::new());
    let err = compute_patch(
        &prior,
        &EffectAddArgs {
            params: Some(map(&[("payload", json!("x".repeat(17_000)))])),
            ..args("blur")
        },
    )
    .expect_err("byte cap rejected");

    match err {
        EffectAddError::BadParams {
            field,
            bound,
            value,
        } => {
            assert_eq!(field, "params.bytes");
            assert_eq!(bound, 16_384);
            assert!(value > 16_384);
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compute_patch_snaps_off_frame_in_tk() {
    let prior = project_with_text_clip(false, false, Vec::new());
    let (patch, warnings, _data) =
        compute_patch(&prior, &args_with_window("blur", 1, 16_000)).expect("snap in_tk");
    let post = apply_patch(&prior, patch);
    let window = post.tracks[0].clips[0].effects[0]
        .window
        .expect("window set");

    assert_eq!(window.in_tk.get(), 0);
    assert_eq!(warnings[0]["code"], W_TIME_SNAPPED_CODE);
    assert_eq!(warnings[0]["details"]["field"], "in_tk");
    assert_eq!(warnings[0]["details"]["from_tk"], 1);
    assert_eq!(warnings[0]["details"]["to_tk"], 0);
}

#[test]
fn compute_patch_snaps_off_frame_out_tk() {
    let prior = project_with_text_clip(false, false, Vec::new());
    let (patch, warnings, _data) =
        compute_patch(&prior, &args_with_window("blur", 0, 16_001)).expect("snap out_tk");
    let post = apply_patch(&prior, patch);
    let window = post.tracks[0].clips[0].effects[0]
        .window
        .expect("window set");

    assert_eq!(window.out_tk.get(), 16_000);
    assert_eq!(warnings[0]["code"], W_TIME_SNAPPED_CODE);
    assert_eq!(warnings[0]["details"]["field"], "out_tk");
    assert_eq!(warnings[0]["details"]["from_tk"], 16_001);
    assert_eq!(warnings[0]["details"]["to_tk"], 16_000);
}

#[test]
fn reconstructor_round_trip() {
    let prior = project_with_text_clip(false, false, Vec::new());
    let args = args("blur");
    let (patch, warnings, _data) = compute_patch(&prior, &args).expect("effect add");
    let post = apply_patch(&prior, patch.clone());
    let expected_data = serde_json::to_value(
        data_envelope_from_args_warnings(&args, &warnings).expect("warning envelope"),
    )
    .expect("data serializes");
    let recorded = RecordedEvent {
        verb: "effect.add".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch,
        warnings,
        post_state: post,
        expected_data,
    };

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(EffectAddVerb))
        .expect("register effect.add");
    let report = validate_reconstructors(&registry, &[recorded]).expect("round trip");
    assert_eq!(report.verbs_checked, vec!["effect.add"]);
}

fn curves_identity_params() -> Map<String, Value> {
    let ramp = json!([[0.0, 0.0], [1.0, 1.0]]);
    map(&[
        ("luma", ramp.clone()),
        ("r", ramp.clone()),
        ("g", ramp.clone()),
        ("b", ramp),
    ])
}

fn hsl_identity_params() -> Map<String, Value> {
    let zero_band = json!({ "hue": 0.0, "saturation": 0.0, "lightness": 0.0 });
    map(&[
        ("red", zero_band.clone()),
        ("orange", zero_band.clone()),
        ("yellow", zero_band.clone()),
        ("green", zero_band.clone()),
        ("cyan", zero_band.clone()),
        ("blue", zero_band.clone()),
        ("purple", zero_band.clone()),
        ("magenta", zero_band),
    ])
}

fn round_trip_kind(kind: &str, params: Map<String, Value>) {
    let prior = project_with_text_clip(false, false, Vec::new());
    let args = EffectAddArgs {
        params: Some(params),
        ..args(kind)
    };

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("identity params accepted");
    let post = apply_patch(&prior, patch.clone());
    let effect = &post.tracks[0].clips[0].effects[0];
    assert_eq!(effect.id, data.effect_id);
    assert_eq!(effect.kind.to_string(), kind);

    let expected_data = serde_json::to_value(
        data_envelope_from_args_warnings(&args, &warnings).expect("warning envelope"),
    )
    .expect("data serializes");
    let recorded = RecordedEvent {
        verb: "effect.add".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch,
        warnings,
        post_state: post,
        expected_data,
    };

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(EffectAddVerb))
        .expect("register effect.add");
    let report = validate_reconstructors(&registry, &[recorded]).expect("round trip");
    assert_eq!(report.verbs_checked, vec!["effect.add"]);
}

#[test]
fn compute_patch_adds_curves_identity_and_round_trips() {
    let prior = project_with_text_clip(false, false, Vec::new());
    let params = curves_identity_params();
    let args = EffectAddArgs {
        params: Some(params.clone()),
        ..args("curves")
    };

    let (patch, _warnings, data) = compute_patch(&prior, &args).expect("curves params accepted");
    let post = apply_patch(&prior, patch);
    let effect = &post.tracks[0].clips[0].effects[0];
    assert_eq!(effect.id, data.effect_id);
    assert_eq!(effect.kind.to_string(), "curves");
    assert_eq!(effect.params["luma"], json!([[0.0, 0.0], [1.0, 1.0]]));

    round_trip_kind("curves", params);
}

#[test]
fn compute_patch_adds_hsl_identity_and_round_trips() {
    let prior = project_with_text_clip(false, false, Vec::new());
    let params = hsl_identity_params();
    let args = EffectAddArgs {
        params: Some(params.clone()),
        ..args("hsl")
    };

    let (patch, _warnings, data) = compute_patch(&prior, &args).expect("hsl params accepted");
    let post = apply_patch(&prior, patch);
    let effect = &post.tracks[0].clips[0].effects[0];
    assert_eq!(effect.id, data.effect_id);
    assert_eq!(effect.kind.to_string(), "hsl");
    assert_eq!(
        effect.params["red"],
        json!({ "hue": 0.0, "saturation": 0.0, "lightness": 0.0 })
    );

    round_trip_kind("hsl", params);
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "effect.add")
        .expect("default_fixtures includes effect.add");
    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(EffectAddVerb))
        .expect("register effect.add");
    let report = validate_reconstructors(&registry, &[fixture]).expect("default fixture");
    assert_eq!(report.verbs_checked, vec!["effect.add"]);
}

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        project_with_text_clip(false, false, Vec::new()),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry clears the gate");

    let outcome = store
        .mutate_via_verb(
            "effect.add",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "target": format!("clip:{CLIP_TEXT_A}"),
                "kind": "blur",
            }),
            None,
        )
        .expect("mutate_via_verb happy path");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("happy path must return Applied, got {outcome:?}");
    };

    let data: EffectAddData = serde_json::from_value(data).expect("effect.add data");
    assert_eq!(data.target_kind, "clip");
    assert_eq!(store.project().tracks[0].clips[0].effects.len(), 1);
    assert_eq!(
        store.project().tracks[0].clips[0].effects[0].id,
        data.effect_id
    );
    assert_eq!(visible_warning_codes(&warnings), Vec::<String>::new());
}

#[test]
fn compute_patch_appends_to_existing_effects_and_allows_duplicate_kind() {
    let prior = project_with_text_clip(false, false, vec![text_effect(EFFECT_A, "blur")]);
    let (patch, _warnings, data) = compute_patch(&prior, &args("blur")).expect("append duplicate");
    let post = apply_patch(&prior, patch);
    let effects = &post.tracks[0].clips[0].effects;

    assert_eq!(effects.len(), 2);
    assert_eq!(effects[0].id, EFFECT_A.parse::<EffectId>().unwrap());
    assert_eq!(effects[0].kind.to_string(), "blur");
    assert_eq!(effects[1].id, data.effect_id);
    assert_eq!(effects[1].kind.to_string(), "blur");
}

#[test]
fn compute_patch_targets_second_clip_by_uuid() {
    let prior = project_with_tracks(vec![text_track(
        TRACK_TEXT_B,
        false,
        CLIP_TEXT_B,
        false,
        Vec::new(),
    )]);
    let (patch, _warnings, data) = compute_patch(
        &prior,
        &EffectAddArgs {
            target: format!("clip:{CLIP_TEXT_B}"),
            ..args("blur")
        },
    )
    .expect("second clip target");
    let post = apply_patch(&prior, patch);

    assert_eq!(data.target_id.to_string(), CLIP_TEXT_B);
    assert_eq!(post.tracks[0].clips[0].effects[0].id, data.effect_id);
}
