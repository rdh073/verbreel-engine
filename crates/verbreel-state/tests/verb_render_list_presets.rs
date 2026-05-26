//! Tests for `render.list_presets` (§11.4) — seventy-sixth production verb.

use std::sync::Arc;

use serde_json::json;
use verbreel_state::verbs::render_list_presets::{
    bundled_presets, compute_patch, data_envelope_from_args,
};
use verbreel_state::{
    MutateOutcome, Preset, Project, RenderListPresetsArgs, RenderListPresetsData,
    RenderListPresetsVerb, Verb, VerbError, VerbRegistry, default_fixtures, default_registry,
    validate_reconstructors,
};

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";

const EXPECTED_NAMES: &[&str] = &[
    "youtube-1080p",
    "youtube-shorts-1080x1920",
    "tiktok-1080p",
    "instagram-reel-1080x1920",
    "square-1080p",
    "prores-master",
    "web-h264-720p",
    "web-vp9-1080p",
];

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture → Project")
}

fn fixture_project_id() -> verbreel_types::ProjectId {
    empty_project().id
}

fn find_preset(name: &str) -> Preset {
    bundled_presets()
        .into_iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("bundled_presets must include `{name}`"))
}

#[test]
fn args_deserialize_with_project_id_ok() {
    let args: RenderListPresetsArgs =
        serde_json::from_value(json!({"project_id": FIXTURE_PROJECT_ID}))
            .expect("well-formed args deserialize");
    assert_eq!(
        args.project_id.expect("project_id present").to_string(),
        FIXTURE_PROJECT_ID
    );
}

#[test]
fn args_deserialize_empty_object_ok() {
    // Spec says `Args: none`. `{}` must be accepted.
    let args: RenderListPresetsArgs =
        serde_json::from_value(json!({})).expect("empty args object should deserialize");
    assert!(args.project_id.is_none());
}

#[test]
fn args_deserialize_null_project_id_ok() {
    let args: RenderListPresetsArgs = serde_json::from_value(json!({"project_id": null}))
        .expect("null project_id should deserialize as None");
    assert!(args.project_id.is_none());
}

#[test]
fn args_wrong_project_id_type_is_bad_args() {
    let prior = empty_project();
    let verb = RenderListPresetsVerb;
    let err = verb
        .compute_patch(&prior, &json!({"project_id": 42}))
        .expect_err("non-string project_id should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn spec_compliant_empty_args_succeeds() {
    let prior = empty_project();
    let verb = RenderListPresetsVerb;
    let (_, data, warnings) = verb
        .compute_patch(&prior, &json!({}))
        .expect("spec-compliant empty args must succeed (§11.4: Args: none)");
    assert!(warnings.is_empty());
    let parsed: RenderListPresetsData =
        serde_json::from_value(data).expect("data deserializes to envelope");
    assert_eq!(parsed.presets.len(), 8);
}

#[test]
fn happy_path_returns_exactly_eight_presets() {
    let prior = empty_project();
    let args = RenderListPresetsArgs {
        project_id: Some(fixture_project_id()),
    };
    let (_, _, data) = compute_patch(&prior, &args).expect("compute_patch should succeed");
    assert_eq!(data.presets.len(), 8);
}

#[test]
fn preset_names_in_spec_order() {
    let prior = empty_project();
    let args = RenderListPresetsArgs {
        project_id: Some(fixture_project_id()),
    };
    let (_, _, data) = compute_patch(&prior, &args).expect("compute_patch should succeed");
    let names: Vec<&str> = data.presets.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, EXPECTED_NAMES);
}

#[test]
fn all_preset_names_are_unique() {
    let presets = bundled_presets();
    let mut names: Vec<String> = presets.iter().map(|p| p.name.clone()).collect();
    let original_len = names.len();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), original_len, "duplicate preset name detected");
}

#[test]
fn youtube_shorts_canvas_is_vertical() {
    let p = find_preset("youtube-shorts-1080x1920");
    assert_eq!(
        p.canvas, "1080x1920",
        "youtube-shorts must be vertical 1080x1920, NOT 1920x1080"
    );
}

#[test]
fn prores_master_fps_is_ntsc_drop_frame() {
    let p = find_preset("prores-master");
    assert_eq!(p.fps_num, 30_000);
    assert_eq!(p.fps_den, 1_001);
}

#[test]
fn prores_master_video_codec_is_prores() {
    let p = find_preset("prores-master");
    assert_eq!(p.video_codec, "prores");
}

#[test]
fn prores_master_audio_codec_is_pcm_s16le() {
    let p = find_preset("prores-master");
    assert_eq!(p.audio_codec, "pcm_s16le");
}

#[test]
fn prores_master_is_lossless() {
    let p = find_preset("prores-master");
    assert_eq!(p.crf, None);
    assert_eq!(p.bitrate_bps, None);
}

#[test]
fn web_h264_720p_canvas_is_1280x720() {
    let p = find_preset("web-h264-720p");
    assert_eq!(p.canvas, "1280x720");
}

#[test]
fn web_vp9_1080p_codecs() {
    let p = find_preset("web-vp9-1080p");
    assert_eq!(p.video_codec, "vp9");
    assert_eq!(p.audio_codec, "opus");
}

#[test]
fn all_h264_presets_carry_crf() {
    let presets = bundled_presets();
    for p in presets.iter().filter(|p| p.video_codec == "h264") {
        assert!(
            p.crf.is_some(),
            "h264 preset `{}` must carry a crf, got None",
            p.name
        );
    }
}

#[test]
fn data_shape_has_single_presets_field() {
    let prior = empty_project();
    let args = RenderListPresetsArgs {
        project_id: Some(fixture_project_id()),
    };
    let (_, _, data) = compute_patch(&prior, &args).expect("compute_patch should succeed");
    let value = serde_json::to_value(&data).expect("data serializes");
    let obj = value.as_object().expect("data is an object");
    assert_eq!(obj.len(), 1);
    assert!(obj.contains_key("presets"));
}

#[test]
fn preset_serialization_skips_none_options() {
    let p = find_preset("prores-master");
    let value = serde_json::to_value(&p).expect("preset serializes");
    let obj = value.as_object().expect("preset is an object");
    // prores-master has bitrate_bps: None and crf: None — both skipped
    assert!(!obj.contains_key("bitrate_bps"));
    assert!(!obj.contains_key("crf"));
    // The six required fields are always present.
    for key in [
        "name",
        "canvas",
        "fps_num",
        "fps_den",
        "video_codec",
        "audio_codec",
    ] {
        assert!(obj.contains_key(key), "preset must serialize `{key}`");
    }
}

#[test]
fn preset_serialization_includes_crf_when_some() {
    let p = find_preset("youtube-1080p");
    let value = serde_json::to_value(&p).expect("preset serializes");
    let obj = value.as_object().expect("preset is an object");
    assert!(obj.contains_key("crf"));
    assert_eq!(obj.get("crf").and_then(|v| v.as_u64()), Some(23));
    // bitrate_bps is still None and stays skipped
    assert!(!obj.contains_key("bitrate_bps"));
}

#[test]
fn preset_clone_and_eq_derive_works() {
    let a = find_preset("youtube-1080p");
    let b = a.clone();
    assert_eq!(a, b);
}

#[test]
fn preset_serde_round_trip() {
    for original in bundled_presets() {
        let serialized = serde_json::to_value(&original).expect("serialize");
        let back: Preset = serde_json::from_value(serialized).expect("deserialize");
        assert_eq!(original, back);
    }
}

#[test]
fn verb_is_project_agnostic() {
    // Two different "projects" (the same empty fixture parsed twice) produce
    // identical preset output — confirming the verb ignores project state.
    let prior_a = empty_project();
    let prior_b = empty_project();
    let args = RenderListPresetsArgs {
        project_id: Some(fixture_project_id()),
    };
    let (_, _, data_a) = compute_patch(&prior_a, &args).expect("compute_patch a");
    let (_, _, data_b) = compute_patch(&prior_b, &args).expect("compute_patch b");
    assert_eq!(data_a, data_b);
}

#[test]
fn compute_patch_returns_empty_patch() {
    let prior = empty_project();
    let args = RenderListPresetsArgs {
        project_id: Some(fixture_project_id()),
    };
    let (patch, _, _) = compute_patch(&prior, &args).expect("compute_patch should succeed");
    assert_eq!(patch, json!([]));
}

#[test]
fn compute_patch_warnings_always_empty() {
    let prior = empty_project();
    let args = RenderListPresetsArgs {
        project_id: Some(fixture_project_id()),
    };
    let (_, warnings, _) = compute_patch(&prior, &args).expect("compute_patch should succeed");
    assert!(warnings.is_empty());
}

#[test]
fn reconstructor_round_trip_byte_identical() {
    let prior = empty_project();
    let args = RenderListPresetsArgs {
        project_id: Some(fixture_project_id()),
    };
    let (patch_value, _, expected) =
        compute_patch(&prior, &args).expect("compute_patch should succeed");
    let patch: json_patch::Patch =
        serde_json::from_value(patch_value).expect("patch is valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("empty patch applies to empty project");

    let envelope = data_envelope_from_args(&args, &post_state)
        .expect("data_envelope_from_args should rebuild same data");
    assert_eq!(envelope, expected);

    let a = serde_json::to_value(&envelope).expect("envelope serializes");
    let b = serde_json::to_value(&expected).expect("expected serializes");
    assert_eq!(a, b);
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "render.list_presets")
        .expect("default_fixtures includes render.list_presets");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(RenderListPresetsVerb))
        .expect("register render.list_presets verb");

    let report = validate_reconstructors(&registry, &[fixture])
        .expect("render.list_presets reconstruct from fixture");
    assert_eq!(report.verbs_checked, vec!["render.list_presets"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn verb_trait_surface_via_default_registry() {
    let registry = default_registry();
    let verb = registry
        .get("render.list_presets")
        .expect("default_registry exposes render.list_presets");
    assert_eq!(verb.verb(), "render.list_presets");

    let prior = empty_project();
    let (patch, data, warnings) = verb
        .compute_patch(&prior, &json!({"project_id": FIXTURE_PROJECT_ID}))
        .expect("registered verb compute_patch should succeed");

    assert!(warnings.is_empty());
    let patch_value = serde_json::to_value(&patch).expect("patch → value");
    assert_eq!(patch_value, json!([]));
    let parsed: RenderListPresetsData =
        serde_json::from_value(data).expect("data deserializes to envelope");
    assert_eq!(parsed.presets.len(), 8);
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
            "render.list_presets",
            json!({"project_id": FIXTURE_PROJECT_ID}),
            None,
        )
        .expect("render.list_presets should route");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("expected Applied outcome from render.list_presets");
    };
    assert!(warnings.is_empty());

    let data: RenderListPresetsData =
        serde_json::from_value(data).expect("render.list_presets data deserializes");
    assert_eq!(data.presets.len(), 8);
    let names: Vec<&str> = data.presets.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, EXPECTED_NAMES);
}
