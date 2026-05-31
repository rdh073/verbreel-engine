//! Tests for `asset.list` (§3.2) — twenty-eighth production verb.

use std::sync::Arc;

use serde_json::json;
use verbreel_state::verbs::asset_list::{compute_patch, data_envelope_from_post_state};
use verbreel_state::{
    Asset, AssetListArgs, AssetListData, AssetListVerb, MutateOutcome, Project, Verb, VerbError,
    VerbRegistry, default_fixtures, default_registry, validate_reconstructors,
};

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const ASSET_ID_AUDIO: &str = "01900000-0000-7000-8000-000000000001";
const ASSET_ID_VIDEO: &str = "01900000-0000-7000-8000-000000000002";
const ASSET_ID_VIDEO_TIE_LOWER: &str = "01900000-0000-7000-8000-000000000003";
const ASSET_ID_VIDEO_TIE_UPPER: &str = "01900000-0000-7000-8000-000000000004";
const ASSET_ID_IMAGE: &str = "01900000-0000-7000-8000-000000000005";
const ASSET_ID_SUBTITLE: &str = "01900000-0000-7000-8000-000000000006";
const VIDEO_HASH: &str = "53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658";
const AUDIO_HASH: &str = "36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da";
const IMAGE_HASH: &str = "aa761291ff9d068556f2d1d6f63c53a4d22e44d65f882c1c252a04372123add3";
const SUBTITLE_HASH: &str = "4000145a4200d4861daaac417051ced93cb850cbb819eb9c8bafe9f62b08e6ba";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture → Project")
}

fn fixture_project_id() -> verbreel_types::ProjectId {
    FIXTURE_PROJECT_ID
        .parse()
        .expect("fixture project id parses")
}

fn make_video_asset(id: &str, imported_at: &str) -> Asset {
    serde_json::from_value(json!({
        "kind": "video",
        "id": id,
        "hash": VIDEO_HASH,
        "path": format!("assets/{}/{}.mp4", &VIDEO_HASH[0..2], VIDEO_HASH),
        "original_filename": "video.mp4",
        "imported_at": imported_at,
        "metadata": {
            "duration_tk": 480000,
            "width": 1920,
            "height": 1080,
            "fps_num": 30,
            "fps_den": 1,
            "video_codec": "h264",
            "container": "mp4",
            "fingerprint": {
                "mtime_ms": 1_700_000_000_000_i64,
                "size_bytes": 1024
            }
        }
    }))
    .expect("video asset fixture parses")
}

fn make_audio_asset(id: &str, imported_at: &str) -> Asset {
    serde_json::from_value(json!({
        "kind": "audio",
        "id": id,
        "hash": AUDIO_HASH,
        "path": format!("assets/{}/{}.mp3", &AUDIO_HASH[0..2], AUDIO_HASH),
        "original_filename": "audio.mp3",
        "imported_at": imported_at,
        "metadata": {
            "duration_tk": 240000,
            "audio_codec": "aac",
            "audio_channels": 2,
            "audio_sample_rate_hz": 48000,
            "container": "mp3",
            "fingerprint": {
                "mtime_ms": 1_700_000_000_001_i64,
                "size_bytes": 512
            }
        }
    }))
    .expect("audio asset fixture parses")
}

fn make_image_asset(id: &str, imported_at: &str) -> Asset {
    serde_json::from_value(json!({
        "kind": "image",
        "id": id,
        "hash": IMAGE_HASH,
        "path": format!("assets/{}/{}.png", &IMAGE_HASH[0..2], IMAGE_HASH),
        "original_filename": "image.png",
        "imported_at": imported_at,
        "metadata": {
            "width": 1920,
            "height": 1080,
            "container": "png",
            "fingerprint": {
                "mtime_ms": 1_700_000_000_002_i64,
                "size_bytes": 256
            }
        }
    }))
    .expect("image asset fixture parses")
}

fn make_subtitle_asset(id: &str, imported_at: &str) -> Asset {
    serde_json::from_value(json!({
        "kind": "subtitle",
        "id": id,
        "hash": SUBTITLE_HASH,
        "path": format!("assets/{}/{}.srt", &SUBTITLE_HASH[0..2], SUBTITLE_HASH),
        "original_filename": "subtitle.srt",
        "imported_at": imported_at,
        "metadata": {
            "container": "srt",
            "segment_count": 8,
            "language": "en",
            "fingerprint": {
                "mtime_ms": 1_700_000_000_003_i64,
                "size_bytes": 64
            }
        }
    }))
    .expect("subtitle asset fixture parses")
}

fn project_with_assets(assets: &[Asset]) -> Project {
    let mut prior = empty_project();
    prior.assets.extend(assets.iter().cloned());
    prior
}

fn make_args(kind: Option<verbreel_state::verbs::asset_list::AssetKindFilter>) -> AssetListArgs {
    AssetListArgs {
        project_id: fixture_project_id(),
        kind,
    }
}

#[test]
fn compute_patch_empty_project_returns_empty_assets() {
    let prior = empty_project();
    let args = make_args(None);
    let (patch, warnings, data) =
        compute_patch(&prior, &args).expect("empty project should compute successfully");

    assert_eq!(patch, json!([]));
    assert!(warnings.is_empty());
    assert!(data.assets.is_empty());
}

#[test]
fn compute_patch_single_asset_returns_one_asset() {
    let prior = project_with_assets(&[make_video_asset(ASSET_ID_VIDEO, "2026-05-01T00:00:00Z")]);
    let args = make_args(None);

    let (patch, warnings, data) =
        compute_patch(&prior, &args).expect("single asset project should compute successfully");
    assert_eq!(patch, json!([]));
    assert!(warnings.is_empty());
    assert_eq!(data.assets.len(), 1);
    assert_eq!(
        data.assets[0],
        make_video_asset(ASSET_ID_VIDEO, "2026-05-01T00:00:00Z")
    );
}

#[test]
fn compute_patch_sorts_assets_across_kinds_by_imported_at() {
    let prior = project_with_assets(&[
        make_video_asset(ASSET_ID_VIDEO, "2026-05-03T00:00:00Z"),
        make_audio_asset(ASSET_ID_AUDIO, "2026-05-01T00:00:00Z"),
        make_image_asset(ASSET_ID_IMAGE, "2026-05-02T00:00:00Z"),
    ]);
    let args = make_args(None);

    let (_patch, _warnings, data) =
        compute_patch(&prior, &args).expect("mixed asset kinds should compute successfully");
    assert_eq!(data.assets.len(), 3);
    assert_eq!(
        data.assets[0],
        make_audio_asset(ASSET_ID_AUDIO, "2026-05-01T00:00:00Z")
    );
    assert_eq!(
        data.assets[1],
        make_image_asset(ASSET_ID_IMAGE, "2026-05-02T00:00:00Z")
    );
    assert_eq!(
        data.assets[2],
        make_video_asset(ASSET_ID_VIDEO, "2026-05-03T00:00:00Z")
    );
}

#[test]
fn filter_video_returns_only_video() {
    let prior = project_with_assets(&[
        make_video_asset(ASSET_ID_VIDEO, "2026-05-03T00:00:00Z"),
        make_video_asset(
            "01900000-0000-7000-8000-000000000007",
            "2026-05-02T00:00:00Z",
        ),
        make_audio_asset(ASSET_ID_AUDIO, "2026-05-01T00:00:00Z"),
    ]);

    let args = make_args(Some(
        verbreel_state::verbs::asset_list::AssetKindFilter::Video,
    ));
    let (_patch, _warnings, data) =
        compute_patch(&prior, &args).expect("video filter should compute successfully");

    assert_eq!(data.assets.len(), 2);
    assert!(
        data.assets
            .iter()
            .all(|asset| matches!(asset, Asset::Video(_)))
    );
}

#[test]
fn filter_audio_returns_only_audio() {
    let prior = project_with_assets(&[
        make_audio_asset(ASSET_ID_AUDIO, "2026-05-01T00:00:00Z"),
        make_video_asset(ASSET_ID_VIDEO, "2026-05-03T00:00:00Z"),
        make_audio_asset(
            "01900000-0000-7000-8000-000000000008",
            "2026-05-02T00:00:00Z",
        ),
    ]);

    let args = make_args(Some(
        verbreel_state::verbs::asset_list::AssetKindFilter::Audio,
    ));
    let (_patch, _warnings, data) =
        compute_patch(&prior, &args).expect("audio filter should compute successfully");

    assert_eq!(data.assets.len(), 2);
    assert!(
        data.assets
            .iter()
            .all(|asset| matches!(asset, Asset::Audio(_)))
    );
}

#[test]
fn filter_image_and_subtitle_returns_expected_kind_assets() {
    let prior = project_with_assets(&[
        make_image_asset(ASSET_ID_IMAGE, "2026-05-01T00:00:00Z"),
        make_audio_asset(ASSET_ID_AUDIO, "2026-05-02T00:00:00Z"),
        make_subtitle_asset(ASSET_ID_SUBTITLE, "2026-05-03T00:00:00Z"),
    ]);

    let image_args = make_args(Some(
        verbreel_state::verbs::asset_list::AssetKindFilter::Image,
    ));
    let (_, _, image_data) =
        compute_patch(&prior, &image_args).expect("image filter should compute successfully");
    assert_eq!(image_data.assets.len(), 1);
    assert!(
        image_data
            .assets
            .iter()
            .all(|asset| matches!(asset, Asset::Image(_)))
    );

    let subtitle_args = make_args(Some(
        verbreel_state::verbs::asset_list::AssetKindFilter::Subtitle,
    ));
    let (_, _, subtitle_data) =
        compute_patch(&prior, &subtitle_args).expect("subtitle filter should compute successfully");
    assert_eq!(subtitle_data.assets.len(), 1);
    assert!(
        subtitle_data
            .assets
            .iter()
            .all(|asset| matches!(asset, Asset::Subtitle(_)))
    );
}

#[test]
fn compute_patch_sort_stability_tiebreaker_by_asset_id() {
    let prior = project_with_assets(&[
        make_video_asset(ASSET_ID_VIDEO_TIE_UPPER, "2026-05-01T00:00:00Z"),
        make_video_asset(ASSET_ID_VIDEO_TIE_LOWER, "2026-05-01T00:00:00Z"),
    ]);
    let args = make_args(None);
    let (_patch, _warnings, data) =
        compute_patch(&prior, &args).expect("tie-breaker case should compute successfully");

    assert_eq!(data.assets.len(), 2);
    assert_eq!(
        data.assets[0],
        make_video_asset(ASSET_ID_VIDEO_TIE_LOWER, "2026-05-01T00:00:00Z")
    );
    assert_eq!(
        data.assets[1],
        make_video_asset(ASSET_ID_VIDEO_TIE_UPPER, "2026-05-01T00:00:00Z")
    );
}

#[test]
fn compute_patch_patch_is_always_empty() {
    let prior = project_with_assets(&[
        make_video_asset(ASSET_ID_VIDEO, "2026-05-01T00:00:00Z"),
        make_audio_asset(ASSET_ID_AUDIO, "2026-05-02T00:00:00Z"),
    ]);
    let args = make_args(None);
    let (patch, _warnings, _) = compute_patch(&prior, &args).expect("should compute patch");
    assert_eq!(patch, json!([]));
}

#[test]
fn compute_patch_warnings_always_empty() {
    let prior = project_with_assets(&[
        make_video_asset(ASSET_ID_VIDEO, "2026-05-01T00:00:00Z"),
        make_audio_asset(ASSET_ID_AUDIO, "2026-05-02T00:00:00Z"),
    ]);
    let args = make_args(None);
    let (_patch, warnings, _) =
        compute_patch(&prior, &args).expect("should compute without warnings");
    assert!(warnings.is_empty());
}

#[test]
fn invalid_kind_string_maps_to_bad_args() {
    let prior = empty_project();
    let verb = AssetListVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({
                "project_id": FIXTURE_PROJECT_ID,
                "kind": "bad-kind",
            }),
        )
        .expect_err("invalid kind should fail");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "asset.list")
        .expect("default_fixtures includes asset.list");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(AssetListVerb))
        .expect("register asset.list");

    let report = validate_reconstructors(&registry, &[fixture])
        .expect("asset.list reconstruct from fixture should pass");
    assert_eq!(report.verbs_checked, vec!["asset.list"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn data_envelope_from_post_state_rebuilds_data() {
    let prior = project_with_assets(&[
        make_video_asset(ASSET_ID_VIDEO, "2026-05-01T00:00:00Z"),
        make_audio_asset(ASSET_ID_AUDIO, "2026-05-02T00:00:00Z"),
    ]);
    let args = make_args(None);

    let (patch, _, expected) = compute_patch(&prior, &args).expect("compute patch");
    let patch: json_patch::Patch = serde_json::from_value(patch).expect("valid patch");
    let post_state = prior
        .apply(&patch)
        .expect("applying empty patch should succeed");

    let data = data_envelope_from_post_state(&args, &post_state)
        .expect("data envelope from post state should rebuild data");
    assert_eq!(data, expected);
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
            "asset.list",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
            }),
            None,
        )
        .expect("asset.list should route");

    let MutateOutcome::NoOp { data, warnings, .. } = outcome else {
        panic!("asset.list expected NoOp outcome");
    };

    assert!(warnings.is_empty());
    let data: AssetListData = serde_json::from_value(data).expect("asset.list data deserializes");
    assert_eq!(data.assets.len(), 0);
}
