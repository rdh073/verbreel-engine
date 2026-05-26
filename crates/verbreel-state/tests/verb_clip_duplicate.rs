//! Tests for `clip.duplicate` (§5.6) — forty-ninth production verb.

use serde_json::{Value, json};
use std::sync::Arc;

use verbreel_state::verbs::clip_duplicate::{
    W_CLIP_DUPLICATE_ENVELOPE_CODE, compute_patch, data_envelope_from_args_warnings,
};
use verbreel_state::{
    ClipDuplicateArgs, ClipDuplicateData, ClipDuplicateError, ClipDuplicateVerb, MutateOutcome,
    Project, RecordedEvent, TrackKind, Verb, VerbRegistry, default_fixtures, default_registry,
    validate_reconstructors,
};
use verbreel_types::{ProjectId, Tick};

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const TRACK_VIDEO_A: &str = "01900000-0000-7000-8000-0000000aa901";
const TRACK_AUDIO_A: &str = "01900000-0000-7000-8000-0000000aa902";
const CLIP_VIDEO_A: &str = "01900000-0000-7000-8000-0000000bb901";
const CLIP_VIDEO_B: &str = "01900000-0000-7000-8000-0000000bb902";
const CLIP_AUDIO_A: &str = "01900000-0000-7000-8000-0000000bb903";
const CLIP_AUDIO_B: &str = "01900000-0000-7000-8000-0000000bb904";
const MISSING_CLIP: &str = "01900000-0000-7000-8000-0000000cc901";
const LINK_GROUP_ID: &str = "01900000-0000-7000-8000-0000000dd901";
const ASSET_VIDEO_ID: &str = "01900000-0000-7000-8000-0000000ee901";
const ASSET_AUDIO_ID: &str = "01900000-0000-7000-8000-0000000ee902";
const KEYFRAME_A: &str = "01900000-0000-7000-8000-0000000ff901";
const EFFECT_A: &str = "01900000-0000-7000-8000-0000000ef901";

#[derive(Debug, Clone)]
struct ClipFixture {
    id: &'static str,
    position_tk: i64,
    source_out_tk: i64,
    locked: bool,
    link_group: Option<&'static str>,
    asset_id: &'static str,
    keyframes: Vec<Value>,
    effects: Vec<Value>,
}

#[derive(Debug, Clone)]
struct TrackFixture {
    id: &'static str,
    kind: TrackKind,
    locked: bool,
    clips: Vec<ClipFixture>,
}

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture -> Project")
}

fn fixture_project_id() -> ProjectId {
    FIXTURE_PROJECT_ID
        .parse()
        .expect("fixture project id parses")
}

fn duplicate_args(clip: &str, gap_tk: Option<i64>, auto_gap: Option<bool>) -> ClipDuplicateArgs {
    ClipDuplicateArgs {
        project_id: fixture_project_id(),
        clip: clip.to_string(),
        gap_tk,
        auto_gap,
    }
}

fn clip(id: &'static str, asset_id: &'static str, position_tk: i64) -> ClipFixture {
    ClipFixture {
        id,
        position_tk,
        source_out_tk: 240_000,
        locked: false,
        link_group: None,
        asset_id,
        keyframes: Vec::new(),
        effects: Vec::new(),
    }
}

fn video_clip(id: &'static str, position_tk: i64) -> ClipFixture {
    clip(id, ASSET_VIDEO_ID, position_tk)
}

fn audio_clip(id: &'static str, position_tk: i64) -> ClipFixture {
    clip(id, ASSET_AUDIO_ID, position_tk)
}

fn linked(mut fixture: ClipFixture) -> ClipFixture {
    fixture.link_group = Some(LINK_GROUP_ID);
    fixture
}

fn locked(mut fixture: ClipFixture) -> ClipFixture {
    fixture.locked = true;
    fixture
}

fn with_duration(mut fixture: ClipFixture, source_out_tk: i64) -> ClipFixture {
    fixture.source_out_tk = source_out_tk;
    fixture
}

fn with_effect_and_keyframe(mut fixture: ClipFixture) -> ClipFixture {
    fixture.effects = vec![json!({
        "id": EFFECT_A,
        "kind": "blur",
        "enabled": true,
        "params": {
            "radius_px": 4
        }
    })];
    fixture.keyframes = vec![json!({
        "id": KEYFRAME_A,
        "property": format!("effects[{EFFECT_A}].params.radius_px"),
        "time_tk": 10_000,
        "value": 8
    })];
    fixture
}

fn video_track(clips: Vec<ClipFixture>) -> TrackFixture {
    TrackFixture {
        id: TRACK_VIDEO_A,
        kind: TrackKind::Video,
        locked: false,
        clips,
    }
}

fn audio_track(clips: Vec<ClipFixture>) -> TrackFixture {
    TrackFixture {
        id: TRACK_AUDIO_A,
        kind: TrackKind::Audio,
        locked: false,
        clips,
    }
}

fn track_locked(mut track: TrackFixture) -> TrackFixture {
    track.locked = true;
    track
}

fn clip_value(kind: TrackKind, fixture: &ClipFixture) -> Value {
    let mut value = json!({
        "id": fixture.id,
        "name": "Clip",
        "asset_id": fixture.asset_id,
        "track_position_tk": fixture.position_tk,
        "source_in_tk": 0,
        "source_out_tk": fixture.source_out_tk,
        "locked": fixture.locked,
        "keyframes": fixture.keyframes,
        "effects": fixture.effects,
    });
    if kind == TrackKind::Audio {
        value["volume"] = json!(1.0);
    }
    if let Some(link_group) = fixture.link_group {
        value["link_group"] = json!(link_group);
    }
    value
}

fn project_with_tracks(tracks: Vec<TrackFixture>) -> Project {
    let mut project = empty_project();
    let duration_tk = tracks
        .iter()
        .flat_map(|track| track.clips.iter())
        .map(|clip| clip.position_tk + clip.source_out_tk)
        .max()
        .unwrap_or(0);
    project.tracks = tracks
        .into_iter()
        .map(|track| {
            let clips = track
                .clips
                .iter()
                .map(|clip| clip_value(track.kind, clip))
                .collect::<Vec<_>>();
            serde_json::from_value(json!({
                "id": track.id,
                "kind": track.kind,
                "name": "Track",
                "locked": track.locked,
                "clips": clips,
            }))
            .expect("track fixture parses")
        })
        .collect();
    project.assets.clear();
    add_assets(&mut project);
    project.duration_tk = Tick::new(duration_tk);
    project
}

fn add_assets(project: &mut Project) {
    project
        .assets
        .push(serde_json::from_value(video_asset()).expect("video asset parses"));
    project
        .assets
        .push(serde_json::from_value(audio_asset()).expect("audio asset parses"));
}

fn video_asset() -> Value {
    json!({
        "id": ASSET_VIDEO_ID,
        "kind": "video",
        "hash": "36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da",
        "path": "assets/36/36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da.mp4",
        "original_filename": "video.mp4",
        "imported_at": "2026-05-24T00:00:00Z",
        "metadata": {
            "duration_tk": 960_000,
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

fn audio_asset() -> Value {
    json!({
        "id": ASSET_AUDIO_ID,
        "kind": "audio",
        "hash": "53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658",
        "path": "assets/53/53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.m4a",
        "original_filename": "audio.m4a",
        "imported_at": "2026-05-24T00:00:00Z",
        "metadata": {
            "duration_tk": 960_000,
            "audio_codec": "aac",
            "audio_channels": 2,
            "audio_sample_rate_hz": 48000,
            "container": "m4a",
            "fingerprint": {
                "mtime_ms": 1_700_000_000_000_i64,
                "size_bytes": 1024,
            }
        }
    })
}

fn apply_patch(prior: &Project, patch: &Value) -> Project {
    let patch: json_patch::Patch =
        serde_json::from_value(patch.clone()).expect("patch parses to typed JSON patch");
    prior
        .apply(&patch)
        .expect("clip.duplicate patch applies cleanly")
}

#[test]
fn singleton_gap_zero_lands_at_source_end_with_new_id() {
    let prior = project_with_tracks(vec![video_track(vec![video_clip(CLIP_VIDEO_A, 0)])]);
    let (patch, warnings, data) =
        compute_patch(&prior, &duplicate_args(CLIP_VIDEO_A, None, None)).expect("duplicate");
    let post = apply_patch(&prior, &patch);

    assert_eq!(post.tracks[0].clips.len(), 2);
    assert_eq!(post.tracks[0].clips[1].track_position_tk.get(), 240_000);
    assert_ne!(post.tracks[0].clips[1].id, CLIP_VIDEO_A.parse().unwrap());
    assert_eq!(post.tracks[0].clips[1].id, data.new_clip_id);
    assert_eq!(data.resolved_gap_tk, 0);
    assert_eq!(
        warnings.last().and_then(|warning| warning["code"].as_str()),
        Some(W_CLIP_DUPLICATE_ENVELOPE_CODE)
    );
}

#[test]
fn singleton_gap_tk_places_after_manual_gap() {
    let prior = project_with_tracks(vec![video_track(vec![video_clip(CLIP_VIDEO_A, 0)])]);
    let (patch, _warnings, data) =
        compute_patch(&prior, &duplicate_args(CLIP_VIDEO_A, Some(100), None)).expect("duplicate");
    let post = apply_patch(&prior, &patch);

    assert_eq!(post.tracks[0].clips[1].track_position_tk.get(), 240_100);
    assert_eq!(data.track_position_tk, 240_100);
    assert_eq!(data.resolved_gap_tk, 100);
}

#[test]
fn auto_gap_no_neighbors_resolves_zero() {
    let prior = project_with_tracks(vec![video_track(vec![video_clip(CLIP_VIDEO_A, 0)])]);
    let (patch, _warnings, data) =
        compute_patch(&prior, &duplicate_args(CLIP_VIDEO_A, None, Some(true))).expect("auto gap");
    let post = apply_patch(&prior, &patch);

    assert_eq!(post.tracks[0].clips[1].track_position_tk.get(), 240_000);
    assert_eq!(data.resolved_gap_tk, 0);
}

#[test]
fn auto_gap_skips_blocked_slot_and_returns_resolved_gap() {
    let prior = project_with_tracks(vec![video_track(vec![
        video_clip(CLIP_VIDEO_A, 0),
        video_clip(CLIP_VIDEO_B, 240_500),
    ])]);
    let (patch, _warnings, data) =
        compute_patch(&prior, &duplicate_args(CLIP_VIDEO_A, None, Some(true))).expect("auto gap");
    let post = apply_patch(&prior, &patch);

    assert_eq!(data.resolved_gap_tk, 240_500);
    assert_eq!(data.track_position_tk, 480_500);
    assert!(
        post.tracks[0]
            .clips
            .iter()
            .any(|clip| clip.id == data.new_clip_id && clip.track_position_tk.get() == 480_500)
    );
}

#[test]
fn linked_video_audio_duplicate_gets_shared_fresh_link_group() {
    let prior = project_with_tracks(vec![
        video_track(vec![linked(video_clip(CLIP_VIDEO_A, 0))]),
        audio_track(vec![linked(audio_clip(CLIP_AUDIO_A, 0))]),
    ]);
    let (patch, _warnings, data) =
        compute_patch(&prior, &duplicate_args(CLIP_VIDEO_A, None, None)).expect("linked");
    let post = apply_patch(&prior, &patch);
    let new_link_group = data.new_link_group.expect("linked duplicate group");

    assert_ne!(new_link_group, LINK_GROUP_ID.parse().unwrap());
    assert_eq!(post.tracks[0].clips[1].link_group, Some(new_link_group));
    assert_eq!(post.tracks[1].clips[1].link_group, Some(new_link_group));
    assert_eq!(data.sibling_duplicates.len(), 1);
}

#[test]
fn linked_auto_gap_applies_same_resolved_delta_to_sibling() {
    let prior = project_with_tracks(vec![
        video_track(vec![
            linked(video_clip(CLIP_VIDEO_A, 0)),
            video_clip(CLIP_VIDEO_B, 240_500),
        ]),
        audio_track(vec![linked(audio_clip(CLIP_AUDIO_A, 0))]),
    ]);
    let (patch, _warnings, data) =
        compute_patch(&prior, &duplicate_args(CLIP_VIDEO_A, None, Some(true))).expect("linked");
    let post = apply_patch(&prior, &patch);

    assert_eq!(data.resolved_gap_tk, 240_500);
    assert_eq!(data.track_position_tk, 480_500);
    assert_eq!(data.sibling_duplicates[0].track_position_tk, 480_500);
    assert_eq!(post.tracks[1].clips[1].track_position_tk.get(), 480_500);
}

#[test]
fn auto_gap_with_positive_gap_is_incompatible() {
    let prior = project_with_tracks(vec![video_track(vec![video_clip(CLIP_VIDEO_A, 0)])]);
    let err = compute_patch(&prior, &duplicate_args(CLIP_VIDEO_A, Some(100), Some(true)))
        .expect_err("incompatible args");

    assert!(matches!(
        err,
        ClipDuplicateError::ArgsIncompatible { hint }
            if hint == "auto_gap is mutually exclusive with non-zero gap_tk"
    ));
}

#[test]
fn negative_gap_is_bad_time() {
    let prior = project_with_tracks(vec![video_track(vec![video_clip(CLIP_VIDEO_A, 0)])]);
    let err = compute_patch(&prior, &duplicate_args(CLIP_VIDEO_A, Some(-1), None))
        .expect_err("negative gap");

    assert!(matches!(
        err,
        ClipDuplicateError::BadTime {
            field: "gap_tk",
            value: -1
        }
    ));
}

#[test]
fn missing_clip_and_bad_selector_error() {
    let prior = project_with_tracks(vec![video_track(vec![video_clip(CLIP_VIDEO_A, 0)])]);

    assert!(matches!(
        compute_patch(&prior, &duplicate_args(MISSING_CLIP, None, None)).expect_err("missing"),
        ClipDuplicateError::ClipNotFound { clip_id } if clip_id == MISSING_CLIP
    ));
    assert!(matches!(
        compute_patch(&prior, &duplicate_args("not-a-uuid", None, None)).expect_err("bad selector"),
        ClipDuplicateError::BadSelector { .. }
    ));
}

#[test]
fn locked_destination_track_blocks_target_or_sibling() {
    let locked_target = project_with_tracks(vec![track_locked(video_track(vec![video_clip(
        CLIP_VIDEO_A,
        0,
    )]))]);
    assert!(matches!(
        compute_patch(&locked_target, &duplicate_args(CLIP_VIDEO_A, None, None)).expect_err("target"),
        ClipDuplicateError::Locked { failed_target } if failed_target == TRACK_VIDEO_A
    ));

    let locked_sibling = project_with_tracks(vec![
        video_track(vec![linked(video_clip(CLIP_VIDEO_A, 0))]),
        track_locked(audio_track(vec![linked(audio_clip(CLIP_AUDIO_A, 0))])),
    ]);
    assert!(matches!(
        compute_patch(&locked_sibling, &duplicate_args(CLIP_VIDEO_A, None, None)).expect_err("sibling"),
        ClipDuplicateError::Locked { failed_target } if failed_target == TRACK_AUDIO_A
    ));
}

#[test]
fn locked_source_clip_does_not_block_duplication() {
    let prior = project_with_tracks(vec![video_track(vec![locked(video_clip(CLIP_VIDEO_A, 0))])]);
    let (patch, _warnings, data) =
        compute_patch(&prior, &duplicate_args(CLIP_VIDEO_A, None, None)).expect("locked source");
    let post = apply_patch(&prior, &patch);

    assert_eq!(post.tracks[0].clips[1].id, data.new_clip_id);
    assert!(post.tracks[0].clips[1].locked);
}

#[test]
fn manual_gap_overlap_on_target_track_errors() {
    let prior = project_with_tracks(vec![video_track(vec![
        video_clip(CLIP_VIDEO_A, 0),
        video_clip(CLIP_VIDEO_B, 250_000),
    ])]);
    let err =
        compute_patch(&prior, &duplicate_args(CLIP_VIDEO_A, Some(100), None)).expect_err("overlap");

    assert!(matches!(
        err,
        ClipDuplicateError::ClipOverlap { failed_clip } if failed_clip == CLIP_VIDEO_A
    ));
}

#[test]
fn linked_same_gap_overlap_on_sibling_track_errors_atomically() {
    let prior = project_with_tracks(vec![
        video_track(vec![linked(video_clip(CLIP_VIDEO_A, 0))]),
        audio_track(vec![
            linked(audio_clip(CLIP_AUDIO_A, 0)),
            audio_clip(CLIP_AUDIO_B, 240_100),
        ]),
    ]);
    let err = compute_patch(&prior, &duplicate_args(CLIP_VIDEO_A, Some(100), None))
        .expect_err("sibling overlap");

    assert!(matches!(
        err,
        ClipDuplicateError::ClipOverlap { failed_clip } if failed_clip == CLIP_AUDIO_A
    ));
}

#[test]
fn data_envelope_has_null_link_group_for_singleton_and_sibling_data_for_linked() {
    let singleton = project_with_tracks(vec![video_track(vec![video_clip(CLIP_VIDEO_A, 0)])]);
    let (_patch, _warnings, singleton_data) =
        compute_patch(&singleton, &duplicate_args(CLIP_VIDEO_A, None, None)).expect("singleton");
    assert!(singleton_data.new_link_group.is_none());
    assert!(singleton_data.sibling_duplicates.is_empty());

    let linked_prior = project_with_tracks(vec![
        video_track(vec![linked(video_clip(CLIP_VIDEO_A, 0))]),
        audio_track(vec![linked(audio_clip(CLIP_AUDIO_A, 0))]),
    ]);
    let (_patch, _warnings, linked_data) =
        compute_patch(&linked_prior, &duplicate_args(CLIP_VIDEO_A, None, None)).expect("linked");
    assert_eq!(linked_data.sibling_duplicates.len(), 1);
    assert_eq!(
        linked_data.sibling_duplicates[0].source_clip_id,
        CLIP_AUDIO_A.parse().unwrap()
    );
    assert_ne!(
        linked_data.sibling_duplicates[0].new_clip_id,
        CLIP_AUDIO_A.parse().unwrap()
    );
}

#[test]
fn keyframes_and_effects_are_deep_copied_with_fresh_ids() {
    let prior = project_with_tracks(vec![video_track(vec![with_effect_and_keyframe(
        video_clip(CLIP_VIDEO_A, 0),
    )])]);
    let (patch, _warnings, _data) =
        compute_patch(&prior, &duplicate_args(CLIP_VIDEO_A, None, None)).expect("duplicate");
    let post = apply_patch(&prior, &patch);
    let original = &post.tracks[0].clips[0];
    let duplicate = &post.tracks[0].clips[1];

    assert_eq!(duplicate.effects.len(), 1);
    assert_eq!(duplicate.keyframes.len(), 1);
    assert_ne!(duplicate.effects[0].id, original.effects[0].id);
    assert_ne!(duplicate.keyframes[0].id, original.keyframes[0].id);
    assert!(
        duplicate.keyframes[0]
            .property
            .as_str()
            .contains(&duplicate.effects[0].id.to_string())
    );
}

#[test]
fn reconstructor_round_trip_singleton() {
    let prior = project_with_tracks(vec![video_track(vec![video_clip(CLIP_VIDEO_A, 0)])]);
    let args = duplicate_args(CLIP_VIDEO_A, None, None);
    let (patch, warnings, _data) = compute_patch(&prior, &args).expect("duplicate");
    let post = apply_patch(&prior, &patch);
    let expected_data = serde_json::to_value(
        data_envelope_from_args_warnings(&args, &warnings, &post).expect("warning envelope"),
    )
    .expect("data serializes");
    let recorded = RecordedEvent {
        verb: "clip.duplicate".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch,
        warnings,
        post_state: post,
        expected_data,
    };

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(ClipDuplicateVerb))
        .expect("register clip.duplicate");
    let report = validate_reconstructors(&registry, &[recorded]).expect("round trip");
    assert_eq!(report.verbs_checked, vec!["clip.duplicate"]);
}

#[test]
fn reconstructor_round_trip_linked_group_with_auto_gap() {
    let prior = project_with_tracks(vec![
        video_track(vec![
            linked(video_clip(CLIP_VIDEO_A, 0)),
            video_clip(CLIP_VIDEO_B, 240_500),
        ]),
        audio_track(vec![linked(audio_clip(CLIP_AUDIO_A, 0))]),
    ]);
    let args = duplicate_args(CLIP_VIDEO_A, None, Some(true));
    let (patch, warnings, _data) = compute_patch(&prior, &args).expect("duplicate");
    let post = apply_patch(&prior, &patch);
    let expected_data = serde_json::to_value(
        data_envelope_from_args_warnings(&args, &warnings, &post).expect("warning envelope"),
    )
    .expect("data serializes");
    let recorded = RecordedEvent {
        verb: "clip.duplicate".to_string(),
        args: serde_json::to_value(&args).expect("args serialize"),
        patch,
        warnings,
        post_state: post,
        expected_data,
    };

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(ClipDuplicateVerb))
        .expect("register clip.duplicate");
    let report = validate_reconstructors(&registry, &[recorded]).expect("round trip");
    assert_eq!(report.verbs_checked, vec!["clip.duplicate"]);
}

#[test]
fn default_fixture_reconstructs() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "clip.duplicate")
        .expect("default_fixtures includes clip.duplicate");
    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(ClipDuplicateVerb))
        .expect("register clip.duplicate");
    let report = validate_reconstructors(&registry, &[fixture]).expect("default fixture");
    assert_eq!(report.verbs_checked, vec!["clip.duplicate"]);
}

#[test]
fn verb_trait_returns_data_and_internal_envelope_warning() {
    let prior = project_with_tracks(vec![video_track(vec![video_clip(CLIP_VIDEO_A, 0)])]);
    let verb = ClipDuplicateVerb;
    let (_patch, data, warnings) = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "clip": CLIP_VIDEO_A,
            }),
        )
        .expect("verb route");

    let data: ClipDuplicateData = serde_json::from_value(data).expect("data envelope");
    assert_eq!(data.source_clip_id, CLIP_VIDEO_A.parse().unwrap());
    assert_eq!(
        warnings.last().and_then(|warning| warning["code"].as_str()),
        Some(W_CLIP_DUPLICATE_ENVELOPE_CODE)
    );
}

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        project_with_tracks(vec![video_track(vec![video_clip(CLIP_VIDEO_A, 0)])]),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry clears the gate");

    let outcome = store
        .mutate_via_verb(
            "clip.duplicate",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "clip": CLIP_VIDEO_A,
            }),
            None,
        )
        .expect("mutate_via_verb happy path");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("happy path must return Applied, got {outcome:?}");
    };

    let data: ClipDuplicateData = serde_json::from_value(data).expect("clip.duplicate data");
    assert_eq!(store.project().tracks[0].clips.len(), 2);
    assert_eq!(store.project().tracks[0].clips[1].id, data.new_clip_id);
    assert_eq!(
        warnings.last().and_then(|warning| warning["code"].as_str()),
        Some(W_CLIP_DUPLICATE_ENVELOPE_CODE)
    );
}

#[test]
fn shorter_clip_can_use_small_auto_gap_slot() {
    let prior = project_with_tracks(vec![video_track(vec![
        with_duration(video_clip(CLIP_VIDEO_A, 0), 400),
        video_clip(CLIP_VIDEO_B, 900),
    ])]);
    let (patch, _warnings, data) =
        compute_patch(&prior, &duplicate_args(CLIP_VIDEO_A, None, Some(true))).expect("auto gap");
    let post = apply_patch(&prior, &patch);

    assert_eq!(data.resolved_gap_tk, 0);
    assert_eq!(post.tracks[0].clips[1].id, data.new_clip_id);
    assert_eq!(post.tracks[0].clips[1].track_position_tk.get(), 400);
}
