//! Regression guards for #384: `Track.effects` is `Vec<Effect>`.
//!
//! The migration from a raw `Vec<serde_json::Value>` to `Vec<Effect>`
//! must (a) flow the `audio.denoise` track patch through the typed
//! `Effect` boundary and (b) preserve the canonical (JCS) bytes of the
//! on-disk effect record so already-recorded event logs and project
//! hashes stay reproducible.

use serde_json::{Value, json};
use verbreel_state::verbs::audio_denoise::compute_patch;
use verbreel_state::{AudioDenoiseArgs, Effect, Project, Track};
use verbreel_types::{ProjectId, Tick};

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const TRACK_AUDIO: &str = "0190b8d3-15e3-7000-bd00-0000000aa904";
const EFFECT_DENOISE: &str = "0190b8d3-15e3-7000-bd00-0000000ee904";

/// The shared empty-project fixture with its default tracks dropped, so
/// each test starts from a clean track list it controls.
fn empty_project() -> Project {
    let mut project: Project = serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture parses");
    project.tracks.clear();
    project.duration_tk = Tick::new(0);
    project
}

/// Build an audio track carrying a single typed denoise effect, sourced
/// from the exact raw-JSON shape that the pre-#384 `Vec<Value>` field
/// stored. The track deserializes through the typed boundary.
fn audio_track_with_denoise(strength: f64) -> Track {
    serde_json::from_value(json!({
        "id": TRACK_AUDIO,
        "kind": "audio",
        "name": "Audio",
        "locked": false,
        "clips": [],
        "effects": [{
            "id": EFFECT_DENOISE,
            "kind": "denoise",
            "enabled": true,
            "params": { "strength": strength },
        }],
    }))
    .expect("audio track with denoise parses")
}

fn args(target: &str, strength: Option<f64>) -> AudioDenoiseArgs {
    AudioDenoiseArgs {
        project_id: PROJECT_ID.parse::<ProjectId>().expect("valid project id"),
        target: target.to_string(),
        strength,
    }
}

/// (a) The track-create patch's `value` payload deserializes as
/// `Vec<Effect>` — the typed boundary, not a raw `Vec<Value>`.
#[test]
fn track_create_patch_value_is_vec_effect() {
    let mut project = empty_project();
    project.tracks.push(
        serde_json::from_value(json!({
            "id": TRACK_AUDIO,
            "kind": "audio",
            "name": "Audio",
            "locked": false,
            "clips": [],
        }))
        .expect("empty audio track parses"),
    );

    let (patch, _warnings, _data) =
        compute_patch(&project, &args(&format!("track:{TRACK_AUDIO}"), Some(0.5)))
            .expect("track create patch");

    let ops = patch.as_array().expect("patch is an array of ops");
    let op = ops
        .iter()
        .find(|op| op.get("path").and_then(Value::as_str) == Some("/tracks/0/effects"));
    let op = op.expect("track create emits a /tracks/0/effects replace op");
    let value = op.get("value").expect("op has a value");

    let effects: Vec<Effect> = serde_json::from_value(value.clone())
        .expect("track effects patch value must deserialize as Vec<Effect>");
    assert_eq!(effects.len(), 1, "one denoise effect appended");
    assert_eq!(effects[0].kind.as_str(), "denoise");
    assert_eq!(effects[0].params["strength"], json!(0.5));
    assert!(effects[0].window.is_none(), "managed denoise has no window");
}

/// (b) Canonical bytes are preserved: the typed `Effect`, re-serialized
/// and JCS-canonicalized, is byte-identical to the canonical form of the
/// raw-Value record the pre-#384 field would have produced.
#[test]
fn typed_track_effect_canonical_bytes_match_raw_value() {
    let track = audio_track_with_denoise(0.5);
    let typed_effect: &Effect = &track.effects[0];

    // What the typed path emits (Effect -> Value -> JCS).
    let typed_value = serde_json::to_value(typed_effect).expect("Effect -> Value");
    let typed_canon =
        verbreel_canon::canonicalize(&typed_value).expect("typed effect canonicalizes");

    // The raw-Value record the legacy `Vec<Value>` field stored verbatim.
    let raw_value = json!({
        "id": EFFECT_DENOISE,
        "kind": "denoise",
        "enabled": true,
        "params": { "strength": 0.5 },
    });
    let raw_canon = verbreel_canon::canonicalize(&raw_value).expect("raw value canonicalizes");

    assert_eq!(
        typed_canon, raw_canon,
        "typed Track.effects[].Effect must canonicalize byte-for-byte \
         identically to the legacy raw-Value record"
    );
}

/// The full track-update patch path round-trips canonically: applying
/// the patch and re-serializing the project yields canonical bytes that
/// equal the same project hand-built with the updated strength.
#[test]
fn track_update_preserves_canonical_project_bytes() {
    let mut project = empty_project();
    project.tracks.push(audio_track_with_denoise(0.5));
    project.duration_tk = Tick::new(0);

    let (patch_value, _warnings, _data) =
        compute_patch(&project, &args(&format!("track:{TRACK_AUDIO}"), Some(0.25)))
            .expect("track update patch");
    let patch: json_patch::Patch =
        serde_json::from_value(patch_value).expect("patch is valid RFC 6902");
    let post = project.apply(&patch).expect("patch applies cleanly");

    // Hand-built expected post-state: same project, strength now 0.25.
    let mut expected = empty_project();
    expected.tracks.push(audio_track_with_denoise(0.25));
    expected.duration_tk = Tick::new(0);

    let post_canon =
        verbreel_canon::canonicalize(&serde_json::to_value(&post).expect("post -> Value"))
            .expect("post canonicalizes");
    let expected_canon =
        verbreel_canon::canonicalize(&serde_json::to_value(&expected).expect("expected -> Value"))
            .expect("expected canonicalizes");

    assert_eq!(
        post_canon, expected_canon,
        "applying the typed track-update patch must yield canonical \
         bytes identical to the hand-built updated project"
    );
}
