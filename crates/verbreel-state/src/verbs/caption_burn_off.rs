//! `caption.burn_off` (§10.5) — fifty-first production verb.
//!
//! Removes managed `burned_caption` effects while leaving the source
//! text track intact. The verb accepts either a source text track, a
//! target clip, or their intersection, and cascade-removes keyframes
//! targeting removed effects per §6.2.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::{ClipId, EffectId, KeyframeId, ProjectId, TrackId};

use crate::clip::Clip;
use crate::effect::Effect;
use crate::invariants::extract_effect_id_from_property;
use crate::keyframe::Keyframe;
use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use crate::track::TrackKind;

/// Warning code emitted when `caption.burn_off` matches no effects.
pub const W_NOOP_CODE: &str = "W_NOOP";

/// Warning code emitted when keyframes are cascade-removed.
pub const W_KEYFRAMES_REMOVED_CODE: &str = "W_KEYFRAMES_REMOVED";

/// Internal warning used to reconstruct destructive verb data.
pub const W_CAPTION_BURN_OFF_ENVELOPE_CODE: &str = "W_CAPTION_BURN_OFF_ENVELOPE";

const BURNED_CAPTION_KIND: &str = "burned_caption";

/// Arguments for `caption.burn_off`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptionBurnOffArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Optional source text track as a bare `UUIDv7`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_track: Option<String>,

    /// Optional target clip as a bare `UUIDv7`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip: Option<String>,
}

/// Envelope returned by `caption.burn_off`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptionBurnOffData {
    /// Removed `burned_caption` effect ids, sorted by UUID string.
    pub removed_effect_ids: Vec<EffectId>,

    /// Clip ids whose effects array changed, sorted by UUID string.
    pub affected_clip_ids: Vec<ClipId>,

    /// Keyframes cascade-removed because they targeted removed effects.
    pub removed_keyframe_ids: Vec<KeyframeId>,

    /// Resolved source text track id, present only when supplied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_text_track_id: Option<TrackId>,
}

/// Verb-level validation failures for `caption.burn_off`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CaptionBurnOffError {
    /// Neither selector was supplied.
    #[error("E_ARGS_INCOMPATIBLE: caption.burn_off: {hint}")]
    ArgsIncompatible {
        /// Remediation hint.
        hint: &'static str,
    },

    /// A supplied bare-UUID selector failed to parse.
    #[error("E_BAD_SELECTOR: caption.burn_off: `{field}` selector parse failed: {detail}")]
    BadSelector {
        /// Argument field name.
        field: &'static str,
        /// Parse failure detail.
        detail: String,
    },

    /// `text_track` parsed but did not resolve to a track.
    #[error("E_TRACK_NOT_FOUND: caption.burn_off: text track `{track_id}` not found")]
    TrackNotFound {
        /// Missing track id string.
        track_id: String,
    },

    /// `text_track` resolved to a non-text track.
    #[error(
        "E_TRACK_KIND_MISMATCH: caption.burn_off: expected {expected_kind} track, got {actual_kind}"
    )]
    TrackKindMismatch {
        /// Expected track kind.
        expected_kind: &'static str,
        /// Actual track kind.
        actual_kind: &'static str,
    },

    /// `clip` parsed but did not resolve to a clip.
    #[error("E_NOT_FOUND: caption.burn_off: clip `{clip_id}` not found")]
    ClipNotFound {
        /// Missing clip id string.
        clip_id: String,
    },

    /// An affected clip or its parent track is locked.
    #[error("E_LOCKED: caption.burn_off: clip `{failed_clip}` or its parent track is locked")]
    Locked {
        /// First locked affected clip in deterministic track/clip order.
        failed_clip: String,
    },
}

#[derive(Debug, Clone, Copy)]
struct LocatedTrack {
    id: TrackId,
    kind: TrackKind,
}

#[derive(Debug, Clone, Copy)]
struct LocatedClip<'a> {
    track_idx: usize,
    clip_idx: usize,
    track_locked: bool,
    clip: &'a Clip,
}

#[derive(Debug, Clone)]
struct MatchedClip<'a> {
    located: LocatedClip<'a>,
    matched_effect_ids: Vec<EffectId>,
}

/// Build the RFC-6902 patch for `caption.burn_off`.
///
/// # Errors
/// Returns [`CaptionBurnOffError`] for incompatible args, selector
/// parse failures, missing entities, track-kind mismatch, or locked
/// affected clips.
pub fn compute_patch(
    prior: &Project,
    args: &CaptionBurnOffArgs,
) -> Result<(Value, Vec<Value>, CaptionBurnOffData), CaptionBurnOffError> {
    if args.text_track.is_none() && args.clip.is_none() {
        return Err(CaptionBurnOffError::ArgsIncompatible {
            hint: "supply at least one of text_track or clip",
        });
    }

    let resolved_text_track_id = resolve_text_track(prior, args)?;
    let resolved_clip_id = resolve_clip(prior, args)?;
    let matched_clips =
        matched_burned_caption_clips(prior, resolved_text_track_id, resolved_clip_id);

    if matched_clips.is_empty() {
        let data = CaptionBurnOffData {
            removed_effect_ids: Vec::new(),
            affected_clip_ids: Vec::new(),
            removed_keyframe_ids: Vec::new(),
            resolved_text_track_id,
        };
        return Ok((
            json!([]),
            vec![no_op_warning(resolved_text_track_id, resolved_clip_id)],
            data,
        ));
    }

    for matched in &matched_clips {
        if matched.located.clip.locked || matched.located.track_locked {
            return Err(CaptionBurnOffError::Locked {
                failed_clip: matched.located.clip.id.to_string(),
            });
        }
    }

    let mut ops = Vec::new();
    let mut warnings = Vec::new();
    let mut removed_effect_ids = Vec::new();
    let mut affected_clip_ids = Vec::new();
    let mut removed_keyframe_ids = Vec::new();

    for matched in &matched_clips {
        let matched_ids = matched
            .matched_effect_ids
            .iter()
            .map(ToString::to_string)
            .collect::<HashSet<_>>();
        let filtered_effects = effects_without(matched.located.clip, &matched_ids);
        let (filtered_keyframes, clip_removed_keyframes) =
            keyframes_without_effect_refs(matched.located.clip, &matched_ids);

        ops.push(json!({
            "op": "replace",
            "path": format!(
                "/tracks/{}/clips/{}/effects",
                matched.located.track_idx, matched.located.clip_idx
            ),
            "value": filtered_effects,
        }));

        if !clip_removed_keyframes.is_empty() {
            ops.push(json!({
                "op": "replace",
                "path": format!(
                    "/tracks/{}/clips/{}/keyframes",
                    matched.located.track_idx, matched.located.clip_idx
                ),
                "value": filtered_keyframes,
            }));
            warnings.push(keyframes_removed_warning(
                matched.located.clip.id,
                &clip_removed_keyframes,
            ));
            removed_keyframe_ids.extend(clip_removed_keyframes);
        }

        removed_effect_ids.extend(matched.matched_effect_ids.iter().copied());
        affected_clip_ids.push(matched.located.clip.id);
    }

    let data = CaptionBurnOffData {
        removed_effect_ids: sort_ids(removed_effect_ids),
        affected_clip_ids: sort_ids(affected_clip_ids),
        removed_keyframe_ids: sort_ids(removed_keyframe_ids),
        resolved_text_track_id,
    };
    warnings.insert(0, envelope_warning(&data));

    Ok((Value::Array(ops), warnings, data))
}

fn resolve_text_track(
    prior: &Project,
    args: &CaptionBurnOffArgs,
) -> Result<Option<TrackId>, CaptionBurnOffError> {
    let Some(raw) = &args.text_track else {
        return Ok(None);
    };

    let track_id = raw
        .parse::<TrackId>()
        .map_err(|err| CaptionBurnOffError::BadSelector {
            field: "text_track",
            detail: err.to_string(),
        })?;

    let located =
        locate_track(prior, track_id).ok_or_else(|| CaptionBurnOffError::TrackNotFound {
            track_id: raw.clone(),
        })?;
    if located.kind != TrackKind::Text {
        return Err(CaptionBurnOffError::TrackKindMismatch {
            expected_kind: "text",
            actual_kind: track_kind_name(located.kind),
        });
    }

    Ok(Some(located.id))
}

fn resolve_clip(
    prior: &Project,
    args: &CaptionBurnOffArgs,
) -> Result<Option<ClipId>, CaptionBurnOffError> {
    let Some(raw) = &args.clip else {
        return Ok(None);
    };

    let clip_id = raw
        .parse::<ClipId>()
        .map_err(|err| CaptionBurnOffError::BadSelector {
            field: "clip",
            detail: err.to_string(),
        })?;
    locate_clip(prior, clip_id)
        .map(|located| located.clip.id)
        .ok_or_else(|| CaptionBurnOffError::ClipNotFound {
            clip_id: raw.clone(),
        })
        .map(Some)
}

fn locate_track(prior: &Project, track_id: TrackId) -> Option<LocatedTrack> {
    prior
        .tracks
        .iter()
        .find(|track| track.id == track_id)
        .map(|track| LocatedTrack {
            id: track.id,
            kind: track.kind,
        })
}

fn locate_clip(prior: &Project, clip_id: ClipId) -> Option<LocatedClip<'_>> {
    for (track_idx, track) in prior.tracks.iter().enumerate() {
        for (clip_idx, clip) in track.clips.iter().enumerate() {
            if clip.id == clip_id {
                return Some(LocatedClip {
                    track_idx,
                    clip_idx,
                    track_locked: track.locked,
                    clip,
                });
            }
        }
    }
    None
}

fn matched_burned_caption_clips(
    prior: &Project,
    resolved_text_track_id: Option<TrackId>,
    resolved_clip_id: Option<ClipId>,
) -> Vec<MatchedClip<'_>> {
    let source_track = resolved_text_track_id.map(|id| id.to_string());
    let mut matched = Vec::new();

    for (track_idx, track) in prior.tracks.iter().enumerate() {
        for (clip_idx, clip) in track.clips.iter().enumerate() {
            if resolved_clip_id.is_some_and(|clip_id| clip.id != clip_id) {
                continue;
            }

            let matched_effect_ids = clip
                .effects
                .iter()
                .filter(|effect| {
                    effect.kind.as_str() == BURNED_CAPTION_KIND
                        && source_track.as_ref().is_none_or(|track_id| {
                            effect
                                .params
                                .get("source_text_track_id")
                                .and_then(Value::as_str)
                                == Some(track_id.as_str())
                        })
                })
                .map(|effect| effect.id)
                .collect::<Vec<_>>();

            if !matched_effect_ids.is_empty() {
                matched.push(MatchedClip {
                    located: LocatedClip {
                        track_idx,
                        clip_idx,
                        track_locked: track.locked,
                        clip,
                    },
                    matched_effect_ids,
                });
            }
        }
    }

    matched
}

fn effects_without(clip: &Clip, matched_effect_ids: &HashSet<String>) -> Vec<Effect> {
    clip.effects
        .iter()
        .filter(|effect| !matched_effect_ids.contains(&effect.id.to_string()))
        .cloned()
        .collect()
}

fn keyframes_without_effect_refs(
    clip: &Clip,
    matched_effect_ids: &HashSet<String>,
) -> (Vec<Keyframe>, Vec<KeyframeId>) {
    let mut filtered = Vec::with_capacity(clip.keyframes.len());
    let mut removed = Vec::new();

    for keyframe in &clip.keyframes {
        if extract_effect_id_from_property(keyframe.property.as_str())
            .is_some_and(|effect_id| matched_effect_ids.contains(effect_id))
        {
            removed.push(keyframe.id);
        } else {
            filtered.push(keyframe.clone());
        }
    }

    (filtered, sort_ids(removed))
}

fn no_op_warning(resolved_text_track_id: Option<TrackId>, clip_id: Option<ClipId>) -> Value {
    let mut details = serde_json::Map::new();
    details.insert(
        "message".to_string(),
        Value::String("no matching burned_caption effects".to_string()),
    );
    if let Some(track_id) = resolved_text_track_id {
        details.insert(
            "resolved_text_track_id".to_string(),
            Value::String(track_id.to_string()),
        );
    }
    if let Some(clip_id) = clip_id {
        details.insert("clip_id".to_string(), Value::String(clip_id.to_string()));
    }

    json!({
        "code": W_NOOP_CODE,
        "message": "no matching burned_caption effects",
        "details": Value::Object(details),
    })
}

fn keyframes_removed_warning(clip_id: ClipId, removed_keyframe_ids: &[KeyframeId]) -> Value {
    json!({
        "code": W_KEYFRAMES_REMOVED_CODE,
        "message": "keyframes targeting removed burned_caption effects were removed",
        "details": {
            "clip_id": clip_id.to_string(),
            "removed_keyframe_ids": stringify_ids(removed_keyframe_ids),
        }
    })
}

fn envelope_warning(data: &CaptionBurnOffData) -> Value {
    json!({
        "code": W_CAPTION_BURN_OFF_ENVELOPE_CODE,
        "message": "caption.burn_off envelope",
        "details": {
            "removed_effect_ids": stringify_ids(&data.removed_effect_ids),
            "affected_clip_ids": stringify_ids(&data.affected_clip_ids),
            "removed_keyframe_ids": stringify_ids(&data.removed_keyframe_ids),
            "resolved_text_track_id": data.resolved_text_track_id.map(|id| id.to_string()),
        }
    })
}

fn track_kind_name(kind: TrackKind) -> &'static str {
    match kind {
        TrackKind::Video => "video",
        TrackKind::Audio => "audio",
        TrackKind::Text => "text",
        TrackKind::Effect => "effect",
    }
}

fn stringify_ids<T: ToString>(ids: &[T]) -> Vec<String> {
    ids.iter().map(ToString::to_string).collect()
}

fn sort_ids<T>(ids: impl IntoIterator<Item = T>) -> Vec<T>
where
    T: ToString,
{
    let mut ids = ids.into_iter().collect::<Vec<_>>();
    ids.sort_by_key(ToString::to_string);
    ids
}

/// Rebuild `CaptionBurnOffData` from recorded args and warnings.
///
/// # Errors
/// Returns [`ReconstructError`] if the internal envelope warning or
/// no-op args are malformed.
pub fn data_envelope_from_args_warnings(
    args: &CaptionBurnOffArgs,
    warnings: &[Value],
) -> Result<CaptionBurnOffData, ReconstructError> {
    if let Some(details) = envelope_details_from_warnings(warnings)? {
        return Ok(CaptionBurnOffData {
            removed_effect_ids: required_id_list(details, "removed_effect_ids")?,
            affected_clip_ids: required_id_list(details, "affected_clip_ids")?,
            removed_keyframe_ids: required_id_list(details, "removed_keyframe_ids")?,
            resolved_text_track_id: optional_id(details, "resolved_text_track_id")?,
        });
    }

    Ok(CaptionBurnOffData {
        removed_effect_ids: Vec::new(),
        affected_clip_ids: Vec::new(),
        removed_keyframe_ids: Vec::new(),
        resolved_text_track_id: args
            .text_track
            .as_deref()
            .map(parse_reconstruct_id)
            .transpose()?,
    })
}

fn envelope_details_from_warnings(warnings: &[Value]) -> Result<Option<&Value>, ReconstructError> {
    for warning in warnings {
        if warning.get("code").and_then(Value::as_str) != Some(W_CAPTION_BURN_OFF_ENVELOPE_CODE) {
            continue;
        }
        return warning
            .get("details")
            .ok_or(ReconstructError::MissingField {
                name: "warnings[].W_CAPTION_BURN_OFF_ENVELOPE.details",
            })
            .map(Some);
    }
    Ok(None)
}

fn parse_reconstruct_id<T>(raw: &str) -> Result<T, ReconstructError>
where
    T: std::str::FromStr,
{
    raw.parse::<T>()
        .map_err(|_| ReconstructError::TypeMismatch {
            name: "args.text_track",
            expected: "UUIDv7 TrackId string",
        })
}

fn optional_id<T>(details: &Value, name: &'static str) -> Result<Option<T>, ReconstructError>
where
    T: std::str::FromStr,
{
    let Some(value) = details.get(name) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let raw = value.as_str().ok_or(ReconstructError::TypeMismatch {
        name,
        expected: "UUIDv7 string or null",
    })?;
    raw.parse::<T>()
        .map(Some)
        .map_err(|_| ReconstructError::TypeMismatch {
            name,
            expected: "UUIDv7 string or null",
        })
}

fn required_id_list<T>(details: &Value, name: &'static str) -> Result<Vec<T>, ReconstructError>
where
    T: std::str::FromStr + ToString,
{
    let values = details
        .get(name)
        .ok_or(ReconstructError::MissingField { name })?
        .as_array()
        .ok_or(ReconstructError::TypeMismatch {
            name,
            expected: "array of UUIDv7 strings",
        })?;
    let ids = values
        .iter()
        .map(|value| {
            let raw = value.as_str().ok_or(ReconstructError::TypeMismatch {
                name,
                expected: "array of UUIDv7 strings",
            })?;
            raw.parse::<T>()
                .map_err(|_| ReconstructError::TypeMismatch {
                    name,
                    expected: "array of UUIDv7 strings",
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(sort_ids(ids))
}

impl From<CaptionBurnOffError> for VerbError {
    fn from(value: CaptionBurnOffError) -> Self {
        match value {
            CaptionBurnOffError::ArgsIncompatible { .. }
            | CaptionBurnOffError::BadSelector { .. }
            | CaptionBurnOffError::TrackNotFound { .. }
            | CaptionBurnOffError::TrackKindMismatch { .. }
            | CaptionBurnOffError::ClipNotFound { .. }
            | CaptionBurnOffError::Locked { .. } => VerbError::BadArgs {
                detail: value.to_string(),
            },
        }
    }
}

/// `caption.burn_off` verb registration entry.
#[derive(Debug, Default)]
pub struct CaptionBurnOffVerb;

impl Verb for CaptionBurnOffVerb {
    fn verb(&self) -> &'static str {
        "caption.burn_off"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: CaptionBurnOffArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("caption.burn_off: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, _data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!(
                    "caption.burn_off: patch construction failed: {err}"
                ))
            })?;

        let _post_state = prior
            .apply(&patch)
            .map_err(|err| VerbError::InvariantViolation {
                detail: format!("caption.burn_off: post-state validation failed: {err}"),
            })?;

        let envelope = data_envelope_from_args_warnings(&typed, &warnings).map_err(|err| {
            VerbError::Custom(format!(
                "caption.burn_off: data envelope reconstruction failed: {err}"
            ))
        })?;
        let data = serde_json::to_value(&envelope).map_err(|err| {
            VerbError::Custom(format!("caption.burn_off: data serialize failed: {err}"))
        })?;

        Ok((patch, data, warnings))
    }

    fn reconstruct(
        &self,
        args: &Value,
        _patch: &Value,
        warnings: &[Value],
        _post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let typed: CaptionBurnOffArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "CaptionBurnOffArgs",
            })?;

        let envelope = data_envelope_from_args_warnings(&typed, warnings)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
