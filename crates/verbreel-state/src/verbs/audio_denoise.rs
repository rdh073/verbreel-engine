//! `audio.denoise` (§9.4) - managed denoise effect marker.
//!
//! This verb records denoise intent as an immediate state mutation. It
//! does not run DSP, spawn render jobs, or touch storage. Clip targets
//! mutate typed [`crate::clip::Clip::effects`]; track targets mutate the
//! current untyped [`crate::track::Track::effects`] JSON values.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use thiserror::Error;
use verbreel_types::{ClipId, EffectId, KeyframeId, ProjectId, TrackId};

use crate::clip::Clip;
use crate::effect::{Effect, EffectKind};
use crate::invariants::extract_effect_id_from_property;
use crate::keyframe::Keyframe;
use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use crate::track::{Track, TrackKind};

/// Warning code emitted when create-case `strength` defaults to `0.5`.
pub const W_DENOISE_DEFAULT_STRENGTH_CODE: &str = "W_DENOISE_DEFAULT_STRENGTH";

/// Warning code emitted when an explicit no-op is requested.
pub const W_NOOP_CODE: &str = "W_NOOP";

/// Warning code emitted when omitted `strength` leaves an existing effect untouched.
pub const W_NOOP_FLAG_CODE: &str = "W_NOOP_FLAG";

/// Warning code emitted when removing denoise also removes targeted keyframes.
pub const W_KEYFRAMES_REMOVED_CODE: &str = "W_KEYFRAMES_REMOVED";

/// Internal warning carrying destructive-branch data for replay.
pub const W_AUDIO_DENOISE_ENVELOPE_CODE: &str = "W_AUDIO_DENOISE_ENVELOPE";

const DENOISE_KIND: &str = "denoise";
const DEFAULT_STRENGTH: f64 = 0.5;
const SELECTOR_HINT: &str = "target must be qualified `clip:<UUIDv7>` or `track:<UUIDv7>`";

/// Args for `audio.denoise`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioDenoiseArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Qualified `clip:<UUIDv7>` or `track:<UUIDv7>` selector.
    pub target: String,

    /// Optional denoise strength in `[0.0, 1.0]`; `0.0` removes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strength: Option<f64>,
}

/// Envelope `data` returned by `audio.denoise`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioDenoiseData {
    /// Resolved clip or track id, without selector prefix.
    pub target_id: String,

    /// Attached denoise effect id after the call, if any.
    pub effect_id: Option<EffectId>,

    /// Removed denoise effect id when `strength: 0` removed one.
    pub removed_effect_id: Option<EffectId>,

    /// Keyframes cascade-removed because they targeted the removed effect.
    pub removed_keyframe_ids: Vec<KeyframeId>,
}

/// Verb-level validation failures for `audio.denoise`.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum AudioDenoiseError {
    /// `args.strength` is non-finite or outside `[0.0, 1.0]`.
    #[error("E_BAD_RANGE: audio.denoise: `strength` value {value} out of range [0, 1]")]
    BadRange {
        /// Offending value.
        value: f64,
    },

    /// `args.target` is empty, bare, malformed, or uses an unsupported prefix.
    #[error(
        "E_BAD_SELECTOR: audio.denoise: `target` selector parse failed: {detail}; hint: {hint}"
    )]
    BadSelector {
        /// Parse failure detail.
        detail: String,
        /// Recovery hint.
        hint: &'static str,
    },

    /// Selector resolved to a noun kind this verb does not accept.
    #[error(
        "E_SELECTOR_KIND_MISMATCH: audio.denoise: selector prefix `{actual_prefix}` is not clip or track"
    )]
    SelectorKindMismatch {
        /// Offending prefix token.
        actual_prefix: String,
    },

    /// Target clip or track id has no match in the project.
    #[error("E_NOT_FOUND: audio.denoise: {target_kind} `{target_id}` not found")]
    NotFound {
        /// Target kind (`"clip"` or `"track"`).
        target_kind: &'static str,
        /// Missing target id string.
        target_id: String,
    },

    /// Reserved for selector forms that structurally match no target.
    #[error("E_NO_MATCH: audio.denoise: selector `{selector}` matched no {target_kind}")]
    NoMatch {
        /// Target kind (`"clip"` or `"track"`).
        target_kind: &'static str,
        /// Original selector string.
        selector: String,
    },

    /// Target clip's parent track is not `kind: "audio"`.
    #[error("E_CLIP_KIND_MISMATCH: audio.denoise: clip `{clip_id}` is on a `{actual_kind}` track")]
    ClipKindMismatch {
        /// Target clip id.
        clip_id: String,
        /// Actual parent track kind.
        actual_kind: &'static str,
    },

    /// Target track is not `kind: "audio"`.
    #[error("E_TRACK_KIND_MISMATCH: audio.denoise: track `{track_id}` is `{actual_kind}` kind")]
    TrackKindMismatch {
        /// Target track id.
        track_id: String,
        /// Actual track kind.
        actual_kind: &'static str,
    },

    /// Target clip, its parent track, or the target track is locked.
    #[error("E_LOCKED: audio.denoise: {kind} `{id}` is locked")]
    Locked {
        /// Locked entity kind (`"clip"` or `"track"`).
        kind: &'static str,
        /// Locked entity id.
        id: String,
    },
}

#[derive(Debug, Clone, Copy)]
enum ParsedTarget {
    Clip(ClipId),
    Track(TrackId),
}

#[derive(Debug, Clone, Copy)]
struct LocatedClip<'a> {
    track_idx: usize,
    clip_idx: usize,
    track_kind: TrackKind,
    track_locked: bool,
    track_id: TrackId,
    clip: &'a Clip,
}

#[derive(Debug, Clone, Copy)]
struct LocatedTrack<'a> {
    track_idx: usize,
    track: &'a Track,
}

#[derive(Debug, Clone, Copy)]
struct LocatedDenoise {
    effect_idx: usize,
    effect_id: EffectId,
}

/// Build the RFC 6902 patch for `audio.denoise`.
///
/// # Errors
///
/// Returns [`AudioDenoiseError`] for range, selector, missing target,
/// non-audio target, locked target, or malformed existing track-effect
/// state.
pub fn compute_patch(
    prior: &Project,
    args: &AudioDenoiseArgs,
) -> Result<(Value, Vec<Value>, AudioDenoiseData), AudioDenoiseError> {
    validate_strength(args.strength)?;
    let parsed = parse_target(&args.target)?;

    match parsed {
        ParsedTarget::Clip(clip_id) => compute_clip_patch(prior, clip_id, args.strength),
        ParsedTarget::Track(track_id) => compute_track_patch(prior, track_id, args.strength),
    }
}

fn validate_strength(strength: Option<f64>) -> Result<(), AudioDenoiseError> {
    if let Some(value) = strength
        && (!value.is_finite() || !(0.0..=1.0).contains(&value))
    {
        return Err(AudioDenoiseError::BadRange { value });
    }
    Ok(())
}

fn parse_target(raw: &str) -> Result<ParsedTarget, AudioDenoiseError> {
    if raw.is_empty() {
        return Err(AudioDenoiseError::BadSelector {
            detail: "selector is empty".to_string(),
            hint: SELECTOR_HINT,
        });
    }

    let (prefix, body) = raw
        .split_once(':')
        .ok_or_else(|| AudioDenoiseError::BadSelector {
            detail: format!("selector `{raw}` is unqualified"),
            hint: SELECTOR_HINT,
        })?;

    match prefix {
        "clip" => body
            .parse::<ClipId>()
            .map(ParsedTarget::Clip)
            .map_err(|err| AudioDenoiseError::BadSelector {
                detail: format!("clip body parse failed: {err}"),
                hint: SELECTOR_HINT,
            }),
        "track" => body
            .parse::<TrackId>()
            .map(ParsedTarget::Track)
            .map_err(|err| AudioDenoiseError::BadSelector {
                detail: format!("track body parse failed: {err}"),
                hint: SELECTOR_HINT,
            }),
        other => Err(AudioDenoiseError::BadSelector {
            detail: format!("unknown selector prefix `{other}`"),
            hint: SELECTOR_HINT,
        }),
    }
}

fn compute_clip_patch(
    prior: &Project,
    clip_id: ClipId,
    strength: Option<f64>,
) -> Result<(Value, Vec<Value>, AudioDenoiseData), AudioDenoiseError> {
    let located = locate_clip(prior, clip_id).ok_or_else(|| AudioDenoiseError::NotFound {
        target_kind: "clip",
        target_id: clip_id.to_string(),
    })?;

    if located.track_kind != TrackKind::Audio {
        return Err(AudioDenoiseError::ClipKindMismatch {
            clip_id: clip_id.to_string(),
            actual_kind: track_kind_name(located.track_kind),
        });
    }
    if located.track_locked {
        return Err(AudioDenoiseError::Locked {
            kind: "track",
            id: located.track_id.to_string(),
        });
    }
    if located.clip.locked {
        return Err(AudioDenoiseError::Locked {
            kind: "clip",
            id: clip_id.to_string(),
        });
    }

    let existing = located
        .clip
        .effects
        .iter()
        .enumerate()
        .find(|(_, effect)| effect.kind.as_str() == DENOISE_KIND)
        .map(|(effect_idx, effect)| LocatedDenoise {
            effect_idx,
            effect_id: effect.id,
        });

    match (existing, strength) {
        (None, None) => Ok(create_clip_denoise(located, DEFAULT_STRENGTH, true)),
        (None, Some(0.0)) => Ok(no_prior_zero_noop(clip_id.to_string())),
        (None, Some(value)) => Ok(create_clip_denoise(located, value, false)),
        (Some(existing), None) => Ok(existing_omitted_noop(
            clip_id.to_string(),
            existing.effect_id,
        )),
        (Some(existing), Some(0.0)) => Ok(remove_clip_denoise(located, existing)),
        (Some(existing), Some(value)) => Ok(update_clip_denoise(located, existing, value)),
    }
}

fn compute_track_patch(
    prior: &Project,
    track_id: TrackId,
    strength: Option<f64>,
) -> Result<(Value, Vec<Value>, AudioDenoiseData), AudioDenoiseError> {
    let located = locate_track(prior, track_id).ok_or_else(|| AudioDenoiseError::NotFound {
        target_kind: "track",
        target_id: track_id.to_string(),
    })?;

    if located.track.kind != TrackKind::Audio {
        return Err(AudioDenoiseError::TrackKindMismatch {
            track_id: track_id.to_string(),
            actual_kind: track_kind_name(located.track.kind),
        });
    }
    if located.track.locked {
        return Err(AudioDenoiseError::Locked {
            kind: "track",
            id: track_id.to_string(),
        });
    }

    let existing = find_track_denoise(located.track)?;

    match (existing, strength) {
        (None, None) => Ok(create_track_denoise(located, DEFAULT_STRENGTH, true)),
        (None, Some(0.0)) => Ok(no_prior_zero_noop(track_id.to_string())),
        (None, Some(value)) => Ok(create_track_denoise(located, value, false)),
        (Some(existing), None) => Ok(existing_omitted_noop(
            track_id.to_string(),
            existing.effect_id,
        )),
        (Some(existing), Some(0.0)) => Ok(remove_track_denoise(located, existing)),
        (Some(existing), Some(value)) => Ok(update_track_denoise(located, existing, value)),
    }
}

fn create_clip_denoise(
    located: LocatedClip<'_>,
    strength: f64,
    defaulted: bool,
) -> (Value, Vec<Value>, AudioDenoiseData) {
    let effect_id = EffectId::now();
    let mut effects = located.clip.effects.clone();
    effects.push(denoise_effect(effect_id, strength));

    let data = AudioDenoiseData {
        target_id: located.clip.id.to_string(),
        effect_id: Some(effect_id),
        removed_effect_id: None,
        removed_keyframe_ids: Vec::new(),
    };

    let warnings = if defaulted {
        vec![default_strength_warning(&data.target_id)]
    } else {
        Vec::new()
    };

    (
        json!([{
            "op": "replace",
            "path": format!("/tracks/{}/clips/{}/effects", located.track_idx, located.clip_idx),
            "value": effects,
        }]),
        warnings,
        data,
    )
}

fn create_track_denoise(
    located: LocatedTrack<'_>,
    strength: f64,
    defaulted: bool,
) -> (Value, Vec<Value>, AudioDenoiseData) {
    let effect_id = EffectId::now();
    let mut effects = located.track.effects.clone();
    effects.push(track_denoise_effect(effect_id, strength));

    let data = AudioDenoiseData {
        target_id: located.track.id.to_string(),
        effect_id: Some(effect_id),
        removed_effect_id: None,
        removed_keyframe_ids: Vec::new(),
    };

    let warnings = if defaulted {
        vec![default_strength_warning(&data.target_id)]
    } else {
        Vec::new()
    };

    (
        json!([{
            "op": "replace",
            "path": format!("/tracks/{}/effects", located.track_idx),
            "value": effects,
        }]),
        warnings,
        data,
    )
}

fn update_clip_denoise(
    located: LocatedClip<'_>,
    existing: LocatedDenoise,
    strength: f64,
) -> (Value, Vec<Value>, AudioDenoiseData) {
    let mut params = located.clip.effects[existing.effect_idx].params.clone();
    params.insert("strength".to_string(), json!(strength));

    (
        json!([{
            "op": "replace",
            "path": format!(
                "/tracks/{}/clips/{}/effects/{}/params",
                located.track_idx, located.clip_idx, existing.effect_idx
            ),
            "value": params,
        }]),
        Vec::new(),
        AudioDenoiseData {
            target_id: located.clip.id.to_string(),
            effect_id: Some(existing.effect_id),
            removed_effect_id: None,
            removed_keyframe_ids: Vec::new(),
        },
    )
}

fn update_track_denoise(
    located: LocatedTrack<'_>,
    existing: LocatedDenoise,
    strength: f64,
) -> (Value, Vec<Value>, AudioDenoiseData) {
    let mut updated = located.track.effects[existing.effect_idx].clone();
    set_track_strength(&mut updated, strength);

    (
        json!([{
            "op": "replace",
            "path": format!("/tracks/{}/effects/{}", located.track_idx, existing.effect_idx),
            "value": updated,
        }]),
        Vec::new(),
        AudioDenoiseData {
            target_id: located.track.id.to_string(),
            effect_id: Some(existing.effect_id),
            removed_effect_id: None,
            removed_keyframe_ids: Vec::new(),
        },
    )
}

fn remove_clip_denoise(
    located: LocatedClip<'_>,
    existing: LocatedDenoise,
) -> (Value, Vec<Value>, AudioDenoiseData) {
    let filtered_effects = located
        .clip
        .effects
        .iter()
        .filter(|effect| effect.id != existing.effect_id)
        .cloned()
        .collect::<Vec<_>>();
    let (filtered_keyframes, removed_keyframe_ids) =
        keyframes_without_effect_refs(located.clip, existing.effect_id);

    let mut ops = vec![json!({
        "op": "replace",
        "path": format!("/tracks/{}/clips/{}/effects", located.track_idx, located.clip_idx),
        "value": filtered_effects,
    })];
    if !removed_keyframe_ids.is_empty() {
        ops.push(json!({
            "op": "replace",
            "path": format!("/tracks/{}/clips/{}/keyframes", located.track_idx, located.clip_idx),
            "value": filtered_keyframes,
        }));
    }

    let data = AudioDenoiseData {
        target_id: located.clip.id.to_string(),
        effect_id: None,
        removed_effect_id: Some(existing.effect_id),
        removed_keyframe_ids,
    };
    let mut warnings = vec![envelope_warning(&data)];
    if !data.removed_keyframe_ids.is_empty() {
        warnings.push(keyframes_removed_warning(
            located.clip.id,
            &data.removed_keyframe_ids,
        ));
    }

    (Value::Array(ops), warnings, data)
}

fn remove_track_denoise(
    located: LocatedTrack<'_>,
    existing: LocatedDenoise,
) -> (Value, Vec<Value>, AudioDenoiseData) {
    let effects = located
        .track
        .effects
        .iter()
        .enumerate()
        .filter(|(idx, _)| *idx != existing.effect_idx)
        .map(|(_, effect)| effect.clone())
        .collect::<Vec<_>>();

    let data = AudioDenoiseData {
        target_id: located.track.id.to_string(),
        effect_id: None,
        removed_effect_id: Some(existing.effect_id),
        removed_keyframe_ids: Vec::new(),
    };

    (
        json!([{
            "op": "replace",
            "path": format!("/tracks/{}/effects", located.track_idx),
            "value": effects,
        }]),
        vec![envelope_warning(&data)],
        data,
    )
}

fn no_prior_zero_noop(target_id: String) -> (Value, Vec<Value>, AudioDenoiseData) {
    (
        json!([]),
        vec![json!({
            "code": W_NOOP_CODE,
            "message": "no denoise effect attached",
            "details": {
                "target_id": target_id,
                "message": "strength=0 with no existing denoise effect",
            }
        })],
        AudioDenoiseData {
            target_id,
            effect_id: None,
            removed_effect_id: None,
            removed_keyframe_ids: Vec::new(),
        },
    )
}

fn existing_omitted_noop(
    target_id: String,
    effect_id: EffectId,
) -> (Value, Vec<Value>, AudioDenoiseData) {
    (
        json!([]),
        vec![json!({
            "code": W_NOOP_FLAG_CODE,
            "message": "no strength supplied; existing effect untouched",
            "details": {
                "target_id": target_id,
                "effect_id": effect_id.to_string(),
                "flag": "strength",
                "message": "no strength supplied; existing effect untouched",
            }
        })],
        AudioDenoiseData {
            target_id,
            effect_id: Some(effect_id),
            removed_effect_id: None,
            removed_keyframe_ids: Vec::new(),
        },
    )
}

fn denoise_effect(effect_id: EffectId, strength: f64) -> Effect {
    let mut params = Map::new();
    params.insert("strength".to_string(), json!(strength));
    Effect {
        id: effect_id,
        kind: EffectKind::new(DENOISE_KIND.to_string()).expect("denoise is a non-empty kind"),
        enabled: true,
        params,
        window: None,
    }
}

fn track_denoise_effect(effect_id: EffectId, strength: f64) -> Value {
    json!({
        "id": effect_id,
        "kind": DENOISE_KIND,
        "enabled": true,
        "params": {
            "strength": strength,
        },
    })
}

fn set_track_strength(effect: &mut Value, strength: f64) {
    let Some(object) = effect.as_object_mut() else {
        return;
    };
    let params = object.entry("params").or_insert_with(|| json!({}));
    if !params.is_object() {
        *params = json!({});
    }
    if let Some(params) = params.as_object_mut() {
        params.insert("strength".to_string(), json!(strength));
    }
}

fn locate_clip(project: &Project, clip_id: ClipId) -> Option<LocatedClip<'_>> {
    for (track_idx, track) in project.tracks.iter().enumerate() {
        for (clip_idx, clip) in track.clips.iter().enumerate() {
            if clip.id == clip_id {
                return Some(LocatedClip {
                    track_idx,
                    clip_idx,
                    track_kind: track.kind,
                    track_locked: track.locked,
                    track_id: track.id,
                    clip,
                });
            }
        }
    }
    None
}

fn locate_track(project: &Project, track_id: TrackId) -> Option<LocatedTrack<'_>> {
    project
        .tracks
        .iter()
        .enumerate()
        .find(|(_, track)| track.id == track_id)
        .map(|(track_idx, track)| LocatedTrack { track_idx, track })
}

fn find_track_denoise(track: &Track) -> Result<Option<LocatedDenoise>, AudioDenoiseError> {
    for (effect_idx, effect) in track.effects.iter().enumerate() {
        if effect.get("kind").and_then(Value::as_str) != Some(DENOISE_KIND) {
            continue;
        }
        let raw =
            effect
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| AudioDenoiseError::NoMatch {
                    target_kind: "track effect",
                    selector: format!("track:{}", track.id),
                })?;
        let effect_id = raw
            .parse::<EffectId>()
            .map_err(|_| AudioDenoiseError::NoMatch {
                target_kind: "track effect",
                selector: format!("track:{}", track.id),
            })?;
        return Ok(Some(LocatedDenoise {
            effect_idx,
            effect_id,
        }));
    }
    Ok(None)
}

fn keyframes_without_effect_refs(
    clip: &Clip,
    effect_id: EffectId,
) -> (Vec<Keyframe>, Vec<KeyframeId>) {
    let effect_id_string = effect_id.to_string();
    let mut filtered = Vec::with_capacity(clip.keyframes.len());
    let mut removed = Vec::new();

    for keyframe in &clip.keyframes {
        if extract_effect_id_from_property(keyframe.property.as_str())
            == Some(effect_id_string.as_str())
        {
            removed.push(keyframe.id);
        } else {
            filtered.push(keyframe.clone());
        }
    }

    removed.sort_by_key(ToString::to_string);
    (filtered, removed)
}

fn default_strength_warning(target_id: &str) -> Value {
    json!({
        "code": W_DENOISE_DEFAULT_STRENGTH_CODE,
        "message": "denoise strength defaulted for new effect",
        "details": {
            "target_id": target_id,
            "applied_strength": DEFAULT_STRENGTH,
        }
    })
}

fn keyframes_removed_warning(clip_id: ClipId, removed_keyframe_ids: &[KeyframeId]) -> Value {
    json!({
        "code": W_KEYFRAMES_REMOVED_CODE,
        "message": "effect keyframes targeting the removed denoise effect were removed",
        "details": {
            "clip_id": clip_id.to_string(),
            "removed_keyframe_ids": stringify_ids(removed_keyframe_ids),
        }
    })
}

fn envelope_warning(data: &AudioDenoiseData) -> Value {
    json!({
        "code": W_AUDIO_DENOISE_ENVELOPE_CODE,
        "message": "audio.denoise envelope",
        "details": {
            "target_id": data.target_id,
            "effect_id": data.effect_id.map(|id| id.to_string()),
            "removed_effect_id": data.removed_effect_id.map(|id| id.to_string()),
            "removed_keyframe_ids": stringify_ids(&data.removed_keyframe_ids),
        }
    })
}

fn stringify_ids<T: ToString>(ids: &[T]) -> Vec<String> {
    ids.iter().map(ToString::to_string).collect()
}

fn track_kind_name(kind: TrackKind) -> &'static str {
    match kind {
        TrackKind::Video => "video",
        TrackKind::Audio => "audio",
        TrackKind::Text => "text",
        TrackKind::Effect => "effect",
    }
}

/// Rebuild `AudioDenoiseData` from recorded args, warnings, and post-state.
///
/// # Errors
///
/// Returns [`ReconstructError`] if recorded args or replay warnings are
/// malformed, or if the target is missing from post-state.
pub fn data_envelope_from_args_warnings_post_state(
    args: &AudioDenoiseArgs,
    warnings: &[Value],
    post_state: &Project,
) -> Result<AudioDenoiseData, ReconstructError> {
    if let Some(details) = envelope_details_from_warnings(warnings)? {
        return serde_json::from_value(details.clone()).map_err(|_| {
            ReconstructError::TypeMismatch {
                name: "warnings[].W_AUDIO_DENOISE_ENVELOPE.details",
                expected: "AudioDenoiseData",
            }
        });
    }

    let parsed = parse_target(&args.target).map_err(|_| ReconstructError::TypeMismatch {
        name: "args.target",
        expected: "qualified `clip:<UUIDv7>` or `track:<UUIDv7>` selector",
    })?;

    let data = match parsed {
        ParsedTarget::Clip(clip_id) => data_from_post_clip(clip_id, post_state)?,
        ParsedTarget::Track(track_id) => data_from_post_track(track_id, post_state)?,
    };

    if args.strength == Some(0.0)
        && data.effect_id.is_none()
        && !warning_code_present(warnings, W_NOOP_CODE)
    {
        return Err(ReconstructError::MissingField {
            name: "warnings[].W_AUDIO_DENOISE_ENVELOPE",
        });
    }

    Ok(data)
}

fn envelope_details_from_warnings(warnings: &[Value]) -> Result<Option<&Value>, ReconstructError> {
    for warning in warnings {
        if warning.get("code").and_then(Value::as_str) != Some(W_AUDIO_DENOISE_ENVELOPE_CODE) {
            continue;
        }
        return warning
            .get("details")
            .ok_or(ReconstructError::MissingField {
                name: "warnings[].W_AUDIO_DENOISE_ENVELOPE.details",
            })
            .map(Some);
    }
    Ok(None)
}

fn data_from_post_clip(
    clip_id: ClipId,
    post_state: &Project,
) -> Result<AudioDenoiseData, ReconstructError> {
    for track in &post_state.tracks {
        for clip in &track.clips {
            if clip.id != clip_id {
                continue;
            }
            let effect_id = clip
                .effects
                .iter()
                .find(|effect| effect.kind.as_str() == DENOISE_KIND)
                .map(|effect| effect.id);
            return Ok(AudioDenoiseData {
                target_id: clip_id.to_string(),
                effect_id,
                removed_effect_id: None,
                removed_keyframe_ids: Vec::new(),
            });
        }
    }

    Err(ReconstructError::PostStateMissing {
        detail: format!("audio.denoise: clip id {clip_id} not found in post_state"),
    })
}

fn data_from_post_track(
    track_id: TrackId,
    post_state: &Project,
) -> Result<AudioDenoiseData, ReconstructError> {
    let track = post_state
        .tracks
        .iter()
        .find(|track| track.id == track_id)
        .ok_or_else(|| ReconstructError::PostStateMissing {
            detail: format!("audio.denoise: track id {track_id} not found in post_state"),
        })?;
    let effect_id = find_track_denoise(track)
        .map_err(|err| ReconstructError::Custom(err.to_string()))?
        .map(|located| located.effect_id);
    Ok(AudioDenoiseData {
        target_id: track_id.to_string(),
        effect_id,
        removed_effect_id: None,
        removed_keyframe_ids: Vec::new(),
    })
}

fn warning_code_present(warnings: &[Value], code: &str) -> bool {
    warnings
        .iter()
        .any(|warning| warning.get("code").and_then(Value::as_str) == Some(code))
}

impl From<AudioDenoiseError> for VerbError {
    fn from(value: AudioDenoiseError) -> Self {
        match value {
            AudioDenoiseError::BadRange { .. }
            | AudioDenoiseError::BadSelector { .. }
            | AudioDenoiseError::SelectorKindMismatch { .. }
            | AudioDenoiseError::NotFound { .. }
            | AudioDenoiseError::NoMatch { .. }
            | AudioDenoiseError::ClipKindMismatch { .. }
            | AudioDenoiseError::TrackKindMismatch { .. }
            | AudioDenoiseError::Locked { .. } => VerbError::BadArgs {
                detail: value.to_string(),
            },
        }
    }
}

/// `audio.denoise` verb registration entry.
#[derive(Debug, Default)]
pub struct AudioDenoiseVerb;

impl Verb for AudioDenoiseVerb {
    fn verb(&self) -> &'static str {
        "audio.denoise"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: AudioDenoiseArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("audio.denoise: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!("audio.denoise: patch construction failed: {err}"))
            })?;
        let _post_state = prior
            .apply(&patch)
            .map_err(|err| VerbError::InvariantViolation {
                detail: format!("audio.denoise: post-state validation failed: {err}"),
            })?;
        let data = serde_json::to_value(&data).map_err(|err| {
            VerbError::Custom(format!("audio.denoise: data serialize failed: {err}"))
        })?;

        Ok((patch, data, warnings))
    }

    fn reconstruct(
        &self,
        args: &Value,
        _patch: &Value,
        warnings: &[Value],
        post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let typed: AudioDenoiseArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "AudioDenoiseArgs",
            })?;

        let envelope = data_envelope_from_args_warnings_post_state(&typed, warnings, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
