//! `asset.import` (§3.1) — content-addressed import surface.
//!
//! The pure verb path (`compute_patch`) stays as a v1 floor because it
//! has no filesystem context. The native kernel path
//! (`ProjectStore::mutate_via_verb`) calls
//! [`compute_patch_with_root`] to wire real CAS writes.

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::ProjectId;

#[cfg(feature = "native")]
use crate::asset::{Asset, AudioAsset, ImageAsset, SubtitleAsset, VideoAsset};
#[cfg(feature = "native")]
use crate::asset_meta::{
    AudioAssetMetadata, FileFingerprint, ImageAssetMetadata, SubtitleAssetMetadata,
    VideoAssetMetadata,
};
#[cfg(feature = "native")]
use crate::newtypes::{AssetPath, Sha256};
use std::collections::HashMap;
#[cfg(feature = "native")]
use std::fs;
#[cfg(feature = "native")]
use std::path::Path;
#[cfg(feature = "native")]
use std::time::UNIX_EPOCH;
#[cfg(feature = "native")]
use verbreel_events::Timestamp;
#[cfg(feature = "native")]
use verbreel_storage::cas::key_for_bytes;
#[cfg(feature = "native")]
use verbreel_storage::fs::atomic_write_bytes;
#[cfg(feature = "native")]
use verbreel_types::AssetId;
#[cfg(feature = "native")]
use verbreel_types::Tick;

/// Whether a verb's compute path may fire persistent side effects.
///
/// The §0.8 mutate path uses [`SideEffects::Persist`] (the CAS write
/// happens). The §0.5.1 `dry_run` compute-only path uses
/// [`SideEffects::ComputeOnly`] — the same patch + hash is computed but
/// no bytes are written to `assets/`, so a dry run leaves no orphaned
/// CAS object on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SideEffects {
    /// Real mutate path: write source bytes to the content-addressed
    /// store.
    Persist,
    /// `dry_run` path (§0.5.1): compute the hash + patch but skip every
    /// persistent write.
    ComputeOnly,
}

/// Per-verb upper bound on the `paths` array.
pub const PATHS_MAX_BATCH: usize = 1000;

/// `E_SCHEMA_VIOLATION` hint when `paths.len() > PATHS_MAX_BATCH`.
pub const SCHEMA_VIOLATION_HINT: &str = "split the batch into smaller calls";

/// Asset import storage mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImportMode {
    /// Byte-copy source into project CAS.
    Copy,
    /// Hard-link request (currently resolves as copy in this slice).
    Link,
}

/// Arguments for `asset.import`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetImportArgs {
    /// Target project id.
    pub project_id: ProjectId,
    /// Source paths to import.
    pub paths: Vec<String>,
    /// Requested storage mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<ImportMode>,
    /// Soft-skip toggle (reserved for fuller batch behavior).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub soft: Option<bool>,
}

/// Success envelope for `asset.import`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetImportData {
    /// Imported assets in input order.
    pub assets: Vec<Value>,
    /// Resolved mode evidence in input order.
    pub modes_used: Vec<Value>,
    /// Soft-skipped paths.
    pub missing_paths: Vec<String>,
    /// Soft-skipped input indices.
    pub skipped_input_indices: Vec<i64>,
}

/// Verb-level error type for `asset.import`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AssetImportError {
    /// `paths.len() > PATHS_MAX_BATCH`.
    #[error(
        "asset.import: schema violation: field `{field}` exceeds maxItems \
         (actual: {actual_count}, max: {max_count}); {hint}"
    )]
    SchemaViolation {
        /// Violating field name.
        field: &'static str,
        /// Recovery hint.
        hint: &'static str,
        /// Caller-supplied count.
        actual_count: usize,
        /// Limit count.
        max_count: usize,
    },
    /// Input path did not exist.
    #[error("asset.import: path not found: {path}")]
    AssetPathNotFound {
        /// Missing path.
        path: String,
    },
    /// Source read or destination write failed.
    #[error("asset.import: io failure on `{path}`: {detail}")]
    Io {
        /// Path involved in the failure.
        path: String,
        /// Underlying error text.
        detail: String,
    },
    /// Produced patch/data violated internal expectations.
    #[error("asset.import: inconsistent patch/data: {detail}")]
    InconsistentPatch {
        /// Internal mismatch detail.
        detail: String,
    },
}

/// Build the empty-batch success envelope.
fn empty_data() -> AssetImportData {
    AssetImportData {
        assets: Vec::new(),
        modes_used: Vec::new(),
        missing_paths: Vec::new(),
        skipped_input_indices: Vec::new(),
    }
}

/// Pure v1 floor for `asset.import`.
///
/// This helper keeps the legacy behavior for callsites that do not
/// provide project-root filesystem context.
///
/// # Errors
///
/// Returns [`AssetImportError::SchemaViolation`] when `paths` exceeds
/// the cap, and [`AssetImportError::AssetPathNotFound`] for non-empty
/// calls.
pub fn import(args: &AssetImportArgs) -> Result<AssetImportData, AssetImportError> {
    if args.paths.len() > PATHS_MAX_BATCH {
        return Err(AssetImportError::SchemaViolation {
            field: "paths",
            hint: SCHEMA_VIOLATION_HINT,
            actual_count: args.paths.len(),
            max_count: PATHS_MAX_BATCH,
        });
    }
    if args.paths.is_empty() {
        return Ok(empty_data());
    }
    Err(AssetImportError::AssetPathNotFound {
        path: args.paths[0].clone(),
    })
}

/// Build the RFC 6902 patch for `asset.import`.
///
/// Pure fallback path with no filesystem context.
///
/// # Errors
///
/// Forwards [`AssetImportError`] from [`import`].
pub fn compute_patch(
    _prior: &Project,
    args: &AssetImportArgs,
) -> Result<(Value, Vec<Value>, AssetImportData), AssetImportError> {
    let data = import(args)?;
    Ok((json!([]), Vec::new(), data))
}

#[cfg(feature = "native")]
fn canonical_ext(path: &Path) -> String {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .filter(|ext| {
            !ext.is_empty()
                && ext
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        })
        .unwrap_or_else(|| "bin".to_string())
}

#[cfg(feature = "native")]
fn fingerprint_for(path: &Path) -> Result<FileFingerprint, AssetImportError> {
    let md = fs::metadata(path).map_err(|e| AssetImportError::Io {
        path: path.display().to_string(),
        detail: e.to_string(),
    })?;
    let mtime_ms = md
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| {
            u64::try_from(d.as_millis().min(u128::from(u64::MAX)))
                .expect("value clamped to u64::MAX")
        });
    Ok(FileFingerprint {
        mtime_ms,
        size_bytes: md.len(),
    })
}

/// The shared fields every `Asset` variant carries: validated hash,
/// validated content-addressed path, original filename, import time, and
/// the file fingerprint. Built once per import so the four variant
/// builders don't each re-run the same newtype validation + filename
/// extraction (DRY: this boilerplate was duplicated per builder).
#[cfg(feature = "native")]
struct AssetCommon {
    id: AssetId,
    hash: Sha256,
    path: AssetPath,
    original_filename: String,
    imported_at: Timestamp,
    fingerprint: FileFingerprint,
}

#[cfg(feature = "native")]
fn build_common(
    source_path: &Path,
    sha256_hex: &str,
    cas_rel_path: &str,
) -> Result<AssetCommon, AssetImportError> {
    let hash =
        Sha256::new(sha256_hex.to_string()).map_err(|e| AssetImportError::InconsistentPatch {
            detail: e.to_string(),
        })?;
    let path = AssetPath::new(cas_rel_path.to_string()).map_err(|e| {
        AssetImportError::InconsistentPatch {
            detail: e.to_string(),
        }
    })?;
    let fingerprint = fingerprint_for(source_path)?;
    Ok(AssetCommon {
        id: AssetId::now(),
        hash,
        path,
        original_filename: source_path.file_name().map_or_else(
            || source_path.display().to_string(),
            |n| n.to_string_lossy().to_string(),
        ),
        // Placeholder epoch — real import time is wired in a follow-up.
        // Constructed through the typed boundary so even the placeholder
        // is validated RFC 3339, not a raw string literal.
        imported_at: Timestamp::parse("1970-01-01T00:00:00Z")
            .expect("epoch literal is valid RFC 3339"),
        fingerprint,
    })
}

#[cfg(feature = "native")]
fn build_subtitle_asset(
    source_path: &Path,
    sha256_hex: &str,
    cas_rel_path: &str,
    ext: &str,
) -> Result<Asset, AssetImportError> {
    let c = build_common(source_path, sha256_hex, cas_rel_path)?;
    Ok(Asset::Subtitle(SubtitleAsset {
        id: c.id,
        hash: c.hash,
        path: c.path,
        original_filename: c.original_filename,
        imported_at: c.imported_at,
        metadata: SubtitleAssetMetadata {
            container: ext.to_string(),
            language: None,
            segment_count: None,
            fingerprint: c.fingerprint,
        },
    }))
}

#[cfg(feature = "native")]
fn build_image_asset(
    source_path: &Path,
    sha256_hex: &str,
    cas_rel_path: &str,
    ext: &str,
    width: u32,
    height: u32,
) -> Result<Asset, AssetImportError> {
    let c = build_common(source_path, sha256_hex, cas_rel_path)?;
    Ok(Asset::Image(ImageAsset {
        id: c.id,
        hash: c.hash,
        path: c.path,
        original_filename: c.original_filename,
        imported_at: c.imported_at,
        metadata: ImageAssetMetadata {
            width,
            height,
            container: ext.to_string(),
            has_alpha: Some(false),
            color_space: Some("srgb".to_string()),
            rotation_deg: None,
            fingerprint: c.fingerprint,
        },
    }))
}

#[cfg(feature = "native")]
fn build_video_asset(
    source_path: &Path,
    sha256_hex: &str,
    cas_rel_path: &str,
    ext: &str,
    probe: &VideoProbe,
) -> Result<Asset, AssetImportError> {
    let c = build_common(source_path, sha256_hex, cas_rel_path)?;
    Ok(Asset::Video(VideoAsset {
        id: c.id,
        hash: c.hash,
        path: c.path,
        original_filename: c.original_filename,
        imported_at: c.imported_at,
        metadata: VideoAssetMetadata {
            duration_tk: Tick::new(probe.duration_tk),
            width: probe.width,
            height: probe.height,
            fps_num: probe.fps_num,
            fps_den: probe.fps_den,
            video_codec: probe.video_codec.clone(),
            audio_codec: None,
            audio_channels: None,
            audio_sample_rate_hz: None,
            bitrate_bps: None,
            color_space: None,
            color_primaries: None,
            container: ext.to_string(),
            has_alpha: None,
            rotation_deg: None,
            fingerprint: c.fingerprint,
        },
    }))
}

#[cfg(feature = "native")]
fn build_audio_asset(
    source_path: &Path,
    sha256_hex: &str,
    cas_rel_path: &str,
    ext: &str,
    probe: &AudioProbe,
) -> Result<Asset, AssetImportError> {
    let c = build_common(source_path, sha256_hex, cas_rel_path)?;
    Ok(Asset::Audio(AudioAsset {
        id: c.id,
        hash: c.hash,
        path: c.path,
        original_filename: c.original_filename,
        imported_at: c.imported_at,
        metadata: AudioAssetMetadata {
            duration_tk: Tick::new(probe.duration_tk),
            audio_codec: probe.audio_codec.clone(),
            audio_channels: probe.audio_channels,
            audio_sample_rate_hz: probe.audio_sample_rate_hz,
            bitrate_bps: None,
            container: ext.to_string(),
            fingerprint: c.fingerprint,
        },
    }))
}

/// Classify an import by inspecting its content (magic bytes) and route
/// to the matching `Asset` variant.
///
/// Invariant restored here (§3.1): an imported file's `Asset` kind
/// reflects its actual media type — video → `Asset::Video`, audio →
/// `Asset::Audio`, image → `Asset::Image`, subtitle → `Asset::Subtitle`.
/// Previously only PPM was recognized as an image and **every** other
/// extension (`.mp4`, `.wav`, ...) fell through to the subtitle
/// catch-all, so an imported video became a `SubtitleAsset` and
/// `clip.add` then rejected it with `E_ASSET_KIND_UNROUTABLE` (§5.1).
///
/// Classification is magic-byte first (the extension is never trusted on
/// its own — exactly like the existing PPM `P6` check). Subtitle is the
/// documented archival fallback for unrecognized bytes (§3.1 subtitle
/// callout): kind detection cannot fail the import, it degrades to an
/// archival `SubtitleAsset` rather than inventing a kind or fabricating
/// metadata.
#[cfg(feature = "native")]
fn build_asset_for_import(
    source_path: &Path,
    bytes: &[u8],
    sha256_hex: &str,
    cas_rel_path: &str,
    ext: &str,
) -> Result<Asset, AssetImportError> {
    if let Some((width, height)) = image_dimensions(bytes, ext) {
        return build_image_asset(source_path, sha256_hex, cas_rel_path, ext, width, height);
    }
    if let Some(probe) = probe_video(bytes) {
        return build_video_asset(source_path, sha256_hex, cas_rel_path, ext, &probe);
    }
    if let Some(probe) = probe_audio(bytes) {
        return build_audio_asset(source_path, sha256_hex, cas_rel_path, ext, &probe);
    }
    build_subtitle_asset(source_path, sha256_hex, cas_rel_path, ext)
}

/// Probed image dimensions, magic-byte first. Covers PPM (the existing
/// path) plus PNG and GIF — formats whose `width`/`height` (both schema
/// `minimum: 1`) are readable from a fixed header offset with no decode.
#[cfg(feature = "native")]
fn image_dimensions(bytes: &[u8], ext: &str) -> Option<(u32, u32)> {
    if ext == "ppm"
        && let Some(dims) = ppm_p6_dimensions(bytes)
    {
        return Some(dims);
    }
    png_dimensions(bytes).or_else(|| gif_dimensions(bytes))
}

/// PNG dimensions from the IHDR chunk. The 8-byte signature is followed
/// by a 4-byte length, the `IHDR` tag, then big-endian `width`/`height`.
#[cfg(feature = "native")]
fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    const SIG: &[u8] = b"\x89PNG\r\n\x1a\n";
    if bytes.get(..8)? != SIG || bytes.get(12..16)? != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(bytes.get(16..20)?.try_into().ok()?);
    let height = u32::from_be_bytes(bytes.get(20..24)?.try_into().ok()?);
    (width >= 1 && height >= 1).then_some((width, height))
}

/// GIF dimensions from the logical screen descriptor (little-endian
/// `width`/`height` immediately after the `GIF87a`/`GIF89a` header).
#[cfg(feature = "native")]
fn gif_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let header = bytes.get(..6)?;
    if header != b"GIF87a" && header != b"GIF89a" {
        return None;
    }
    let width = u32::from(u16::from_le_bytes(bytes.get(6..8)?.try_into().ok()?));
    let height = u32::from(u16::from_le_bytes(bytes.get(8..10)?.try_into().ok()?));
    (width >= 1 && height >= 1).then_some((width, height))
}

/// Real probed video metadata from an in-process container header parse.
///
/// The §3.1 probe is documented as ffprobe, but the crate-dependency
/// rule (`CLAUDE.md`) keeps `verbreel-state` off the codec/FFmpeg crates,
/// so this is a minimal header parser mirroring the PPM precedent — it
/// reads the values the schema marks `minimum: 1`, never fabricates them.
#[cfg(feature = "native")]
struct VideoProbe {
    width: u32,
    height: u32,
    fps_num: u32,
    fps_den: u32,
    duration_tk: i64,
    video_codec: String,
}

/// Real probed audio metadata. Same in-process-parse rationale as
/// [`VideoProbe`].
#[cfg(feature = "native")]
struct AudioProbe {
    duration_tk: i64,
    audio_codec: String,
    audio_channels: u32,
    audio_sample_rate_hz: u32,
}

/// Convert a `media_duration / timescale` pair into engine ticks
/// (240,000 Hz), clamped to the schema `minimum: 1`.
#[cfg(feature = "native")]
fn ticks_from_duration(duration_units: u64, timescale: u32) -> Option<i64> {
    if timescale == 0 {
        return None;
    }
    let tk = u128::from(duration_units).checked_mul(u128::from(verbreel_types::TICK_RATE_HZ))?
        / u128::from(timescale);
    Some(i64::try_from(tk).ok()?.max(1))
}

/// Probe an ISO-BMFF (`.mp4` / `.mov`) container for the first video
/// track's real dimensions, frame rate and duration. Returns `None` for
/// anything that is not a confirmed ISO-BMFF video — the dispatch then
/// falls through to audio/subtitle, never minting a fake video record.
#[cfg(feature = "native")]
fn probe_video(bytes: &[u8]) -> Option<VideoProbe> {
    // ISO-BMFF marker: `ftyp` box type at offset 4. The container magic,
    // like the PPM `P6` token, is the gate before any field is trusted.
    if bytes.get(4..8)? != b"ftyp" {
        return None;
    }
    let moov = find_box(bytes, *b"moov")?;
    let trak = find_video_trak(moov)?;
    let mdia = find_box(trak, *b"mdia")?;
    let mdhd = find_box(mdia, *b"mdhd")?;
    let (timescale, media_duration) = parse_mdhd(mdhd)?;

    let minf = find_box(mdia, *b"minf")?;
    let stbl = find_box(minf, *b"stbl")?;
    let stsd = find_box(stbl, *b"stsd")?;
    let (width, height, codec) = parse_stsd_video(stsd)?;

    let sample_count = find_box(stbl, *b"stsz")
        .and_then(parse_stsz_count)
        .unwrap_or(0);
    let (fps_num, fps_den) = video_fps(sample_count, timescale, media_duration);
    let duration_tk = ticks_from_duration(media_duration, timescale)?;

    Some(VideoProbe {
        width,
        height,
        fps_num,
        fps_den,
        duration_tk,
        video_codec: codec,
    })
}

/// Compute the video frame rate `(fps_num, fps_den)` from `sample_count *
/// timescale / media_duration`, the exact rational the container yields —
/// no float, no rounding to a "nice" rate.
///
/// fps is *non-essential* metadata: the file has already been positively
/// identified as video (valid `ftyp` + `vide` handler + `stsd`
/// dimensions) before this is called. So this is **infallible** — it
/// degrades to `1/1` (one frame per duration unit, schema-honest, not a
/// fabricated 30fps lie) when sample timing is absent OR when the raw
/// product would overflow `u32`. Letting an fps-representation failure
/// abort the probe would drop the file to the subtitle fallback and
/// re-trigger `E_ASSET_KIND_UNROUTABLE` on `clip.add` — the exact bug
/// this surface exists to eliminate — so it must never `?`-out here.
///
/// The fraction is reduced by gcd before the `u32` fit, which both stores
/// a sane rate (`1152000/48000` → `24/1`) and avoids the overflow in the
/// common case (a 2-hour clip at timescale 90000 has `sample_count *
/// timescale` far past `u32::MAX` unreduced, but a tidy ratio reduced).
#[cfg(feature = "native")]
fn video_fps(sample_count: u32, timescale: u32, media_duration: u64) -> (u32, u32) {
    if sample_count < 1 || media_duration < 1 {
        return (1, 1);
    }
    let num = u64::from(sample_count) * u64::from(timescale);
    let den = media_duration;
    let g = gcd(num, den).max(1);
    let (num, den) = (num / g, den / g);
    match (u32::try_from(num), u32::try_from(den)) {
        (Ok(n), Ok(d)) if n >= 1 && d >= 1 => (n, d),
        // Reduced ratio still doesn't fit u32 (pathological timescale):
        // degrade honestly rather than abort classification of a
        // confirmed video.
        _ => (1, 1),
    }
}

#[cfg(feature = "native")]
fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// One ISO-BMFF box parsed from a sibling walk: its 4-byte type, where
/// its body begins (after the 8- or 16-byte header), and where the box —
/// and thus the next sibling — ends.
#[cfg(feature = "native")]
struct BoxSpan {
    kind: [u8; 4],
    body_start: usize,
    box_end: usize,
}

/// Decode the box header at `offset`. Handles the three ISO-BMFF size
/// encodings: a normal 32-bit size, `size == 0` ("to end of data"), and
/// `size == 1` (64-bit extended size in the 8 bytes after the type).
/// Returns `None` only when the header itself is malformed/truncated so
/// the *whole* walk should stop — a single oversized or extended box is
/// reported with its real `box_end` so the caller can **skip** it and
/// keep scanning later siblings (e.g. a 64-bit `mdat` preceding `moov`).
#[cfg(feature = "native")]
fn parse_box_header(data: &[u8], offset: usize) -> Option<BoxSpan> {
    let size = u32::from_be_bytes(data.get(offset..offset + 4)?.try_into().ok()?) as usize;
    let kind: [u8; 4] = data.get(offset + 4..offset + 8)?.try_into().ok()?;
    let (header_len, box_size) = if size == 1 {
        // 64-bit extended size occupies bytes offset+8..offset+16.
        let large = u64::from_be_bytes(data.get(offset + 8..offset + 16)?.try_into().ok()?);
        (16usize, usize::try_from(large).ok()?)
    } else {
        (8usize, size)
    };
    let body_start = offset.checked_add(header_len)?;
    let box_end = if size == 0 {
        data.len()
    } else {
        offset.checked_add(box_size)?
    };
    if box_end < body_start || box_end > data.len() {
        return None;
    }
    Some(BoxSpan {
        kind,
        body_start,
        box_end,
    })
}

/// Walk the immediate child boxes of an ISO-BMFF box body, returning the
/// **body** (header stripped) of the first child whose 4-byte type
/// matches `want`. Boxes are `[u32 size][u8;4 type][body]`, big-endian.
/// A box using the 64-bit extended size (`size == 1`, common on large
/// `mdat`) is skipped rather than aborting the scan, so a later `moov`
/// sibling is still found.
#[cfg(feature = "native")]
fn find_box(data: &[u8], want: [u8; 4]) -> Option<&[u8]> {
    let mut offset = 0usize;
    while offset + 8 <= data.len() {
        let span = parse_box_header(data, offset)?;
        if span.kind == want {
            return data.get(span.body_start..span.box_end);
        }
        offset = span.box_end;
    }
    None
}

/// Find the first `trak` box under `moov` whose `mdia/hdlr` handler type
/// is `vide` (a video track), returning the `trak` body.
#[cfg(feature = "native")]
fn find_video_trak(moov: &[u8]) -> Option<&[u8]> {
    let mut offset = 0usize;
    while offset + 8 <= moov.len() {
        let span = parse_box_header(moov, offset)?;
        if span.kind == *b"trak"
            && let Some(trak) = moov.get(span.body_start..span.box_end)
            && trak_is_video(trak)
        {
            return Some(trak);
        }
        offset = span.box_end;
    }
    None
}

#[cfg(feature = "native")]
fn trak_is_video(trak: &[u8]) -> bool {
    find_box(trak, *b"mdia")
        .and_then(|mdia| find_box(mdia, *b"hdlr"))
        // hdlr layout: version(1) flags(3) pre_defined(4) handler_type(4).
        .and_then(|hdlr| hdlr.get(8..12))
        == Some(b"vide".as_slice())
}

/// Parse an `mdhd` box → `(timescale, duration)`. Version 0 uses 32-bit
/// fields, version 1 uses 64-bit. Layout after the 1-byte version +
/// 3-byte flags: creation, modification, timescale, duration.
#[cfg(feature = "native")]
fn parse_mdhd(mdhd: &[u8]) -> Option<(u32, u64)> {
    let version = *mdhd.first()?;
    if version == 1 {
        let timescale = u32::from_be_bytes(mdhd.get(20..24)?.try_into().ok()?);
        let duration = u64::from_be_bytes(mdhd.get(24..32)?.try_into().ok()?);
        Some((timescale, duration))
    } else {
        let timescale = u32::from_be_bytes(mdhd.get(12..16)?.try_into().ok()?);
        let duration = u64::from(u32::from_be_bytes(mdhd.get(16..20)?.try_into().ok()?));
        Some((timescale, duration))
    }
}

/// Parse the first sample entry of an `stsd` box → `(width, height,
/// codec)`. After `version(1)`/`flags(3)`/`entry_count(4)` comes the sample
/// entry: `[u32 size][u8;4 format][...]` where a visual sample entry
/// carries 16-bit `width`/`height` at a fixed offset (ISO/IEC 14496-12).
#[cfg(feature = "native")]
fn parse_stsd_video(stsd: &[u8]) -> Option<(u32, u32, String)> {
    let entry = stsd.get(8..)?;
    let format = entry.get(4..8)?;
    let codec = codec_for_fourcc(format);
    // VisualSampleEntry: 8 (box header) + 6 reserved + 2 data_ref_index
    // + 16 predefined/reserved = 32, then 16-bit width, 16-bit height.
    let width = u32::from(u16::from_be_bytes(entry.get(32..34)?.try_into().ok()?));
    let height = u32::from(u16::from_be_bytes(entry.get(34..36)?.try_into().ok()?));
    (width >= 1 && height >= 1).then_some((width, height, codec))
}

#[cfg(feature = "native")]
fn codec_for_fourcc(format: &[u8]) -> String {
    match format {
        b"avc1" | b"avc3" => "h264".to_string(),
        b"hev1" | b"hvc1" => "hevc".to_string(),
        b"av01" => "av1".to_string(),
        b"vp09" => "vp9".to_string(),
        other => String::from_utf8_lossy(other)
            .trim_end_matches('\0')
            .to_string(),
    }
}

/// Parse an `stsz` box → sample count (the field after version/flags +
/// `sample_size`).
#[cfg(feature = "native")]
fn parse_stsz_count(stsz: &[u8]) -> Option<u32> {
    Some(u32::from_be_bytes(stsz.get(8..12)?.try_into().ok()?))
}

/// Probe an audio container. Covers canonical RIFF/WAVE — the
/// `fmt `/`data` chunk pair yields real `channels`, `sample_rate` and a
/// `duration` derived from the data-chunk byte length, all schema
/// `minimum: 1`. Returns `None` for non-WAVE bytes.
#[cfg(feature = "native")]
fn probe_audio(bytes: &[u8]) -> Option<AudioProbe> {
    if bytes.get(..4)? != b"RIFF" || bytes.get(8..12)? != b"WAVE" {
        return None;
    }
    let body = bytes.get(12..)?;
    let fmt = find_riff_chunk(body, *b"fmt ")?;
    let channels = u32::from(u16::from_le_bytes(fmt.get(2..4)?.try_into().ok()?));
    let sample_rate = u32::from_le_bytes(fmt.get(4..8)?.try_into().ok()?);
    let bits_per_sample = u32::from(u16::from_le_bytes(fmt.get(14..16)?.try_into().ok()?));
    if channels < 1 || sample_rate < 1 || bits_per_sample < 1 {
        return None;
    }
    let data = find_riff_chunk(body, *b"data")?;
    let bytes_per_frame = channels.checked_mul(bits_per_sample / 8)?.max(1);
    let frames = u64::try_from(data.len()).ok()? / u64::from(bytes_per_frame);
    let duration_tk = ticks_from_duration(frames, sample_rate)?;
    Some(AudioProbe {
        duration_tk,
        audio_codec: "pcm".to_string(),
        audio_channels: channels,
        audio_sample_rate_hz: sample_rate,
    })
}

/// Find a RIFF chunk body by 4-byte id. Chunks are `[u8;4 id][u32_le
/// size][body]`, with bodies padded to even length.
#[cfg(feature = "native")]
fn find_riff_chunk(data: &[u8], want: [u8; 4]) -> Option<&[u8]> {
    let mut offset = 0usize;
    while offset + 8 <= data.len() {
        let id = data.get(offset..offset + 4)?;
        let size = u32::from_le_bytes(data.get(offset + 4..offset + 8)?.try_into().ok()?) as usize;
        let body_start = offset + 8;
        let body_end = body_start.checked_add(size)?;
        if body_end > data.len() {
            return None;
        }
        if id == want {
            return data.get(body_start..body_end);
        }
        offset = body_end + (size & 1);
    }
    None
}

#[cfg(feature = "native")]
fn ppm_p6_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut cursor = PpmCursor { bytes, offset: 0 };
    if cursor.next_token()? != b"P6" {
        return None;
    }
    let width = parse_positive_u32(cursor.next_token()?)?;
    let height = parse_positive_u32(cursor.next_token()?)?;
    let maxval = parse_positive_u32(cursor.next_token()?)?;
    let expected_len = usize::try_from(width)
        .ok()?
        .checked_mul(usize::try_from(height).ok()?)?
        .checked_mul(3)?;
    if maxval > 255 || cursor.raster_len_after_separator()? < expected_len {
        return None;
    }
    Some((width, height))
}

#[cfg(feature = "native")]
struct PpmCursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

#[cfg(feature = "native")]
impl<'a> PpmCursor<'a> {
    fn next_token(&mut self) -> Option<&'a [u8]> {
        self.skip_ws_and_comments();
        let start = self.offset;
        while self
            .bytes
            .get(self.offset)
            .is_some_and(|b| !b.is_ascii_whitespace() && *b != b'#')
        {
            self.offset += 1;
        }
        (self.offset > start).then_some(&self.bytes[start..self.offset])
    }

    fn skip_ws_and_comments(&mut self) {
        loop {
            while self
                .bytes
                .get(self.offset)
                .is_some_and(u8::is_ascii_whitespace)
            {
                self.offset += 1;
            }
            if self.bytes.get(self.offset) != Some(&b'#') {
                return;
            }
            while self.bytes.get(self.offset).is_some_and(|b| *b != b'\n') {
                self.offset += 1;
            }
        }
    }

    fn raster_len_after_separator(&mut self) -> Option<usize> {
        self.bytes
            .get(self.offset)
            .is_some_and(u8::is_ascii_whitespace)
            .then_some(self.bytes.len().saturating_sub(self.offset + 1))
    }
}

#[cfg(feature = "native")]
fn parse_positive_u32(token: &[u8]) -> Option<u32> {
    if token.is_empty() || !token.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let value = std::str::from_utf8(token).ok()?.parse::<u32>().ok()?;
    (value > 0).then_some(value)
}

fn mode_used_for(_args: &AssetImportArgs) -> &'static str {
    "copy"
}

/// Native import path with project-root filesystem context.
///
/// Invariant repaired here: non-empty `paths` now write source bytes to
/// CAS (`assets/<aa>/<sha256>.<ext>`) using
/// `verbreel_storage::cas::key_for_bytes`, and patch-add a
/// corresponding `Asset` record carrying canonical hash/path fields.
///
/// ## §0.5.1 compute-only (`dry_run`) mode
///
/// `persist` selects whether the CAS write actually fires. The persist
/// path (`SideEffects::Persist`) writes the source bytes to
/// `assets/<aa>/<sha256>.<ext>` exactly as before. The compute-only path
/// (`SideEffects::ComputeOnly`, used by the `dry_run` route) still reads
/// the source, computes the real sha256 + CAS key, and builds the
/// identical would-be patch — but **skips** `atomic_write_bytes` so no
/// orphaned CAS object is left on disk. This is the §0.5.1 guarantee:
/// "the patch reflects real probed values" (so `Asset.hash` is the true
/// content hash) while "no asset bytes are copied into `assets/`". The
/// content-match read against a pre-existing CAS object is a read, not a
/// write, so it runs in both modes.
///
/// # Errors
///
/// Returns schema violations, missing-path errors, and I/O errors.
#[cfg(feature = "native")]
pub fn compute_patch_with_root(
    _prior: &Project,
    args: &AssetImportArgs,
    project_root: &Path,
    persist: SideEffects,
) -> Result<(Value, Vec<Value>, AssetImportData), AssetImportError> {
    if args.paths.len() > PATHS_MAX_BATCH {
        return Err(AssetImportError::SchemaViolation {
            field: "paths",
            hint: SCHEMA_VIOLATION_HINT,
            actual_count: args.paths.len(),
            max_count: PATHS_MAX_BATCH,
        });
    }
    if args.paths.is_empty() {
        return Ok((json!([]), Vec::new(), empty_data()));
    }

    let mut patch_ops = Vec::with_capacity(args.paths.len());
    let mut assets = Vec::with_capacity(args.paths.len());
    let mut modes_used = Vec::with_capacity(args.paths.len());

    for input_path in &args.paths {
        let src = Path::new(input_path);
        let bytes = fs::read(src).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                AssetImportError::AssetPathNotFound {
                    path: input_path.clone(),
                }
            } else {
                AssetImportError::Io {
                    path: input_path.clone(),
                    detail: e.to_string(),
                }
            }
        })?;

        let ext = canonical_ext(src);
        let key = key_for_bytes(&bytes, &ext).map_err(|e| AssetImportError::InconsistentPatch {
            detail: e.to_string(),
        })?;
        let dst = project_root.join(&key.relative_path);
        if dst.exists() {
            // Reading an already-present CAS object to verify it matches
            // is a read, not a write — runs in both Persist and
            // ComputeOnly mode (§0.5.1 permits source reads under dry_run).
            let existing = fs::read(&dst).map_err(|e| AssetImportError::Io {
                path: dst.display().to_string(),
                detail: e.to_string(),
            })?;
            if existing != bytes {
                return Err(AssetImportError::Io {
                    path: dst.display().to_string(),
                    detail: "existing CAS object contents do not match expected hash".to_string(),
                });
            }
        } else if persist == SideEffects::Persist {
            // §0.5.1: only the real mutate path writes to assets/. The
            // dry_run / compute-only path skips this so it leaves no
            // orphaned CAS object — the patch above already carries the
            // real content hash, which is the dry-run guarantee.
            if let Some(parent) = dst.parent() {
                fs::create_dir_all(parent).map_err(|e| AssetImportError::Io {
                    path: parent.display().to_string(),
                    detail: e.to_string(),
                })?;
            }
            atomic_write_bytes(&dst, &bytes).map_err(|e| AssetImportError::Io {
                path: dst.display().to_string(),
                detail: e.to_string(),
            })?;
        }

        let asset = build_asset_for_import(src, &bytes, &key.sha256_hex, &key.relative_path, &ext)?;
        let asset_value =
            serde_json::to_value(&asset).map_err(|e| AssetImportError::InconsistentPatch {
                detail: e.to_string(),
            })?;
        patch_ops.push(json!({
            "op": "add",
            "path": "/assets/-",
            "value": asset_value.clone(),
        }));
        assets.push(asset_value);
        modes_used.push(json!({
            "asset_id": asset.id().to_string(),
            "mode_used": mode_used_for(args),
            "input_path": input_path,
        }));
    }

    Ok((
        Value::Array(patch_ops),
        Vec::new(),
        AssetImportData {
            assets,
            modes_used,
            missing_paths: Vec::new(),
            skipped_input_indices: Vec::new(),
        },
    ))
}

fn asset_ids_from_patch(patch: &Value) -> Result<Vec<String>, ReconstructError> {
    let ops = patch.as_array().ok_or_else(|| {
        ReconstructError::Custom("asset.import: patch must be an array".to_string())
    })?;
    let mut ids = Vec::with_capacity(ops.len());
    for op in ops {
        let op_path = op.get("path").and_then(Value::as_str).unwrap_or_default();
        let op_kind = op.get("op").and_then(Value::as_str).unwrap_or_default();
        if op_kind != "add" || op_path != "/assets/-" {
            return Err(ReconstructError::Custom(
                "asset.import: patch op must be `add /assets/-`".to_string(),
            ));
        }
        let id = op
            .get("value")
            .and_then(|v| v.get("id"))
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ReconstructError::Custom("asset.import: patch asset missing id".to_string())
            })?;
        ids.push(id.to_string());
    }
    Ok(ids)
}

fn replay_data_from_patch(
    args: &AssetImportArgs,
    patch: &Value,
    post_state: &Project,
) -> Result<AssetImportData, ReconstructError> {
    let ids = asset_ids_from_patch(patch)?;
    if ids.len() != args.paths.len() {
        return Err(ReconstructError::Custom(format!(
            "asset.import: patch assets count {} does not match args.paths count {}",
            ids.len(),
            args.paths.len()
        )));
    }

    let mut by_id: HashMap<String, Value> = HashMap::new();
    for asset in &post_state.assets {
        let value =
            serde_json::to_value(asset).map_err(|e| ReconstructError::Custom(e.to_string()))?;
        by_id.insert(asset.id().to_string(), value);
    }

    let mut assets = Vec::with_capacity(ids.len());
    let mut modes_used = Vec::with_capacity(ids.len());
    for (idx, id) in ids.iter().enumerate() {
        let asset = by_id.get(id).ok_or_else(|| {
            ReconstructError::Custom(format!(
                "asset.import: post_state missing asset id from patch: {id}"
            ))
        })?;
        assets.push(asset.clone());
        modes_used.push(json!({
            "asset_id": id,
            "mode_used": mode_used_for(args),
            "input_path": args.paths[idx],
        }));
    }

    Ok(AssetImportData {
        assets,
        modes_used,
        missing_paths: Vec::new(),
        skipped_input_indices: Vec::new(),
    })
}

/// Reconstruct the data envelope from `(args, post_state)`.
///
/// Pure fallback path uses [`compute_patch`] and is valid for the
/// no-root floor behavior.
///
/// # Errors
///
/// Reuses [`compute_patch`], mapped into [`ReconstructError`].
pub fn data_envelope_from_args(
    args: &AssetImportArgs,
    post_state: &Project,
) -> Result<AssetImportData, ReconstructError> {
    let (_, _, data) =
        compute_patch(post_state, args).map_err(|e| ReconstructError::Custom(e.to_string()))?;
    Ok(data)
}

impl From<AssetImportError> for VerbError {
    fn from(value: AssetImportError) -> Self {
        match value {
            AssetImportError::SchemaViolation { .. } => VerbError::BadArgs {
                detail: value.to_string(),
            },
            AssetImportError::AssetPathNotFound { .. }
            | AssetImportError::Io { .. }
            | AssetImportError::InconsistentPatch { .. } => VerbError::Custom(value.to_string()),
        }
    }
}

/// The §0.8 verb for `asset.import`.
#[derive(Debug, Default)]
pub struct AssetImportVerb;

impl Verb for AssetImportVerb {
    fn verb(&self) -> &'static str {
        "asset.import"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: AssetImportArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("asset.import: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch = serde_json::from_value(patch_value).map_err(|err| {
            VerbError::Custom(format!("asset.import: patch construction failed: {err}"))
        })?;

        let data = serde_json::to_value(&data).map_err(|err| {
            VerbError::Custom(format!("asset.import: data envelope failed: {err}"))
        })?;

        Ok((patch, data, warnings))
    }

    fn reconstruct(
        &self,
        args: &Value,
        patch: &Value,
        _warnings: &[Value],
        post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let typed: AssetImportArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "AssetImportArgs",
            })?;

        let envelope = replay_data_from_patch(&typed, patch, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
