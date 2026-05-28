//! `text.style` (§7.3) — thirty-second production verb in the engine.
//!
//! ## Spec quote (`spec/commands/text.md` §7.3, summarized)
//!
//! `text.style` accepts `style: Partial<TextElement> | { shadow: null }`
//! and updates text rendering fields on a text clip. `shadow: null` is
//! an args-layer sentinel that removes the optional `shadow` field from
//! project state; persisted `TextElement.shadow` is non-null when present.
//!
//! `E_ARGS_INCOMPATIBLE` for CLI flags (`--no_shadow` + `--shadow_*`) is
//! a verb-args/CLI responsibility. The state layer receives a resolved
//! `style` object where `shadow` is absent, null, or an object.

use crate::font_registry;
use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use crate::shadow::Shadow;
use crate::text_element::{TextAlign, TextElement};
use crate::track::TrackKind;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::{ClipId, ProjectId};

/// Warning code emitted when incoming style leaves do not change state.
pub const W_NOOP_CODE: &str = "W_NOOP";

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

/// Args for `text.style`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextStyleArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Target clip id as bare `UUIDv7`.
    pub clip: String,

    /// Partial [`TextElement`] update object.
    ///
    /// `style.shadow = null` is handled manually as the §7.3 remove
    /// sentinel. All other fields are deserialized per leaf so absent
    /// keys mean "leave as-is".
    pub style: Value,
}

/// Envelope `data` returned by `text.style`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextStyleData {
    /// Target clip id.
    pub clip_id: ClipId,

    /// Full text element in post-state.
    pub text: TextElement,
}

/// Verb-level validation failures for `text.style`.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum TextStyleError {
    /// `args.clip` is not parseable as `UUIDv7`.
    #[error("text.style: `clip` selector parse failed: {detail}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
    },

    /// No clip exists for `clip`.
    #[error("text.style: clip `{clip_id}` not found")]
    ClipNotFound {
        /// Missing clip id string.
        clip_id: String,
    },

    /// Target clip is on a non-text track.
    #[error("text.style: clip `{clip_id}` is on a {found_kind:?} track, not text")]
    ClipKindMismatch {
        /// Target clip id string.
        clip_id: String,

        /// Actual track kind.
        found_kind: TrackKind,
    },

    /// The target clip is locked.
    #[error("text.style: clip `{clip_id}` is locked")]
    Locked {
        /// Locked clip id.
        clip_id: String,
    },

    /// Style payload violates the text/shadow schema.
    #[error("text.style: style schema violation: {detail}")]
    SchemaViolation {
        /// Human-readable detail.
        detail: String,
    },

    /// `font_family` is not present in the canonical registry.
    #[error(
        "E_FONT_UNKNOWN: text.style: font family `{family}` is unavailable; details.available={available:?}"
    )]
    FontUnknown {
        /// Requested family.
        family: String,
        /// Canonical available family names.
        available: Vec<String>,
    },
}

fn schema_violation(detail: impl Into<String>) -> TextStyleError {
    TextStyleError::SchemaViolation {
        detail: detail.into(),
    }
}

fn deserialize_leaf<T>(field: &'static str, value: &Value) -> Result<T, TextStyleError>
where
    T: DeserializeOwned,
{
    serde_json::from_value(value.clone())
        .map_err(|err| schema_violation(format!("field `{field}` is invalid: {err}")))
}

fn finite_f64(field: &'static str, value: f64) -> Result<f64, TextStyleError> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(schema_violation(format!(
            "field `{field}` value {value} is not finite"
        )))
    }
}

fn validate_min(field: &'static str, value: f64, min: f64) -> Result<f64, TextStyleError> {
    let value = finite_f64(field, value)?;
    if value < min {
        return Err(schema_violation(format!(
            "field `{field}` value {value} is below minimum {min}"
        )));
    }
    Ok(value)
}

fn validate_shadow(shadow: &Shadow) -> Result<(), TextStyleError> {
    validate_min("shadow.blur_px", shadow.blur_px, 0.0)?;
    finite_f64("shadow.offset_x", shadow.offset_x)?;
    finite_f64("shadow.offset_y", shadow.offset_y)?;
    Ok(())
}

fn resolve_font_family(font_family: &str) -> Result<String, TextStyleError> {
    font_registry::resolve(font_family)
        .map(|family| family.name)
        .ok_or_else(|| TextStyleError::FontUnknown {
            family: font_family.to_string(),
            available: font_registry::available(),
        })
}

fn f64_changed(next: f64, current: f64) -> bool {
    #[allow(clippy::float_cmp)]
    {
        next != current
    }
}

fn no_op_warning(clip_id: ClipId, style: &Value) -> Value {
    json!({
        "code": W_NOOP_CODE,
        "message": "text style unchanged",
        "details": {
            "verb": "text.style",
            "clip_id": clip_id.to_string(),
            "style": style,
        }
    })
}

fn push_text_op<T>(
    ops: &mut Vec<Value>,
    t_idx: usize,
    c_idx: usize,
    field: &'static str,
    value: &T,
) -> Result<(), TextStyleError>
where
    T: Serialize,
{
    let value = serde_json::to_value(value)
        .map_err(|err| schema_violation(format!("field `{field}` serialization failed: {err}")))?;
    ops.push(json!({
        "op": "replace",
        "path": format!("/tracks/{t_idx}/clips/{c_idx}/text/{field}"),
        "value": value,
    }));
    Ok(())
}

fn push_optional_text_op<T>(
    ops: &mut Vec<Value>,
    t_idx: usize,
    c_idx: usize,
    field: &'static str,
    exists: bool,
    value: &T,
) -> Result<(), TextStyleError>
where
    T: Serialize,
{
    let value = serde_json::to_value(value)
        .map_err(|err| schema_violation(format!("field `{field}` serialization failed: {err}")))?;
    ops.push(json!({
        "op": if exists { "replace" } else { "add" },
        "path": format!("/tracks/{t_idx}/clips/{c_idx}/text/{field}"),
        "value": value,
    }));
    Ok(())
}

/// Build the RFC-6902 patch for `text.style`.
///
/// # Errors
///
/// - [`TextStyleError::BadSelector`] for non-UUIDv7 `args.clip`.
/// - [`TextStyleError::ClipNotFound`] if `args.clip` resolves to no clip.
/// - [`TextStyleError::ClipKindMismatch`] if clip parent track is not text.
/// - [`TextStyleError::Locked`] if target clip is locked.
/// - [`TextStyleError::SchemaViolation`] if any present style leaf fails
///   the text/shadow schema constraints enforced by this slice.
/// - idempotent no-op path: empty patch + [`W_NOOP_CODE`] warning when
///   no supplied leaf changes current state.
#[allow(clippy::too_many_lines)]
pub fn compute_patch(
    prior: &Project,
    args: &TextStyleArgs,
) -> Result<(Value, Vec<Value>, TextStyleData), TextStyleError> {
    let clip_id = args
        .clip
        .parse::<ClipId>()
        .map_err(|err| TextStyleError::BadSelector {
            detail: err.to_string(),
        })?;

    let mut location: Option<(usize, usize, &crate::track::Track, &crate::clip::Clip)> = None;
    for (t_idx, track) in prior.tracks.iter().enumerate() {
        for (c_idx, clip) in track.clips.iter().enumerate() {
            if clip.id == clip_id {
                location = Some((t_idx, c_idx, track, clip));
                break;
            }
        }
        if location.is_some() {
            break;
        }
    }

    let (t_idx, c_idx, track, clip) = location.ok_or_else(|| TextStyleError::ClipNotFound {
        clip_id: args.clip.clone(),
    })?;

    if track.kind != TrackKind::Text {
        return Err(TextStyleError::ClipKindMismatch {
            clip_id: args.clip.clone(),
            found_kind: track.kind,
        });
    }

    if clip.locked {
        return Err(TextStyleError::Locked {
            clip_id: args.clip.clone(),
        });
    }

    let style = args
        .style
        .as_object()
        .ok_or_else(|| schema_violation("`style` must be an object"))?;
    for field in style.keys() {
        if !TEXT_STYLE_FIELDS.contains(&field.as_str()) {
            return Err(schema_violation(format!("unknown style field `{field}`")));
        }
    }

    let current_text = clip
        .text
        .as_ref()
        .ok_or_else(|| schema_violation(format!("clip `{clip_id}` has no text element")))?;
    let mut next_text = current_text.clone();
    let mut ops = Vec::new();

    if let Some(value) = style.get("content") {
        let content: String = deserialize_leaf("content", value)?;
        let content_len = content.chars().count();
        if content_len > super::text_edit::MAX_CONTENT_LEN {
            return Err(schema_violation(format!(
                "field `content` length {content_len} exceeds max {}",
                super::text_edit::MAX_CONTENT_LEN
            )));
        }
        if content != current_text.content {
            push_text_op(&mut ops, t_idx, c_idx, "content", &content)?;
            next_text.content = content;
        }
    }

    if let Some(value) = style.get("font_family") {
        let font_family: String = deserialize_leaf("font_family", value)?;
        let font_family = resolve_font_family(&font_family)?;
        if font_family != current_text.font_family {
            push_text_op(&mut ops, t_idx, c_idx, "font_family", &font_family)?;
            next_text.font_family = font_family;
        }
    }

    if let Some(value) = style.get("font_size_px") {
        let font_size_px = validate_min(
            "font_size_px",
            deserialize_leaf("font_size_px", value)?,
            1.0,
        )?;
        if f64_changed(font_size_px, current_text.font_size_px) {
            push_text_op(&mut ops, t_idx, c_idx, "font_size_px", &font_size_px)?;
            next_text.font_size_px = font_size_px;
        }
    }

    if let Some(value) = style.get("font_weight") {
        let font_weight: u32 = deserialize_leaf("font_weight", value)?;
        if !(100..=900).contains(&font_weight) {
            return Err(schema_violation(format!(
                "field `font_weight` value {font_weight} is outside 100..=900"
            )));
        }
        if font_weight != current_text.font_weight {
            push_text_op(&mut ops, t_idx, c_idx, "font_weight", &font_weight)?;
            next_text.font_weight = font_weight;
        }
    }

    if let Some(value) = style.get("italic") {
        let italic: bool = deserialize_leaf("italic", value)?;
        if italic != current_text.italic {
            push_text_op(&mut ops, t_idx, c_idx, "italic", &italic)?;
            next_text.italic = italic;
        }
    }

    if let Some(value) = style.get("color") {
        let color = deserialize_leaf("color", value)?;
        if color != current_text.color {
            push_text_op(&mut ops, t_idx, c_idx, "color", &color)?;
            next_text.color = color;
        }
    }

    if let Some(value) = style.get("bg_color") {
        let bg_color = deserialize_leaf("bg_color", value)?;
        if current_text.bg_color.as_ref() != Some(&bg_color) {
            push_optional_text_op(
                &mut ops,
                t_idx,
                c_idx,
                "bg_color",
                current_text.bg_color.is_some(),
                &bg_color,
            )?;
            next_text.bg_color = Some(bg_color);
        }
    }

    if let Some(value) = style.get("stroke_color") {
        let stroke_color = deserialize_leaf("stroke_color", value)?;
        if current_text.stroke_color.as_ref() != Some(&stroke_color) {
            push_optional_text_op(
                &mut ops,
                t_idx,
                c_idx,
                "stroke_color",
                current_text.stroke_color.is_some(),
                &stroke_color,
            )?;
            next_text.stroke_color = Some(stroke_color);
        }
    }

    if let Some(value) = style.get("stroke_px") {
        let stroke_px = validate_min("stroke_px", deserialize_leaf("stroke_px", value)?, 0.0)?;
        if f64_changed(stroke_px, current_text.stroke_px) {
            push_text_op(&mut ops, t_idx, c_idx, "stroke_px", &stroke_px)?;
            next_text.stroke_px = stroke_px;
        }
    }

    if let Some(value) = style.get("align") {
        let align: TextAlign = deserialize_leaf("align", value)?;
        if align != current_text.align {
            push_text_op(&mut ops, t_idx, c_idx, "align", &align)?;
            next_text.align = align;
        }
    }

    if let Some(value) = style.get("letter_spacing") {
        let letter_spacing =
            finite_f64("letter_spacing", deserialize_leaf("letter_spacing", value)?)?;
        if f64_changed(letter_spacing, current_text.letter_spacing) {
            push_text_op(&mut ops, t_idx, c_idx, "letter_spacing", &letter_spacing)?;
            next_text.letter_spacing = letter_spacing;
        }
    }

    if let Some(value) = style.get("line_height") {
        let line_height =
            validate_min("line_height", deserialize_leaf("line_height", value)?, 0.5)?;
        if f64_changed(line_height, current_text.line_height) {
            push_text_op(&mut ops, t_idx, c_idx, "line_height", &line_height)?;
            next_text.line_height = line_height;
        }
    }

    if let Some(value) = style.get("shadow") {
        if value.is_null() {
            if current_text.shadow.is_some() {
                ops.push(json!({
                    "op": "remove",
                    "path": format!("/tracks/{t_idx}/clips/{c_idx}/text/shadow"),
                }));
                next_text.shadow = None;
            }
        } else {
            let shadow: Shadow = deserialize_leaf("shadow", value)?;
            validate_shadow(&shadow)?;
            if current_text.shadow.as_ref() != Some(&shadow) {
                push_optional_text_op(
                    &mut ops,
                    t_idx,
                    c_idx,
                    "shadow",
                    current_text.shadow.is_some(),
                    &shadow,
                )?;
                next_text.shadow = Some(shadow);
            }
        }
    }

    if let Some(value) = style.get("padding_px") {
        let padding_px = validate_min("padding_px", deserialize_leaf("padding_px", value)?, 0.0)?;
        if f64_changed(padding_px, current_text.padding_px) {
            push_text_op(&mut ops, t_idx, c_idx, "padding_px", &padding_px)?;
            next_text.padding_px = padding_px;
        }
    }

    // TODO(text.style): spec/commands/text.md §7.3 `E_ARGS_INCOMPATIBLE`
    // is CLI/verb-args scope; this state-layer verb receives a fully
    // resolved `style` object and cannot observe conflicting flags.
    if ops.is_empty() {
        return Ok((
            json!([]),
            vec![no_op_warning(clip_id, &args.style)],
            TextStyleData {
                clip_id,
                text: current_text.clone(),
            },
        ));
    }

    Ok((
        Value::Array(ops),
        Vec::new(),
        TextStyleData {
            clip_id,
            text: next_text,
        },
    ))
}

/// Rebuilds the envelope from `(args, post_state)`.
///
/// # Errors
///
/// Returns [`ReconstructError::TypeMismatch`] when `args.clip` is not a
/// valid `UUIDv7`, or [`ReconstructError::PostStateMissing`] when the
/// post-state does not contain the target clip or clip text.
pub fn data_envelope_from_post_state(
    args: &TextStyleArgs,
    post_state: &Project,
) -> Result<TextStyleData, ReconstructError> {
    let clip_id = args
        .clip
        .parse::<ClipId>()
        .map_err(|_| ReconstructError::TypeMismatch {
            name: "args.clip",
            expected: "UUIDv7 ClipId string",
        })?;

    for track in &post_state.tracks {
        for clip in &track.clips {
            if clip.id == clip_id {
                let text = clip
                    .text
                    .clone()
                    .ok_or_else(|| ReconstructError::PostStateMissing {
                        detail: format!("text.style: clip {clip_id} has no text element"),
                    })?;
                return Ok(TextStyleData { clip_id, text });
            }
        }
    }

    Err(ReconstructError::PostStateMissing {
        detail: format!("text.style: clip {clip_id} not found in post_state"),
    })
}

/// `text.style` verb registration entry.
#[derive(Debug, Default)]
pub struct TextStyleVerb;

impl From<TextStyleError> for VerbError {
    fn from(value: TextStyleError) -> Self {
        match value {
            TextStyleError::BadSelector { .. }
            | TextStyleError::ClipNotFound { .. }
            | TextStyleError::ClipKindMismatch { .. }
            | TextStyleError::Locked { .. }
            | TextStyleError::SchemaViolation { .. }
            | TextStyleError::FontUnknown { .. } => VerbError::BadArgs {
                detail: value.to_string(),
            },
        }
    }
}

impl Verb for TextStyleVerb {
    fn verb(&self) -> &'static str {
        "text.style"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: TextStyleArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("text.style: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, _data) = compute_patch(prior, &typed)?;

        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!("text.style: patch construction failed: {err}"))
            })?;

        let post_state = prior
            .apply(&patch)
            .map_err(|err| VerbError::InvariantViolation {
                detail: format!("text.style: post-state validation failed: {err}"),
            })?;

        let envelope = data_envelope_from_post_state(&typed, &post_state).map_err(|err| {
            VerbError::Custom(format!(
                "text.style: data envelope reconstruction failed: {err}"
            ))
        })?;

        Ok((
            patch,
            serde_json::to_value(&envelope).map_err(|err| {
                VerbError::Custom(format!(
                    "text.style: data envelope serialization failed: {err}"
                ))
            })?,
            warnings,
        ))
    }

    fn reconstruct(
        &self,
        args: &Value,
        _patch: &Value,
        _warnings: &[Value],
        post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let typed: TextStyleArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "TextStyleArgs",
            })?;

        let envelope = data_envelope_from_post_state(&typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
