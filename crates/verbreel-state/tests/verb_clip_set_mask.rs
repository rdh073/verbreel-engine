//! Tests for `clip.set_mask` (§5.19) — forty-first production verb.

use std::sync::Arc;

use serde_json::{Map, Value, json};
use verbreel_state::verbs::clip_set_mask::{
    W_KEYFRAMES_REMOVED_CODE, compute_patch, data_envelope_from_post_state_warnings,
};
use verbreel_state::{
    ClipMask, ClipSetMaskArgs, ClipSetMaskData, ClipSetMaskError, ClipSetMaskVerb, MaskKind,
    MutateOutcome, Project, Track, VerbRegistry, default_fixtures, default_registry,
    validate_reconstructors,
};
use verbreel_types::{ProjectId, Tick};

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const TRACK_ID: &str = "01900000-0000-7000-8000-0000000aa501";
const CLIP_ID: &str = "01900000-0000-7000-8000-0000000bb501";
const MISSING_CLIP_ID: &str = "01900000-0000-7000-8000-0000000bb599";
const VIDEO_ASSET_ID: &str = "01900000-0000-7000-8000-0000000cc501";
const IMAGE_ASSET_ID: &str = "01900000-0000-7000-8000-0000000cc502";
const AUDIO_ASSET_ID: &str = "01900000-0000-7000-8000-0000000cc503";
const MISSING_ASSET_ID: &str = "01900000-0000-7000-8000-0000000cc599";
const K1: &str = "01900000-0000-7000-8000-0000000ff501";
const K2: &str = "01900000-0000-7000-8000-0000000ff502";
const K3: &str = "01900000-0000-7000-8000-0000000ff503";
const K4: &str = "01900000-0000-7000-8000-0000000ff504";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture -> Project")
}

fn fixture_project_id() -> ProjectId {
    FIXTURE_PROJECT_ID
        .parse()
        .expect("fixture project id parses")
}

fn mask(kind: MaskKind, params: Value) -> ClipMask {
    ClipMask {
        kind,
        params: params.as_object().expect("params object").clone(),
        feather_px: 2.0,
        inverted: false,
    }
}

fn rect_mask() -> ClipMask {
    mask(
        MaskKind::Rect,
        json!({
            "x": 10.0,
            "y": 20.0,
            "w": 320.0,
            "h": 180.0,
        }),
    )
}

fn ellipse_mask() -> ClipMask {
    mask(
        MaskKind::Ellipse,
        json!({
            "cx": 100.0,
            "cy": 120.0,
            "rx": 80.0,
            "ry": 60.0,
        }),
    )
}

fn polygon_mask(point_count: usize) -> ClipMask {
    let points = (0..point_count)
        .map(|idx| json!([idx as f64, (idx % 7) as f64]))
        .collect::<Vec<_>>();
    mask(MaskKind::Polygon, json!({ "points": points }))
}

fn asset_mask(asset_id: &str, threshold: Option<Value>) -> ClipMask {
    let mut params = Map::new();
    params.insert("asset_id".to_string(), json!(asset_id));
    if let Some(threshold) = threshold {
        params.insert("threshold".to_string(), threshold);
    }
    ClipMask {
        kind: MaskKind::Asset,
        params,
        feather_px: 0.0,
        inverted: false,
    }
}

#[derive(Debug, Clone, Default)]
struct ProjectOptions {
    track_locked: bool,
    clip_locked: bool,
    prior_mask: Option<ClipMask>,
    keyframes: Vec<Value>,
    include_image_asset: bool,
    include_audio_asset: bool,
}

fn video_asset() -> Value {
    json!({
        "id": VIDEO_ASSET_ID,
        "kind": "video",
        "hash": "53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658",
        "path": "assets/53/53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.mp4",
        "original_filename": "clip-set-mask.mp4",
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

fn image_asset() -> Value {
    json!({
        "id": IMAGE_ASSET_ID,
        "kind": "image",
        "hash": "8d969eef6ecad3c29a3a629280e686cf0c3f5d5a86aff3ca12020c923adc6c92",
        "path": "assets/8d/8d969eef6ecad3c29a3a629280e686cf0c3f5d5a86aff3ca12020c923adc6c92.png",
        "original_filename": "mask.png",
        "imported_at": "2026-05-24T00:00:00Z",
        "metadata": {
            "width": 640,
            "height": 360,
            "container": "png",
            "has_alpha": true,
            "fingerprint": {
                "mtime_ms": 1_700_000_000_000_i64,
                "size_bytes": 512,
            }
        }
    })
}

fn audio_asset() -> Value {
    json!({
        "id": AUDIO_ASSET_ID,
        "kind": "audio",
        "hash": "36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da",
        "path": "assets/36/36edd72e6e1929f34401d60618f260e1a1e6869e3789619618eb08e6c063d1da.m4a",
        "original_filename": "mask.m4a",
        "imported_at": "2026-05-24T00:00:00Z",
        "metadata": {
            "duration_tk": 480_000,
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

fn project_with_options(options: ProjectOptions) -> Project {
    let mut project = empty_project();
    project.tracks.clear();
    project.assets.clear();
    project
        .assets
        .push(serde_json::from_value(video_asset()).expect("video asset parses"));
    if options.include_image_asset {
        project
            .assets
            .push(serde_json::from_value(image_asset()).expect("image asset parses"));
    }
    if options.include_audio_asset {
        project
            .assets
            .push(serde_json::from_value(audio_asset()).expect("audio asset parses"));
    }

    let mut clip = json!({
        "id": CLIP_ID,
        "name": "Masked Clip",
        "asset_id": VIDEO_ASSET_ID,
        "track_position_tk": 0,
        "source_in_tk": 0,
        "source_out_tk": 480_000,
        "locked": options.clip_locked,
        "keyframes": options.keyframes,
    });
    if let Some(mask) = options.prior_mask {
        clip.as_object_mut().expect("clip object").insert(
            "mask".to_string(),
            serde_json::to_value(mask).expect("mask serializes"),
        );
    }

    let track: Track = serde_json::from_value(json!({
        "id": TRACK_ID,
        "kind": "video",
        "name": "Video",
        "locked": options.track_locked,
        "clips": [clip],
    }))
    .expect("track fixture parses");

    project.tracks.push(track);
    project.duration_tk = Tick::new(480_000);
    project
}

fn project() -> Project {
    project_with_options(ProjectOptions::default())
}

fn args(mask: Option<ClipMask>) -> ClipSetMaskArgs {
    ClipSetMaskArgs {
        project_id: fixture_project_id(),
        clip: CLIP_ID.to_string(),
        mask,
    }
}

fn keyframe(id: &str, property: &str) -> Value {
    json!({
        "id": id,
        "property": property,
        "time_tk": 0,
        "value": 1.0,
        "easing": "linear",
    })
}

fn apply_patch(prior: &Project, patch: Value) -> Project {
    let patch: json_patch::Patch = serde_json::from_value(patch).expect("patch parses");
    prior.apply(&patch).expect("patch applies")
}

fn keyframe_ids(project: &Project) -> Vec<String> {
    project.tracks[0].clips[0]
        .keyframes
        .iter()
        .map(|keyframe| keyframe.id.to_string())
        .collect()
}

#[test]
fn compute_patch_sets_rect_mask() {
    let prior = project();
    let (patch, warnings, data) = compute_patch(&prior, &args(Some(rect_mask()))).expect("rect");

    assert!(warnings.is_empty());
    assert_eq!(patch.as_array().expect("patch array").len(), 1);
    assert_eq!(patch[0]["path"], "/tracks/0/clips/0/mask");
    assert_eq!(data.clip_id.to_string(), CLIP_ID);
    assert_eq!(data.mask.expect("mask").kind, MaskKind::Rect);
}

#[test]
fn compute_patch_sets_ellipse_mask() {
    let prior = project();
    let (_patch, warnings, data) =
        compute_patch(&prior, &args(Some(ellipse_mask()))).expect("ellipse");

    assert!(warnings.is_empty());
    assert_eq!(data.mask.expect("mask").kind, MaskKind::Ellipse);
}

#[test]
fn compute_patch_sets_polygon_mask() {
    let prior = project();
    let (_patch, warnings, data) =
        compute_patch(&prior, &args(Some(polygon_mask(3)))).expect("polygon");

    assert!(warnings.is_empty());
    assert_eq!(data.mask.expect("mask").kind, MaskKind::Polygon);
}

#[test]
fn compute_patch_sets_asset_mask_for_image_asset() {
    let prior = project_with_options(ProjectOptions {
        include_image_asset: true,
        ..ProjectOptions::default()
    });

    let (_patch, warnings, data) = compute_patch(
        &prior,
        &args(Some(asset_mask(IMAGE_ASSET_ID, Some(json!(0.5))))),
    )
    .expect("asset mask");

    assert!(warnings.is_empty());
    assert_eq!(data.mask.expect("mask").kind, MaskKind::Asset);
}

#[test]
fn compute_patch_null_removes_existing_mask() {
    let prior = project_with_options(ProjectOptions {
        prior_mask: Some(rect_mask()),
        ..ProjectOptions::default()
    });

    let (patch, warnings, data) = compute_patch(&prior, &args(None)).expect("remove mask");

    assert!(warnings.is_empty());
    assert_eq!(patch[0]["value"], Value::Null);
    assert!(data.mask.is_none());
}

#[test]
fn compute_patch_missing_clip_errors() {
    let prior = project();
    let mut args = args(Some(rect_mask()));
    args.clip = MISSING_CLIP_ID.to_string();

    let err = compute_patch(&prior, &args).expect_err("missing clip");
    assert!(matches!(err, ClipSetMaskError::ClipNotFound { .. }));
}

#[test]
fn compute_patch_bad_selector_errors() {
    let prior = project();
    let mut args = args(Some(rect_mask()));
    args.clip = "not-a-uuid".to_string();

    let err = compute_patch(&prior, &args).expect_err("bad selector");
    assert!(matches!(err, ClipSetMaskError::BadSelector { .. }));
}

#[test]
fn compute_patch_locked_clip_errors() {
    let prior = project_with_options(ProjectOptions {
        clip_locked: true,
        ..ProjectOptions::default()
    });

    let err = compute_patch(&prior, &args(Some(rect_mask()))).expect_err("locked clip");
    assert!(matches!(err, ClipSetMaskError::Locked { kind: "clip", .. }));
}

#[test]
fn compute_patch_locked_track_errors() {
    let prior = project_with_options(ProjectOptions {
        track_locked: true,
        ..ProjectOptions::default()
    });

    let err = compute_patch(&prior, &args(Some(rect_mask()))).expect_err("locked track");
    assert!(matches!(
        err,
        ClipSetMaskError::Locked { kind: "track", .. }
    ));
}

#[test]
fn compute_patch_rejects_rect_non_positive_w() {
    let prior = project();
    let err = compute_patch(
        &prior,
        &args(Some(mask(
            MaskKind::Rect,
            json!({ "x": 0.0, "y": 0.0, "w": 0.0, "h": 10.0 }),
        ))),
    )
    .expect_err("bad rect");

    assert!(matches!(
        err,
        ClipSetMaskError::MaskInvalidParams {
            kind: "rect",
            field: "w",
            ..
        }
    ));
}

#[test]
fn compute_patch_rejects_ellipse_non_positive_rx() {
    let prior = project();
    let err = compute_patch(
        &prior,
        &args(Some(mask(
            MaskKind::Ellipse,
            json!({ "cx": 0.0, "cy": 0.0, "rx": 0.0, "ry": 10.0 }),
        ))),
    )
    .expect_err("bad ellipse");

    assert!(matches!(
        err,
        ClipSetMaskError::MaskInvalidParams {
            kind: "ellipse",
            field: "rx",
            ..
        }
    ));
}

#[test]
fn compute_patch_rejects_polygon_with_too_few_points() {
    let prior = project();
    let err = compute_patch(&prior, &args(Some(polygon_mask(2)))).expect_err("too few points");

    assert!(matches!(
        err,
        ClipSetMaskError::MaskInvalidParams {
            kind: "polygon",
            field: "points",
            ..
        }
    ));
}

#[test]
fn compute_patch_rejects_polygon_with_too_many_points() {
    let prior = project();
    let err = compute_patch(&prior, &args(Some(polygon_mask(257)))).expect_err("too many points");

    assert!(matches!(
        err,
        ClipSetMaskError::MaskInvalidParams {
            kind: "polygon",
            field: "points",
            ..
        }
    ));
}

#[test]
fn compute_patch_rejects_asset_threshold_out_of_range() {
    let prior = project_with_options(ProjectOptions {
        include_image_asset: true,
        ..ProjectOptions::default()
    });
    let err = compute_patch(
        &prior,
        &args(Some(asset_mask(IMAGE_ASSET_ID, Some(json!(1.01))))),
    )
    .expect_err("bad threshold");

    assert!(matches!(
        err,
        ClipSetMaskError::MaskInvalidParams {
            kind: "asset",
            field: "threshold",
            ..
        }
    ));
}

#[test]
fn compute_patch_rejects_missing_asset_mask_asset() {
    let prior = project();
    let err = compute_patch(&prior, &args(Some(asset_mask(MISSING_ASSET_ID, None))))
        .expect_err("missing asset");

    assert!(matches!(err, ClipSetMaskError::AssetNotFound { .. }));
}

#[test]
fn compute_patch_rejects_video_asset_as_mask() {
    let prior = project();
    let err = compute_patch(&prior, &args(Some(asset_mask(VIDEO_ASSET_ID, None))))
        .expect_err("video asset");

    assert!(matches!(
        err,
        ClipSetMaskError::TrackKindMismatch {
            expected_kind: "image",
            actual_kind: "video",
            ..
        }
    ));
}

#[test]
fn compute_patch_rejects_audio_asset_as_mask() {
    let prior = project_with_options(ProjectOptions {
        include_audio_asset: true,
        ..ProjectOptions::default()
    });
    let err = compute_patch(&prior, &args(Some(asset_mask(AUDIO_ASSET_ID, None))))
        .expect_err("audio asset");

    assert!(matches!(
        err,
        ClipSetMaskError::TrackKindMismatch {
            expected_kind: "image",
            actual_kind: "audio",
            ..
        }
    ));
}

#[test]
fn compute_patch_warns_and_removes_incompatible_keyframes_on_kind_change() {
    let prior = project_with_options(ProjectOptions {
        prior_mask: Some(rect_mask()),
        keyframes: vec![
            keyframe(K1, "mask.params.x"),
            keyframe(K2, "mask.params.w"),
            keyframe(K3, "mask.feather_px"),
            keyframe(K4, "opacity"),
        ],
        ..ProjectOptions::default()
    });

    let (patch, warnings, _data) =
        compute_patch(&prior, &args(Some(ellipse_mask()))).expect("kind change");
    let post = apply_patch(&prior, patch);

    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], W_KEYFRAMES_REMOVED_CODE);
    assert_eq!(
        warnings[0]["details"]["removed_keyframe_ids"],
        json!([K1, K2])
    );
    assert_eq!(keyframe_ids(&post), vec![K3.to_string(), K4.to_string()]);
}

#[test]
fn compute_patch_warns_and_removes_all_mask_keyframes_on_null() {
    let prior = project_with_options(ProjectOptions {
        prior_mask: Some(rect_mask()),
        keyframes: vec![
            keyframe(K1, "mask.params.x"),
            keyframe(K2, "mask.feather_px"),
            keyframe(K3, "opacity"),
        ],
        ..ProjectOptions::default()
    });

    let (patch, warnings, _data) = compute_patch(&prior, &args(None)).expect("clear mask");
    let post = apply_patch(&prior, patch);

    assert_eq!(warnings.len(), 1);
    assert_eq!(
        warnings[0]["details"]["removed_keyframe_ids"],
        json!([K1, K2])
    );
    assert_eq!(keyframe_ids(&post), vec![K3.to_string()]);
}

#[test]
fn compute_patch_does_not_cascade_when_prior_mask_absent() {
    let prior = project();

    let (patch, warnings, _data) =
        compute_patch(&prior, &args(Some(rect_mask()))).expect("no prior mask");

    assert!(warnings.is_empty());
    assert_eq!(patch.as_array().expect("patch array").len(), 1);
}

#[test]
fn compute_patch_does_not_cascade_when_kind_unchanged() {
    let prior = project_with_options(ProjectOptions {
        prior_mask: Some(rect_mask()),
        keyframes: vec![keyframe(K1, "mask.params.x")],
        ..ProjectOptions::default()
    });

    let (patch, warnings, _data) =
        compute_patch(&prior, &args(Some(rect_mask()))).expect("same kind");

    assert!(warnings.is_empty());
    assert_eq!(patch.as_array().expect("patch array").len(), 1);
}

#[test]
fn reconstructor_round_trips_with_cascade_warning() {
    let prior = project_with_options(ProjectOptions {
        prior_mask: Some(rect_mask()),
        keyframes: vec![
            keyframe(K1, "mask.params.x"),
            keyframe(K2, "mask.feather_px"),
        ],
        ..ProjectOptions::default()
    });
    let args = args(Some(ellipse_mask()));
    let (patch, warnings, data) = compute_patch(&prior, &args).expect("cascade");
    let post = apply_patch(&prior, patch);

    let reconstructed =
        data_envelope_from_post_state_warnings(&args, &warnings, &post).expect("reconstructs");

    assert_eq!(reconstructed, data);
    assert_eq!(reconstructed.mask.expect("mask").kind, MaskKind::Ellipse);
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "clip.set_mask")
        .expect("default_fixtures includes clip.set_mask");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(ClipSetMaskVerb))
        .expect("register clip.set_mask verb");

    let report = validate_reconstructors(&registry, &[fixture])
        .expect("clip.set_mask reconstruct from fixture");
    assert_eq!(report.verbs_checked, vec!["clip.set_mask"]);
    assert_eq!(report.fixtures_run, 1);
}

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        project(),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry clears the gate and writes project.json");

    let outcome = store
        .mutate_via_verb(
            "clip.set_mask",
            json!({
                "project_id": FIXTURE_PROJECT_ID,
                "clip": CLIP_ID,
                "mask": {
                    "kind": "rect",
                    "params": { "x": 0.0, "y": 0.0, "w": 100.0, "h": 100.0 },
                    "feather_px": 1.0,
                    "inverted": false
                }
            }),
            None,
        )
        .expect("mutate_via_verb happy path");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("happy path must return Applied, got {outcome:?}");
    };

    let data: ClipSetMaskData =
        serde_json::from_value(data).expect("clip.set_mask data is ClipSetMaskData");
    assert_eq!(data.clip_id.to_string(), CLIP_ID);
    assert_eq!(data.mask.expect("mask").kind, MaskKind::Rect);
    assert_eq!(
        store.project().tracks[0].clips[0]
            .mask
            .as_ref()
            .expect("mask")
            .kind,
        MaskKind::Rect
    );
    assert!(warnings.is_empty());
}
