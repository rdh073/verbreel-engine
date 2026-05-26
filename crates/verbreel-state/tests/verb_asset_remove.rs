//! Tests for `asset.remove` (§3.4) — fifty-eighth production verb.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::asset_remove::{
    AssetRemoveArgs, AssetRemoveData, AssetRemoveError, AssetRemoveVerb,
    W_ASSET_REMOVE_ENVELOPE_CODE, compute_patch, data_envelope_from_args_warnings,
};
use verbreel_state::{
    Project, TrackKind, VerbRegistry, default_fixtures, default_registry, validate_reconstructors,
};
use verbreel_types::Tick;

#[cfg(feature = "native")]
use verbreel_state::{MutateOutcome, ProjectStore};

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";

const VIDEO_TRACK_A: &str = "01900000-0000-7000-8000-0000000aa901";
const VIDEO_TRACK_B: &str = "01900000-0000-7000-8000-0000000aa902";

const CLIP_A1: &str = "01900000-0000-7000-8000-0000000bb901";
const CLIP_A2: &str = "01900000-0000-7000-8000-0000000bb902";
const CLIP_B1: &str = "01900000-0000-7000-8000-0000000bb903";
const SURVIVOR_CLIP: &str = "01900000-0000-7000-8000-0000000bb999";

const TARGET_ASSET: &str = "01900000-0000-7000-8000-0000000cc901";
const SURVIVOR_ASSET: &str = "01900000-0000-7000-8000-0000000cc902";
const MISSING_ASSET: &str = "01900000-0000-7000-8000-0000000cc999";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture -> Project")
}

fn fixture_project_id() -> verbreel_types::ProjectId {
    FIXTURE_PROJECT_ID
        .parse()
        .expect("fixture project id parses")
}

fn target_asset_value() -> Value {
    json!({
        "id": TARGET_ASSET,
        "kind": "video",
        "hash": "53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658",
        "path": "assets/53/53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.mp4",
        "original_filename": "asset-remove-target.mp4",
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

fn survivor_asset_value() -> Value {
    json!({
        "id": SURVIVOR_ASSET,
        "kind": "audio",
        "hash": "36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da",
        "path": "assets/36/36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da.m4a",
        "original_filename": "asset-remove-survivor.m4a",
        "imported_at": "2026-05-24T00:00:00Z",
        "metadata": {
            "duration_tk": 240_000,
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

fn video_clip(id: &str, asset_id: &str, position_tk: i64, length_tk: i64) -> Value {
    json!({
        "id": id,
        "name": "Clip",
        "asset_id": asset_id,
        "track_position_tk": position_tk,
        "source_in_tk": 0,
        "source_out_tk": length_tk,
        "locked": false,
    })
}

fn track(id: &str, kind: TrackKind, clips: Vec<Value>) -> Value {
    json!({
        "id": id,
        "kind": kind,
        "name": "Track",
        "locked": false,
        "clips": clips,
    })
}

/// Project with only the target asset registered (no clips, no
/// referencing tracks). Used for the orphan-asset happy path.
fn orphan_project() -> Project {
    let mut project = empty_project();
    project.assets.clear();
    project
        .assets
        .push(serde_json::from_value(target_asset_value()).expect("asset parses"));
    project.duration_tk = Tick::new(0);
    project
}

/// Project with the target asset plus N video clips on a single track
/// that all reference it, alongside a survivor clip on a separate track
/// pinning the project duration so cascade removal doesn't drift
/// `check_duration_tk`.
fn project_with_referencing_clips(target_clips: Vec<Value>) -> Project {
    let mut project = empty_project();
    project.assets.clear();
    project
        .assets
        .push(serde_json::from_value(target_asset_value()).expect("target asset parses"));
    project
        .assets
        .push(serde_json::from_value(survivor_asset_value()).expect("survivor asset parses"));
    project.tracks = vec![
        serde_json::from_value(track(VIDEO_TRACK_A, TrackKind::Video, target_clips))
            .expect("track A parses"),
        serde_json::from_value(track(
            VIDEO_TRACK_B,
            TrackKind::Audio,
            vec![json!({
                "id": SURVIVOR_CLIP,
                "name": "Survivor",
                "asset_id": SURVIVOR_ASSET,
                "track_position_tk": 0,
                "source_in_tk": 0,
                "source_out_tk": 480_000,
                "volume": 1.0,
                "locked": false,
            })],
        ))
        .expect("track B parses"),
    ];
    project.duration_tk = Tick::new(480_000);
    project
}

fn base_args(asset_id: &str, cascade: Option<bool>) -> AssetRemoveArgs {
    AssetRemoveArgs {
        project_id: fixture_project_id(),
        asset_id: asset_id.to_string(),
        cascade,
    }
}

fn patch_ops(patch: &Value) -> &[Value] {
    patch.as_array().expect("patch is array")
}

fn apply_patch(prior: &Project, patch: Value) -> Project {
    let patch: json_patch::Patch = serde_json::from_value(patch).expect("patch parses");
    prior.apply(&patch).expect("patch applies")
}

// ---------------------------------------------------------------------
// Args shape / deserialization
// ---------------------------------------------------------------------

#[test]
fn args_deserialize_ok_minimal() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "asset_id": TARGET_ASSET,
    });
    let parsed: AssetRemoveArgs = serde_json::from_value(raw).expect("minimal args parse");
    assert_eq!(parsed.asset_id, TARGET_ASSET);
    assert_eq!(parsed.cascade, None);
}

#[test]
fn args_deserialize_ok_with_cascade() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "asset_id": TARGET_ASSET,
        "cascade": true,
    });
    let parsed: AssetRemoveArgs = serde_json::from_value(raw).expect("cascade args parse");
    assert_eq!(parsed.cascade, Some(true));
}

#[test]
fn args_missing_required_asset_id_fails() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
    });
    let result: Result<AssetRemoveArgs, _> = serde_json::from_value(raw);
    assert!(result.is_err(), "missing asset_id must fail to deserialize");
}

#[test]
fn args_wrong_type_for_asset_id_fails() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "asset_id": 12345,
    });
    let result: Result<AssetRemoveArgs, _> = serde_json::from_value(raw);
    assert!(result.is_err(), "numeric asset_id must fail to deserialize");
}

#[test]
fn args_cascade_defaults_to_false_when_omitted() {
    let prior = orphan_project();
    let args = base_args(TARGET_ASSET, None);
    // Orphan asset → succeeds even though cascade defaults to false:
    // there is nothing in use.
    let (_patch, _warnings, data) = compute_patch(&prior, &args).expect("orphan removal");
    assert!(data.removed_clip_ids.is_empty());
}

// ---------------------------------------------------------------------
// Error variants
// ---------------------------------------------------------------------

#[test]
fn compute_patch_bad_selector_returns_e_bad_selector() {
    let prior = orphan_project();
    let args = base_args("not-a-uuid", None);
    let err = compute_patch(&prior, &args).expect_err("bad selector");
    assert!(matches!(err, AssetRemoveError::BadSelector { .. }));
}

#[test]
fn compute_patch_bad_selector_v4_uuid_rejected() {
    // v4 UUIDs have version nibble `4`; AssetId is UUIDv7 only.
    let prior = orphan_project();
    let args = base_args("550e8400-e29b-41d4-a716-446655440000", None);
    let err = compute_patch(&prior, &args).expect_err("v4 uuid");
    assert!(matches!(err, AssetRemoveError::BadSelector { .. }));
}

#[test]
fn compute_patch_asset_not_found_returns_e_asset_not_found() {
    let prior = orphan_project();
    let args = base_args(MISSING_ASSET, None);
    let err = compute_patch(&prior, &args).expect_err("missing asset");
    assert!(matches!(
        err,
        AssetRemoveError::AssetNotFound { ref asset_id } if asset_id == MISSING_ASSET
    ));
}

#[test]
fn compute_patch_asset_in_use_default_cascade_returns_e_asset_in_use() {
    let prior = project_with_referencing_clips(vec![video_clip(CLIP_A1, TARGET_ASSET, 0, 240_000)]);
    let args = base_args(TARGET_ASSET, None);
    let err = compute_patch(&prior, &args).expect_err("in use");
    assert!(matches!(
        err,
        AssetRemoveError::AssetInUse {
            ref asset_id,
            referencing_count: 1,
        } if asset_id == TARGET_ASSET
    ));
}

#[test]
fn compute_patch_asset_in_use_explicit_cascade_false_returns_e_asset_in_use() {
    let prior = project_with_referencing_clips(vec![video_clip(CLIP_A1, TARGET_ASSET, 0, 240_000)]);
    let args = base_args(TARGET_ASSET, Some(false));
    let err = compute_patch(&prior, &args).expect_err("in use");
    assert!(matches!(err, AssetRemoveError::AssetInUse { .. }));
}

#[test]
fn compute_patch_asset_in_use_count_matches_referencing_clip_count() {
    let prior = project_with_referencing_clips(vec![
        video_clip(CLIP_A1, TARGET_ASSET, 0, 100_000),
        video_clip(CLIP_A2, TARGET_ASSET, 100_000, 100_000),
    ]);
    let args = base_args(TARGET_ASSET, None);
    let err = compute_patch(&prior, &args).expect_err("in use");
    assert!(matches!(
        err,
        AssetRemoveError::AssetInUse {
            referencing_count: 2,
            ..
        }
    ));
}

// ---------------------------------------------------------------------
// Happy paths
// ---------------------------------------------------------------------

#[test]
fn compute_patch_orphan_asset_no_cascade_succeeds_clip_ids_empty() {
    let prior = orphan_project();
    let args = base_args(TARGET_ASSET, None);
    let (patch, warnings, data) = compute_patch(&prior, &args).expect("orphan removal");

    assert_eq!(patch_ops(&patch).len(), 1);
    assert_eq!(patch_ops(&patch)[0]["op"], "remove");
    assert_eq!(patch_ops(&patch)[0]["path"], "/assets/0");
    assert!(data.removed_clip_ids.is_empty());
    assert!(!data.file_orphaned);
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_ASSET_REMOVE_ENVELOPE_CODE);
}

#[test]
fn compute_patch_cascade_removes_single_referencing_clip() {
    let prior = project_with_referencing_clips(vec![video_clip(CLIP_A1, TARGET_ASSET, 0, 240_000)]);
    let args = base_args(TARGET_ASSET, Some(true));
    let (patch, _warnings, data) = compute_patch(&prior, &args).expect("cascade removal");

    let ops = patch_ops(&patch);
    assert_eq!(ops.len(), 2);
    assert_eq!(ops[0]["op"], "remove");
    assert_eq!(ops[0]["path"], "/tracks/0/clips/0");
    assert_eq!(ops[1]["op"], "remove");
    assert_eq!(ops[1]["path"], "/assets/0");
    assert_eq!(data.removed_clip_ids.len(), 1);
    assert_eq!(data.removed_clip_ids[0].to_string(), CLIP_A1);
}

#[test]
fn compute_patch_cascade_removes_multiple_clips_descending_indices() {
    // Two clips on the same track that both reference the target asset.
    // Patch removes them in descending clip-index order so each later
    // index stays valid after earlier removals — mirrors clip.delete.
    let prior = project_with_referencing_clips(vec![
        video_clip(CLIP_A1, TARGET_ASSET, 0, 100_000),
        video_clip(CLIP_A2, TARGET_ASSET, 100_000, 100_000),
    ]);
    let args = base_args(TARGET_ASSET, Some(true));
    let (patch, _warnings, data) = compute_patch(&prior, &args).expect("cascade removal");

    let ops = patch_ops(&patch);
    assert_eq!(ops.len(), 3);
    assert_eq!(ops[0]["path"], "/tracks/0/clips/1");
    assert_eq!(ops[1]["path"], "/tracks/0/clips/0");
    assert_eq!(ops[2]["path"], "/assets/0");
    assert_eq!(data.removed_clip_ids.len(), 2);
}

#[test]
fn compute_patch_cascade_removes_clips_across_multiple_tracks() {
    // Two tracks, both with a clip referencing the target asset, plus a
    // survivor audio track to pin duration_tk after applying the patch.
    let mut project = empty_project();
    project.assets.clear();
    project
        .assets
        .push(serde_json::from_value(target_asset_value()).expect("target asset parses"));
    project
        .assets
        .push(serde_json::from_value(survivor_asset_value()).expect("survivor asset parses"));
    project.tracks = vec![
        serde_json::from_value(track(
            VIDEO_TRACK_A,
            TrackKind::Video,
            vec![video_clip(CLIP_A1, TARGET_ASSET, 0, 100_000)],
        ))
        .expect("track A parses"),
        serde_json::from_value(track(
            VIDEO_TRACK_B,
            TrackKind::Video,
            vec![video_clip(CLIP_B1, TARGET_ASSET, 0, 100_000)],
        ))
        .expect("track B parses"),
        serde_json::from_value(track(
            "01900000-0000-7000-8000-0000000aa903",
            TrackKind::Audio,
            vec![json!({
                "id": SURVIVOR_CLIP,
                "name": "Survivor",
                "asset_id": SURVIVOR_ASSET,
                "track_position_tk": 0,
                "source_in_tk": 0,
                "source_out_tk": 480_000,
                "volume": 1.0,
                "locked": false,
            })],
        ))
        .expect("survivor track parses"),
    ];
    project.duration_tk = Tick::new(480_000);

    let args = base_args(TARGET_ASSET, Some(true));
    let (_patch, _warnings, data) = compute_patch(&project, &args).expect("cross-track cascade");

    // Both target clips removed regardless of which track they live on.
    let removed: Vec<String> = data
        .removed_clip_ids
        .iter()
        .map(ToString::to_string)
        .collect();
    assert!(removed.contains(&CLIP_A1.to_string()));
    assert!(removed.contains(&CLIP_B1.to_string()));
    assert_eq!(removed.len(), 2);
}

#[test]
fn compute_patch_cascade_zero_referencing_clips_no_clip_removal() {
    let prior = orphan_project();
    let args = base_args(TARGET_ASSET, Some(true));
    let (patch, _warnings, data) =
        compute_patch(&prior, &args).expect("orphan cascade is a no-op for clips");

    let ops = patch_ops(&patch);
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0]["path"], "/assets/0");
    assert!(data.removed_clip_ids.is_empty());
}

#[test]
fn compute_patch_multi_asset_only_target_removed() {
    // Two assets in the registry; removing the target leaves the
    // survivor in place.
    let prior = project_with_referencing_clips(vec![]);
    let args = base_args(TARGET_ASSET, None);
    let (patch, _warnings, data) = compute_patch(&prior, &args).expect("remove target only");

    let ops = patch_ops(&patch);
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0]["path"], "/assets/0");
    assert_eq!(data.removed_asset_id.to_string(), TARGET_ASSET);

    let post = apply_patch(&prior, patch);
    assert_eq!(post.assets.len(), 1);
    assert_eq!(post.assets[0].id().to_string(), SURVIVOR_ASSET);
}

// ---------------------------------------------------------------------
// Envelope warning
// ---------------------------------------------------------------------

#[test]
fn envelope_warning_emitted_on_orphan_removal() {
    let prior = orphan_project();
    let args = base_args(TARGET_ASSET, None);
    let (_patch, warnings, _data) = compute_patch(&prior, &args).expect("orphan removal");

    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_ASSET_REMOVE_ENVELOPE_CODE);
    assert_eq!(warnings[0]["details"]["removed_asset_id"], TARGET_ASSET);
    assert_eq!(warnings[0]["details"]["removed_clip_ids"], json!([]));
    assert_eq!(warnings[0]["details"]["file_orphaned"], json!(false));
}

#[test]
fn envelope_warning_carries_cascade_removed_clip_ids_sorted() {
    let prior = project_with_referencing_clips(vec![
        video_clip(CLIP_A2, TARGET_ASSET, 100_000, 100_000),
        video_clip(CLIP_A1, TARGET_ASSET, 0, 100_000),
    ]);
    let args = base_args(TARGET_ASSET, Some(true));
    let (_patch, warnings, data) = compute_patch(&prior, &args).expect("cascade removal");

    // `removed_clip_ids` is sorted by UUID string regardless of input
    // order, so the envelope is deterministic.
    let envelope_ids = &warnings[0]["details"]["removed_clip_ids"];
    assert_eq!(envelope_ids, &json!([CLIP_A1, CLIP_A2]));
    assert_eq!(data.removed_clip_ids.len(), 2);
    assert_eq!(data.removed_clip_ids[0].to_string(), CLIP_A1);
    assert_eq!(data.removed_clip_ids[1].to_string(), CLIP_A2);
}

#[test]
fn envelope_warning_file_orphaned_is_false_until_cross_project_check_lands() {
    // Documented deferral: the cross-project projects-index walk is
    // wired in a follow-up. Until it ships, the envelope always reports
    // `file_orphaned: false`.
    let prior = orphan_project();
    let args = base_args(TARGET_ASSET, None);
    let (_patch, _warnings, data) = compute_patch(&prior, &args).expect("orphan removal");
    assert!(!data.file_orphaned);
}

// ---------------------------------------------------------------------
// Reconstructor round-trips
// ---------------------------------------------------------------------

#[test]
fn reconstruct_round_trip_no_cascade() {
    let prior = orphan_project();
    let args = base_args(TARGET_ASSET, None);
    let (_patch, warnings, data) = compute_patch(&prior, &args).expect("orphan removal");

    let reconstructed = data_envelope_from_args_warnings(&args, &warnings).expect("round-trip");
    assert_eq!(data, reconstructed);
}

#[test]
fn reconstruct_round_trip_cascade() {
    let prior = project_with_referencing_clips(vec![
        video_clip(CLIP_A1, TARGET_ASSET, 0, 100_000),
        video_clip(CLIP_A2, TARGET_ASSET, 100_000, 100_000),
    ]);
    let args = base_args(TARGET_ASSET, Some(true));
    let (_patch, warnings, data) = compute_patch(&prior, &args).expect("cascade removal");

    let reconstructed = data_envelope_from_args_warnings(&args, &warnings).expect("round-trip");
    assert_eq!(data, reconstructed);
}

#[test]
fn reconstruct_round_trip_via_json_value_equality() {
    // Stronger check: serialize both `data` envelopes through JSON and
    // assert byte-equal canonical forms.
    let prior = project_with_referencing_clips(vec![video_clip(CLIP_A1, TARGET_ASSET, 0, 240_000)]);
    let args = base_args(TARGET_ASSET, Some(true));
    let (_patch, warnings, data) = compute_patch(&prior, &args).expect("cascade removal");

    let original_value = serde_json::to_value(&data).expect("data serialises");
    let reconstructed = data_envelope_from_args_warnings(&args, &warnings).expect("round-trip");
    let reconstructed_value = serde_json::to_value(&reconstructed).expect("data serialises");

    assert_eq!(original_value, reconstructed_value);
}

#[test]
fn reconstruct_rejects_warning_set_without_envelope() {
    let args = base_args(TARGET_ASSET, None);
    let warnings: Vec<Value> = vec![json!({
        "code": "W_OTHER",
        "message": "not the envelope",
        "details": {},
    })];
    let err = data_envelope_from_args_warnings(&args, &warnings)
        .expect_err("missing envelope warning must surface");
    assert!(matches!(
        err,
        verbreel_state::ReconstructError::MissingField { .. }
    ));
}

// ---------------------------------------------------------------------
// Apply round-trips (post-state assertions)
// ---------------------------------------------------------------------

#[test]
fn apply_patch_no_cascade_yields_post_state_without_asset() {
    let prior = project_with_referencing_clips(vec![]);
    let args = base_args(TARGET_ASSET, None);
    let (patch, _warnings, _data) = compute_patch(&prior, &args).expect("removal");
    let post = apply_patch(&prior, patch);

    assert_eq!(post.assets.len(), 1);
    assert_eq!(post.assets[0].id().to_string(), SURVIVOR_ASSET);
}

#[test]
fn apply_patch_cascade_yields_post_state_without_asset_or_referencing_clips() {
    let prior = project_with_referencing_clips(vec![video_clip(CLIP_A1, TARGET_ASSET, 0, 240_000)]);
    let args = base_args(TARGET_ASSET, Some(true));
    let (patch, _warnings, _data) = compute_patch(&prior, &args).expect("cascade removal");
    let post = apply_patch(&prior, patch);

    // Target asset removed; survivor asset retained.
    assert_eq!(post.assets.len(), 1);
    assert_eq!(post.assets[0].id().to_string(), SURVIVOR_ASSET);
    // Track A no longer carries the cascaded clip.
    assert_eq!(post.tracks[0].clips.len(), 0);
    // Survivor audio clip on track B is untouched.
    assert_eq!(post.tracks[1].clips.len(), 1);
    assert_eq!(post.tracks[1].clips[0].id.to_string(), SURVIVOR_CLIP);
}

// ---------------------------------------------------------------------
// Verb-trait + registry integration
// ---------------------------------------------------------------------

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "asset.remove")
        .expect("default_fixtures includes asset.remove");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(AssetRemoveVerb))
        .expect("register asset.remove verb");

    let report =
        validate_reconstructors(&registry, &[fixture]).expect("asset.remove reconstructor passes");
    assert_eq!(report.verbs_checked, vec!["asset.remove"]);
    assert_eq!(report.fixtures_run, 1);
}

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let prior = orphan_project();
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        prior,
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry succeeds");

    let outcome = store
        .mutate_via_verb(
            "asset.remove",
            serde_json::to_value(base_args(TARGET_ASSET, None)).expect("args serialize"),
            None,
        )
        .expect("asset.remove should route");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("happy path must apply");
    };
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_ASSET_REMOVE_ENVELOPE_CODE);
    let envelope: AssetRemoveData = serde_json::from_value(data).expect("data parses");
    assert_eq!(envelope.removed_asset_id.to_string(), TARGET_ASSET);
    assert!(envelope.removed_clip_ids.is_empty());
    assert!(!envelope.file_orphaned);
}
