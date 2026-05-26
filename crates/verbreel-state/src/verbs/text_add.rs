//! `text.add` (§7.1) — fifty-fourth production verb in the engine.
//!
//! Adds a text clip to an existing text track or auto-creates the first
//! text track when the project has none. This slice intentionally
//! defers the preset registry, font registry, and structural `text[N]`
//! selectors. Preset names return `E_PRESET_UNKNOWN`, any font string
//! in a literal style object is accepted, and `track` accepts only a
//! bare `UUIDv7` or `track:<uuid>`.

use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Map, Value, json};
use thiserror::Error;
use unicode_segmentation::UnicodeSegmentation;
use verbreel_types::{ClipId, ProjectId, TICK_RATE_HZ, Tick, TrackId};

use crate::clip::{BlendMode, Clip, FadeCurve};
use crate::invariants::timeline_duration_tk;
use crate::newtypes::AssetRef;
use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use crate::shadow::Shadow;
use crate::text_element::{TextAlign, TextElement};
use crate::track::{Track, TrackKind};
use crate::transform::Transform;
use crate::verbs::clip_set_fade::W_TIME_SNAPPED_CODE;
use crate::verbs::project_set_fps::is_off_frame;

/// Maximum accepted `content` length in Unicode scalar values.
pub const MAX_CONTENT_CHARS: usize = 8192;
/// Maximum accepted/derived clip name length in UAX #29 grapheme clusters.
pub const MAX_NAME_GRAPHEMES: usize = 128;
/// Maximum backup window for word-boundary truncation.
pub const MAX_BACKUP_CLUSTERS: usize = 16;
/// Internal warning code carrying minted IDs for reconstructor replay.
pub const W_TEXT_ADD_ENVELOPE_CODE: &str = "W_TEXT_ADD_ENVELOPE";
const PRESET_REGISTRY_DEFERRED_HINT: &str =
    "preset registry not implemented this slice; supply a literal style object";

const TEXT_STYLE_FIELDS: &[&str] = &[
    "content",
    "font_family",
    "font_size_px",
    "font_weight",
    "italic",
    "color",
    "bg_color",
    "stroke_color",
    "stroke_px",
    "align",
    "letter_spacing",
    "line_height",
    "shadow",
    "padding_px",
];

/// Args for `text.add`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextAddArgs {
    /// Target project id.
    pub project_id: ProjectId,
    /// Text content. Accepted length: 0..=8192 chars.
    pub content: String,
    /// Timeline start position in ticks before frame snapping.
    pub track_position_tk: i64,
    /// Text clip duration in ticks.
    pub duration_tk: i64,
    /// Optional text track selector: bare `UUIDv7` or `track:<uuid>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track: Option<String>,
    /// Optional preset name or literal partial `TextElement` object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<StyleArg>,
    /// Optional display name. Omitted names are derived from content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// `text.add` style argument: either a preset name or a partial style object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StyleArg {
    /// Preset name lookup. Preset registry is deferred in this slice.
    Preset(String),
    /// Partial [`TextElement`] object merged over defaults.
    Object(Map<String, Value>),
}

/// Envelope `data` returned by `text.add`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextAddData {
    /// Freshly minted text clip id.
    pub clip_id: ClipId,
    /// Existing or freshly minted text track id.
    pub text_track_id: TrackId,
}

/// Verb-level validation failures for `text.add`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TextAddError {
    /// String or number field violates schema bounds.
    #[error("E_SCHEMA_VIOLATION: text.add: field `{field}` value {value} violates bound {bound}")]
    SchemaViolation {
        /// Failing field.
        field: &'static str,
        /// Bound description.
        bound: &'static str,
        /// Observed value.
        value: String,
    },

    /// Style payload violates the `TextElement` schema subset enforced here.
    #[error("E_SCHEMA_VIOLATION: text.add: style schema violation: {detail}")]
    StyleSchemaViolation {
        /// Human-readable detail.
        detail: String,
    },

    /// `track_position_tk` is negative.
    #[error("E_BAD_TIME: text.add: `track_position_tk` value {value} must be >= 0")]
    BadTime {
        /// Invalid value.
        value: i64,
    },

    /// `track` selector failed to parse or used a deferred/foreign prefix.
    #[error("E_BAD_SELECTOR: text.add: `track` selector parse failed: {detail}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
    },

    /// Track selector parsed but resolved to no track.
    #[error("E_TRACK_NOT_FOUND: text.add: track `{track_id}` not found")]
    TrackNotFound {
        /// Missing track id.
        track_id: String,
    },

    /// Track selector resolved to a non-text track.
    #[error("E_TRACK_KIND_MISMATCH: text.add: track `{track_id}` is {found_kind:?}, not text")]
    TrackKindMismatch {
        /// Track id.
        track_id: String,
        /// Actual kind.
        found_kind: TrackKind,
    },

    /// Target text track is locked.
    #[error("E_LOCKED: text.add: track `{track_id}` is locked")]
    Locked {
        /// Locked track id.
        track_id: String,
    },

    /// Preset registry is out of scope for this slice.
    #[error("E_PRESET_UNKNOWN: text.add: preset `{preset}` unknown; {hint}")]
    PresetUnknown {
        /// Requested preset name.
        preset: String,
        /// Recovery hint.
        hint: &'static str,
    },

    /// Planned text clip would overlap an existing clip on its target track.
    #[error("E_CLIP_OVERLAP: text.add: new clip would overlap on track `{track_id}`")]
    ClipOverlap {
        /// Target track id.
        track_id: String,
    },
}

#[derive(Debug, Clone, Copy)]
struct TargetTrack {
    idx: usize,
    id: TrackId,
    auto_created: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EnvelopeWarningDetails {
    clip_id: ClipId,
    text_track_id: TrackId,
    auto_created_track: bool,
}

/// Build the RFC 6902 patch and envelope for `text.add`.
///
/// # Errors
///
/// Returns [`TextAddError`] for schema violations, unsupported preset
/// names, bad/missing/non-text track selectors, locked tracks, negative
/// positions, or clip overlap.
#[allow(clippy::too_many_lines)]
pub fn compute_patch(
    prior: &Project,
    args: &TextAddArgs,
) -> Result<(Value, Vec<Value>, TextAddData), TextAddError> {
    validate_content(&args.content)?;
    validate_duration(args.duration_tk)?;
    validate_position(args.track_position_tk)?;
    validate_name(args.name.as_deref())?;

    let mut text = resolve_style(args.style.as_ref())?;
    text.content.clone_from(&args.content);

    let (target, maybe_new_track) = resolve_target_track(prior, args.track.as_deref())?;
    let target_clips = if target.auto_created {
        &[][..]
    } else {
        prior.tracks[target.idx].clips.as_slice()
    };

    if !target.auto_created && prior.tracks[target.idx].locked {
        return Err(TextAddError::Locked {
            track_id: target.id.to_string(),
        });
    }

    let mut warnings = Vec::new();
    let snapped_position_tk = snap_position(prior, args.track_position_tk, &mut warnings);
    let derived_name = derive_name(args.name.as_deref(), &args.content, target_clips);
    let new_end_tk = snapped_position_tk.saturating_add(args.duration_tk);
    check_overlap(
        target_clips,
        snapped_position_tk,
        new_end_tk,
        target.id.to_string(),
    )?;

    let clip_id = ClipId::now();
    let clip = build_clip(
        clip_id,
        derived_name,
        text,
        snapped_position_tk,
        args.duration_tk,
    );

    let data = TextAddData {
        clip_id,
        text_track_id: target.id,
    };
    warnings.push(envelope_warning(&data, target.auto_created));
    let patch = build_patch(prior, target, maybe_new_track, &clip, new_end_tk);
    Ok((patch, warnings, data))
}

fn validate_content(content: &str) -> Result<(), TextAddError> {
    let len = content.chars().count();
    if len > MAX_CONTENT_CHARS {
        return Err(TextAddError::SchemaViolation {
            field: "content",
            bound: "8192",
            value: len.to_string(),
        });
    }
    Ok(())
}

fn validate_duration(duration_tk: i64) -> Result<(), TextAddError> {
    if duration_tk < 1 {
        return Err(TextAddError::SchemaViolation {
            field: "duration_tk",
            bound: ">= 1",
            value: duration_tk.to_string(),
        });
    }
    Ok(())
}

fn validate_position(position_tk: i64) -> Result<(), TextAddError> {
    if position_tk < 0 {
        return Err(TextAddError::BadTime { value: position_tk });
    }
    Ok(())
}

fn validate_name(name: Option<&str>) -> Result<(), TextAddError> {
    let Some(name) = name else {
        return Ok(());
    };
    let len = name.graphemes(true).count();
    if !(1..=MAX_NAME_GRAPHEMES).contains(&len) {
        return Err(TextAddError::SchemaViolation {
            field: "name",
            bound: "1..=128 grapheme clusters",
            value: len.to_string(),
        });
    }
    Ok(())
}

fn resolve_style(style: Option<&StyleArg>) -> Result<TextElement, TextAddError> {
    match style {
        None => Ok(TextElement::default()),
        Some(StyleArg::Preset(preset)) => Err(TextAddError::PresetUnknown {
            preset: preset.clone(),
            hint: PRESET_REGISTRY_DEFERRED_HINT,
        }),
        Some(StyleArg::Object(map)) => merge_style_object(map),
    }
}

fn merge_style_object(map: &Map<String, Value>) -> Result<TextElement, TextAddError> {
    for field in map.keys() {
        if !TEXT_STYLE_FIELDS.contains(&field.as_str()) {
            return Err(style_schema_violation(format!(
                "unknown style field `{field}`"
            )));
        }
    }

    validate_style_object(map)?;
    let mut value = serde_json::to_value(TextElement::default())
        .map_err(|err| style_schema_violation(err.to_string()))?;
    let object = value
        .as_object_mut()
        .expect("TextElement serializes to object");
    for (field, field_value) in map {
        object.insert(field.clone(), field_value.clone());
    }

    serde_json::from_value(value)
        .map_err(|err| style_schema_violation(format!("partial TextElement merge failed: {err}")))
}

fn validate_style_object(map: &Map<String, Value>) -> Result<(), TextAddError> {
    if let Some(value) = map.get("content") {
        let content: String = deserialize_style_leaf("content", value)?;
        let len = content.chars().count();
        if len > MAX_CONTENT_CHARS {
            return Err(style_schema_violation(format!(
                "field `content` length {len} exceeds max {MAX_CONTENT_CHARS}"
            )));
        }
    }
    if let Some(value) = map.get("font_family") {
        let _: String = deserialize_style_leaf("font_family", value)?;
    }
    if let Some(value) = map.get("font_size_px") {
        validate_min(
            "font_size_px",
            deserialize_style_leaf("font_size_px", value)?,
            1.0,
        )?;
    }
    if let Some(value) = map.get("font_weight") {
        let font_weight: u32 = deserialize_style_leaf("font_weight", value)?;
        if !(100..=900).contains(&font_weight) {
            return Err(style_schema_violation(format!(
                "field `font_weight` value {font_weight} is outside 100..=900"
            )));
        }
    }
    if let Some(value) = map.get("italic") {
        let _: bool = deserialize_style_leaf("italic", value)?;
    }
    if let Some(value) = map.get("color") {
        let _: crate::newtypes::Color = deserialize_style_leaf("color", value)?;
    }
    if let Some(value) = map.get("bg_color")
        && !value.is_null()
    {
        let _: crate::newtypes::Color = deserialize_style_leaf("bg_color", value)?;
    }
    if let Some(value) = map.get("stroke_color")
        && !value.is_null()
    {
        let _: crate::newtypes::Color = deserialize_style_leaf("stroke_color", value)?;
    }
    if let Some(value) = map.get("stroke_px") {
        validate_min(
            "stroke_px",
            deserialize_style_leaf("stroke_px", value)?,
            0.0,
        )?;
    }
    if let Some(value) = map.get("align") {
        let _: TextAlign = deserialize_style_leaf("align", value)?;
    }
    if let Some(value) = map.get("letter_spacing") {
        finite_f64(
            "letter_spacing",
            deserialize_style_leaf("letter_spacing", value)?,
        )?;
    }
    if let Some(value) = map.get("line_height") {
        validate_min(
            "line_height",
            deserialize_style_leaf("line_height", value)?,
            0.5,
        )?;
    }
    if let Some(value) = map.get("shadow")
        && !value.is_null()
    {
        let shadow: Shadow = deserialize_style_leaf("shadow", value)?;
        validate_shadow(&shadow)?;
    }
    if let Some(value) = map.get("padding_px") {
        validate_min(
            "padding_px",
            deserialize_style_leaf("padding_px", value)?,
            0.0,
        )?;
    }
    Ok(())
}

fn style_schema_violation(detail: impl Into<String>) -> TextAddError {
    TextAddError::StyleSchemaViolation {
        detail: detail.into(),
    }
}

fn deserialize_style_leaf<T>(field: &'static str, value: &Value) -> Result<T, TextAddError>
where
    T: DeserializeOwned,
{
    serde_json::from_value(value.clone())
        .map_err(|err| style_schema_violation(format!("field `{field}` is invalid: {err}")))
}

fn finite_f64(field: &'static str, value: f64) -> Result<f64, TextAddError> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(style_schema_violation(format!(
            "field `{field}` value {value} is not finite"
        )))
    }
}

fn validate_min(field: &'static str, value: f64, min: f64) -> Result<f64, TextAddError> {
    let value = finite_f64(field, value)?;
    if value < min {
        return Err(style_schema_violation(format!(
            "field `{field}` value {value} is below minimum {min}"
        )));
    }
    Ok(value)
}

fn validate_shadow(shadow: &Shadow) -> Result<(), TextAddError> {
    validate_min("shadow.blur_px", shadow.blur_px, 0.0)?;
    finite_f64("shadow.offset_x", shadow.offset_x)?;
    finite_f64("shadow.offset_y", shadow.offset_y)?;
    Ok(())
}

fn resolve_target_track(
    prior: &Project,
    track_selector: Option<&str>,
) -> Result<(TargetTrack, Option<Track>), TextAddError> {
    if let Some(selector) = track_selector {
        let track_id = parse_track_selector(selector)?;
        let (idx, track) = prior
            .tracks
            .iter()
            .enumerate()
            .find(|(_, track)| track.id == track_id)
            .ok_or_else(|| TextAddError::TrackNotFound {
                track_id: track_id.to_string(),
            })?;

        if track.kind != TrackKind::Text {
            return Err(TextAddError::TrackKindMismatch {
                track_id: track_id.to_string(),
                found_kind: track.kind,
            });
        }

        return Ok((
            TargetTrack {
                idx,
                id: track.id,
                auto_created: false,
            },
            None,
        ));
    }

    if let Some((idx, track)) = prior
        .tracks
        .iter()
        .enumerate()
        .find(|(_, track)| track.kind == TrackKind::Text)
    {
        return Ok((
            TargetTrack {
                idx,
                id: track.id,
                auto_created: false,
            },
            None,
        ));
    }

    let idx = insertion_idx_for_kind(prior, TrackKind::Text);
    let track = Track {
        id: TrackId::now(),
        kind: TrackKind::Text,
        name: auto_track_name(prior, TrackKind::Text),
        clips: Vec::new(),
        muted: false,
        solo: false,
        locked: false,
        hidden: false,
        volume: 1.0,
        pan: 0.0,
        effects: Vec::new(),
    };

    Ok((
        TargetTrack {
            idx,
            id: track.id,
            auto_created: true,
        },
        Some(track),
    ))
}

fn parse_track_selector(selector: &str) -> Result<TrackId, TextAddError> {
    if let Some((prefix, rest)) = selector.split_once(':') {
        if prefix != "track" {
            return Err(TextAddError::BadSelector {
                detail: format!("unsupported selector prefix `{prefix}`"),
            });
        }
        return rest
            .parse::<TrackId>()
            .map_err(|err| TextAddError::BadSelector {
                detail: err.to_string(),
            });
    }

    selector
        .parse::<TrackId>()
        .map_err(|err| TextAddError::BadSelector {
            detail: err.to_string(),
        })
}

fn insertion_idx_for_kind(prior: &Project, kind: TrackKind) -> usize {
    prior
        .tracks
        .iter()
        .enumerate()
        .filter(|(_, track)| track.kind == kind)
        .map(|(idx, _)| idx + 1)
        .next_back()
        .unwrap_or(prior.tracks.len())
}

fn auto_track_name(prior: &Project, kind: TrackKind) -> String {
    let label = kind_label(kind);
    let re = auto_track_name_regex(kind);
    let max_seen = prior
        .tracks
        .iter()
        .filter(|track| track.kind == kind)
        .filter_map(|track| re.captures(&track.name))
        .filter_map(|captures| captures.get(1)?.as_str().parse::<usize>().ok())
        .max()
        .unwrap_or(0);
    format!("{label} {}", max_seen + 1)
}

fn kind_label(kind: TrackKind) -> &'static str {
    match kind {
        TrackKind::Video => "Video",
        TrackKind::Audio => "Audio",
        TrackKind::Text => "Text",
        TrackKind::Effect => "Effect",
    }
}

fn auto_track_name_regex(kind: TrackKind) -> &'static Regex {
    match kind {
        TrackKind::Video => {
            static RE: OnceLock<Regex> = OnceLock::new();
            RE.get_or_init(|| Regex::new(r"^Video (0|[1-9][0-9]*)$").expect("valid regex"))
        }
        TrackKind::Audio => {
            static RE: OnceLock<Regex> = OnceLock::new();
            RE.get_or_init(|| Regex::new(r"^Audio (0|[1-9][0-9]*)$").expect("valid regex"))
        }
        TrackKind::Text => {
            static RE: OnceLock<Regex> = OnceLock::new();
            RE.get_or_init(|| Regex::new(r"^Text (0|[1-9][0-9]*)$").expect("valid regex"))
        }
        TrackKind::Effect => {
            static RE: OnceLock<Regex> = OnceLock::new();
            RE.get_or_init(|| Regex::new(r"^Effect (0|[1-9][0-9]*)$").expect("valid regex"))
        }
    }
}

fn snap_position(prior: &Project, value_tk: i64, warnings: &mut Vec<Value>) -> i64 {
    if !is_off_frame(Tick::new(value_tk), prior.fps_num, prior.fps_den) {
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

fn derive_name(explicit: Option<&str>, content: &str, clips: &[Clip]) -> String {
    if let Some(name) = explicit {
        return name.to_string();
    }

    let normalized = content.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return auto_clip_name(clips);
    }

    truncate_name(&normalized)
}

fn truncate_name(name: &str) -> String {
    let graphemes = name.grapheme_indices(true).collect::<Vec<_>>();
    if graphemes.len() <= MAX_NAME_GRAPHEMES {
        return name.to_string();
    }

    let hard_cut = graphemes
        .get(MAX_NAME_GRAPHEMES)
        .map_or(name.len(), |(idx, _)| *idx);
    if name.is_char_boundary(hard_cut) && is_word_boundary(name, hard_cut) {
        return name[..hard_cut].trim_end().to_string();
    }

    let min_cluster = MAX_NAME_GRAPHEMES.saturating_sub(MAX_BACKUP_CLUSTERS);
    let min_byte = graphemes.get(min_cluster).map_or(0, |(idx, _)| *idx);
    if let Some(boundary) = previous_word_boundary(name, hard_cut, min_byte) {
        let candidate = name[..boundary].trim_end();
        if !candidate.is_empty() {
            return candidate.to_string();
        }
    }

    name[..hard_cut].trim_end().to_string()
}

fn is_word_boundary(value: &str, byte_idx: usize) -> bool {
    value
        .split_word_bound_indices()
        .any(|(idx, _)| idx == byte_idx)
        || byte_idx == value.len()
}

fn previous_word_boundary(value: &str, hard_cut: usize, min_byte: usize) -> Option<usize> {
    value
        .split_word_bound_indices()
        .map(|(idx, _)| idx)
        .rfind(|idx| *idx < hard_cut && *idx >= min_byte)
}

fn auto_clip_name(clips: &[Clip]) -> String {
    let re = text_clip_name_regex();
    let max_seen = clips
        .iter()
        .filter_map(|clip| re.captures(&clip.name))
        .filter_map(|captures| captures.get(1)?.as_str().parse::<usize>().ok())
        .max()
        .unwrap_or(0);
    format!("Text {}", max_seen + 1)
}

fn text_clip_name_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^Text (0|[1-9][0-9]*)$").expect("valid regex"))
}

fn check_overlap(
    clips: &[Clip],
    new_start_tk: i64,
    new_end_tk: i64,
    track_id: String,
) -> Result<(), TextAddError> {
    for clip in clips {
        let start_tk = clip.track_position_tk.get();
        let duration_tk = timeline_duration_tk(clip.source_in_tk, clip.source_out_tk, clip.speed);
        let end_tk = start_tk.saturating_add(duration_tk.get());
        if intervals_overlap(new_start_tk, new_end_tk, start_tk, end_tk) {
            return Err(TextAddError::ClipOverlap { track_id });
        }
    }
    Ok(())
}

fn intervals_overlap(a_start: i64, a_end: i64, b_start: i64, b_end: i64) -> bool {
    a_start < b_end && b_start < a_end
}

fn build_clip(
    id: ClipId,
    name: String,
    text: TextElement,
    track_position_tk: i64,
    duration_tk: i64,
) -> Clip {
    Clip {
        id,
        name,
        asset_id: AssetRef::nil(),
        track_position_tk: Tick::new(track_position_tk),
        source_in_tk: Tick::ZERO,
        source_out_tk: Tick::new(duration_tk),
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
        text: Some(text),
        locked: false,
        link_group: None,
        blend_mode: BlendMode::Normal,
        mask: None,
        speed_curve: None,
    }
}

fn build_patch(
    prior: &Project,
    target: TargetTrack,
    new_track: Option<Track>,
    clip: &Clip,
    new_end_tk: i64,
) -> Value {
    let mut ops = Vec::new();
    if let Some(track) = new_track {
        ops.push(json!({
            "op": "add",
            "path": format!("/tracks/{}", target.idx),
            "value": track,
        }));
    }
    ops.push(json!({
        "op": "add",
        "path": format!("/tracks/{}/clips/-", target.idx),
        "value": clip,
    }));
    if new_end_tk > prior.duration_tk.get() {
        ops.push(json!({
            "op": "replace",
            "path": "/duration_tk",
            "value": new_end_tk,
        }));
    }
    Value::Array(ops)
}

fn envelope_warning(data: &TextAddData, auto_created_track: bool) -> Value {
    json!({
        "code": W_TEXT_ADD_ENVELOPE_CODE,
        "message": "text.add envelope",
        "details": {
            "clip_id": data.clip_id,
            "text_track_id": data.text_track_id,
            "auto_created_track": auto_created_track,
        }
    })
}

/// Rebuild `TextAddData` from the recorded internal envelope warning.
///
/// # Errors
///
/// Returns [`ReconstructError`] if the internal warning is absent or
/// malformed.
pub fn data_envelope_from_warnings(warnings: &[Value]) -> Result<TextAddData, ReconstructError> {
    let details = envelope_details_from_warnings(warnings)?;
    Ok(TextAddData {
        clip_id: details.clip_id,
        text_track_id: details.text_track_id,
    })
}

fn envelope_details_from_warnings(
    warnings: &[Value],
) -> Result<EnvelopeWarningDetails, ReconstructError> {
    for warning in warnings {
        if warning.get("code").and_then(Value::as_str) != Some(W_TEXT_ADD_ENVELOPE_CODE) {
            continue;
        }
        let details = warning
            .get("details")
            .ok_or(ReconstructError::MissingField {
                name: "warnings[].W_TEXT_ADD_ENVELOPE.details",
            })?;
        return serde_json::from_value(details.clone()).map_err(|_| {
            ReconstructError::TypeMismatch {
                name: "warnings[].W_TEXT_ADD_ENVELOPE.details",
                expected: "TextAdd envelope details",
            }
        });
    }

    Err(ReconstructError::MissingField {
        name: "warnings[].W_TEXT_ADD_ENVELOPE",
    })
}

/// `text.add` verb registration entry.
#[derive(Debug, Default)]
pub struct TextAddVerb;

impl From<TextAddError> for VerbError {
    fn from(value: TextAddError) -> Self {
        VerbError::BadArgs {
            detail: value.to_string(),
        }
    }
}

impl Verb for TextAddVerb {
    fn verb(&self) -> &'static str {
        "text.add"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: TextAddArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("text.add: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!("text.add: patch construction failed: {err}"))
            })?;

        let post_state = prior
            .apply(&patch)
            .map_err(|err| VerbError::InvariantViolation {
                detail: format!("text.add: post-state validation failed: {err}"),
            })?;
        drop(post_state);

        let data = serde_json::to_value(&data)
            .map_err(|err| VerbError::Custom(format!("text.add: data serialize failed: {err}")))?;

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
