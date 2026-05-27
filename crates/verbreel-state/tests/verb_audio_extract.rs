//! Tests for `audio.extract` (§9.1) — eighty-ninth production verb.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::audio_extract::{compute_patch, resolved_codec};
use verbreel_state::{
    AudioExtractArgs, AudioExtractCodec, AudioExtractData, AudioExtractError, AudioExtractVerb,
    Project, Verb, VerbError, VerbRegistry, default_fixtures, default_registry,
    validate_reconstructors,
};
use verbreel_types::{AssetId, ClipId, LinkGroupId, ProjectId, TrackId};

#[cfg(feature = "native")]
use verbreel_state::{LifecycleError, ProjectStore};

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const CLIP_VIDEO_A: &str = "0190b8d3-15e3-7000-bd00-0000000bb901";
const TRACK_AUDIO_A: &str = "0190b8d3-15e3-7000-bd00-0000000aa901";
const ASSET_AUDIO_A: &str = "0190b8d3-15e3-7000-bd00-0000000cc901";
const LINK_GROUP_A: &str = "0190b8d3-15e3-7000-bd00-0000000dd901";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture -> Project")
}

fn fixture_project_id() -> ProjectId {
    empty_project().id
}

fn id<T>(raw: &str) -> T
where
    T: std::str::FromStr,
    T::Err: std::fmt::Debug,
{
    raw.parse().expect("hard-coded UUIDv7 parses")
}

fn args_default() -> AudioExtractArgs {
    AudioExtractArgs {
        project_id: fixture_project_id(),
        clip: CLIP_VIDEO_A.to_string(),
        to_track: None,
        codec: None,
    }
}

fn args_value() -> Value {
    json!({
        "project_id": FIXTURE_PROJECT_ID,
        "clip": CLIP_VIDEO_A,
    })
}

// --- args and codec surface -------------------------------------------------

#[test]
fn args_deserialize_ok_with_all_fields() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "clip": CLIP_VIDEO_A,
        "to_track": format!("track:{TRACK_AUDIO_A}"),
        "codec": "aac",
    });

    let typed: AudioExtractArgs = serde_json::from_value(raw).expect("well-formed args parse");

    assert_eq!(typed.project_id.to_string(), FIXTURE_PROJECT_ID);
    assert_eq!(typed.clip, CLIP_VIDEO_A);
    assert_eq!(typed.to_track, Some(format!("track:{TRACK_AUDIO_A}")));
    assert_eq!(typed.codec, Some(AudioExtractCodec::Aac));
}

#[test]
fn args_deserialize_ok_with_omitted_optionals() {
    let typed: AudioExtractArgs = serde_json::from_value(args_value()).expect("args parse");

    assert_eq!(typed.project_id.to_string(), FIXTURE_PROJECT_ID);
    assert_eq!(typed.clip, CLIP_VIDEO_A);
    assert_eq!(typed.to_track, None);
    assert_eq!(typed.codec, None);
}

#[test]
fn args_missing_project_id_fails_through_verb() {
    let prior = empty_project();
    let verb = AudioExtractVerb;

    let err = verb
        .compute_patch(&prior, &json!({ "clip": CLIP_VIDEO_A }))
        .expect_err("missing project_id should map to BadArgs");

    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_missing_clip_fails_through_verb() {
    let prior = empty_project();
    let verb = AudioExtractVerb;

    let err = verb
        .compute_patch(&prior, &json!({ "project_id": FIXTURE_PROJECT_ID }))
        .expect_err("missing clip should map to BadArgs");

    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_unknown_field_fails_through_verb() {
    let prior = empty_project();
    let verb = AudioExtractVerb;

    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "clip": CLIP_VIDEO_A,
                "extra": true,
            }),
        )
        .expect_err("deny_unknown_fields should reject extra args");

    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn codec_serde_accepts_pcm_s16le() {
    let codec: AudioExtractCodec =
        serde_json::from_value(json!("pcm_s16le")).expect("pcm_s16le parses");
    assert_eq!(codec, AudioExtractCodec::PcmS16Le);
    assert_eq!(
        serde_json::to_value(codec).expect("codec serializes"),
        json!("pcm_s16le")
    );
}

#[test]
fn codec_serde_accepts_aac() {
    let codec: AudioExtractCodec = serde_json::from_value(json!("aac")).expect("aac parses");
    assert_eq!(codec, AudioExtractCodec::Aac);
    assert_eq!(
        serde_json::to_value(codec).expect("codec serializes"),
        json!("aac")
    );
}

#[test]
fn codec_serde_rejects_unknown_literal() {
    let err = serde_json::from_value::<AudioExtractCodec>(json!("flac"))
        .expect_err("non-spec codec literal rejects");
    let msg = err.to_string();
    assert!(
        msg.contains("unknown variant") || msg.contains("flac"),
        "unexpected codec error: {msg}",
    );
}

#[test]
fn default_codec_helper_resolves_omitted_codec_to_pcm_s16le() {
    let args = args_default();
    assert_eq!(resolved_codec(&args), AudioExtractCodec::PcmS16Le);
}

#[test]
fn default_codec_helper_preserves_explicit_codec() {
    let mut args = args_default();
    args.codec = Some(AudioExtractCodec::Aac);
    assert_eq!(resolved_codec(&args), AudioExtractCodec::Aac);
}

// --- data shape -------------------------------------------------------------

#[test]
fn data_shape_omits_optional_fields_when_none() {
    let data = AudioExtractData {
        asset_id: id::<AssetId>(ASSET_AUDIO_A),
        clip_id: None,
        track_id: None,
        link_group: None,
    };

    let value = serde_json::to_value(&data).expect("data serializes");
    let obj = value.as_object().expect("data is object");

    assert_eq!(obj.len(), 1, "only asset_id should serialize");
    assert_eq!(obj.get("asset_id"), Some(&json!(ASSET_AUDIO_A)));
    assert!(!obj.contains_key("clip_id"));
    assert!(!obj.contains_key("track_id"));
    assert!(!obj.contains_key("link_group"));
}

#[test]
fn data_shape_includes_optional_fields_when_some() {
    let data = AudioExtractData {
        asset_id: id::<AssetId>(ASSET_AUDIO_A),
        clip_id: Some(id::<ClipId>(CLIP_VIDEO_A)),
        track_id: Some(id::<TrackId>(TRACK_AUDIO_A)),
        link_group: Some(id::<LinkGroupId>(LINK_GROUP_A)),
    };

    let value = serde_json::to_value(&data).expect("data serializes");
    let obj = value.as_object().expect("data is object");

    assert_eq!(obj.len(), 4);
    assert_eq!(obj.get("asset_id"), Some(&json!(ASSET_AUDIO_A)));
    assert_eq!(obj.get("clip_id"), Some(&json!(CLIP_VIDEO_A)));
    assert_eq!(obj.get("track_id"), Some(&json!(TRACK_AUDIO_A)));
    assert_eq!(obj.get("link_group"), Some(&json!(LINK_GROUP_A)));
}

// --- v1 floor: every well-formed call errors -------------------------------

#[test]
fn compute_patch_always_returns_io_for_well_formed_args() {
    let prior = empty_project();
    let args = args_default();

    let err = compute_patch(&prior, &args).expect_err("v1 floor always errors");

    assert!(matches!(err, AudioExtractError::Io { .. }));
}

#[test]
fn error_text_contains_e_io() {
    let prior = empty_project();
    let args = args_default();
    let err = compute_patch(&prior, &args).expect_err("v1 floor always errors");
    let msg = err.to_string();

    assert!(
        msg.contains("E_IO"),
        "error message `{msg}` should mention E_IO"
    );
}

#[test]
fn error_maps_to_custom_not_bad_args() {
    let prior = empty_project();
    let verb = AudioExtractVerb;

    let err = verb
        .compute_patch(&prior, &args_value())
        .expect_err("v1 floor always errors");

    assert!(
        matches!(err, VerbError::Custom(_)),
        "expected VerbError::Custom, got {err:?}",
    );
}

#[test]
fn verb_custom_error_detail_contains_e_io() {
    let prior = empty_project();
    let verb = AudioExtractVerb;

    let err = verb
        .compute_patch(&prior, &args_value())
        .expect_err("v1 floor always errors");

    let VerbError::Custom(detail) = err else {
        panic!("expected Custom, got {err:?}");
    };
    assert!(detail.contains("E_IO"));
}

#[test]
fn error_path_is_project_agnostic() {
    let prior_a = empty_project();
    let mut prior_b = empty_project();
    prior_b.name = "different-name".to_string();
    prior_b.duration_tk = verbreel_types::Tick::new(987_654);

    let args = args_default();
    let err_a = compute_patch(&prior_a, &args).expect_err("a errors");
    let err_b = compute_patch(&prior_b, &args).expect_err("b errors");

    assert_eq!(err_a, err_b);
}

// --- reconstructor / fixture -----------------------------------------------

#[test]
fn reconstruct_returns_null_for_well_formed_args() {
    let verb = AudioExtractVerb;
    let prior = empty_project();

    let data = verb
        .reconstruct(&args_value(), &json!([]), &[], &prior)
        .expect("reconstruct succeeds for well-formed args");

    assert_eq!(data, Value::Null);
}

#[test]
fn reconstruct_rejects_malformed_args() {
    let verb = AudioExtractVerb;
    let prior = empty_project();

    let err = verb
        .reconstruct(&json!({}), &json!([]), &[], &prior)
        .expect_err("malformed args should fail reconstruction");
    let msg = err.to_string();

    assert!(
        msg.contains("AudioExtractArgs") || msg.contains("wrong type"),
        "unexpected reconstruct error: {msg}",
    );
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "audio.extract")
        .expect("default_fixtures includes audio.extract");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(AudioExtractVerb))
        .expect("register audio.extract verb");

    let report = validate_reconstructors(&registry, &[fixture])
        .expect("audio.extract reconstruct from fixture");

    assert_eq!(report.verbs_checked, vec!["audio.extract"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn default_fixture_has_no_patch_or_warnings() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "audio.extract")
        .expect("default_fixtures includes audio.extract");

    assert_eq!(fixture.patch, json!([]));
    assert!(fixture.warnings.is_empty());
    assert_eq!(fixture.expected_data, Value::Null);
}

#[test]
fn verb_trait_surface_lookup_via_default_registry() {
    let registry = default_registry();
    let verb = registry
        .get("audio.extract")
        .expect("audio.extract registered in default_registry");
    let prior = empty_project();

    let err = verb
        .compute_patch(&prior, &args_value())
        .expect_err("v1 floor always errors");

    let VerbError::Custom(detail) = err else {
        panic!("expected Custom, got {err:?}");
    };
    assert!(detail.contains("E_IO"));
}

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb_and_returns_custom() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        empty_project(),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry succeeds");

    let outcome = store.mutate_via_verb("audio.extract", args_value(), None);

    let Err(LifecycleError::VerbExecutionFailed { source, .. }) = outcome else {
        panic!("expected VerbExecutionFailed for v1 floor, got {outcome:?}");
    };
    let VerbError::Custom(detail) = source else {
        panic!("expected Custom source, got {source:?}");
    };
    assert!(detail.contains("E_IO"));
}
