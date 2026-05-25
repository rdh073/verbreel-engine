//! Tests for `track.remove` (§4.2) — thirty-seventh production verb.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::track_remove::{
    TrackRemoveArgs, TrackRemoveData, TrackRemoveError, TrackRemoveVerb, W_KEYFRAMES_REMOVED_CODE,
    compute_patch, data_envelope_from_args_warnings,
};
use verbreel_state::{
    MutateOutcome, Project, TrackKind, VerbRegistry, default_fixtures, default_registry,
    validate_reconstructors,
};
use verbreel_types::Tick;

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";

const TARGET_TRACK: &str = "01900000-0000-7000-8000-0000000aa801";
const SURVIVOR_TRACK: &str = "01900000-0000-7000-8000-0000000aa802";
const SECOND_SURVIVOR_TRACK: &str = "01900000-0000-7000-8000-0000000aa803";
const TEXT_TRACK: &str = "01900000-0000-7000-8000-0000000aa804";
const MISSING_TRACK: &str = "01900000-0000-7000-8000-0000000aa899";

const TARGET_CLIP: &str = "01900000-0000-7000-8000-0000000bb801";
const TARGET_CLIP_2: &str = "01900000-0000-7000-8000-0000000bb802";
const SURVIVOR_CLIP: &str = "01900000-0000-7000-8000-0000000bb803";
const SECOND_SURVIVOR_CLIP: &str = "01900000-0000-7000-8000-0000000bb804";

const ASSET_ID: &str = "01900000-0000-7000-8000-0000000cc801";
const BURNED_EFFECT: &str = "01900000-0000-7000-8000-0000000ee801";
const BLUR_EFFECT: &str = "01900000-0000-7000-8000-0000000ee802";
const KEYFRAME_ID: &str = "01900000-0000-7000-8000-0000000ff801";
const OTHER_KEYFRAME_ID: &str = "01900000-0000-7000-8000-0000000ff802";
const LINK_GROUP: &str = "01900000-0000-7000-8000-0000000dd801";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture -> Project")
}

fn fixture_project_id() -> verbreel_types::ProjectId {
    FIXTURE_PROJECT_ID
        .parse()
        .expect("fixture project id parses")
}

fn video_asset() -> Value {
    json!({
        "id": ASSET_ID,
        "kind": "video",
        "hash": "53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658",
        "path": "assets/53/53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.mp4",
        "original_filename": "track-remove.mp4",
        "imported_at": "2026-05-24T00:00:00Z",
        "metadata": {
            "duration_tk": 480_000,
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

fn video_clip(
    id: &str,
    locked: bool,
    link_group: Option<&str>,
    effects: Vec<Value>,
    keyframes: Vec<Value>,
) -> Value {
    let mut clip = json!({
        "id": id,
        "name": "Video Clip",
        "asset_id": ASSET_ID,
        "track_position_tk": 0,
        "source_in_tk": 0,
        "source_out_tk": 480_000,
        "locked": locked,
        "effects": effects,
        "keyframes": keyframes,
    });
    if let Some(link_group) = link_group {
        clip["link_group"] = json!(link_group);
    }
    clip
}

fn text_clip(id: &str, locked: bool) -> Value {
    json!({
        "id": id,
        "name": "Text Clip",
        "asset_id": "00000000-0000-0000-0000-000000000000",
        "track_position_tk": 0,
        "source_in_tk": 0,
        "source_out_tk": 480_000,
        "locked": locked,
        "text": {
            "content": "Caption",
            "font_family": "Arial",
            "font_size_px": 24
        },
    })
}

fn track(id: &str, kind: TrackKind, locked: bool, clips: Vec<Value>) -> Value {
    json!({
        "id": id,
        "kind": kind,
        "name": "Track",
        "locked": locked,
        "clips": clips,
    })
}

fn project_with_tracks(tracks: Vec<Value>) -> Project {
    let mut project = empty_project();
    project.tracks = tracks
        .into_iter()
        .map(|track| serde_json::from_value(track).expect("track fixture parses"))
        .collect();
    project.assets.clear();
    project
        .assets
        .push(serde_json::from_value(video_asset()).expect("asset fixture parses"));
    project.duration_tk = Tick::new(480_000);
    project
}

fn base_project(target_clips: Vec<Value>) -> Project {
    project_with_tracks(vec![
        track(TARGET_TRACK, TrackKind::Video, false, target_clips),
        track(
            SURVIVOR_TRACK,
            TrackKind::Video,
            false,
            vec![video_clip(SURVIVOR_CLIP, false, None, vec![], vec![])],
        ),
    ])
}

fn base_args(track_id: &str) -> TrackRemoveArgs {
    TrackRemoveArgs {
        project_id: fixture_project_id(),
        track: track_id.to_string(),
    }
}

fn patch_ops(patch: &Value) -> &[Value] {
    patch.as_array().expect("patch is array")
}

fn apply_patch(prior: &Project, patch: Value) -> Project {
    let patch: json_patch::Patch = serde_json::from_value(patch).expect("patch parses");
    prior.apply(&patch).expect("patch applies")
}

fn burned_caption_effect() -> Value {
    json!({
        "id": BURNED_EFFECT,
        "kind": "burned_caption",
        "enabled": true,
        "params": {
            "source_text_track_id": TEXT_TRACK,
        },
    })
}

fn blur_effect() -> Value {
    json!({
        "id": BLUR_EFFECT,
        "kind": "blur",
        "enabled": true,
        "params": {
            "radius_px": 2,
        },
    })
}

fn effect_keyframe(id: &str, effect_id: &str) -> Value {
    json!({
        "id": id,
        "property": format!("effects[{effect_id}].params.opacity"),
        "time_tk": 0,
        "value": 1.0,
        "easing": "linear",
    })
}

fn opacity_keyframe(id: &str) -> Value {
    json!({
        "id": id,
        "property": "opacity",
        "time_tk": 0,
        "value": 1.0,
        "easing": "linear",
    })
}

fn text_cascade_project(effects: Vec<Value>, keyframes: Vec<Value>) -> Project {
    project_with_tracks(vec![
        track(
            TEXT_TRACK,
            TrackKind::Text,
            false,
            vec![text_clip(TARGET_CLIP, false)],
        ),
        track(
            SURVIVOR_TRACK,
            TrackKind::Video,
            false,
            vec![video_clip(SURVIVOR_CLIP, false, None, effects, keyframes)],
        ),
    ])
}

#[test]
fn compute_patch_happy_remove_track_with_one_clip() {
    let prior = base_project(vec![video_clip(TARGET_CLIP, false, None, vec![], vec![])]);

    let (patch, warnings, data) = compute_patch(&prior, &base_args(TARGET_TRACK)).expect("remove");

    assert_eq!(patch_ops(&patch).len(), 1);
    assert_eq!(patch_ops(&patch)[0]["op"], "remove");
    assert_eq!(patch_ops(&patch)[0]["path"], "/tracks/0");
    assert_eq!(warnings[0]["code"], "W_TRACK_REMOVE_ENVELOPE");
    assert_eq!(data.removed_track_id.to_string(), TARGET_TRACK);
    assert_eq!(data.removed_clip_ids[0].to_string(), TARGET_CLIP);
    assert!(data.removed_burned_effect_ids.is_empty());
    assert!(data.removed_keyframe_ids.is_empty());
    assert!(data.cleared_link_group_clip_ids.is_empty());
}

#[test]
fn compute_patch_happy_remove_track_with_multiple_clips() {
    let prior = base_project(vec![
        video_clip(TARGET_CLIP_2, false, None, vec![], vec![]),
        video_clip(TARGET_CLIP, false, None, vec![], vec![]),
    ]);

    let (_patch, _warnings, data) =
        compute_patch(&prior, &base_args(TARGET_TRACK)).expect("remove");

    assert_eq!(
        data.removed_clip_ids
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        vec![TARGET_CLIP.to_string(), TARGET_CLIP_2.to_string()]
    );
}

#[test]
fn compute_patch_text_track_cascades_burned_caption_effects() {
    let prior = text_cascade_project(vec![burned_caption_effect(), blur_effect()], vec![]);

    let (patch, warnings, data) = compute_patch(&prior, &base_args(TEXT_TRACK)).expect("remove");

    assert_eq!(warnings.len(), 1);
    assert_eq!(data.removed_burned_effect_ids[0].to_string(), BURNED_EFFECT);
    let ops = patch_ops(&patch);
    assert_eq!(ops[0]["op"], "remove");
    assert_eq!(ops[0]["path"], "/tracks/1/clips/0/effects/0");
    assert_eq!(ops[1]["path"], "/tracks/0");
}

#[test]
fn compute_patch_text_track_cascades_dangling_effect_keyframes_with_w_keyframes_removed() {
    let prior = text_cascade_project(
        vec![burned_caption_effect()],
        vec![
            effect_keyframe(KEYFRAME_ID, BURNED_EFFECT),
            opacity_keyframe(OTHER_KEYFRAME_ID),
        ],
    );

    let (patch, warnings, data) = compute_patch(&prior, &base_args(TEXT_TRACK)).expect("remove");

    let ops = patch_ops(&patch);
    assert_eq!(ops[0]["path"], "/tracks/1/clips/0/keyframes/0");
    assert_eq!(ops[1]["path"], "/tracks/1/clips/0/effects/0");
    assert_eq!(ops[2]["path"], "/tracks/0");
    assert_eq!(warnings.len(), 2);
    assert_eq!(warnings[0]["code"], "W_TRACK_REMOVE_ENVELOPE");
    assert_eq!(warnings[1]["code"], W_KEYFRAMES_REMOVED_CODE);
    assert_eq!(warnings[1]["details"]["clip_id"], SURVIVOR_CLIP);
    assert_eq!(
        warnings[1]["details"]["removed_keyframe_ids"],
        json!([KEYFRAME_ID])
    );
    assert_eq!(data.removed_keyframe_ids[0].to_string(), KEYFRAME_ID);
}

#[test]
fn compute_patch_link_group_lone_survivor_clears_link_group() {
    let prior = base_project(vec![video_clip(
        TARGET_CLIP,
        false,
        Some(LINK_GROUP),
        vec![],
        vec![],
    )]);
    let mut prior = prior;
    prior.tracks[1].clips[0].link_group = Some(LINK_GROUP.parse().expect("link group parses"));

    let (patch, _warnings, data) = compute_patch(&prior, &base_args(TARGET_TRACK)).expect("remove");

    let ops = patch_ops(&patch);
    assert_eq!(ops[0]["op"], "replace");
    assert_eq!(ops[0]["path"], "/tracks/1/clips/0/link_group");
    assert_eq!(ops[0]["value"], Value::Null);
    assert_eq!(ops[1]["path"], "/tracks/0");
    assert_eq!(
        data.cleared_link_group_clip_ids[0].to_string(),
        SURVIVOR_CLIP
    );
}

#[test]
fn compute_patch_link_group_drops_to_zero_no_clear_op() {
    let prior = base_project(vec![video_clip(
        TARGET_CLIP,
        false,
        Some(LINK_GROUP),
        vec![],
        vec![],
    )]);

    let (patch, _warnings, data) = compute_patch(&prior, &base_args(TARGET_TRACK)).expect("remove");

    assert_eq!(patch_ops(&patch).len(), 1);
    assert!(data.cleared_link_group_clip_ids.is_empty());
}

#[test]
fn compute_patch_link_group_stays_at_two_no_clear_op() {
    let mut prior = project_with_tracks(vec![
        track(
            TARGET_TRACK,
            TrackKind::Video,
            false,
            vec![video_clip(
                TARGET_CLIP,
                false,
                Some(LINK_GROUP),
                vec![],
                vec![],
            )],
        ),
        track(
            SURVIVOR_TRACK,
            TrackKind::Video,
            false,
            vec![video_clip(
                SURVIVOR_CLIP,
                false,
                Some(LINK_GROUP),
                vec![],
                vec![],
            )],
        ),
        track(
            SECOND_SURVIVOR_TRACK,
            TrackKind::Video,
            false,
            vec![video_clip(
                SECOND_SURVIVOR_CLIP,
                false,
                Some(LINK_GROUP),
                vec![],
                vec![],
            )],
        ),
    ]);
    prior.duration_tk = Tick::new(480_000);

    let (patch, _warnings, data) = compute_patch(&prior, &base_args(TARGET_TRACK)).expect("remove");

    assert_eq!(patch_ops(&patch).len(), 1);
    assert!(data.cleared_link_group_clip_ids.is_empty());
}

#[test]
fn compute_patch_last_track_in_project_is_e_track_last_in_project() {
    let prior = project_with_tracks(vec![track(
        TARGET_TRACK,
        TrackKind::Video,
        false,
        vec![video_clip(TARGET_CLIP, false, None, vec![], vec![])],
    )]);

    let err = compute_patch(&prior, &base_args(TARGET_TRACK)).expect_err("last track rejects");

    assert!(matches!(err, TrackRemoveError::LastInProject));
}

#[test]
fn compute_patch_locked_track_rejects_with_e_locked() {
    let prior = project_with_tracks(vec![
        track(
            TARGET_TRACK,
            TrackKind::Video,
            true,
            vec![video_clip(TARGET_CLIP, false, None, vec![], vec![])],
        ),
        track(
            SURVIVOR_TRACK,
            TrackKind::Video,
            false,
            vec![video_clip(SURVIVOR_CLIP, false, None, vec![], vec![])],
        ),
    ]);

    let err = compute_patch(&prior, &base_args(TARGET_TRACK)).expect_err("locked rejects");

    assert!(matches!(
        err,
        TrackRemoveError::Locked {
            kind: "track",
            ref failed_target,
        } if failed_target == TARGET_TRACK
    ));
}

#[test]
fn compute_patch_locked_clip_on_target_track_rejects_with_e_locked() {
    let prior = base_project(vec![video_clip(TARGET_CLIP, true, None, vec![], vec![])]);

    let err = compute_patch(&prior, &base_args(TARGET_TRACK)).expect_err("locked rejects");

    assert!(matches!(
        err,
        TrackRemoveError::Locked {
            kind: "clip",
            ref failed_target,
        } if failed_target == TARGET_CLIP
    ));
}

#[test]
fn compute_patch_last_track_precedes_locked() {
    let prior = project_with_tracks(vec![track(
        TARGET_TRACK,
        TrackKind::Video,
        true,
        vec![video_clip(TARGET_CLIP, true, None, vec![], vec![])],
    )]);

    let err = compute_patch(&prior, &base_args(TARGET_TRACK)).expect_err("last track first");

    assert!(matches!(err, TrackRemoveError::LastInProject));
}

#[test]
fn compute_patch_bad_selector() {
    let prior = base_project(vec![video_clip(TARGET_CLIP, false, None, vec![], vec![])]);

    let err = compute_patch(&prior, &base_args("not-a-uuid")).expect_err("bad selector");

    assert!(matches!(err, TrackRemoveError::BadSelector { .. }));
}

#[test]
fn compute_patch_selector_kind_mismatch_clip_resolves_to_clip() {
    let prior = base_project(vec![video_clip(TARGET_CLIP, false, None, vec![], vec![])]);

    let err = compute_patch(&prior, &base_args(TARGET_CLIP)).expect_err("clip selector rejects");

    assert!(matches!(
        err,
        TrackRemoveError::SelectorKindMismatch {
            ref selector,
            resolved_kind: "clip",
        } if selector == TARGET_CLIP
    ));
}

#[test]
fn compute_patch_track_not_found() {
    let prior = base_project(vec![video_clip(TARGET_CLIP, false, None, vec![], vec![])]);

    let err = compute_patch(&prior, &base_args(MISSING_TRACK)).expect_err("missing rejects");

    assert!(matches!(err, TrackRemoveError::TrackNotFound { .. }));
}

#[test]
fn compute_patch_video_track_no_burned_caption_cascade() {
    let prior = project_with_tracks(vec![
        track(
            TARGET_TRACK,
            TrackKind::Video,
            false,
            vec![video_clip(TARGET_CLIP, false, None, vec![], vec![])],
        ),
        track(
            SURVIVOR_TRACK,
            TrackKind::Video,
            false,
            vec![video_clip(
                SURVIVOR_CLIP,
                false,
                None,
                vec![burned_caption_effect()],
                vec![],
            )],
        ),
    ]);

    let (patch, _warnings, data) = compute_patch(&prior, &base_args(TARGET_TRACK)).expect("remove");

    assert_eq!(patch_ops(&patch).len(), 1);
    assert!(data.removed_burned_effect_ids.is_empty());
}

#[test]
fn compute_patch_envelope_warning_always_present() {
    let prior = base_project(vec![video_clip(TARGET_CLIP, false, None, vec![], vec![])]);

    let (_patch, warnings, _data) =
        compute_patch(&prior, &base_args(TARGET_TRACK)).expect("remove");

    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], "W_TRACK_REMOVE_ENVELOPE");
    assert_eq!(
        warnings[0]["details"]["removed_burned_effect_ids"],
        json!([])
    );
    assert_eq!(warnings[0]["details"]["removed_keyframe_ids"], json!([]));
    assert_eq!(
        warnings[0]["details"]["cleared_link_group_clip_ids"],
        json!([])
    );
}

#[test]
fn reconstruct_from_warning_round_trip() {
    let prior = text_cascade_project(
        vec![burned_caption_effect()],
        vec![effect_keyframe(KEYFRAME_ID, BURNED_EFFECT)],
    );
    let args = base_args(TEXT_TRACK);

    let (_patch, warnings, data) = compute_patch(&prior, &args).expect("remove");
    let reconstructed =
        data_envelope_from_args_warnings(&args, &warnings).expect("reconstructs from warning");

    assert_eq!(data, reconstructed);
}

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let prior = base_project(vec![video_clip(TARGET_CLIP, false, None, vec![], vec![])]);
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        prior,
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry succeeds");

    let outcome = store
        .mutate_via_verb(
            "track.remove",
            serde_json::to_value(base_args(TARGET_TRACK)).expect("args serialize"),
            None,
        )
        .expect("track.remove should route");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("happy path must apply");
    };
    assert_eq!(warnings.len(), 1);
    let envelope: TrackRemoveData = serde_json::from_value(data).expect("data parses");
    assert_eq!(envelope.removed_track_id.to_string(), TARGET_TRACK);
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "track.remove")
        .expect("default_fixtures includes track.remove");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(TrackRemoveVerb))
        .expect("register track.remove verb");

    let report = validate_reconstructors(&registry, &[fixture])
        .expect("track.remove reconstructor should pass");
    assert_eq!(report.verbs_checked, vec!["track.remove"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn round_trip_track_remove() {
    let prior = text_cascade_project(
        vec![burned_caption_effect()],
        vec![
            effect_keyframe(KEYFRAME_ID, BURNED_EFFECT),
            opacity_keyframe(OTHER_KEYFRAME_ID),
        ],
    );
    let args = base_args(TEXT_TRACK);

    let (patch, warnings, data) = compute_patch(&prior, &args).expect("remove");
    let post = apply_patch(&prior, patch);
    let reconstructed = data_envelope_from_args_warnings(&args, &warnings).expect("reconstructs");

    assert_eq!(data, reconstructed);
    assert_eq!(post.tracks.len(), 1);
    assert_eq!(post.tracks[0].id.to_string(), SURVIVOR_TRACK);
    assert!(post.tracks[0].clips[0].effects.is_empty());
    assert_eq!(post.tracks[0].clips[0].keyframes.len(), 1);
    assert_eq!(
        post.tracks[0].clips[0].keyframes[0].id.to_string(),
        OTHER_KEYFRAME_ID
    );
}
