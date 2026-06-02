//! Tests for `asset.import` (§3.1) — eighty-fourth production verb.
//!
//! The pure verb surface remains a v1 floor (no root filesystem
//! context), while the native kernel route wires non-empty imports
//! through CAS. These tests lock both surfaces, plus reconstructor
//! behavior through default fixtures.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::asset_import::{compute_patch, data_envelope_from_args, import};
use verbreel_state::{
    ASSET_IMPORT_SCHEMA_VIOLATION_HINT, AssetImportArgs, AssetImportData, AssetImportError,
    AssetImportVerb, ImportMode, PATHS_MAX_BATCH, Project, Verb, VerbError, VerbRegistry,
    default_fixtures, default_registry, validate_reconstructors,
};

#[cfg(feature = "native")]
use verbreel_state::MutateOutcome;
#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture → Project")
}

fn fixture_project_id() -> verbreel_types::ProjectId {
    empty_project().id
}

fn empty_paths_args() -> AssetImportArgs {
    AssetImportArgs {
        project_id: fixture_project_id(),
        paths: Vec::new(),
        mode: None,
        soft: None,
    }
}

fn single_path_args(path: &str) -> AssetImportArgs {
    AssetImportArgs {
        project_id: fixture_project_id(),
        paths: vec![path.to_string()],
        mode: None,
        soft: None,
    }
}

// --- args deserialization ---------------------------------------------------

#[test]
fn args_deserialize_happy_path() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "paths": ["/tmp/a.mp4", "/tmp/b.mp4"],
        "mode": "copy",
        "soft": true,
    });
    let typed: AssetImportArgs = serde_json::from_value(raw).expect("well-formed args deserialize");
    assert_eq!(typed.project_id.to_string(), FIXTURE_PROJECT_ID);
    assert_eq!(typed.paths.len(), 2);
    assert_eq!(typed.mode, Some(ImportMode::Copy));
    assert_eq!(typed.soft, Some(true));
}

#[test]
fn args_reject_unknown_fields() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "paths": [],
        "totally_made_up_field": true,
    });
    let err = serde_json::from_value::<AssetImportArgs>(raw)
        .expect_err("deny_unknown_fields must reject extras");
    assert!(
        err.to_string().contains("totally_made_up_field"),
        "error should name the offending field, got: {err}"
    );
}

#[test]
fn args_accept_missing_optional_mode_and_soft() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "paths": ["/tmp/a.mp4"],
    });
    let typed: AssetImportArgs =
        serde_json::from_value(raw).expect("mode/soft optional and may be omitted");
    assert!(typed.mode.is_none());
    assert!(typed.soft.is_none());
}

#[test]
fn args_mode_link_round_trips() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "paths": [],
        "mode": "link",
    });
    let typed: AssetImportArgs =
        serde_json::from_value(raw).expect("mode `link` should deserialize");
    assert_eq!(typed.mode, Some(ImportMode::Link));
    let back = serde_json::to_value(&typed).expect("serialize");
    assert_eq!(back.get("mode").and_then(Value::as_str), Some("link"));
}

// --- empty-paths → spec-documented no-op ------------------------------------

#[test]
fn empty_paths_returns_empty_assets() {
    let data = import(&empty_paths_args()).expect("empty paths is a successful no-op (§3.1)");
    assert!(data.assets.is_empty());
}

#[test]
fn empty_paths_returns_empty_modes_used() {
    let data = import(&empty_paths_args()).expect("empty paths is a successful no-op");
    assert!(data.modes_used.is_empty());
}

#[test]
fn empty_paths_returns_empty_missing_paths() {
    let data = import(&empty_paths_args()).expect("empty paths is a successful no-op");
    assert!(data.missing_paths.is_empty());
}

#[test]
fn empty_paths_returns_empty_skipped_input_indices() {
    let data = import(&empty_paths_args()).expect("empty paths is a successful no-op");
    assert!(data.skipped_input_indices.is_empty());
}

// --- >1000 paths → E_SCHEMA_VIOLATION ---------------------------------------

#[test]
fn paths_over_cap_fires_schema_violation() {
    let mut args = empty_paths_args();
    args.paths = (0..=PATHS_MAX_BATCH).map(|i| format!("/p{i}")).collect();
    assert_eq!(args.paths.len(), PATHS_MAX_BATCH + 1);
    let err = import(&args).expect_err("`maxItems: 1000` cap should fire");
    assert!(
        matches!(err, AssetImportError::SchemaViolation { .. }),
        "expected SchemaViolation, got {err:?}"
    );
}

#[test]
fn schema_violation_names_paths_field_and_hint() {
    let mut args = empty_paths_args();
    args.paths = vec!["/x".to_string(); PATHS_MAX_BATCH + 1];
    let err = import(&args).expect_err("cap fires");
    match err {
        AssetImportError::SchemaViolation { field, hint, .. } => {
            assert_eq!(field, "paths");
            assert_eq!(hint, ASSET_IMPORT_SCHEMA_VIOLATION_HINT);
        }
        other => panic!("expected SchemaViolation, got {other:?}"),
    }
}

#[test]
fn schema_violation_carries_actual_and_max_counts() {
    let mut args = empty_paths_args();
    args.paths = vec!["/x".to_string(); PATHS_MAX_BATCH + 1];
    let err = import(&args).expect_err("cap fires");
    match err {
        AssetImportError::SchemaViolation {
            actual_count,
            max_count,
            ..
        } => {
            assert_eq!(actual_count, PATHS_MAX_BATCH + 1);
            assert_eq!(max_count, PATHS_MAX_BATCH);
        }
        other => panic!("expected SchemaViolation, got {other:?}"),
    }
}

#[test]
fn paths_exactly_at_cap_is_not_a_schema_violation() {
    // At the cap, the cap itself is satisfied (>, not >=). The next
    // rejection step is the v1 floor's AssetPathNotFound.
    let mut args = empty_paths_args();
    args.paths = vec!["/x".to_string(); PATHS_MAX_BATCH];
    let err = import(&args).expect_err("v1 floor: non-empty → AssetPathNotFound");
    assert!(
        matches!(err, AssetImportError::AssetPathNotFound { .. }),
        "expected AssetPathNotFound at the cap, got {err:?}"
    );
}

// --- >=1 path → v1 floor E_ASSET_PATH_NOT_FOUND ----------------------------

#[test]
fn single_path_returns_asset_path_not_found() {
    let args = single_path_args("/tmp/does-not-exist.mp4");
    let err = import(&args).expect_err("v1 floor: non-empty → AssetPathNotFound");
    assert!(matches!(err, AssetImportError::AssetPathNotFound { .. }));
}

#[test]
fn multiple_paths_report_first_path_in_error() {
    let mut args = empty_paths_args();
    args.paths = vec![
        "/tmp/first.mp4".to_string(),
        "/tmp/second.mp4".to_string(),
        "/tmp/third.mp4".to_string(),
    ];
    let err = import(&args).expect_err("v1 floor: non-empty → AssetPathNotFound");
    match err {
        AssetImportError::AssetPathNotFound { path } => {
            assert_eq!(path, "/tmp/first.mp4", "must report the first path");
        }
        other => panic!("expected AssetPathNotFound, got {other:?}"),
    }
}

#[test]
fn asset_path_not_found_carries_input_path_verbatim() {
    let args = single_path_args("/some/weird path with spaces.mp4");
    let err = import(&args).expect_err("v1 floor: non-empty → AssetPathNotFound");
    match err {
        AssetImportError::AssetPathNotFound { path } => {
            assert_eq!(path, "/some/weird path with spaces.mp4");
        }
        other => panic!("expected AssetPathNotFound, got {other:?}"),
    }
}

// --- mode / soft defaults ---------------------------------------------------

#[test]
fn mode_omitted_round_trips_as_none() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "paths": [],
    });
    let typed: AssetImportArgs = serde_json::from_value(raw).expect("omitted mode is allowed");
    assert!(typed.mode.is_none());
}

#[test]
fn soft_omitted_round_trips_as_none() {
    let raw = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "paths": [],
    });
    let typed: AssetImportArgs = serde_json::from_value(raw).expect("omitted soft is allowed");
    assert!(typed.soft.is_none());
}

// --- verb-trait surface -----------------------------------------------------

#[test]
fn verb_trait_compute_patch_empty_paths_returns_empty_patch_and_envelope() {
    let prior = empty_project();
    let verb = AssetImportVerb;
    let args = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "paths": [],
    });
    let (patch, data, warnings) = verb
        .compute_patch(&prior, &args)
        .expect("empty paths → spec-documented no-op success");
    assert!(warnings.is_empty());
    let patch_value = serde_json::to_value(&patch).expect("patch → value");
    assert_eq!(patch_value, json!([]));
    let parsed: AssetImportData = serde_json::from_value(data).expect("data deserializes");
    assert!(parsed.assets.is_empty());
    assert!(parsed.modes_used.is_empty());
    assert!(parsed.missing_paths.is_empty());
    assert!(parsed.skipped_input_indices.is_empty());
}

#[test]
fn verb_trait_args_missing_project_id_is_bad_args() {
    let prior = empty_project();
    let verb = AssetImportVerb;
    let err = verb
        .compute_patch(&prior, &json!({"paths": []}))
        .expect_err("missing project_id should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn verb_trait_schema_violation_is_bad_args() {
    let prior = empty_project();
    let verb = AssetImportVerb;
    let paths: Vec<String> = (0..=PATHS_MAX_BATCH).map(|i| format!("/p{i}")).collect();
    let err = verb
        .compute_patch(
            &prior,
            &json!({"project_id": FIXTURE_PROJECT_ID, "paths": paths}),
        )
        .expect_err("schema violation maps to BadArgs (arg-shape rejection)");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn verb_trait_asset_path_not_found_is_custom() {
    let prior = empty_project();
    let verb = AssetImportVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({"project_id": FIXTURE_PROJECT_ID, "paths": ["/x"]}),
        )
        .expect_err("AssetPathNotFound maps to Custom (runtime-class rejection)");
    assert!(matches!(err, VerbError::Custom(_)));
}

#[test]
fn verb_registered_in_default_registry() {
    let registry = default_registry();
    let verb = registry
        .get("asset.import")
        .expect("default_registry exposes asset.import");
    assert_eq!(verb.verb(), "asset.import");
}

// --- native route ----------------------------------------------------------

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb_for_empty_paths() {
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
            "asset.import",
            json!({"project_id": FIXTURE_PROJECT_ID, "paths": []}),
            None,
        )
        .expect("asset.import should route");

    // asset.import over an empty `paths` array imports nothing — empty
    // patch → NoOp, no event line (§0.6/§0.8 no-op fast-path).
    let MutateOutcome::NoOp { data, warnings, .. } = outcome else {
        panic!("expected NoOp outcome for empty-paths asset.import, got {outcome:?}");
    };
    assert!(warnings.is_empty());

    let data: AssetImportData =
        serde_json::from_value(data).expect("asset.import data deserializes");
    assert!(data.assets.is_empty());
    assert!(data.modes_used.is_empty());
    assert!(data.missing_paths.is_empty());
    assert!(data.skipped_input_indices.is_empty());
}

#[cfg(feature = "native")]
#[test]
fn mutate_via_verb_non_empty_import_writes_cas_and_records_canonical_hash_path() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let source = dir.path().join("sample.SRT");
    std::fs::write(&source, b"1\n00:00:00,000 --> 00:00:00,500\nhi\n").expect("write source file");

    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        empty_project(),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry succeeds");

    let outcome = store
        .mutate_via_verb(
            "asset.import",
            json!({"project_id": FIXTURE_PROJECT_ID, "paths": [source.to_string_lossy()]}),
            None,
        )
        .expect("asset.import non-empty should succeed on native route");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("expected Applied outcome for non-empty asset.import");
    };
    assert!(warnings.is_empty());

    let data: AssetImportData =
        serde_json::from_value(data).expect("asset.import data deserializes");
    assert_eq!(data.assets.len(), 1);
    assert_eq!(data.modes_used.len(), 1);
    assert!(data.missing_paths.is_empty());
    assert!(data.skipped_input_indices.is_empty());

    let imported = data.assets[0]
        .as_object()
        .expect("data.assets[0] is an object");
    let hash = imported
        .get("hash")
        .and_then(Value::as_str)
        .expect("imported asset has hash");
    let path = imported
        .get("path")
        .and_then(Value::as_str)
        .expect("imported asset has path");
    assert_eq!(hash.len(), 64);
    assert_eq!(path, format!("assets/{}/{}.srt", &hash[..2], hash));

    let cas_target = dir.path().join(path);
    assert!(
        cas_target.exists(),
        "CAS target must exist: {}",
        cas_target.display()
    );
}

#[cfg(feature = "native")]
#[test]
fn mutate_via_verb_ppm_import_records_image_asset() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let source = dir.path().join("frame.PPM");
    std::fs::write(
        &source,
        b"P6\n# generated fixture\n2 1\n255\n\x00\x00\x00\xff\xff\xff",
    )
    .expect("write source file");

    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        empty_project(),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry succeeds");

    let outcome = store
        .mutate_via_verb(
            "asset.import",
            json!({"project_id": FIXTURE_PROJECT_ID, "paths": [source.to_string_lossy()]}),
            None,
        )
        .expect("asset.import non-empty should succeed on native route");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("expected Applied outcome for non-empty asset.import");
    };
    assert!(warnings.is_empty());

    let data: AssetImportData =
        serde_json::from_value(data).expect("asset.import data deserializes");
    let imported = data.assets[0]
        .as_object()
        .expect("data.assets[0] is an object");
    assert_eq!(imported.get("kind").and_then(Value::as_str), Some("image"));
    assert_eq!(
        imported
            .get("metadata")
            .and_then(|v| v.get("width"))
            .and_then(Value::as_u64),
        Some(2)
    );
    assert_eq!(
        imported
            .get("metadata")
            .and_then(|v| v.get("height"))
            .and_then(Value::as_u64),
        Some(1)
    );
    let hash = imported
        .get("hash")
        .and_then(Value::as_str)
        .expect("imported asset has hash");
    let path = imported
        .get("path")
        .and_then(Value::as_str)
        .expect("imported asset has path");
    assert_eq!(path, format!("assets/{}/{}.ppm", &hash[..2], hash));
}

#[cfg(feature = "native")]
#[test]
fn mutate_via_verb_truncated_ppm_does_not_record_image_asset() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let source = dir.path().join("truncated.ppm");
    std::fs::write(&source, b"P6\n2 1\n255\n\x00\x00\x00").expect("write source file");

    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        empty_project(),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry succeeds");

    let outcome = store
        .mutate_via_verb(
            "asset.import",
            json!({"project_id": FIXTURE_PROJECT_ID, "paths": [source.to_string_lossy()]}),
            None,
        )
        .expect("asset.import non-empty should succeed on native route");

    let MutateOutcome::Applied { data, warnings, .. } = outcome else {
        panic!("expected Applied outcome for non-empty asset.import");
    };
    assert!(warnings.is_empty());

    let data: AssetImportData =
        serde_json::from_value(data).expect("asset.import data deserializes");
    let imported = data.assets[0]
        .as_object()
        .expect("data.assets[0] is an object");
    assert_eq!(
        imported.get("kind").and_then(Value::as_str),
        Some("subtitle")
    );
}

#[cfg(feature = "native")]
#[test]
fn mutate_via_verb_rejects_corrupt_existing_cas_object() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let source = dir.path().join("sample.srt");
    std::fs::write(&source, b"1\n00:00:00,000 --> 00:00:00,500\nhi\n").expect("write source file");

    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        empty_project(),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry succeeds");

    let args = json!({"project_id": FIXTURE_PROJECT_ID, "paths": [source.to_string_lossy()]});
    let outcome = store
        .mutate_via_verb("asset.import", args.clone(), None)
        .expect("initial import writes CAS object");
    let MutateOutcome::Applied { data, .. } = outcome else {
        panic!("expected Applied outcome for initial asset.import");
    };
    let data: AssetImportData =
        serde_json::from_value(data).expect("asset.import data deserializes");
    let path = data.assets[0]
        .get("path")
        .and_then(Value::as_str)
        .expect("imported asset has path");
    std::fs::write(dir.path().join(path), b"corrupt bytes").expect("corrupt CAS object");

    let err = store
        .mutate_via_verb("asset.import", args, None)
        .expect_err("corrupt CAS object must be rejected");
    assert!(
        err.to_string()
            .contains("existing CAS object contents do not match expected hash"),
        "unexpected error: {err}"
    );
}

// --- media-kind classification (§3.1) ---------------------------------------

/// Build a minimal but structurally valid ISO-BMFF box: `[u32 size][type
/// (4 bytes)][body]`, big-endian size covering header + body.
#[cfg(feature = "native")]
fn iso_box(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let size = u32::try_from(8 + body.len()).expect("box fits u32");
    let mut out = size.to_be_bytes().to_vec();
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    out
}

/// A minimal single-video-track ISO-BMFF (`.mp4`) fixture with real,
/// parseable `width`/`height`/`timescale`/`duration`/`sample_count`.
/// `avc1` (h264), 640x360, timescale 24000, duration 48000 (= 2.0s),
/// sample_count 48 → fps 48*24000/48000 = 24000/48000 → 24fps.
#[cfg(feature = "native")]
fn minimal_mp4(width: u16, height: u16, timescale: u32, duration: u32, samples: u32) -> Vec<u8> {
    // ftyp box: major brand `isom`, minor 0, compatible `isom`.
    let ftyp = iso_box(b"ftyp", b"isom\x00\x00\x00\x00isom");

    // hdlr: version/flags(4) + pre_defined(4) + handler_type "vide" + 12
    // reserved + 1 null name byte.
    let mut hdlr_body = vec![0u8; 8];
    hdlr_body.extend_from_slice(b"vide");
    hdlr_body.extend_from_slice(&[0u8; 12]);
    hdlr_body.push(0);
    let hdlr = iso_box(b"hdlr", &hdlr_body);

    // mdhd v0: version/flags(4) + creation(4) + modification(4) +
    // timescale(4) + duration(4) + language(2) + pre_defined(2).
    let mut mdhd_body = vec![0u8; 4];
    mdhd_body.extend_from_slice(&0u32.to_be_bytes());
    mdhd_body.extend_from_slice(&0u32.to_be_bytes());
    mdhd_body.extend_from_slice(&timescale.to_be_bytes());
    mdhd_body.extend_from_slice(&duration.to_be_bytes());
    mdhd_body.extend_from_slice(&[0u8; 4]);
    let mdhd = iso_box(b"mdhd", &mdhd_body);

    // VisualSampleEntry body: 6 reserved + 2 data_ref_index + 16
    // pre_defined/reserved + width(2) + height(2), padded so width/height
    // sit at body offsets 24/26 → entry offsets 32/34.
    let mut avc1_body = vec![0u8; 6 + 2 + 16];
    avc1_body.extend_from_slice(&width.to_be_bytes());
    avc1_body.extend_from_slice(&height.to_be_bytes());
    let avc1 = iso_box(b"avc1", &avc1_body);
    // stsd: version/flags(4) + entry_count(4) + sample entry.
    let mut stsd_body = vec![0u8; 4];
    stsd_body.extend_from_slice(&1u32.to_be_bytes());
    stsd_body.extend_from_slice(&avc1);
    let stsd = iso_box(b"stsd", &stsd_body);

    // stsz: version/flags(4) + sample_size(4, 0 = variable) + count(4).
    let mut stsz_body = vec![0u8; 4];
    stsz_body.extend_from_slice(&0u32.to_be_bytes());
    stsz_body.extend_from_slice(&samples.to_be_bytes());
    let stsz = iso_box(b"stsz", &stsz_body);

    let mut stbl_body = Vec::new();
    stbl_body.extend_from_slice(&stsd);
    stbl_body.extend_from_slice(&stsz);
    let stbl = iso_box(b"stbl", &stbl_body);
    let minf = iso_box(b"minf", &stbl);

    let mut mdia_body = Vec::new();
    mdia_body.extend_from_slice(&hdlr);
    mdia_body.extend_from_slice(&mdhd);
    mdia_body.extend_from_slice(&minf);
    let mdia = iso_box(b"mdia", &mdia_body);
    let trak = iso_box(b"trak", &mdia);
    let moov = iso_box(b"moov", &trak);

    let mut out = ftyp;
    out.extend_from_slice(&moov);
    out
}

/// Build an ISO-BMFF box using the 64-bit extended size form (`size ==
/// 1`, real size in the 8 bytes after the type). Large `mdat` boxes on
/// phone captures routinely use this encoding, so the sibling walk must
/// skip it rather than abort.
#[cfg(feature = "native")]
fn iso_box_64bit(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let total = 16u64 + u64::try_from(body.len()).expect("body fits u64");
    let mut out = 1u32.to_be_bytes().to_vec(); // size == 1 → 64-bit form
    out.extend_from_slice(kind);
    out.extend_from_slice(&total.to_be_bytes()); // 64-bit largesize
    out.extend_from_slice(body);
    out
}

/// Same payload as [`minimal_mp4`] but with a 64-bit-sized `mdat` box
/// inserted between `ftyp` and `moov` (the real-world layout for large
/// captures). The walk must skip the extended-size `mdat` and still
/// reach `moov`.
#[cfg(feature = "native")]
fn mp4_with_leading_64bit_mdat(
    width: u16,
    height: u16,
    timescale: u32,
    duration: u32,
    samples: u32,
) -> Vec<u8> {
    let full = minimal_mp4(width, height, timescale, duration, samples);
    // Split the canonical fixture at the `moov` box (right after `ftyp`).
    let moov_pos = full
        .windows(4)
        .position(|w| w == b"moov")
        .expect("fixture contains a moov box");
    let ftyp_end = moov_pos - 4; // moov size field precedes its type.
    let mdat = iso_box_64bit(b"mdat", &[0u8; 32]);

    let mut out = full[..ftyp_end].to_vec();
    out.extend_from_slice(&mdat);
    out.extend_from_slice(&full[ftyp_end..]);
    out
}

/// A minimal canonical RIFF/WAVE fixture: PCM, `channels` channels,
/// `sample_rate` Hz, 16-bit, with `frames` sample frames of zeroed data.
#[cfg(feature = "native")]
fn minimal_wav(channels: u16, sample_rate: u32, frames: u32) -> Vec<u8> {
    let bits = 16u16;
    let block_align = channels * (bits / 8);
    let data_len = frames * u32::from(block_align);

    let mut fmt = Vec::new();
    fmt.extend_from_slice(&1u16.to_le_bytes()); // PCM
    fmt.extend_from_slice(&channels.to_le_bytes());
    fmt.extend_from_slice(&sample_rate.to_le_bytes());
    fmt.extend_from_slice(&(sample_rate * u32::from(block_align)).to_le_bytes());
    fmt.extend_from_slice(&block_align.to_le_bytes());
    fmt.extend_from_slice(&bits.to_le_bytes());

    let mut body = Vec::new();
    body.extend_from_slice(b"WAVE");
    body.extend_from_slice(b"fmt ");
    body.extend_from_slice(&u32::try_from(fmt.len()).unwrap().to_le_bytes());
    body.extend_from_slice(&fmt);
    body.extend_from_slice(b"data");
    body.extend_from_slice(&data_len.to_le_bytes());
    body.extend(std::iter::repeat_n(0u8, data_len as usize));

    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&u32::try_from(body.len()).unwrap().to_le_bytes());
    out.extend_from_slice(&body);
    out
}

#[cfg(feature = "native")]
fn import_single(dir: &std::path::Path, filename: &str, bytes: &[u8]) -> Value {
    let source = dir.join(filename);
    std::fs::write(&source, bytes).expect("write source file");

    let mut store = ProjectStore::create_with_registry(
        dir,
        empty_project(),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry succeeds");

    let outcome = store
        .mutate_via_verb(
            "asset.import",
            json!({"project_id": FIXTURE_PROJECT_ID, "paths": [source.to_string_lossy()]}),
            None,
        )
        .expect("asset.import non-empty should succeed on native route");

    let MutateOutcome::Applied { data, .. } = outcome else {
        panic!("expected Applied outcome for non-empty asset.import");
    };
    let data: AssetImportData =
        serde_json::from_value(data).expect("asset.import data deserializes");
    data.assets.into_iter().next().expect("one imported asset")
}

#[cfg(feature = "native")]
#[test]
fn mutate_via_verb_mp4_import_records_video_asset_not_subtitle() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    // 640x360, timescale 24000, duration 48000 (2.0s), 48 samples (24fps).
    let mp4 = minimal_mp4(640, 360, 24_000, 48_000, 48);
    let imported = import_single(dir.path(), "clip.mp4", &mp4);
    let obj = imported.as_object().expect("asset object");

    // The regression: an imported mp4 must be `video`, NOT `subtitle`.
    assert_eq!(
        obj.get("kind").and_then(Value::as_str),
        Some("video"),
        "imported mp4 must classify as video, not subtitle"
    );
    let meta = obj.get("metadata").expect("metadata");
    assert_eq!(meta.get("width").and_then(Value::as_u64), Some(640));
    assert_eq!(meta.get("height").and_then(Value::as_u64), Some(360));
    assert_eq!(
        meta.get("video_codec").and_then(Value::as_str),
        Some("h264")
    );
    // duration_tk = 48000 * 240000 / 24000 = 480000.
    assert_eq!(
        meta.get("duration_tk").and_then(Value::as_i64),
        Some(480_000)
    );
    // fps = samples*timescale / duration = 48*24000 / 48000 = 24/1,
    // gcd-reduced (the stored rational is the reduced form, which both
    // avoids u32 overflow on long/high-timescale media and reads sanely).
    assert_eq!(meta.get("fps_num").and_then(Value::as_u64), Some(24));
    assert_eq!(meta.get("fps_den").and_then(Value::as_u64), Some(1));
    // Every schema-`minimum: 1` field is >= 1 — no fabricated placeholder.
    for field in ["width", "height", "fps_num", "fps_den", "duration_tk"] {
        assert!(
            meta.get(field).and_then(Value::as_i64).unwrap_or(0) >= 1,
            "video metadata `{field}` must satisfy schema minimum: 1"
        );
    }
}

#[cfg(feature = "native")]
#[test]
fn mutate_via_verb_long_high_timescale_mp4_still_classifies_as_video() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    // ~2-hour clip at timescale 90000 (a very common container timescale)
    // at 30fps: samples = 30 * 7200 = 216000, duration = 7200 * 90000 =
    // 648_000_000. The UNREDUCED fps product samples*timescale =
    // 216000 * 90000 = 1.944e10 overflows u32, and duration also overflows
    // u32 — before the gcd-reduce + infallible-degrade fix, both
    // `u32::try_from(..).ok()?` failed, `probe_video` returned None, and
    // the file fell through to the subtitle fallback (re-triggering
    // E_ASSET_KIND_UNROUTABLE on clip.add). fps must reduce to 30/1.
    let samples = 30u32 * 7200;
    let duration = 7200u32 * 90000; // 648_000_000, fits u32 here on purpose
    let mp4 = minimal_mp4(1920, 1080, 90_000, duration, samples);
    let imported = import_single(dir.path(), "long.mp4", &mp4);
    let obj = imported.as_object().expect("asset object");

    assert_eq!(
        obj.get("kind").and_then(Value::as_str),
        Some("video"),
        "a long high-timescale mp4 must still classify as video, not subtitle"
    );
    let meta = obj.get("metadata").expect("metadata");
    // 216000*90000 / 648000000 = 30/1, gcd-reduced (unreduced numerator
    // 1.944e10 > u32::MAX, so this only fits after reduction).
    assert_eq!(meta.get("fps_num").and_then(Value::as_u64), Some(30));
    assert_eq!(meta.get("fps_den").and_then(Value::as_u64), Some(1));
}

#[cfg(feature = "native")]
#[test]
fn mutate_via_verb_mp4_with_64bit_mdat_before_moov_still_classifies_as_video() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    // Real-world layout: a 64-bit-sized (`size == 1`) `mdat` precedes
    // `moov`. Before the find_box skip-don't-abort fix, the extended-size
    // box aborted the top-level walk, `probe_video` returned None, and the
    // file fell through to the subtitle fallback — the exact bug this PR
    // exists to kill, reintroduced for a common input class.
    let mp4 = mp4_with_leading_64bit_mdat(640, 360, 24_000, 48_000, 48);
    let imported = import_single(dir.path(), "phone.mp4", &mp4);
    let obj = imported.as_object().expect("asset object");

    assert_eq!(
        obj.get("kind").and_then(Value::as_str),
        Some("video"),
        "mp4 with a 64-bit-sized mdat before moov must still classify as video"
    );
    let meta = obj.get("metadata").expect("metadata");
    assert_eq!(meta.get("width").and_then(Value::as_u64), Some(640));
    assert_eq!(meta.get("height").and_then(Value::as_u64), Some(360));
    assert_eq!(
        meta.get("video_codec").and_then(Value::as_str),
        Some("h264")
    );
}

#[cfg(feature = "native")]
#[test]
fn mutate_via_verb_wav_import_records_audio_asset() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    // 2 channels, 48000 Hz, 96000 frames = 2.0s.
    let wav = minimal_wav(2, 48_000, 96_000);
    let imported = import_single(dir.path(), "track.wav", &wav);
    let obj = imported.as_object().expect("asset object");

    assert_eq!(obj.get("kind").and_then(Value::as_str), Some("audio"));
    let meta = obj.get("metadata").expect("metadata");
    assert_eq!(meta.get("audio_channels").and_then(Value::as_u64), Some(2));
    assert_eq!(
        meta.get("audio_sample_rate_hz").and_then(Value::as_u64),
        Some(48_000)
    );
    // duration_tk = 96000 * 240000 / 48000 = 480000.
    assert_eq!(
        meta.get("duration_tk").and_then(Value::as_i64),
        Some(480_000)
    );
    for field in ["duration_tk", "audio_channels", "audio_sample_rate_hz"] {
        assert!(
            meta.get(field).and_then(Value::as_i64).unwrap_or(0) >= 1,
            "audio metadata `{field}` must satisfy schema minimum: 1"
        );
    }
}

#[cfg(feature = "native")]
#[test]
fn mutate_via_verb_png_import_records_image_asset() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    // PNG signature + IHDR length(13) + "IHDR" + width(4)=16 + height(4)=9.
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend_from_slice(&13u32.to_be_bytes());
    png.extend_from_slice(b"IHDR");
    png.extend_from_slice(&16u32.to_be_bytes());
    png.extend_from_slice(&9u32.to_be_bytes());
    png.extend_from_slice(&[8, 6, 0, 0, 0]); // bit depth, color type, etc.

    let imported = import_single(dir.path(), "frame.png", &png);
    let obj = imported.as_object().expect("asset object");
    assert_eq!(obj.get("kind").and_then(Value::as_str), Some("image"));
    let meta = obj.get("metadata").expect("metadata");
    assert_eq!(meta.get("width").and_then(Value::as_u64), Some(16));
    assert_eq!(meta.get("height").and_then(Value::as_u64), Some(9));
}

#[cfg(feature = "native")]
#[test]
fn mutate_via_verb_gif_import_records_image_asset() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    // GIF89a header + logical screen descriptor: LE width(2)=12, height(2)=7,
    // then packed/bg/aspect bytes. gif_dimensions is a production path
    // (build_asset_for_import → image_dimensions → ...or_else(gif_dimensions))
    // that previously had no test fixture.
    let mut gif = b"GIF89a".to_vec();
    gif.extend_from_slice(&12u16.to_le_bytes());
    gif.extend_from_slice(&7u16.to_le_bytes());
    gif.extend_from_slice(&[0u8, 0, 0]); // packed fields, bg color, aspect

    let imported = import_single(dir.path(), "anim.gif", &gif);
    let obj = imported.as_object().expect("asset object");
    assert_eq!(obj.get("kind").and_then(Value::as_str), Some("image"));
    let meta = obj.get("metadata").expect("metadata");
    assert_eq!(meta.get("width").and_then(Value::as_u64), Some(12));
    assert_eq!(meta.get("height").and_then(Value::as_u64), Some(7));
}

#[cfg(feature = "native")]
#[test]
fn mutate_via_verb_srt_import_still_records_subtitle_asset() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let srt = b"1\n00:00:00,000 --> 00:00:01,000\nhello\n";
    let imported = import_single(dir.path(), "subs.srt", srt);
    let obj = imported.as_object().expect("asset object");
    // §3.1: subtitle imports remain SubtitleAsset (archival-only).
    assert_eq!(obj.get("kind").and_then(Value::as_str), Some("subtitle"));
}

#[cfg(feature = "native")]
#[test]
fn mutate_via_verb_garbage_takes_explicit_subtitle_fallback() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    // Unrecognized bytes with an `.mp4` extension: an extension is never
    // trusted on its own (the `ftyp` magic is absent), so this is NOT a
    // video — it degrades to the documented archival subtitle fallback,
    // proving the catch-all is not silently broadened to video.
    let garbage = b"this is not any known container format at all";
    let imported = import_single(dir.path(), "fake.mp4", garbage);
    let obj = imported.as_object().expect("asset object");
    assert_eq!(
        obj.get("kind").and_then(Value::as_str),
        Some("subtitle"),
        "unrecognized bytes must take the explicit archival fallback, not become a fake video"
    );
}

// --- data shape lock --------------------------------------------------------

#[test]
fn empty_envelope_serializes_to_four_keys() {
    let data = import(&empty_paths_args()).expect("empty paths");
    let value = serde_json::to_value(&data).expect("envelope serializes");
    let obj = value.as_object().expect("envelope is an object");
    assert_eq!(obj.len(), 4, "envelope must have exactly four keys");
    for key in [
        "assets",
        "modes_used",
        "missing_paths",
        "skipped_input_indices",
    ] {
        assert!(obj.contains_key(key), "envelope must serialize `{key}`");
    }
}

#[test]
fn empty_envelope_all_values_are_empty_arrays() {
    let data = import(&empty_paths_args()).expect("empty paths");
    let value = serde_json::to_value(&data).expect("envelope serializes");
    let obj = value.as_object().expect("envelope is an object");
    for key in [
        "assets",
        "modes_used",
        "missing_paths",
        "skipped_input_indices",
    ] {
        let arr = obj
            .get(key)
            .and_then(Value::as_array)
            .unwrap_or_else(|| panic!("`{key}` must be an array"));
        assert!(arr.is_empty(), "`{key}` must be empty at v1");
    }
}

#[test]
fn envelope_round_trip_byte_identical() {
    let data = import(&empty_paths_args()).expect("empty paths");
    let serialized = serde_json::to_value(&data).expect("serialize");
    let back: AssetImportData = serde_json::from_value(serialized).expect("deserialize");
    assert_eq!(data, back);
}

// --- reconstructor / fixture ------------------------------------------------

#[test]
fn reconstructor_round_trip_byte_identical() {
    let args = empty_paths_args();
    let prior = empty_project();
    let (patch_value, _, expected) =
        compute_patch(&prior, &args).expect("compute_patch succeeds for empty paths");
    let patch: json_patch::Patch =
        serde_json::from_value(patch_value).expect("patch is valid RFC 6902");
    let post_state = prior
        .apply(&patch)
        .expect("empty patch applies to empty project");

    let envelope = data_envelope_from_args(&args, &post_state)
        .expect("data_envelope_from_args should rebuild same data");
    assert_eq!(envelope, expected);
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "asset.import")
        .expect("default_fixtures includes asset.import");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(AssetImportVerb))
        .expect("register asset.import verb");

    let report = validate_reconstructors(&registry, &[fixture])
        .expect("asset.import reconstruct from fixture");
    assert_eq!(report.verbs_checked, vec!["asset.import"]);
    assert_eq!(report.fixtures_run, 1);
}
