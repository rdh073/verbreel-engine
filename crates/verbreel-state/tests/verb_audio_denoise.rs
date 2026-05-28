//! Tests for `audio.denoise` (§9.4) - managed denoise marker.

use std::sync::Arc;

use serde_json::{Map, Value, json};
use verbreel_state::verbs::audio_denoise::{
    W_AUDIO_DENOISE_ENVELOPE_CODE, W_DENOISE_DEFAULT_STRENGTH_CODE, W_KEYFRAMES_REMOVED_CODE,
    W_NOOP_CODE, W_NOOP_FLAG_CODE, compute_patch, data_envelope_from_args_warnings_post_state,
};
use verbreel_state::verbs::effect_add::compute_patch as effect_add_compute_patch;
use verbreel_state::verbs::effect_remove::compute_patch as effect_remove_compute_patch;
use verbreel_state::verbs::effect_set_param::compute_patch as effect_set_param_compute_patch;
use verbreel_state::{
    AudioDenoiseArgs, AudioDenoiseData, AudioDenoiseError, AudioDenoiseVerb, EffectAddArgs,
    EffectAddError, EffectRemoveArgs, EffectRemoveError, EffectSetParamArgs, EffectSetParamError,
    MutateOutcome, Project, RecordedEvent, Track, TrackKind, Verb, VerbError, VerbRegistry,
    default_fixtures, default_registry, validate_reconstructors,
};
use verbreel_types::{ProjectId, Tick};

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const TRACK_AUDIO_A: &str = "0190b8d3-15e3-7000-bd00-0000000aa904";
const TRACK_AUDIO_B: &str = "0190b8d3-15e3-7000-bd00-0000000aa905";
const TRACK_VIDEO_A: &str = "0190b8d3-15e3-7000-bd00-0000000aa9a4";
const TRACK_TEXT_A: &str = "0190b8d3-15e3-7000-bd00-0000000aa9a5";
const CLIP_AUDIO_A: &str = "0190b8d3-15e3-7000-bd00-0000000bb904";
const CLIP_AUDIO_B: &str = "0190b8d3-15e3-7000-bd00-0000000bb905";
const CLIP_VIDEO_A: &str = "0190b8d3-15e3-7000-bd00-0000000bb9a4";
const CLIP_TEXT_A: &str = "0190b8d3-15e3-7000-bd00-0000000bb9a5";
const MISSING_CLIP: &str = "0190b8d3-15e3-7000-bd00-0000000cc9a4";
const MISSING_TRACK: &str = "0190b8d3-15e3-7000-bd00-0000000cc9a5";
const ASSET_AUDIO_ID: &str = "0190b8d3-15e3-7000-bd00-0000000dd904";
const ASSET_VIDEO_ID: &str = "0190b8d3-15e3-7000-bd00-0000000dd9a4";
const EFFECT_DENOISE: &str = "0190b8d3-15e3-7000-bd00-0000000ee904";
const EFFECT_OTHER: &str = "0190b8d3-15e3-7000-bd00-0000000ee905";
const KEYFRAME_A: &str = "0190b8d3-15e3-7000-bd00-0000000ff904";
const KEYFRAME_B: &str = "0190b8d3-15e3-7000-bd00-0000000ff905";
const KEYFRAME_C: &str = "0190b8d3-15e3-7000-bd00-0000000ff906";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture -> Project")
}

fn fixture_project_id() -> ProjectId {
    empty_project().id
}

fn audio_asset_json(id: &str) -> Value {
    json!({
        "id": id,
        "kind": "audio",
        "hash": "53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658",
        "path": "assets/53/53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.m4a",
        "original_filename": "audio-denoise.m4a",
        "imported_at": "2026-05-24T00:00:00Z",
        "metadata": {
            "duration_tk": 240_000,
            "audio_codec": "aac",
            "audio_channels": 2,
            "audio_sample_rate_hz": 48_000,
            "container": "m4a",
            "fingerprint": {
                "mtime_ms": 1_700_000_000_000_i64,
                "size_bytes": 512,
            }
        }
    })
}

fn video_asset_json(id: &str) -> Value {
    json!({
        "id": id,
        "kind": "video",
        "hash": "36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da",
        "path": "assets/36/36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da.mp4",
        "original_filename": "video.mp4",
        "imported_at": "2026-05-24T00:00:00Z",
        "metadata": {
            "duration_tk": 240_000,
            "width": 1920,
            "height": 1080,
            "fps_num": 30,
            "fps_den": 1,
            "video_codec": "h264",
            "container": "mp4",
            "fingerprint": {
                "mtime_ms": 1_700_000_000_000_i64,
                "size_bytes": 1024,
            }
        }
    })
}

fn denoise_effect(id: &str, strength: f64) -> Value {
    json!({
        "id": id,
        "kind": "denoise",
        "enabled": true,
        "params": {
            "strength": strength,
        },
    })
}

fn other_effect(id: &str) -> Value {
    json!({
        "id": id,
        "kind": "eq",
        "enabled": true,
        "params": {
            "gain_db": 3,
        },
    })
}

fn keyframe(id: &str, property: &str) -> Value {
    json!({
        "id": id,
        "property": property,
        "time_tk": 10,
        "value": 0.25,
        "easing": "linear",
    })
}

fn effect_property(effect_id: &str, leaf: &str) -> String {
    format!("effects[{effect_id}].params.{leaf}")
}

#[allow(clippy::too_many_arguments)]
fn clip_track(
    kind: TrackKind,
    track_id: &str,
    track_locked: bool,
    clip_id: &str,
    clip_locked: bool,
    asset_id: &str,
    effects: Vec<Value>,
    keyframes: Vec<Value>,
) -> Track {
    let mut clip = json!({
        "id": clip_id,
        "name": "Denoise Clip",
        "asset_id": asset_id,
        "track_position_tk": 0,
        "source_in_tk": 0,
        "source_out_tk": 240_000,
        "locked": clip_locked,
        "effects": effects,
        "keyframes": keyframes,
    });
    if kind == TrackKind::Text {
        clip.as_object_mut().expect("clip is object").insert(
            "text".to_string(),
            json!({
                "content": "hello",
                "font_family": "Arial",
                "font_size_px": 24,
            }),
        );
    }

    serde_json::from_value(json!({
        "id": track_id,
        "kind": kind,
        "name": "Track",
        "locked": track_locked,
        "clips": [clip],
    }))
    .expect("clip track fixture parses")
}

fn track_with_effects(kind: TrackKind, track_id: &str, locked: bool, effects: Vec<Value>) -> Track {
    serde_json::from_value(json!({
        "id": track_id,
        "kind": kind,
        "name": "Track",
        "locked": locked,
        "clips": [],
        "effects": effects,
    }))
    .expect("track fixture parses")
}

fn project_with_tracks(tracks: Vec<Track>) -> Project {
    let mut project = empty_project();
    let any_clip = tracks.iter().any(|track| !track.clips.is_empty());
    project.tracks = tracks;
    project.duration_tk = if any_clip {
        Tick::new(240_000)
    } else {
        Tick::new(0)
    };
    project
}

fn project_with_audio_clip(
    effects: Vec<Value>,
    keyframes: Vec<Value>,
    track_locked: bool,
    clip_locked: bool,
) -> Project {
    let mut project = project_with_tracks(vec![clip_track(
        TrackKind::Audio,
        TRACK_AUDIO_A,
        track_locked,
        CLIP_AUDIO_A,
        clip_locked,
        ASSET_AUDIO_ID,
        effects,
        keyframes,
    )]);
    project.assets.push(
        serde_json::from_value(audio_asset_json(ASSET_AUDIO_ID)).expect("audio asset parses"),
    );
    project
}

fn project_with_audio_track_effects(effects: Vec<Value>, locked: bool) -> Project {
    project_with_tracks(vec![track_with_effects(
        TrackKind::Audio,
        TRACK_AUDIO_A,
        locked,
        effects,
    )])
}

fn args(target: &str, strength: Option<f64>) -> AudioDenoiseArgs {
    AudioDenoiseArgs {
        project_id: fixture_project_id(),
        target: target.to_string(),
        strength,
    }
}

fn apply_patch(prior: &Project, patch: Value) -> Project {
    let patch: json_patch::Patch = serde_json::from_value(patch).expect("patch parses");
    prior.apply(&patch).expect("patch applies")
}

fn warning_codes(warnings: &[Value]) -> Vec<String> {
    warnings
        .iter()
        .map(|warning| warning["code"].as_str().expect("code string").to_string())
        .collect()
}

fn clip_denoise_ids(project: &Project) -> Vec<String> {
    project.tracks[0].clips[0]
        .effects
        .iter()
        .filter(|effect| effect.kind.as_str() == "denoise")
        .map(|effect| effect.id.to_string())
        .collect()
}

fn clip_denoise_strength(project: &Project) -> f64 {
    project.tracks[0].clips[0]
        .effects
        .iter()
        .find(|effect| effect.kind.as_str() == "denoise")
        .and_then(|effect| effect.params.get("strength"))
        .and_then(Value::as_f64)
        .expect("denoise strength present")
}

fn track_denoise_strength(project: &Project) -> f64 {
    project.tracks[0]
        .effects
        .iter()
        .find(|effect| effect.kind.as_str() == "denoise")
        .and_then(|effect| effect.params["strength"].as_f64())
        .expect("track denoise strength present")
}

fn keyframe_ids(project: &Project) -> Vec<String> {
    project.tracks[0].clips[0]
        .keyframes
        .iter()
        .map(|keyframe| keyframe.id.to_string())
        .collect()
}

fn assert_reconstruct_round_trip(prior: &Project, args: &AudioDenoiseArgs) {
    let (patch, warnings, data) = compute_patch(prior, args).expect("compute patch");
    let post = apply_patch(prior, patch.clone());
    let reconstructed =
        data_envelope_from_args_warnings_post_state(args, &warnings, &post).expect("reconstruct");
    assert_eq!(reconstructed, data);

    let verb = AudioDenoiseVerb;
    let value = verb
        .reconstruct(
            &serde_json::to_value(args).expect("args serialize"),
            &patch,
            &warnings,
            &post,
        )
        .expect("verb reconstruct");
    assert_eq!(value, serde_json::to_value(data).expect("data serializes"));
}

#[test]
fn args_deserialize_round_trip() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "target": format!("clip:{CLIP_AUDIO_A}"),
        "strength": 0.75,
    });
    let typed: AudioDenoiseArgs = serde_json::from_value(raw).expect("args parse");
    assert_eq!(typed.target, format!("clip:{CLIP_AUDIO_A}"));
    assert_eq!(typed.strength, Some(0.75));
}

#[test]
fn args_unknown_field_is_rejected() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "target": format!("clip:{CLIP_AUDIO_A}"),
        "strength": 0.75,
        "codec": "aac",
    });
    let err = serde_json::from_value::<AudioDenoiseArgs>(raw).expect_err("unknown field rejects");
    assert!(err.to_string().contains("unknown field"));
}

#[test]
fn verb_missing_target_maps_to_bad_args() {
    let prior = project_with_audio_clip(Vec::new(), Vec::new(), false, false);
    let verb = AudioDenoiseVerb;
    let err = verb
        .compute_patch(&prior, &json!({ "project_id": FIXTURE_PROJECT_ID }))
        .expect_err("missing target rejects");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn verb_unknown_field_maps_to_bad_args() {
    let prior = project_with_audio_clip(Vec::new(), Vec::new(), false, false);
    let verb = AudioDenoiseVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "target": format!("clip:{CLIP_AUDIO_A}"),
                "strength": 0.2,
                "unexpected": true,
            }),
        )
        .expect_err("unknown field rejects");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn bare_target_errors_bad_selector() {
    let prior = project_with_audio_clip(Vec::new(), Vec::new(), false, false);
    let err = compute_patch(&prior, &args(CLIP_AUDIO_A, Some(0.5))).expect_err("bare target");
    assert!(matches!(err, AudioDenoiseError::BadSelector { .. }));
    assert!(err.to_string().contains("E_BAD_SELECTOR"));
}

#[test]
fn empty_target_errors_bad_selector() {
    let prior = project_with_audio_clip(Vec::new(), Vec::new(), false, false);
    let err = compute_patch(&prior, &args("", Some(0.5))).expect_err("empty target");
    assert!(matches!(err, AudioDenoiseError::BadSelector { .. }));
}

#[test]
fn bad_prefix_errors_bad_selector() {
    let prior = project_with_audio_clip(Vec::new(), Vec::new(), false, false);
    let err = compute_patch(
        &prior,
        &args(&format!("effect:{EFFECT_DENOISE}"), Some(0.5)),
    )
    .expect_err("bad prefix");
    assert!(matches!(err, AudioDenoiseError::BadSelector { .. }));
}

#[test]
fn malformed_clip_target_errors_bad_selector() {
    let prior = project_with_audio_clip(Vec::new(), Vec::new(), false, false);
    let err = compute_patch(&prior, &args("clip:not-a-uuid", Some(0.5)))
        .expect_err("malformed clip selector");
    assert!(matches!(err, AudioDenoiseError::BadSelector { .. }));
}

#[test]
fn malformed_track_target_errors_bad_selector() {
    let prior = project_with_audio_clip(Vec::new(), Vec::new(), false, false);
    let err = compute_patch(&prior, &args("track:not-a-uuid", Some(0.5)))
        .expect_err("malformed track selector");
    assert!(matches!(err, AudioDenoiseError::BadSelector { .. }));
}

#[test]
fn strength_negative_errors_bad_range() {
    let prior = project_with_audio_clip(Vec::new(), Vec::new(), false, false);
    let err = compute_patch(&prior, &args(&format!("clip:{CLIP_AUDIO_A}"), Some(-0.1)))
        .expect_err("negative strength");
    assert!(matches!(err, AudioDenoiseError::BadRange { .. }));
    assert!(err.to_string().contains("E_BAD_RANGE"));
}

#[test]
fn strength_above_one_errors_bad_range() {
    let prior = project_with_audio_clip(Vec::new(), Vec::new(), false, false);
    let err = compute_patch(&prior, &args(&format!("clip:{CLIP_AUDIO_A}"), Some(1.1)))
        .expect_err("strength > 1");
    assert!(matches!(err, AudioDenoiseError::BadRange { .. }));
}

#[test]
fn strength_nan_errors_bad_range() {
    let prior = project_with_audio_clip(Vec::new(), Vec::new(), false, false);
    let err = compute_patch(
        &prior,
        &AudioDenoiseArgs {
            project_id: fixture_project_id(),
            target: format!("clip:{CLIP_AUDIO_A}"),
            strength: Some(f64::NAN),
        },
    )
    .expect_err("NaN strength");
    assert!(matches!(err, AudioDenoiseError::BadRange { .. }));
}

#[test]
fn strength_infinity_errors_bad_range() {
    let prior = project_with_audio_clip(Vec::new(), Vec::new(), false, false);
    let err = compute_patch(
        &prior,
        &AudioDenoiseArgs {
            project_id: fixture_project_id(),
            target: format!("clip:{CLIP_AUDIO_A}"),
            strength: Some(f64::INFINITY),
        },
    )
    .expect_err("infinite strength");
    assert!(matches!(err, AudioDenoiseError::BadRange { .. }));
}

#[test]
fn missing_clip_errors_not_found() {
    let prior = project_with_audio_clip(Vec::new(), Vec::new(), false, false);
    let err = compute_patch(&prior, &args(&format!("clip:{MISSING_CLIP}"), Some(0.5)))
        .expect_err("missing clip");
    assert!(matches!(
        err,
        AudioDenoiseError::NotFound {
            target_kind: "clip",
            ..
        }
    ));
}

#[test]
fn missing_track_errors_not_found() {
    let prior = project_with_audio_clip(Vec::new(), Vec::new(), false, false);
    let err = compute_patch(&prior, &args(&format!("track:{MISSING_TRACK}"), Some(0.5)))
        .expect_err("missing track");
    assert!(matches!(
        err,
        AudioDenoiseError::NotFound {
            target_kind: "track",
            ..
        }
    ));
}

#[test]
fn video_clip_errors_clip_kind_mismatch() {
    let mut prior = project_with_tracks(vec![clip_track(
        TrackKind::Video,
        TRACK_VIDEO_A,
        false,
        CLIP_VIDEO_A,
        false,
        ASSET_VIDEO_ID,
        Vec::new(),
        Vec::new(),
    )]);
    prior.assets.push(
        serde_json::from_value(video_asset_json(ASSET_VIDEO_ID)).expect("video asset parses"),
    );
    let err = compute_patch(&prior, &args(&format!("clip:{CLIP_VIDEO_A}"), Some(0.5)))
        .expect_err("video clip rejected");
    assert!(matches!(
        err,
        AudioDenoiseError::ClipKindMismatch {
            actual_kind: "video",
            ..
        }
    ));
}

#[test]
fn text_clip_errors_clip_kind_mismatch() {
    let prior = project_with_tracks(vec![clip_track(
        TrackKind::Text,
        TRACK_TEXT_A,
        false,
        CLIP_TEXT_A,
        false,
        "00000000-0000-0000-0000-000000000000",
        Vec::new(),
        Vec::new(),
    )]);
    let err = compute_patch(&prior, &args(&format!("clip:{CLIP_TEXT_A}"), Some(0.5)))
        .expect_err("text clip rejected");
    assert!(matches!(
        err,
        AudioDenoiseError::ClipKindMismatch {
            actual_kind: "text",
            ..
        }
    ));
}

#[test]
fn non_audio_track_errors_track_kind_mismatch() {
    let prior = project_with_tracks(vec![track_with_effects(
        TrackKind::Video,
        TRACK_VIDEO_A,
        false,
        Vec::new(),
    )]);
    let err = compute_patch(&prior, &args(&format!("track:{TRACK_VIDEO_A}"), Some(0.5)))
        .expect_err("video track rejected");
    assert!(matches!(
        err,
        AudioDenoiseError::TrackKindMismatch {
            actual_kind: "video",
            ..
        }
    ));
}

#[test]
fn locked_clip_errors() {
    let prior = project_with_audio_clip(Vec::new(), Vec::new(), false, true);
    let err = compute_patch(&prior, &args(&format!("clip:{CLIP_AUDIO_A}"), Some(0.5)))
        .expect_err("locked clip");
    assert!(matches!(
        err,
        AudioDenoiseError::Locked { kind: "clip", .. }
    ));
}

#[test]
fn locked_parent_track_errors() {
    let prior = project_with_audio_clip(Vec::new(), Vec::new(), true, false);
    let err = compute_patch(&prior, &args(&format!("clip:{CLIP_AUDIO_A}"), Some(0.5)))
        .expect_err("locked parent track");
    assert!(matches!(
        err,
        AudioDenoiseError::Locked { kind: "track", .. }
    ));
}

#[test]
fn locked_target_track_errors() {
    let prior = project_with_audio_track_effects(Vec::new(), true);
    let err = compute_patch(&prior, &args(&format!("track:{TRACK_AUDIO_A}"), Some(0.5)))
        .expect_err("locked target track");
    assert!(matches!(
        err,
        AudioDenoiseError::Locked { kind: "track", .. }
    ));
}

#[test]
fn resolution_table_clip_branches_match_spec() {
    let cases = [
        (
            "no_prior_omitted",
            Vec::new(),
            None,
            false,
            Some(W_DENOISE_DEFAULT_STRENGTH_CODE),
        ),
        (
            "no_prior_zero",
            Vec::new(),
            Some(0.0),
            true,
            Some(W_NOOP_CODE),
        ),
        ("no_prior_explicit", Vec::new(), Some(0.4), false, None),
        (
            "prior_omitted",
            vec![denoise_effect(EFFECT_DENOISE, 0.6)],
            None,
            true,
            Some(W_NOOP_FLAG_CODE),
        ),
        (
            "prior_update",
            vec![denoise_effect(EFFECT_DENOISE, 0.6)],
            Some(0.2),
            false,
            None,
        ),
        (
            "prior_remove",
            vec![denoise_effect(EFFECT_DENOISE, 0.6)],
            Some(0.0),
            false,
            Some(W_AUDIO_DENOISE_ENVELOPE_CODE),
        ),
    ];

    for (name, effects, strength, patch_empty, warning_code) in cases {
        let prior = project_with_audio_clip(effects, Vec::new(), false, false);
        let (patch, warnings, _data) =
            compute_patch(&prior, &args(&format!("clip:{CLIP_AUDIO_A}"), strength))
                .unwrap_or_else(|err| panic!("{name} must compute: {err}"));
        assert_eq!(
            patch.as_array().expect("patch array").is_empty(),
            patch_empty,
            "{name} patch emptiness"
        );
        if let Some(code) = warning_code {
            assert!(
                warning_codes(&warnings).contains(&code.to_string()),
                "{name} warnings include {code}"
            );
        } else {
            assert!(warnings.is_empty(), "{name} warnings empty");
        }
    }
}

#[test]
fn clip_create_default_adds_effect_and_warning() {
    let prior = project_with_audio_clip(Vec::new(), Vec::new(), false, false);
    let (patch, warnings, data) =
        compute_patch(&prior, &args(&format!("clip:{CLIP_AUDIO_A}"), None))
            .expect("create default");
    let post = apply_patch(&prior, patch);

    assert_eq!(data.target_id, CLIP_AUDIO_A);
    assert!(data.effect_id.is_some());
    assert!(data.removed_effect_id.is_none());
    assert!((clip_denoise_strength(&post) - 0.5).abs() < 1e-12);
    assert_eq!(
        warning_codes(&warnings),
        vec![W_DENOISE_DEFAULT_STRENGTH_CODE]
    );
    assert_eq!(warnings[0]["details"]["target_id"], CLIP_AUDIO_A);
    assert_eq!(warnings[0]["details"]["applied_strength"], 0.5);
}

#[test]
fn clip_create_explicit_uses_supplied_strength() {
    let prior = project_with_audio_clip(Vec::new(), Vec::new(), false, false);
    let (patch, warnings, data) =
        compute_patch(&prior, &args(&format!("clip:{CLIP_AUDIO_A}"), Some(0.7)))
            .expect("create explicit");
    let post = apply_patch(&prior, patch);

    assert!(data.effect_id.is_some());
    assert!(warnings.is_empty());
    assert!((clip_denoise_strength(&post) - 0.7).abs() < 1e-12);
}

#[test]
fn track_create_default_mutates_untyped_track_effects() {
    let prior = project_with_audio_track_effects(Vec::new(), false);
    let (patch, warnings, data) =
        compute_patch(&prior, &args(&format!("track:{TRACK_AUDIO_A}"), None))
            .expect("track create default");
    let post = apply_patch(&prior, patch);

    assert_eq!(data.target_id, TRACK_AUDIO_A);
    assert!(data.effect_id.is_some());
    assert!((track_denoise_strength(&post) - 0.5).abs() < 1e-12);
    assert_eq!(
        warning_codes(&warnings),
        vec![W_DENOISE_DEFAULT_STRENGTH_CODE]
    );
}

#[test]
fn existing_clip_omitted_strength_is_noop_flag() {
    let prior = project_with_audio_clip(
        vec![denoise_effect(EFFECT_DENOISE, 0.6)],
        Vec::new(),
        false,
        false,
    );
    let (patch, warnings, data) =
        compute_patch(&prior, &args(&format!("clip:{CLIP_AUDIO_A}"), None))
            .expect("omitted existing");

    assert!(patch.as_array().expect("patch array").is_empty());
    assert_eq!(
        data.effect_id.expect("effect id").to_string(),
        EFFECT_DENOISE
    );
    assert_eq!(warning_codes(&warnings), vec![W_NOOP_FLAG_CODE]);
    assert_eq!(warnings[0]["details"]["flag"], "strength");
}

#[test]
fn existing_track_omitted_strength_is_noop_flag() {
    let prior = project_with_audio_track_effects(vec![denoise_effect(EFFECT_DENOISE, 0.6)], false);
    let (patch, warnings, data) =
        compute_patch(&prior, &args(&format!("track:{TRACK_AUDIO_A}"), None))
            .expect("track omitted existing");

    assert!(patch.as_array().expect("patch array").is_empty());
    assert_eq!(
        data.effect_id.expect("effect id").to_string(),
        EFFECT_DENOISE
    );
    assert_eq!(warning_codes(&warnings), vec![W_NOOP_FLAG_CODE]);
}

#[test]
fn existing_clip_updates_strength_without_new_effect() {
    let prior = project_with_audio_clip(
        vec![
            other_effect(EFFECT_OTHER),
            denoise_effect(EFFECT_DENOISE, 0.6),
        ],
        Vec::new(),
        false,
        false,
    );
    let (patch, warnings, data) =
        compute_patch(&prior, &args(&format!("clip:{CLIP_AUDIO_A}"), Some(0.2)))
            .expect("update existing");
    let post = apply_patch(&prior, patch);

    assert!(warnings.is_empty());
    assert_eq!(
        data.effect_id.expect("effect id").to_string(),
        EFFECT_DENOISE
    );
    assert_eq!(clip_denoise_ids(&post), vec![EFFECT_DENOISE.to_string()]);
    assert!((clip_denoise_strength(&post) - 0.2).abs() < 1e-12);
    assert_eq!(post.tracks[0].clips[0].effects.len(), 2);
}

#[test]
fn existing_track_updates_strength_without_new_effect() {
    let prior = project_with_audio_track_effects(vec![denoise_effect(EFFECT_DENOISE, 0.6)], false);
    let (patch, warnings, data) =
        compute_patch(&prior, &args(&format!("track:{TRACK_AUDIO_A}"), Some(0.25)))
            .expect("track update existing");
    let post = apply_patch(&prior, patch);

    assert!(warnings.is_empty());
    assert_eq!(
        data.effect_id.expect("effect id").to_string(),
        EFFECT_DENOISE
    );
    assert!((track_denoise_strength(&post) - 0.25).abs() < 1e-12);
    assert_eq!(post.tracks[0].effects.len(), 1);
}

#[test]
fn no_prior_zero_is_noop_with_null_effect_id() {
    let prior = project_with_audio_clip(Vec::new(), Vec::new(), false, false);
    let (patch, warnings, data) =
        compute_patch(&prior, &args(&format!("clip:{CLIP_AUDIO_A}"), Some(0.0)))
            .expect("no prior zero no-op");

    assert!(patch.as_array().expect("patch array").is_empty());
    assert!(data.effect_id.is_none());
    assert!(data.removed_effect_id.is_none());
    assert_eq!(warning_codes(&warnings), vec![W_NOOP_CODE]);
}

#[test]
fn clip_remove_existing_without_keyframes_returns_removed_effect() {
    let prior = project_with_audio_clip(
        vec![denoise_effect(EFFECT_DENOISE, 0.6)],
        Vec::new(),
        false,
        false,
    );
    let (patch, warnings, data) =
        compute_patch(&prior, &args(&format!("clip:{CLIP_AUDIO_A}"), Some(0.0)))
            .expect("remove existing");
    let post = apply_patch(&prior, patch);

    assert!(data.effect_id.is_none());
    assert_eq!(
        data.removed_effect_id.expect("removed").to_string(),
        EFFECT_DENOISE
    );
    assert!(data.removed_keyframe_ids.is_empty());
    assert!(clip_denoise_ids(&post).is_empty());
    assert_eq!(
        warning_codes(&warnings),
        vec![W_AUDIO_DENOISE_ENVELOPE_CODE]
    );
}

#[test]
fn clip_remove_existing_cascades_targeted_keyframes() {
    let prior = project_with_audio_clip(
        vec![
            denoise_effect(EFFECT_DENOISE, 0.6),
            other_effect(EFFECT_OTHER),
        ],
        vec![
            keyframe(KEYFRAME_A, &effect_property(EFFECT_DENOISE, "strength")),
            keyframe(KEYFRAME_B, &effect_property(EFFECT_OTHER, "gain_db")),
            keyframe(KEYFRAME_C, "volume"),
        ],
        false,
        false,
    );
    let (patch, warnings, data) =
        compute_patch(&prior, &args(&format!("clip:{CLIP_AUDIO_A}"), Some(0.0)))
            .expect("remove with cascade");
    let post = apply_patch(&prior, patch);

    assert_eq!(
        data.removed_keyframe_ids
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        vec![KEYFRAME_A.to_string()]
    );
    assert_eq!(
        keyframe_ids(&post),
        vec![KEYFRAME_B.to_string(), KEYFRAME_C.to_string()]
    );
    assert_eq!(
        warning_codes(&warnings),
        vec![W_AUDIO_DENOISE_ENVELOPE_CODE, W_KEYFRAMES_REMOVED_CODE]
    );
    assert_eq!(warnings[1]["details"]["clip_id"], CLIP_AUDIO_A);
    assert_eq!(
        warnings[1]["details"]["removed_keyframe_ids"],
        json!([KEYFRAME_A])
    );
}

#[test]
fn track_remove_existing_has_no_keyframe_cascade() {
    let prior = project_with_audio_track_effects(vec![denoise_effect(EFFECT_DENOISE, 0.6)], false);
    let (patch, warnings, data) =
        compute_patch(&prior, &args(&format!("track:{TRACK_AUDIO_A}"), Some(0.0)))
            .expect("track remove");
    let post = apply_patch(&prior, patch);

    assert!(data.effect_id.is_none());
    assert_eq!(
        data.removed_effect_id.expect("removed").to_string(),
        EFFECT_DENOISE
    );
    assert!(data.removed_keyframe_ids.is_empty());
    assert!(post.tracks[0].effects.is_empty());
    assert_eq!(
        warning_codes(&warnings),
        vec![W_AUDIO_DENOISE_ENVELOPE_CODE]
    );
}

#[test]
fn create_does_not_duplicate_existing_denoise() {
    let prior = project_with_audio_clip(
        vec![denoise_effect(EFFECT_DENOISE, 0.6)],
        Vec::new(),
        false,
        false,
    );
    let (patch, _warnings, _data) =
        compute_patch(&prior, &args(&format!("clip:{CLIP_AUDIO_A}"), Some(0.8)))
            .expect("updates existing");
    let post = apply_patch(&prior, patch);

    assert_eq!(clip_denoise_ids(&post), vec![EFFECT_DENOISE.to_string()]);
}

#[test]
fn data_null_fields_serialize_as_json_null() {
    let data = AudioDenoiseData {
        target_id: CLIP_AUDIO_A.to_string(),
        effect_id: None,
        removed_effect_id: None,
        removed_keyframe_ids: Vec::new(),
    };
    let value = serde_json::to_value(data).expect("data serializes");
    assert!(value.get("effect_id").expect("effect_id present").is_null());
    assert!(
        value
            .get("removed_effect_id")
            .expect("removed_effect_id present")
            .is_null()
    );
    assert_eq!(value["removed_keyframe_ids"], json!([]));
}

#[test]
fn warnings_have_expected_shapes() {
    let prior = project_with_audio_clip(Vec::new(), Vec::new(), false, false);
    let (_patch, default_warnings, _data) =
        compute_patch(&prior, &args(&format!("clip:{CLIP_AUDIO_A}"), None))
            .expect("default create");
    assert_eq!(default_warnings[0]["details"]["applied_strength"], 0.5);

    let (_patch, noop_warnings, _data) =
        compute_patch(&prior, &args(&format!("clip:{CLIP_AUDIO_A}"), Some(0.0))).expect("no-op");
    assert_eq!(noop_warnings[0]["code"], W_NOOP_CODE);
    assert_eq!(noop_warnings[0]["details"]["target_id"], CLIP_AUDIO_A);

    let prior = project_with_audio_clip(
        vec![denoise_effect(EFFECT_DENOISE, 0.6)],
        Vec::new(),
        false,
        false,
    );
    let (_patch, flag_warnings, _data) =
        compute_patch(&prior, &args(&format!("clip:{CLIP_AUDIO_A}"), None)).expect("flag no-op");
    assert_eq!(flag_warnings[0]["details"]["flag"], "strength");
    assert_eq!(flag_warnings[0]["details"]["effect_id"], EFFECT_DENOISE);
}

#[test]
fn reconstruct_create_round_trip() {
    let prior = project_with_audio_clip(Vec::new(), Vec::new(), false, false);
    assert_reconstruct_round_trip(&prior, &args(&format!("clip:{CLIP_AUDIO_A}"), Some(0.3)));
}

#[test]
fn reconstruct_update_round_trip() {
    let prior = project_with_audio_clip(
        vec![denoise_effect(EFFECT_DENOISE, 0.6)],
        Vec::new(),
        false,
        false,
    );
    assert_reconstruct_round_trip(&prior, &args(&format!("clip:{CLIP_AUDIO_A}"), Some(0.3)));
}

#[test]
fn reconstruct_remove_round_trip() {
    let prior = project_with_audio_clip(
        vec![denoise_effect(EFFECT_DENOISE, 0.6)],
        vec![keyframe(
            KEYFRAME_A,
            &effect_property(EFFECT_DENOISE, "strength"),
        )],
        false,
        false,
    );
    assert_reconstruct_round_trip(&prior, &args(&format!("clip:{CLIP_AUDIO_A}"), Some(0.0)));
}

#[test]
fn reconstruct_noop_round_trip() {
    let prior = project_with_audio_clip(Vec::new(), Vec::new(), false, false);
    assert_reconstruct_round_trip(&prior, &args(&format!("clip:{CLIP_AUDIO_A}"), Some(0.0)));
}

#[test]
fn removal_reconstruct_requires_internal_envelope_warning() {
    let prior = project_with_audio_clip(
        vec![denoise_effect(EFFECT_DENOISE, 0.6)],
        Vec::new(),
        false,
        false,
    );
    let (patch, _warnings, _data) =
        compute_patch(&prior, &args(&format!("clip:{CLIP_AUDIO_A}"), Some(0.0))).expect("remove");
    let post = apply_patch(&prior, patch);
    let err = data_envelope_from_args_warnings_post_state(
        &args(&format!("clip:{CLIP_AUDIO_A}"), Some(0.0)),
        &[],
        &post,
    )
    .expect_err("removal without envelope cannot reconstruct");
    assert!(err.to_string().contains(W_AUDIO_DENOISE_ENVELOPE_CODE));
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "audio.denoise")
        .expect("default_fixtures includes audio.denoise");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(AudioDenoiseVerb))
        .expect("register audio.denoise verb");

    let report = validate_reconstructors(&registry, &[fixture])
        .expect("audio.denoise reconstruct from fixture");
    assert_eq!(report.verbs_checked, vec!["audio.denoise"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn default_registry_contains_audio_denoise() {
    let registry = default_registry();
    registry
        .get("audio.denoise")
        .expect("audio.denoise registered in default_registry");
}

#[test]
fn standalone_recorded_event_validates() {
    let prior = project_with_audio_clip(Vec::new(), Vec::new(), false, false);
    let args = args(&format!("clip:{CLIP_AUDIO_A}"), Some(0.4));
    let (patch, warnings, data) = compute_patch(&prior, &args).expect("compute");
    let post_state = apply_patch(&prior, patch.clone());
    let recorded = RecordedEvent {
        verb: "audio.denoise".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch,
        warnings,
        post_state,
        expected_data: serde_json::to_value(data).expect("data serialize"),
    };

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(AudioDenoiseVerb))
        .expect("register audio.denoise verb");
    let report = validate_reconstructors(&registry, &[recorded]).expect("validation passes");
    assert_eq!(report.fixtures_run, 1);
}

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        project_with_audio_clip(Vec::new(), Vec::new(), false, false),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry clears the gate and writes project.json");

    let outcome = store
        .mutate_via_verb(
            "audio.denoise",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "target": format!("clip:{CLIP_AUDIO_A}"),
                "strength": 0.35,
            }),
            None,
        )
        .expect("mutate_via_verb happy path");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("happy path must return Applied, got {outcome:?}");
    };

    let data: AudioDenoiseData =
        serde_json::from_value(data).expect("audio.denoise data is AudioDenoiseData");
    assert_eq!(data.target_id, CLIP_AUDIO_A);
    assert!(data.effect_id.is_some());
    assert!(warnings.is_empty());
    assert!((clip_denoise_strength(store.project()) - 0.35).abs() < 1e-12);
}

#[test]
fn raw_effect_add_rejects_managed_denoise() {
    let prior = project_with_audio_clip(Vec::new(), Vec::new(), false, false);
    let err = effect_add_compute_patch(
        &prior,
        &EffectAddArgs {
            project_id: fixture_project_id(),
            target: format!("clip:{CLIP_AUDIO_A}"),
            kind: "denoise".to_string(),
            params: Some(Map::new()),
            in_tk: None,
            out_tk: None,
        },
    )
    .expect_err("raw effect.add denoise rejects");

    assert!(matches!(
        err,
        EffectAddError::ManagedEffect {
            managing_verb: "audio.denoise",
            ..
        }
    ));
}

#[test]
fn raw_effect_remove_rejects_managed_denoise() {
    let prior = project_with_audio_clip(
        vec![denoise_effect(EFFECT_DENOISE, 0.6)],
        Vec::new(),
        false,
        false,
    );
    let err = effect_remove_compute_patch(
        &prior,
        &EffectRemoveArgs {
            project_id: fixture_project_id(),
            effect: EFFECT_DENOISE.to_string(),
        },
    )
    .expect_err("raw effect.remove denoise rejects");

    assert!(matches!(
        err,
        EffectRemoveError::ManagedEffect {
            managing_verb: "audio.denoise",
            ..
        }
    ));
}

#[test]
fn raw_effect_set_param_rejects_managed_denoise() {
    let prior = project_with_audio_clip(
        vec![denoise_effect(EFFECT_DENOISE, 0.6)],
        Vec::new(),
        false,
        false,
    );
    let mut params = Map::new();
    params.insert("strength".to_string(), json!(0.2));
    let err = effect_set_param_compute_patch(
        &prior,
        &EffectSetParamArgs {
            project_id: fixture_project_id(),
            effect: EFFECT_DENOISE.to_string(),
            params,
        },
    )
    .expect_err("raw effect.set_param denoise rejects");

    assert!(matches!(
        err,
        EffectSetParamError::ManagedEffect {
            managing_verb: "audio.denoise",
            ..
        }
    ));
}

#[test]
fn track_target_preserves_other_track_effect_values() {
    let prior = project_with_audio_track_effects(
        vec![
            other_effect(EFFECT_OTHER),
            denoise_effect(EFFECT_DENOISE, 0.6),
        ],
        false,
    );
    let (patch, _warnings, _data) =
        compute_patch(&prior, &args(&format!("track:{TRACK_AUDIO_A}"), Some(0.45)))
            .expect("track update");
    let post = apply_patch(&prior, patch);

    assert_eq!(post.tracks[0].effects.len(), 2);
    assert_eq!(post.tracks[0].effects[0].id.to_string(), EFFECT_OTHER);
    assert!((track_denoise_strength(&post) - 0.45).abs() < 1e-12);
}

#[test]
fn selector_can_target_second_audio_clip() {
    let mut project = project_with_audio_clip(Vec::new(), Vec::new(), false, false);
    project.tracks.push(clip_track(
        TrackKind::Audio,
        TRACK_AUDIO_B,
        false,
        CLIP_AUDIO_B,
        false,
        ASSET_AUDIO_ID,
        Vec::new(),
        Vec::new(),
    ));
    let (patch, _warnings, data) =
        compute_patch(&project, &args(&format!("clip:{CLIP_AUDIO_B}"), Some(0.5)))
            .expect("second clip target");
    let post = apply_patch(&project, patch);

    assert_eq!(data.target_id, CLIP_AUDIO_B);
    assert!(post.tracks[0].clips[0].effects.is_empty());
    assert_eq!(post.tracks[1].clips[0].effects.len(), 1);
}
