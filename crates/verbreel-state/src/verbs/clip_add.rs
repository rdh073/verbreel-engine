//! `clip.add` (§5.1) — fifty-sixth production verb in the engine.
//!
//! Places a video / audio / image asset slice on a track at a given
//! timeline position. The verb mints a fresh `ClipId`, snaps the
//! position to the project fps grid (for non-audio target tracks),
//! checks atomic overlap, and inserts the new clip into the target
//! track's `clips[]`.
//!
//! ## Out of scope (deferred)
//!
//! - **Auto-pair audio (§5.15)** — placing a video-with-audio asset on
//!   a video track does NOT this slice also mint an audio sibling on
//!   an auto-named audio track. Follow-up issue.
//! - **Edge snap (§0.17)** — `snap_to_edges` / `snap_tolerance_tk` /
//!   the `W_EDGE_SNAPPED` warning.
//! - **`W_AUTO_PAIR_TRACK_NAME_TRUNCATED`** — only fires from the
//!   auto-pair path.
//! - **Structural selectors** like `video[0]` or `audio[name="vox"]`.
//!   The `track` arg accepts only a bare `UUIDv7` or `track:<uuid>`.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::{AssetId, ClipId, ProjectId, TICK_RATE_HZ, Tick, TrackId};

use crate::asset::Asset;
use crate::clip::{BlendMode, Clip, FadeCurve};
use crate::invariants::timeline_duration_tk;
use crate::newtypes::AssetRef;
use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use crate::track::{Track, TrackKind};
use crate::transform::Transform;
use crate::verbs::project_set_fps::is_off_frame;

/// Warning code emitted when an off-frame `track_position_tk` snapped
/// to a frame boundary.
pub const W_TIME_SNAPPED_CODE: &str = "W_TIME_SNAPPED";

/// Internal warning code carrying minted ids for reconstructor replay.
pub const W_CLIP_ADD_ENVELOPE_CODE: &str = "W_CLIP_ADD_ENVELOPE";

/// Default image-clip display duration in ticks (5 seconds at 240k Hz).
pub const DEFAULT_IMAGE_DURATION_TK: i64 = 5 * 240_000;

const SCHEMA_HINT_IMAGE_SOURCE_IN: &str = "image-kind clips must have source_in_tk=0";

/// Args for `clip.add`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipAddArgs {
    /// Target project id.
    pub project_id: ProjectId,
    /// Bare `UUIDv7` of the asset to place.
    pub asset_id: String,
    /// Track selector: bare `UUIDv7` or `track:<uuid>`.
    pub track: String,
    /// Timeline start tick before frame snapping.
    pub track_position_tk: i64,
    /// Source slice start tick. Defaults to `0`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_in_tk: Option<i64>,
    /// Source slice end tick (exclusive). Defaults per asset kind:
    /// video/audio → `asset.metadata.duration_tk`, image →
    /// `source_in_tk + 5 * 240000`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_out_tk: Option<i64>,
    /// Optional display name. Defaults to the asset path basename.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Envelope `data` returned by `clip.add`. The auto-pair fields
/// (`linked_audio_clip_id`, `linked_audio_track_id`, `link_group`) are
/// intentionally omitted in this slice — see module docs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipAddData {
    /// Freshly minted clip id.
    pub clip_id: ClipId,
    /// Resolved target track id.
    pub track_id: TrackId,
}

/// Verb-level validation failures for `clip.add`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ClipAddError {
    /// `asset_id` or `track` failed to parse as `UUIDv7`, or `track`
    /// used a non-`track` selector prefix.
    #[error("E_BAD_SELECTOR: clip.add: {field}: {detail}")]
    BadSelector {
        /// Failing field name.
        field: &'static str,
        /// Parse failure detail.
        detail: String,
    },

    /// `asset_id` resolved to no asset in `project.assets[]`.
    #[error("E_ASSET_NOT_FOUND: clip.add: asset `{asset_id}` not found")]
    AssetNotFound {
        /// Missing asset id string.
        asset_id: String,
    },

    /// `asset_id` resolved to a subtitle asset; no v1 routing exists.
    #[error(
        "E_ASSET_KIND_UNROUTABLE: clip.add: asset `{asset_id}` kind `{asset_kind}` has no v1 clip.add path"
    )]
    AssetKindUnroutable {
        /// Offending asset id.
        asset_id: String,
        /// Reported asset kind (lowercase: `"subtitle"`).
        asset_kind: &'static str,
    },

    /// `track` resolved to no track.
    #[error("E_TRACK_NOT_FOUND: clip.add: track `{track_id}` not found")]
    TrackNotFound {
        /// Missing track id string.
        track_id: String,
    },

    /// Asset kind cannot be routed onto the resolved track kind. The
    /// mapping is the §5.1 asset routing matrix.
    #[error(
        "E_TRACK_KIND_MISMATCH: clip.add: asset `{asset_id}` (kind `{asset_kind}`) cannot place \
         onto {track_kind:?} track `{track_id}`"
    )]
    TrackKindMismatch {
        /// Offending asset id.
        asset_id: String,
        /// Asset kind label.
        asset_kind: &'static str,
        /// Resolved target track id.
        track_id: String,
        /// Resolved target track kind.
        track_kind: TrackKind,
    },

    /// A time field violates a non-negativity / ordering / range bound.
    #[error("E_BAD_TIME: clip.add: `{field}` value {value} violates bound {bound}")]
    BadTime {
        /// Failing field.
        field: &'static str,
        /// Bound description.
        bound: String,
        /// Observed value.
        value: i64,
    },

    /// Image-kind clip received non-zero `source_in_tk` — §0.13 pins
    /// image clips to `source_in_tk == 0`.
    #[error("E_SCHEMA_VIOLATION: clip.add: `{field}` invariant violated: {hint}; got {value}")]
    SchemaViolation {
        /// Failing field.
        field: &'static str,
        /// Recovery hint.
        hint: &'static str,
        /// Observed value.
        value: String,
    },

    /// Target track is locked.
    #[error("E_LOCKED: clip.add: track `{track_id}` is locked")]
    Locked {
        /// Locked track id.
        track_id: String,
    },

    /// Planned clip would overlap an existing clip on the target track.
    #[error("E_CLIP_OVERLAP: clip.add: new clip would overlap on track `{track_id}`")]
    ClipOverlap {
        /// Target track id.
        track_id: String,
    },
}

#[derive(Debug, Clone, Copy)]
struct ResolvedTrack<'a> {
    idx: usize,
    track: &'a Track,
}

#[derive(Debug, Clone, Copy)]
enum AssetKindLabel {
    Video,
    Audio,
    Image,
    Subtitle,
}

impl AssetKindLabel {
    fn from_asset(asset: &Asset) -> Self {
        match asset {
            Asset::Video(_) => AssetKindLabel::Video,
            Asset::Audio(_) => AssetKindLabel::Audio,
            Asset::Image(_) => AssetKindLabel::Image,
            Asset::Subtitle(_) => AssetKindLabel::Subtitle,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            AssetKindLabel::Video => "video",
            AssetKindLabel::Audio => "audio",
            AssetKindLabel::Image => "image",
            AssetKindLabel::Subtitle => "subtitle",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EnvelopeWarningDetails {
    clip_id: ClipId,
    track_id: TrackId,
}

/// Build the RFC 6902 patch and envelope for `clip.add`.
///
/// # Errors
///
/// Returns [`ClipAddError`] for selector parse failures, missing asset
/// / track, asset-kind / track-kind mismatches, negative or out-of-range
/// time fields, locked tracks, or overlap conflicts.
#[allow(clippy::too_many_lines)]
pub fn compute_patch(
    prior: &Project,
    args: &ClipAddArgs,
) -> Result<(Value, Vec<Value>, ClipAddData), ClipAddError> {
    let asset_id = parse_asset_id(&args.asset_id)?;
    let asset = locate_asset(prior, &asset_id).ok_or_else(|| ClipAddError::AssetNotFound {
        asset_id: args.asset_id.clone(),
    })?;
    let kind = AssetKindLabel::from_asset(asset);

    if matches!(kind, AssetKindLabel::Subtitle) {
        return Err(ClipAddError::AssetKindUnroutable {
            asset_id: args.asset_id.clone(),
            asset_kind: kind.as_str(),
        });
    }

    let track_id = parse_track_selector(&args.track)?;
    let resolved = locate_track(prior, track_id).ok_or_else(|| ClipAddError::TrackNotFound {
        track_id: track_id.to_string(),
    })?;

    enforce_routing(asset, kind, resolved.track, &args.asset_id)?;

    if resolved.track.locked {
        return Err(ClipAddError::Locked {
            track_id: resolved.track.id.to_string(),
        });
    }

    let source_in_tk = args.source_in_tk.unwrap_or(0);
    if source_in_tk < 0 {
        return Err(ClipAddError::BadTime {
            field: "source_in_tk",
            bound: ">= 0".to_string(),
            value: source_in_tk,
        });
    }

    if matches!(kind, AssetKindLabel::Image) && source_in_tk != 0 {
        return Err(ClipAddError::SchemaViolation {
            field: "source_in_tk",
            hint: SCHEMA_HINT_IMAGE_SOURCE_IN,
            value: source_in_tk.to_string(),
        });
    }

    let source_out_tk = resolve_source_out_tk(asset, kind, source_in_tk, args.source_out_tk)?;

    if source_in_tk >= source_out_tk {
        return Err(ClipAddError::BadTime {
            field: "source_out_tk",
            bound: format!("> source_in_tk ({source_in_tk})"),
            value: source_out_tk,
        });
    }

    if let Some(asset_duration_tk) = asset_duration_tk(asset)
        && source_out_tk > asset_duration_tk
    {
        return Err(ClipAddError::BadTime {
            field: "source_out_tk",
            bound: format!("<= asset.metadata.duration_tk ({asset_duration_tk})"),
            value: source_out_tk,
        });
    }

    if args.track_position_tk < 0 {
        return Err(ClipAddError::BadTime {
            field: "track_position_tk",
            bound: ">= 0".to_string(),
            value: args.track_position_tk,
        });
    }

    let mut warnings = Vec::new();
    let snapped_position_tk = snap_position(
        prior,
        resolved.track.kind,
        args.track_position_tk,
        &mut warnings,
    );

    let new_end_tk = snapped_position_tk.saturating_add(source_out_tk - source_in_tk);
    check_overlap(
        &resolved.track.clips,
        snapped_position_tk,
        new_end_tk,
        resolved.track.id.to_string(),
    )?;

    let clip_id = ClipId::now();
    let derived_name = derive_name(args.name.as_deref(), asset, kind);
    let clip = build_clip(
        clip_id,
        derived_name,
        *asset.id(),
        source_in_tk,
        source_out_tk,
        snapped_position_tk,
    );

    let data = ClipAddData {
        clip_id,
        track_id: resolved.track.id,
    };
    warnings.push(envelope_warning(&data));
    let patch = build_patch(prior, resolved.idx, &clip, new_end_tk);
    Ok((patch, warnings, data))
}

fn parse_asset_id(raw: &str) -> Result<AssetId, ClipAddError> {
    raw.parse::<AssetId>()
        .map_err(|err| ClipAddError::BadSelector {
            field: "asset_id",
            detail: err.to_string(),
        })
}

fn parse_track_selector(raw: &str) -> Result<TrackId, ClipAddError> {
    if let Some((prefix, rest)) = raw.split_once(':') {
        if prefix != "track" {
            return Err(ClipAddError::BadSelector {
                field: "track",
                detail: format!("unsupported selector prefix `{prefix}`"),
            });
        }
        return rest
            .parse::<TrackId>()
            .map_err(|err| ClipAddError::BadSelector {
                field: "track",
                detail: err.to_string(),
            });
    }
    raw.parse::<TrackId>()
        .map_err(|err| ClipAddError::BadSelector {
            field: "track",
            detail: err.to_string(),
        })
}

fn locate_asset<'a>(project: &'a Project, asset_id: &AssetId) -> Option<&'a Asset> {
    project.assets.iter().find(|asset| asset.id() == asset_id)
}

fn locate_track(project: &Project, track_id: TrackId) -> Option<ResolvedTrack<'_>> {
    project
        .tracks
        .iter()
        .enumerate()
        .find(|(_, track)| track.id == track_id)
        .map(|(idx, track)| ResolvedTrack { idx, track })
}

fn enforce_routing(
    asset: &Asset,
    kind: AssetKindLabel,
    track: &Track,
    asset_id_str: &str,
) -> Result<(), ClipAddError> {
    if routing_allowed(asset, kind, track.kind) {
        return Ok(());
    }
    Err(ClipAddError::TrackKindMismatch {
        asset_id: asset_id_str.to_string(),
        asset_kind: kind.as_str(),
        track_id: track.id.to_string(),
        track_kind: track.kind,
    })
}

fn routing_allowed(asset: &Asset, kind: AssetKindLabel, track_kind: TrackKind) -> bool {
    match (kind, track_kind) {
        // §5.1 routing matrix: video-with-audio on an audio track plays
        // the asset's audio sub-stream only; the other unconditional
        // matches share the `true` body.
        (AssetKindLabel::Video, TrackKind::Audio) => video_has_audio_stream(asset),
        (AssetKindLabel::Video | AssetKindLabel::Image, TrackKind::Video)
        | (AssetKindLabel::Audio, TrackKind::Audio) => true,
        _ => false,
    }
}

fn video_has_audio_stream(asset: &Asset) -> bool {
    matches!(asset, Asset::Video(v) if v.metadata.audio_codec.is_some())
}

fn asset_duration_tk(asset: &Asset) -> Option<i64> {
    match asset {
        Asset::Video(v) => Some(v.metadata.duration_tk.get()),
        Asset::Audio(a) => Some(a.metadata.duration_tk.get()),
        Asset::Image(_) | Asset::Subtitle(_) => None,
    }
}

fn resolve_source_out_tk(
    asset: &Asset,
    kind: AssetKindLabel,
    source_in_tk: i64,
    explicit: Option<i64>,
) -> Result<i64, ClipAddError> {
    if let Some(value) = explicit {
        if value < 1 {
            return Err(ClipAddError::BadTime {
                field: "source_out_tk",
                bound: ">= 1".to_string(),
                value,
            });
        }
        return Ok(value);
    }
    match kind {
        AssetKindLabel::Video | AssetKindLabel::Audio => {
            asset_duration_tk(asset).ok_or(ClipAddError::BadTime {
                field: "source_out_tk",
                bound: "asset has duration_tk".to_string(),
                value: 0,
            })
        }
        AssetKindLabel::Image => Ok(source_in_tk.saturating_add(DEFAULT_IMAGE_DURATION_TK)),
        AssetKindLabel::Subtitle => unreachable!("subtitle assets are rejected earlier"),
    }
}

fn snap_position(
    prior: &Project,
    track_kind: TrackKind,
    value_tk: i64,
    warnings: &mut Vec<Value>,
) -> i64 {
    // Audio tracks are sub-frame addressable per §0.17 — no snap.
    if matches!(track_kind, TrackKind::Audio)
        || !is_off_frame(Tick::new(value_tk), prior.fps_num, prior.fps_den)
    {
        return value_tk;
    }

    let snapped_tk = nearest_frame_tick(value_tk, prior.fps_num, prior.fps_den);
    if snapped_tk != value_tk {
        warnings.push(json!({
            "code": W_TIME_SNAPPED_CODE,
            "message": "time value snapped to frame boundary",
            "details": {
                "from_tk": value_tk,
                "to_tk": snapped_tk,
                "field": "track_position_tk",
            }
        }));
    }
    snapped_tk
}

fn nearest_frame_tick(value_tk: i64, fps_num: u32, fps_den: u32) -> i64 {
    if fps_num == 0 {
        return value_tk;
    }
    let frame_clock = u64::from(TICK_RATE_HZ) * u64::from(fps_den);
    let step_tk = frame_clock / gcd_u64(frame_clock, u64::from(fps_num));
    if step_tk == 0 {
        return value_tk;
    }
    let value = i128::from(value_tk.max(0));
    let step = i128::from(step_tk);
    let snapped = ((value + (step / 2)) / step) * step;
    i64::try_from(snapped).unwrap_or(i64::MAX)
}

fn gcd_u64(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        let rem = a % b;
        a = b;
        b = rem;
    }
    a
}

fn check_overlap(
    clips: &[Clip],
    new_start_tk: i64,
    new_end_tk: i64,
    track_id: String,
) -> Result<(), ClipAddError> {
    for clip in clips {
        let start_tk = clip.track_position_tk.get();
        let duration_tk = timeline_duration_tk(clip.source_in_tk, clip.source_out_tk, clip.speed);
        let end_tk = start_tk.saturating_add(duration_tk.get());
        if intervals_overlap(new_start_tk, new_end_tk, start_tk, end_tk) {
            return Err(ClipAddError::ClipOverlap { track_id });
        }
    }
    Ok(())
}

fn intervals_overlap(a_start: i64, a_end: i64, b_start: i64, b_end: i64) -> bool {
    a_start < b_end && b_start < a_end
}

fn derive_name(explicit: Option<&str>, asset: &Asset, kind: AssetKindLabel) -> String {
    if let Some(name) = explicit {
        return name.to_string();
    }
    let path_str = asset.path().as_str();
    let basename = path_str.rsplit('/').next().unwrap_or(path_str);
    let stem = basename.rsplit_once('.').map_or(basename, |(stem, _)| stem);
    if stem.is_empty() {
        return format!("{} clip", kind.as_str());
    }
    stem.to_string()
}

fn build_clip(
    id: ClipId,
    name: String,
    asset_id: AssetId,
    source_in_tk: i64,
    source_out_tk: i64,
    track_position_tk: i64,
) -> Clip {
    Clip {
        id,
        name,
        asset_id: AssetRef::from_id(asset_id),
        track_position_tk: Tick::new(track_position_tk),
        source_in_tk: Tick::new(source_in_tk),
        source_out_tk: Tick::new(source_out_tk),
        speed: 1.0,
        reversed: false,
        transform: Transform::default(),
        opacity: 1.0,
        volume: 1.0,
        fade_in_tk: Tick::ZERO,
        fade_out_tk: Tick::ZERO,
        fade_in_curve: FadeCurve::Linear,
        fade_out_curve: FadeCurve::Linear,
        effects: Vec::new(),
        keyframes: Vec::new(),
        text: None,
        locked: false,
        link_group: None,
        blend_mode: BlendMode::Normal,
        mask: None,
        speed_curve: None,
    }
}

fn build_patch(prior: &Project, track_idx: usize, clip: &Clip, new_end_tk: i64) -> Value {
    let mut ops = vec![json!({
        "op": "add",
        "path": format!("/tracks/{track_idx}/clips/-"),
        "value": clip,
    })];
    if new_end_tk > prior.duration_tk.get() {
        ops.push(json!({
            "op": "replace",
            "path": "/duration_tk",
            "value": new_end_tk,
        }));
    }
    Value::Array(ops)
}

fn envelope_warning(data: &ClipAddData) -> Value {
    json!({
        "code": W_CLIP_ADD_ENVELOPE_CODE,
        "message": "clip.add envelope",
        "details": {
            "clip_id": data.clip_id,
            "track_id": data.track_id,
        }
    })
}

/// Rebuild [`ClipAddData`] from the internal envelope warning.
///
/// # Errors
///
/// Returns [`ReconstructError`] if the envelope warning is absent or
/// malformed.
pub fn data_envelope_from_warnings(warnings: &[Value]) -> Result<ClipAddData, ReconstructError> {
    let details = envelope_details_from_warnings(warnings)?;
    Ok(ClipAddData {
        clip_id: details.clip_id,
        track_id: details.track_id,
    })
}

fn envelope_details_from_warnings(
    warnings: &[Value],
) -> Result<EnvelopeWarningDetails, ReconstructError> {
    for warning in warnings {
        if warning.get("code").and_then(Value::as_str) != Some(W_CLIP_ADD_ENVELOPE_CODE) {
            continue;
        }
        let details = warning
            .get("details")
            .ok_or(ReconstructError::MissingField {
                name: "warnings[].W_CLIP_ADD_ENVELOPE.details",
            })?;
        return serde_json::from_value(details.clone()).map_err(|_| {
            ReconstructError::TypeMismatch {
                name: "warnings[].W_CLIP_ADD_ENVELOPE.details",
                expected: "ClipAdd envelope details",
            }
        });
    }
    Err(ReconstructError::MissingField {
        name: "warnings[].W_CLIP_ADD_ENVELOPE",
    })
}

/// `clip.add` verb registration entry.
#[derive(Debug, Default)]
pub struct ClipAddVerb;

impl From<ClipAddError> for VerbError {
    fn from(value: ClipAddError) -> Self {
        VerbError::BadArgs {
            detail: value.to_string(),
        }
    }
}

impl Verb for ClipAddVerb {
    fn verb(&self) -> &'static str {
        "clip.add"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: ClipAddArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("clip.add: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!("clip.add: patch construction failed: {err}"))
            })?;

        let post_state = prior
            .apply(&patch)
            .map_err(|err| VerbError::InvariantViolation {
                detail: format!("clip.add: post-state validation failed: {err}"),
            })?;
        drop(post_state);

        let data = serde_json::to_value(&data)
            .map_err(|err| VerbError::Custom(format!("clip.add: data serialize failed: {err}")))?;

        Ok((patch, data, warnings))
    }

    fn reconstruct(
        &self,
        _args: &Value,
        _patch: &Value,
        warnings: &[Value],
        _post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let envelope = data_envelope_from_warnings(warnings)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
