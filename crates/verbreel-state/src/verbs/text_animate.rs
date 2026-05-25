//! `text.animate` (§7.4) — thirty-sixth production verb in the engine.
//!
//! ## Spec quote (`spec/commands/text.md` §7.4, summarized)
//!
//! `text.animate` adds a preset animation to a text clip by expanding
//! the preset into a batch of clip-scoped `keyframe.add` operations.
//! Optional `in_tk` / `out_tk` arguments scope the preset to a
//! clip-relative window and are pair-required.
//!
//! ## Preset collision policy
//!
//! This v1 implementation rejects collisions with existing keyframes
//! using [`TextAnimateError::Duplicate`]. The spec allows an overwrite
//! interpretation, but rejecting keeps `text.animate` aligned with
//! `keyframe.add` uniqueness: agents can clear pre-existing keyframes
//! explicitly before applying a preset.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::{ClipId, KeyframeId, ProjectId, Tick, TrackId};

use crate::invariants::timeline_duration_tk;
use crate::keyframe::{Easing, Keyframe, KeyframeProperty};
use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use crate::track::TrackKind;

/// Warning code emitted when preset keyframes are clamped to the clip
/// end tick.
pub const W_PRESET_KEYFRAMES_CLAMPED: &str = "W_PRESET_KEYFRAMES_CLAMPED";

/// Warning code emitted when two preset entries collapse onto the same
/// `(property, time_tk)` and the later entry is kept.
pub const W_KEYFRAMES_DEDUPED: &str = "W_KEYFRAMES_DEDUPED";

/// v1 text animation preset definitions.
///
/// Preset values are v1 best-effort; tweakable in future patches. The
/// slide presets use a fixed 1920px offscreen distance because §7.4
/// does not surface canvas-size parameters. Consumers can chain a
/// `transform.scale` keyframe set or use cubic-bezier easing for
/// smoother feel. `typewriter` is a coarse stepped-opacity simulation
/// until per-character timing exists in `TextElement`.
pub mod presets {
    use super::Easing;

    /// One keyframe template entry in a preset.
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub struct PresetTemplate {
        /// Fractional position in the caller's `[in_tk, out_tk]`
        /// window.
        pub fraction: f64,
        /// Keyframed clip property.
        pub property: &'static str,
        /// Numeric keyframe value.
        pub value: f64,
        /// Keyframe easing.
        pub easing: Easing,
    }

    /// Fixed offscreen travel distance used by slide presets.
    pub const OFFSCREEN_DISTANCE_PX: f64 = 1920.0;

    /// `fade_in` preset templates.
    pub const FADE_IN: &[PresetTemplate] = &[
        PresetTemplate {
            fraction: 0.0,
            property: "opacity",
            value: 0.0,
            easing: Easing::EaseOut,
        },
        PresetTemplate {
            fraction: 1.0,
            property: "opacity",
            value: 1.0,
            easing: Easing::EaseOut,
        },
    ];

    /// `fade_out` preset templates.
    pub const FADE_OUT: &[PresetTemplate] = &[
        PresetTemplate {
            fraction: 0.0,
            property: "opacity",
            value: 1.0,
            easing: Easing::EaseIn,
        },
        PresetTemplate {
            fraction: 1.0,
            property: "opacity",
            value: 0.0,
            easing: Easing::EaseIn,
        },
    ];

    /// `pop` preset templates.
    pub const POP: &[PresetTemplate] = &[
        PresetTemplate {
            fraction: 0.0,
            property: "transform.scale_x",
            value: 0.0,
            easing: Easing::EaseOut,
        },
        PresetTemplate {
            fraction: 1.0,
            property: "transform.scale_x",
            value: 1.0,
            easing: Easing::EaseOut,
        },
        PresetTemplate {
            fraction: 0.0,
            property: "transform.scale_y",
            value: 0.0,
            easing: Easing::EaseOut,
        },
        PresetTemplate {
            fraction: 1.0,
            property: "transform.scale_y",
            value: 1.0,
            easing: Easing::EaseOut,
        },
    ];

    /// `slide_left` preset templates.
    pub const SLIDE_LEFT: &[PresetTemplate] = &[
        PresetTemplate {
            fraction: 0.0,
            property: "transform.x",
            value: -OFFSCREEN_DISTANCE_PX,
            easing: Easing::EaseOut,
        },
        PresetTemplate {
            fraction: 1.0,
            property: "transform.x",
            value: 0.0,
            easing: Easing::EaseOut,
        },
    ];

    /// `slide_right` preset templates.
    pub const SLIDE_RIGHT: &[PresetTemplate] = &[
        PresetTemplate {
            fraction: 0.0,
            property: "transform.x",
            value: OFFSCREEN_DISTANCE_PX,
            easing: Easing::EaseOut,
        },
        PresetTemplate {
            fraction: 1.0,
            property: "transform.x",
            value: 0.0,
            easing: Easing::EaseOut,
        },
    ];

    /// `slide_up` preset templates.
    pub const SLIDE_UP: &[PresetTemplate] = &[
        PresetTemplate {
            fraction: 0.0,
            property: "transform.y",
            value: -OFFSCREEN_DISTANCE_PX,
            easing: Easing::EaseOut,
        },
        PresetTemplate {
            fraction: 1.0,
            property: "transform.y",
            value: 0.0,
            easing: Easing::EaseOut,
        },
    ];

    /// `slide_down` preset templates.
    pub const SLIDE_DOWN: &[PresetTemplate] = &[
        PresetTemplate {
            fraction: 0.0,
            property: "transform.y",
            value: OFFSCREEN_DISTANCE_PX,
            easing: Easing::EaseOut,
        },
        PresetTemplate {
            fraction: 1.0,
            property: "transform.y",
            value: 0.0,
            easing: Easing::EaseOut,
        },
    ];

    /// `typewriter` preset templates.
    pub const TYPEWRITER: &[PresetTemplate] = &[
        PresetTemplate {
            fraction: 0.0,
            property: "opacity",
            value: 0.0,
            easing: Easing::Step,
        },
        PresetTemplate {
            fraction: 0.25,
            property: "opacity",
            value: 0.25,
            easing: Easing::Step,
        },
        PresetTemplate {
            fraction: 0.5,
            property: "opacity",
            value: 0.5,
            easing: Easing::Step,
        },
        PresetTemplate {
            fraction: 0.75,
            property: "opacity",
            value: 0.75,
            easing: Easing::Step,
        },
        PresetTemplate {
            fraction: 1.0,
            property: "opacity",
            value: 1.0,
            easing: Easing::Step,
        },
    ];

    /// `bounce` preset templates.
    pub const BOUNCE: &[PresetTemplate] = &[
        PresetTemplate {
            fraction: 0.0,
            property: "transform.y",
            value: 0.0,
            easing: Easing::EaseOut,
        },
        PresetTemplate {
            fraction: 0.3,
            property: "transform.y",
            value: -50.0,
            easing: Easing::EaseOut,
        },
        PresetTemplate {
            fraction: 0.55,
            property: "transform.y",
            value: -25.0,
            easing: Easing::EaseIn,
        },
        PresetTemplate {
            fraction: 0.75,
            property: "transform.y",
            value: -10.0,
            easing: Easing::EaseOut,
        },
        PresetTemplate {
            fraction: 1.0,
            property: "transform.y",
            value: 0.0,
            easing: Easing::EaseIn,
        },
    ];

    /// Return the templates for a registered v1 preset name.
    #[must_use]
    pub fn get(name: &str) -> Option<&'static [PresetTemplate]> {
        match name {
            "fade_in" => Some(FADE_IN),
            "fade_out" => Some(FADE_OUT),
            "pop" => Some(POP),
            "slide_left" => Some(SLIDE_LEFT),
            "slide_right" => Some(SLIDE_RIGHT),
            "slide_up" => Some(SLIDE_UP),
            "slide_down" => Some(SLIDE_DOWN),
            "typewriter" => Some(TYPEWRITER),
            "bounce" => Some(BOUNCE),
            _ => None,
        }
    }
}

/// Arguments for `text.animate`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextAnimateArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Target clip id as bare `UUIDv7`.
    pub clip: String,

    /// v1 preset name.
    pub preset: String,

    /// Optional inclusive preset-window start tick.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_tk: Option<i64>,

    /// Optional exclusive preset-window end tick.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out_tk: Option<i64>,
}

/// Envelope returned by `text.animate`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextAnimateData {
    /// Target text clip id.
    pub clip_id: ClipId,

    /// IDs of added keyframes, sorted by UUID string.
    pub added_keyframe_ids: Vec<KeyframeId>,

    /// IDs of added keyframes whose times were clamped, sorted by UUID
    /// string.
    pub clamped_keyframe_ids: Vec<KeyframeId>,
}

/// Verb-level validation failures for `text.animate`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TextAnimateError {
    /// `args.clip` is not parseable as `UUIDv7`.
    #[error("text.animate: `clip` selector parse failed: {detail}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
    },

    /// No clip exists for `clip`.
    #[error("text.animate: clip `{clip_id}` not found")]
    ClipNotFound {
        /// Missing clip id string.
        clip_id: String,
    },

    /// Selector resolved to a track, not a clip.
    #[error("text.animate: selector `{selector}` resolved to a track, not a clip")]
    SelectorKindMismatch {
        /// Offending selector string.
        selector: String,
    },

    /// Target clip is on a non-text track.
    #[error("text.animate: clip `{clip_id}` is on a {found_kind:?} track, not text")]
    ClipKindMismatch {
        /// Target clip id string.
        clip_id: String,
        /// Actual track kind.
        found_kind: TrackKind,
    },

    /// Parent track or target clip is locked.
    #[error("text.animate: {kind} `{id}` is locked")]
    Locked {
        /// Locked entity kind.
        kind: &'static str,
        /// Locked entity id.
        id: String,
    },

    /// Pair-required or other argument-schema violation.
    #[error("text.animate: schema violation on `{field}`: {detail}")]
    SchemaViolation {
        /// Field that violated the schema rule.
        field: &'static str,
        /// Validation detail.
        detail: &'static str,
    },

    /// Preset name is not registered.
    #[error("text.animate: unknown preset `{preset}`")]
    PresetUnknown {
        /// Unknown preset name.
        preset: String,
    },

    /// Window is outside accepted time bounds.
    #[error("text.animate: bad time window in_tk={in_tk}, out_tk={out_tk}")]
    BadTime {
        /// Requested start tick.
        in_tk: i64,
        /// Requested end tick.
        out_tk: i64,
    },

    /// Existing keyframe already targets `(clip_id, property, time_tk)`.
    #[error("text.animate: duplicate keyframe `{existing_keyframe_id}`")]
    Duplicate {
        /// Existing conflicting keyframe id.
        existing_keyframe_id: KeyframeId,
    },

    /// Built-in preset table is internally invalid.
    #[error("text.animate: internal preset invalid: {detail}")]
    PresetInvalid {
        /// Validation detail.
        detail: String,
    },
}

#[derive(Debug, Clone)]
struct CandidateKeyframe {
    order: usize,
    property: KeyframeProperty,
    time_tk: i64,
    value: Value,
    easing: Easing,
    clamped: bool,
}

#[derive(Debug, Clone)]
struct AddedKeyframe {
    keyframe: Keyframe,
    clamped: bool,
}

#[derive(Debug, Clone, Copy)]
struct LocatedTextClip<'a> {
    track_idx: usize,
    clip_idx: usize,
    track_id: TrackId,
    track_locked: bool,
    track_kind: TrackKind,
    clip: &'a crate::clip::Clip,
}

/// Build the RFC-6902 patch for `text.animate`.
///
/// # Errors
/// Returns [`TextAnimateError`] for selector, text-kind, lock, window,
/// preset, duplicate-keyframe, or internal preset validation failures.
#[allow(clippy::too_many_lines)]
pub fn compute_patch(
    prior: &Project,
    args: &TextAnimateArgs,
) -> Result<(Value, Vec<Value>, TextAnimateData), TextAnimateError> {
    let clip_id = args
        .clip
        .parse::<ClipId>()
        .map_err(|err| TextAnimateError::BadSelector {
            detail: err.to_string(),
        })?;

    let located = locate_clip(prior, clip_id).ok_or_else(|| {
        if selector_matches_track(prior, &args.clip) {
            TextAnimateError::SelectorKindMismatch {
                selector: args.clip.clone(),
            }
        } else {
            TextAnimateError::ClipNotFound {
                clip_id: args.clip.clone(),
            }
        }
    })?;

    if located.track_kind != TrackKind::Text {
        return Err(TextAnimateError::ClipKindMismatch {
            clip_id: args.clip.clone(),
            found_kind: located.track_kind,
        });
    }

    if located.track_locked {
        return Err(TextAnimateError::Locked {
            kind: "track",
            id: located.track_id.to_string(),
        });
    }
    if located.clip.locked {
        return Err(TextAnimateError::Locked {
            kind: "clip",
            id: located.clip.id.to_string(),
        });
    }

    let clip_duration_tk = timeline_duration_tk(
        located.clip.source_in_tk,
        located.clip.source_out_tk,
        located.clip.speed,
    )
    .get();

    let (in_tk, out_tk) = window_ticks(args, clip_duration_tk)?;
    if in_tk < 0 || out_tk < 0 || in_tk >= out_tk {
        return Err(TextAnimateError::BadTime { in_tk, out_tk });
    }

    let templates = presets::get(&args.preset).ok_or_else(|| TextAnimateError::PresetUnknown {
        preset: args.preset.clone(),
    })?;

    let end_tk = clip_duration_tk - 1;
    let mut clamped_count = 0_usize;
    let mut candidates = Vec::with_capacity(templates.len());

    for (order, template) in templates.iter().enumerate() {
        let raw_time = scaled_time_tk(in_tk, out_tk, template.fraction);
        let (time_tk, clamped) = if raw_time > end_tk {
            clamped_count += 1;
            (end_tk, true)
        } else {
            (raw_time, false)
        };

        let property =
            KeyframeProperty::try_from(template.property.to_string()).map_err(|err| {
                TextAnimateError::PresetInvalid {
                    detail: err.to_string(),
                }
            })?;
        let value = json!(template.value);
        super::keyframe_add::validate_value(property.as_str(), &value).map_err(|err| {
            TextAnimateError::PresetInvalid {
                detail: err.to_string(),
            }
        })?;

        candidates.push(CandidateKeyframe {
            order,
            property,
            time_tk,
            value,
            easing: template.easing,
            clamped,
        });
    }

    let (survivors, deduped_count) = dedupe_candidates(candidates);

    for candidate in &survivors {
        if let Some(existing) = located.clip.keyframes.iter().find(|keyframe| {
            keyframe.property == candidate.property && keyframe.time_tk.get() == candidate.time_tk
        }) {
            return Err(TextAnimateError::Duplicate {
                existing_keyframe_id: existing.id,
            });
        }
    }

    let mut added = Vec::with_capacity(survivors.len());
    for candidate in survivors {
        let mut keyframe = Keyframe::new(
            KeyframeId::now(),
            candidate.property,
            Tick::new(candidate.time_tk),
            candidate.value,
        );
        keyframe.easing = candidate.easing;
        added.push(AddedKeyframe {
            keyframe,
            clamped: candidate.clamped,
        });
    }
    added.sort_by(|left, right| {
        keyframe_sort_key(&left.keyframe).cmp(&keyframe_sort_key(&right.keyframe))
    });

    let mut ops = Vec::with_capacity(added.len());
    for added_keyframe in &added {
        let value = serde_json::to_value(&added_keyframe.keyframe).map_err(|err| {
            TextAnimateError::PresetInvalid {
                detail: format!("keyframe serialization failed: {err}"),
            }
        })?;
        ops.push(json!({
            "op": "add",
            "path": format!(
                "/tracks/{}/clips/{}/keyframes/-",
                located.track_idx, located.clip_idx
            ),
            "value": value,
        }));
    }

    let added_keyframe_ids = sort_ids(added.iter().map(|entry| entry.keyframe.id));
    let clamped_keyframe_ids = sort_ids(
        added
            .iter()
            .filter(|entry| entry.clamped)
            .map(|entry| entry.keyframe.id),
    );

    let mut warnings = Vec::new();
    if clamped_count > 0 {
        warnings.push(clamped_warning(
            &args.preset,
            clamped_count,
            &clamped_keyframe_ids,
        ));
    }
    if deduped_count > 0 {
        warnings.push(deduped_warning(&args.preset, deduped_count));
    }

    Ok((
        Value::Array(ops),
        warnings,
        TextAnimateData {
            clip_id,
            added_keyframe_ids,
            clamped_keyframe_ids,
        },
    ))
}

fn locate_clip(prior: &Project, clip_id: ClipId) -> Option<LocatedTextClip<'_>> {
    for (track_idx, track) in prior.tracks.iter().enumerate() {
        for (clip_idx, clip) in track.clips.iter().enumerate() {
            if clip.id == clip_id {
                return Some(LocatedTextClip {
                    track_idx,
                    clip_idx,
                    track_id: track.id,
                    track_locked: track.locked,
                    track_kind: track.kind,
                    clip,
                });
            }
        }
    }
    None
}

fn selector_matches_track(prior: &Project, raw: &str) -> bool {
    raw.parse::<TrackId>()
        .ok()
        .is_some_and(|track_id| prior.tracks.iter().any(|track| track.id == track_id))
}

#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
fn scaled_time_tk(in_tk: i64, out_tk: i64, fraction: f64) -> i64 {
    in_tk + (((out_tk - in_tk) as f64) * fraction).round() as i64
}

fn window_ticks(
    args: &TextAnimateArgs,
    clip_duration_tk: i64,
) -> Result<(i64, i64), TextAnimateError> {
    match (args.in_tk, args.out_tk) {
        (Some(in_tk), Some(out_tk)) => Ok((in_tk, out_tk)),
        (None, None) => Ok((0, clip_duration_tk)),
        (Some(_), None) => Err(TextAnimateError::SchemaViolation {
            field: "out_tk",
            detail: "`out_tk` is required when `in_tk` is supplied",
        }),
        (None, Some(_)) => Err(TextAnimateError::SchemaViolation {
            field: "in_tk",
            detail: "`in_tk` is required when `out_tk` is supplied",
        }),
    }
}

fn dedupe_candidates(candidates: Vec<CandidateKeyframe>) -> (Vec<CandidateKeyframe>, usize) {
    let mut latest_by_key = HashMap::new();
    let mut dropped_orders = HashSet::new();

    for (index, candidate) in candidates.iter().enumerate() {
        let key = (candidate.property.to_string(), candidate.time_tk);
        if let Some(previous_index) = latest_by_key.insert(key, index) {
            dropped_orders.insert(candidates[previous_index].order);
        }
    }

    let deduped_count = dropped_orders.len();
    let survivors = candidates
        .into_iter()
        .filter(|candidate| !dropped_orders.contains(&candidate.order))
        .collect();
    (survivors, deduped_count)
}

fn keyframe_sort_key(keyframe: &Keyframe) -> (String, i64) {
    (keyframe.property.to_string(), keyframe.time_tk.get())
}

fn sort_ids(ids: impl IntoIterator<Item = KeyframeId>) -> Vec<KeyframeId> {
    let mut ids: Vec<KeyframeId> = ids.into_iter().collect();
    ids.sort_by_key(ToString::to_string);
    ids
}

fn clamped_warning(
    preset: &str,
    clamped_count: usize,
    clamped_keyframe_ids: &[KeyframeId],
) -> Value {
    json!({
        "code": W_PRESET_KEYFRAMES_CLAMPED,
        "message": "preset keyframes clamped to clip duration",
        "details": {
            "preset": preset,
            "clamped_count": clamped_count,
            "clamped_keyframe_ids": clamped_keyframe_ids
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
        }
    })
}

fn deduped_warning(preset: &str, deduped_count: usize) -> Value {
    json!({
        "code": W_KEYFRAMES_DEDUPED,
        "message": "preset keyframes deduped after time scaling",
        "details": {
            "preset": preset,
            "deduped_count": deduped_count,
        }
    })
}

/// Rebuild `TextAnimateData` from recorded args, patch, and warnings.
///
/// # Errors
/// Returns [`ReconstructError`] when recorded inputs are malformed.
pub fn data_envelope_from_args_patch_warnings(
    args: &TextAnimateArgs,
    patch: &Value,
    warnings: &[Value],
) -> Result<TextAnimateData, ReconstructError> {
    let clip_id = args
        .clip
        .parse::<ClipId>()
        .map_err(|_| ReconstructError::TypeMismatch {
            name: "args.clip",
            expected: "UUIDv7 ClipId string",
        })?;
    let added_keyframe_ids = added_ids_from_patch(patch)?;
    let clamped_keyframe_ids = clamped_ids_from_warnings(warnings)?;

    Ok(TextAnimateData {
        clip_id,
        added_keyframe_ids: sort_ids(added_keyframe_ids),
        clamped_keyframe_ids: sort_ids(clamped_keyframe_ids),
    })
}

fn added_ids_from_patch(patch: &Value) -> Result<Vec<KeyframeId>, ReconstructError> {
    let ops = patch.as_array().ok_or(ReconstructError::TypeMismatch {
        name: "patch",
        expected: "array",
    })?;
    let mut ids = Vec::with_capacity(ops.len());
    for (index, op) in ops.iter().enumerate() {
        let op = op.as_object().ok_or(ReconstructError::TypeMismatch {
            name: "patch[]",
            expected: "object",
        })?;
        if op.get("op").and_then(Value::as_str) != Some("add") {
            return Err(ReconstructError::TypeMismatch {
                name: "patch[].op",
                expected: "add",
            });
        }
        let op_path =
            op.get("path")
                .and_then(Value::as_str)
                .ok_or(ReconstructError::MissingField {
                    name: "patch[].path",
                })?;
        if !op_path.starts_with("/tracks/") || !op_path.ends_with("/keyframes/-") {
            return Err(ReconstructError::TypeMismatch {
                name: "patch[].path",
                expected: "/tracks/{ti}/clips/{ci}/keyframes/-",
            });
        }
        let value = op.get("value").ok_or(ReconstructError::MissingField {
            name: "patch[].value",
        })?;
        let raw_id =
            value
                .get("id")
                .and_then(Value::as_str)
                .ok_or(ReconstructError::MissingField {
                    name: "patch[].value.id",
                })?;
        ids.push(
            raw_id
                .parse::<KeyframeId>()
                .map_err(|_| ReconstructError::TypeMismatch {
                    name: "patch[].value.id",
                    expected: "UUIDv7 KeyframeId string",
                })?,
        );
        if value.get("property").is_none() {
            return Err(ReconstructError::MissingField {
                name: "patch[].value.property",
            });
        }
        if value.get("time_tk").is_none() {
            return Err(ReconstructError::MissingField {
                name: "patch[].value.time_tk",
            });
        }
        if value.get("value").is_none() {
            return Err(ReconstructError::MissingField {
                name: "patch[].value.value",
            });
        }
        let _ = index;
    }
    Ok(ids)
}

fn clamped_ids_from_warnings(warnings: &[Value]) -> Result<Vec<KeyframeId>, ReconstructError> {
    let mut ids = Vec::new();
    for warning in warnings {
        if warning.get("code").and_then(Value::as_str) != Some(W_PRESET_KEYFRAMES_CLAMPED) {
            continue;
        }
        let details = warning
            .get("details")
            .ok_or(ReconstructError::MissingField {
                name: "warnings[].details",
            })?;
        let raw_ids = details
            .get("clamped_keyframe_ids")
            .and_then(Value::as_array)
            .ok_or(ReconstructError::MissingField {
                name: "warnings[].details.clamped_keyframe_ids",
            })?;
        for raw_id in raw_ids {
            let raw_id = raw_id.as_str().ok_or(ReconstructError::TypeMismatch {
                name: "warnings[].details.clamped_keyframe_ids[]",
                expected: "string",
            })?;
            ids.push(
                raw_id
                    .parse::<KeyframeId>()
                    .map_err(|_| ReconstructError::TypeMismatch {
                        name: "warnings[].details.clamped_keyframe_ids[]",
                        expected: "UUIDv7 KeyframeId string",
                    })?,
            );
        }
    }
    Ok(ids)
}

impl From<TextAnimateError> for VerbError {
    fn from(value: TextAnimateError) -> Self {
        match value {
            TextAnimateError::BadSelector { .. }
            | TextAnimateError::ClipNotFound { .. }
            | TextAnimateError::SelectorKindMismatch { .. }
            | TextAnimateError::ClipKindMismatch { .. }
            | TextAnimateError::Locked { .. }
            | TextAnimateError::SchemaViolation { .. }
            | TextAnimateError::PresetUnknown { .. }
            | TextAnimateError::BadTime { .. }
            | TextAnimateError::Duplicate { .. } => VerbError::BadArgs {
                detail: value.to_string(),
            },
            TextAnimateError::PresetInvalid { .. } => VerbError::Custom(value.to_string()),
        }
    }
}

/// The §0.8 verb for `text.animate`.
#[derive(Debug, Default)]
pub struct TextAnimateVerb;

impl Verb for TextAnimateVerb {
    fn verb(&self) -> &'static str {
        "text.animate"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: TextAnimateArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("text.animate: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!("text.animate: patch construction failed: {err}"))
            })?;

        prior
            .apply(&patch)
            .map_err(|err| VerbError::InvariantViolation {
                detail: format!("text.animate: post-state validation failed: {err}"),
            })?;

        let data = serde_json::to_value(&data)
            .map_err(|err| VerbError::Custom(format!("text.animate: data failed: {err}")))?;

        Ok((patch, data, warnings))
    }

    fn reconstruct(
        &self,
        args: &Value,
        patch: &Value,
        warnings: &[Value],
        _post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let typed: TextAnimateArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "TextAnimateArgs",
            })?;

        let envelope = data_envelope_from_args_patch_warnings(&typed, patch, warnings)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
