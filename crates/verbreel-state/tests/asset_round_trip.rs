//! Round-trip + schema-validation tests for the typed [`Asset`] enum.
//!
//! Mirrors the existing `tests/round_trip.rs` + `tests/schema_validation.rs`
//! pattern but against a 4-asset fixture so each variant exercises the
//! `#[serde(tag = "kind")]` discriminator.

use std::path::{Path, PathBuf};

use serde_json::Value;
use verbreel_state::asset_meta::{
    AudioAssetMetadata, FileFingerprint, ImageAssetMetadata, SubtitleAssetMetadata,
    VideoAssetMetadata,
};
use verbreel_state::{
    Asset, AssetPath, AudioAsset, ImageAsset, Project, Sha256, SubtitleAsset, VideoAsset,
};
use verbreel_types::{AssetId, Tick, UuidV7};

const FIXTURE_EMPTY: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_WITH_ASSETS: &str = include_str!("fixtures/project_with_assets.json");

/// Schema discovery — same logic as `tests/schema_validation.rs`.
fn find_schema() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("VERBREEL_SPEC_DIR") {
        let p = PathBuf::from(dir).join("spec").join("project-schema.json");
        if p.is_file() {
            return Some(p);
        }
    }
    let start = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut cur: &Path = &start;
    loop {
        let candidate = cur
            .join("..")
            .join("verbreel-spec")
            .join("spec")
            .join("project-schema.json");
        if let Ok(canonical) = candidate.canonicalize()
            && canonical.is_file()
        {
            return Some(canonical);
        }
        match cur.parent() {
            Some(parent) => cur = parent,
            None => return None,
        }
    }
}

#[test]
fn round_trip_project_with_assets() {
    let p1: Project = serde_json::from_str(FIXTURE_WITH_ASSETS).expect("fixture → Project");
    let s = serde_json::to_string_pretty(&p1).expect("Project → JSON");

    let v_in: Value = serde_json::from_str(FIXTURE_WITH_ASSETS).expect("fixture → Value");
    let v_out: Value = serde_json::from_str(&s).expect("round-trip JSON → Value");
    assert_eq!(
        v_in, v_out,
        "round-trip Value must equal the 4-asset fixture (left = fixture, right = serialized)"
    );

    let p2: Project = serde_json::from_str(&s).expect("round-trip JSON → Project");
    assert_eq!(p1, p2, "Project PartialEq must hold across the round trip");
    assert_eq!(p1.assets.len(), 4, "fixture carries 4 assets");
}

#[test]
fn empty_project_still_round_trips_after_asset_typing() {
    // Lock the existing slice's invariant: Vec<Asset>::new() must
    // serialize identically to the pre-typed Vec<Value>::new() — i.e.
    // `"assets": []`. If this fails, the typed Asset wiring leaked
    // into the empty-fixture round-trip.
    let p1: Project = serde_json::from_str(FIXTURE_EMPTY).expect("empty fixture → Project");
    let s = serde_json::to_string_pretty(&p1).expect("Project → JSON");
    let v_in: Value = serde_json::from_str(FIXTURE_EMPTY).expect("fixture → Value");
    let v_out: Value = serde_json::from_str(&s).expect("round-trip JSON → Value");
    assert_eq!(v_in, v_out, "empty-assets fixture must still round-trip");
}

#[test]
fn schema_validates_project_with_assets() {
    let Some(schema_path) = find_schema() else {
        println!(
            "skipping schema_validates_project_with_assets: VERBREEL_SPEC_DIR unset and no \
             sibling verbreel-spec checkout"
        );
        return;
    };
    let schema_text = std::fs::read_to_string(&schema_path)
        .unwrap_or_else(|e| panic!("read schema at {}: {e}", schema_path.display()));
    let schema: Value = serde_json::from_str(&schema_text).expect("schema parses");
    let validator = jsonschema::validator_for(&schema).expect("schema compiles");

    let fixture_value: Value = serde_json::from_str(FIXTURE_WITH_ASSETS).expect("fixture parses");
    if let Err(errors) = validator.validate(&fixture_value) {
        panic!("4-asset fixture must validate against project-schema.json: {errors}");
    }

    let project: Project = serde_json::from_str(FIXTURE_WITH_ASSETS).expect("fixture → Project");
    let typed_value = serde_json::to_value(&project).expect("Project → Value");
    if let Err(errors) = validator.validate(&typed_value) {
        panic!("typed Project re-serialized must validate against project-schema.json: {errors}");
    }
}

/// Build a minimal VideoAsset for the discriminator tests. The
/// metadata carries only the required fields; optional fields stay
/// `None` (skip_serializing_if avoids polluting the output JSON).
fn make_video_asset() -> Asset {
    let id: AssetId = AssetId::from_uuid_v7(
        "0190b8d3-15e3-7000-bd00-0000000000a1"
            .parse::<UuidV7>()
            .unwrap(),
    );
    Asset::Video(VideoAsset {
        id,
        hash: Sha256::new(
            "53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658".to_string(),
        )
        .unwrap(),
        path: AssetPath::new(
            "assets/53/53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.mp4"
                .to_string(),
        )
        .unwrap(),
        original_filename: "v.mp4".to_string(),
        imported_at: "2026-05-24T00:00:00Z".to_string(),
        metadata: VideoAssetMetadata {
            duration_tk: Tick::new(240000),
            width: 1920,
            height: 1080,
            fps_num: 30,
            fps_den: 1,
            video_codec: "h264".to_string(),
            audio_codec: None,
            audio_channels: None,
            audio_sample_rate_hz: None,
            bitrate_bps: None,
            color_space: None,
            color_primaries: None,
            container: "mp4".to_string(),
            has_alpha: None,
            rotation_deg: None,
            fingerprint: FileFingerprint {
                mtime_ms: 1_700_000_000_000,
                size_bytes: 1024,
            },
        },
    })
}

fn make_audio_asset() -> Asset {
    let id: AssetId = AssetId::from_uuid_v7(
        "0190b8d3-15e3-7000-bd00-0000000000a2"
            .parse::<UuidV7>()
            .unwrap(),
    );
    Asset::Audio(AudioAsset {
        id,
        hash: Sha256::new(
            "36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da".to_string(),
        )
        .unwrap(),
        path: AssetPath::new(
            "assets/36/36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da.m4a"
                .to_string(),
        )
        .unwrap(),
        original_filename: "a.m4a".to_string(),
        imported_at: "2026-05-24T00:00:00Z".to_string(),
        metadata: AudioAssetMetadata {
            duration_tk: Tick::new(120000),
            audio_codec: "aac".to_string(),
            audio_channels: 2,
            audio_sample_rate_hz: 48000,
            bitrate_bps: None,
            container: "m4a".to_string(),
            fingerprint: FileFingerprint {
                mtime_ms: 1_700_000_000_000,
                size_bytes: 512,
            },
        },
    })
}

fn make_image_asset() -> Asset {
    let id: AssetId = AssetId::from_uuid_v7(
        "0190b8d3-15e3-7000-bd00-0000000000a3"
            .parse::<UuidV7>()
            .unwrap(),
    );
    Asset::Image(ImageAsset {
        id,
        hash: Sha256::new(
            "aa761291ff9d068556f2d1d6f63c53a4d22e44d65f882c1c252a04372123add3".to_string(),
        )
        .unwrap(),
        path: AssetPath::new(
            "assets/aa/aa761291ff9d068556f2d1d6f63c53a4d22e44d65f882c1c252a04372123add3.png"
                .to_string(),
        )
        .unwrap(),
        original_filename: "i.png".to_string(),
        imported_at: "2026-05-24T00:00:00Z".to_string(),
        metadata: ImageAssetMetadata {
            width: 1920,
            height: 1080,
            container: "png".to_string(),
            has_alpha: Some(true),
            color_space: None,
            rotation_deg: None,
            fingerprint: FileFingerprint {
                mtime_ms: 1_700_000_000_000,
                size_bytes: 256,
            },
        },
    })
}

fn make_subtitle_asset() -> Asset {
    let id: AssetId = AssetId::from_uuid_v7(
        "0190b8d3-15e3-7000-bd00-0000000000a4"
            .parse::<UuidV7>()
            .unwrap(),
    );
    Asset::Subtitle(SubtitleAsset {
        id,
        hash: Sha256::new(
            "4000145a4200d4861daaac417051ced93cb850cbb819eb9c8bafe9f62b08e6ba".to_string(),
        )
        .unwrap(),
        path: AssetPath::new(
            "assets/40/4000145a4200d4861daaac417051ced93cb850cbb819eb9c8bafe9f62b08e6ba.srt"
                .to_string(),
        )
        .unwrap(),
        original_filename: "s.srt".to_string(),
        imported_at: "2026-05-24T00:00:00Z".to_string(),
        metadata: SubtitleAssetMetadata {
            container: "srt".to_string(),
            language: Some("en".to_string()),
            segment_count: Some(12),
            fingerprint: FileFingerprint {
                mtime_ms: 1_700_000_000_000,
                size_bytes: 64,
            },
        },
    })
}

/// Assert the on-wire shape carries `"kind": "<tag>"` at the SAME
/// object level as `id` / `hash` — i.e. flat, not wrapped in an
/// enum-discriminator key like `{ "Video": { ... } }`.
fn assert_kind_tag_is_flat(asset: &Asset, expected_kind: &str) {
    let v = serde_json::to_value(asset).expect("Asset → Value");
    let obj = v
        .as_object()
        .expect("Asset must serialize to a JSON object, not a string/array/null");
    assert!(
        !obj.contains_key("Video")
            && !obj.contains_key("Audio")
            && !obj.contains_key("Image")
            && !obj.contains_key("Subtitle"),
        "Asset object must NOT carry an enum-variant key (got keys: {:?})",
        obj.keys().collect::<Vec<_>>()
    );
    let kind = obj
        .get("kind")
        .and_then(Value::as_str)
        .expect("Asset must carry a `kind` field at the object root");
    assert_eq!(
        kind, expected_kind,
        "discriminator `kind` field must match the variant"
    );
    assert!(
        obj.contains_key("id") && obj.contains_key("hash") && obj.contains_key("path"),
        "variant payload fields must appear at the SAME level as `kind`, got keys: {:?}",
        obj.keys().collect::<Vec<_>>()
    );
}

#[test]
fn asset_kind_tag_discriminator_video() {
    assert_kind_tag_is_flat(&make_video_asset(), "video");
}

#[test]
fn asset_kind_tag_discriminator_audio() {
    assert_kind_tag_is_flat(&make_audio_asset(), "audio");
}

#[test]
fn asset_kind_tag_discriminator_image() {
    assert_kind_tag_is_flat(&make_image_asset(), "image");
}

#[test]
fn asset_kind_tag_discriminator_subtitle() {
    assert_kind_tag_is_flat(&make_subtitle_asset(), "subtitle");
}
