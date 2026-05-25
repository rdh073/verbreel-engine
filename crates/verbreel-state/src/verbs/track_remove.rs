//! `track.remove` (§4.2) — thirty-seventh production verb in the engine.
//!
//! ## Spec quote (`spec/commands/track.md` §4.2, summarized)
//!
//! `track.remove` deletes a track and every clip on it. Removing a
//! text track also cascades `burned_caption` effects that reference the
//! removed track, plus any keyframes that targeted those removed
//! effects. Link-group survivors are cleared when removing the target
//! track drops their group population to exactly one.
//!
//! ## Reconstructor compatibility
//!
//! The deleted track, clips, effects, and keyframes are absent from
//! post-state, so `reconstruct()` cannot derive the data envelope from
//! post-state alone. The forward path therefore emits one hidden
//! internal warning (`W_TRACK_REMOVE_ENVELOPE`) carrying every removed
//! or cleared id list. The reconstructor reads that warning back into
//! [`TrackRemoveData`], mirroring the warning-driven pattern used by
//! `keyframe.remove`.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::{ClipId, EffectId, KeyframeId, LinkGroupId, ProjectId, TrackId};

use crate::invariants::extract_effect_id_from_property;
use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use crate::track::TrackKind;

const W_TRACK_REMOVE_ENVELOPE_CODE: &str = "W_TRACK_REMOVE_ENVELOPE";

/// Warning code emitted when keyframes are cascade-removed because
/// their target effect was cascade-removed.
pub const W_KEYFRAMES_REMOVED_CODE: &str = "W_KEYFRAMES_REMOVED";

/// Arguments for `track.remove`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackRemoveArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Target track selector. This slice accepts bare `UUIDv7`.
    pub track: String,
}

/// Envelope returned by `track.remove`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackRemoveData {
    /// Removed track id.
    pub removed_track_id: TrackId,

    /// Removed clip ids, sorted by UUID string.
    pub removed_clip_ids: Vec<ClipId>,

    /// Removed `burned_caption` effect ids, sorted by UUID string.
    pub removed_burned_effect_ids: Vec<EffectId>,

    /// Removed dangling keyframe ids, sorted by UUID string.
    pub removed_keyframe_ids: Vec<KeyframeId>,

    /// Surviving clip ids whose `link_group` was cleared, sorted by UUID string.
    pub cleared_link_group_clip_ids: Vec<ClipId>,
}

/// Verb-level validation failures for `track.remove`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TrackRemoveError {
    /// `args.track` is not parseable as `UUIDv7`.
    #[error("track.remove: `track` selector parse failed: {detail}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
    },

    /// Selector resolved to another entity kind.
    #[error("track.remove: selector `{selector}` resolved to {resolved_kind}, not a track")]
    SelectorKindMismatch {
        /// Offending selector string.
        selector: String,
        /// Entity kind the selector matched.
        resolved_kind: &'static str,
    },

    /// No track exists for `track`.
    #[error("track.remove: track `{track_id}` not found")]
    TrackNotFound {
        /// Missing track id string.
        track_id: String,
    },

    /// Project would violate `Project.tracks.minItems: 1`.
    #[error(
        "track.remove: cannot remove the last track; call track.add to create a replacement before removing the last track"
    )]
    LastInProject,

    /// Target track or a clip on it is locked.
    #[error("track.remove: {kind} `{failed_target}` is locked")]
    Locked {
        /// Locked entity kind.
        kind: &'static str,
        /// Locked track or clip id.
        failed_target: String,
    },
}

#[derive(Debug, Clone)]
struct LocatedTrack<'a> {
    idx: usize,
    id: TrackId,
    kind: TrackKind,
    locked: bool,
    track: &'a crate::track::Track,
}

#[derive(Debug, Clone)]
struct EffectRemoval {
    track_idx: usize,
    clip_idx: usize,
    effect_idx: usize,
    effect_id: EffectId,
}

#[derive(Debug, Clone)]
struct KeyframeRemoval {
    track_idx: usize,
    clip_idx: usize,
    keyframe_idx: usize,
    clip_id: ClipId,
    keyframe_id: KeyframeId,
}

/// Build the RFC-6902 patch for `track.remove`.
///
/// # Errors
/// Returns [`TrackRemoveError`] for bad selector, selector kind
/// mismatch, missing target track, last-track removal, or locked
/// target content.
#[allow(clippy::too_many_lines)]
pub fn compute_patch(
    prior: &Project,
    args: &TrackRemoveArgs,
) -> Result<(Value, Vec<Value>, TrackRemoveData), TrackRemoveError> {
    let track_id = args
        .track
        .parse::<TrackId>()
        .map_err(|err| TrackRemoveError::BadSelector {
            detail: err.to_string(),
        })?;

    let located = locate_track(prior, track_id).ok_or_else(|| {
        if selector_matches_clip(prior, &args.track) {
            TrackRemoveError::SelectorKindMismatch {
                selector: args.track.clone(),
                resolved_kind: "clip",
            }
        } else {
            TrackRemoveError::TrackNotFound {
                track_id: args.track.clone(),
            }
        }
    })?;

    if prior.tracks.len() == 1 {
        return Err(TrackRemoveError::LastInProject);
    }

    if located.locked {
        return Err(TrackRemoveError::Locked {
            kind: "track",
            failed_target: located.id.to_string(),
        });
    }
    if let Some(locked_clip) = located.track.clips.iter().find(|clip| clip.locked) {
        return Err(TrackRemoveError::Locked {
            kind: "clip",
            failed_target: locked_clip.id.to_string(),
        });
    }

    let removed_clip_ids = sort_ids(located.track.clips.iter().map(|clip| clip.id));
    let effect_removals = if located.kind == TrackKind::Text {
        burned_caption_effect_removals(prior, located.idx, located.id)
    } else {
        Vec::new()
    };
    let effect_ids_by_clip = effect_ids_by_clip(&effect_removals);
    let keyframe_removals = dangling_keyframe_removals(prior, &effect_ids_by_clip);
    let link_group_clears = lone_survivor_link_group_clears(prior, located.idx);

    let removed_burned_effect_ids = sort_ids(effect_removals.iter().map(|entry| entry.effect_id));
    let removed_keyframe_ids = sort_ids(keyframe_removals.iter().map(|entry| entry.keyframe_id));
    let cleared_link_group_clip_ids = sort_ids(link_group_clears.iter().map(|(_, _, id)| *id));

    let data = TrackRemoveData {
        removed_track_id: located.id,
        removed_clip_ids,
        removed_burned_effect_ids,
        removed_keyframe_ids,
        cleared_link_group_clip_ids,
    };

    let mut ops = Vec::new();

    let mut sorted_clears = link_group_clears;
    sorted_clears.sort_by_key(|entry| (entry.0, entry.1));
    for (track_idx, clip_idx, _) in &sorted_clears {
        ops.push(json!({
            "op": "replace",
            "path": format!("/tracks/{track_idx}/clips/{clip_idx}/link_group"),
            "value": null,
        }));
    }

    let mut sorted_keyframes = keyframe_removals;
    sorted_keyframes.sort_by(|left, right| {
        (right.keyframe_idx, right.clip_idx, right.track_idx).cmp(&(
            left.keyframe_idx,
            left.clip_idx,
            left.track_idx,
        ))
    });
    for removal in &sorted_keyframes {
        ops.push(json!({
            "op": "remove",
            "path": format!(
                "/tracks/{}/clips/{}/keyframes/{}",
                removal.track_idx, removal.clip_idx, removal.keyframe_idx
            ),
        }));
    }

    let mut sorted_effects = effect_removals;
    sorted_effects.sort_by(|left, right| {
        (right.effect_idx, right.clip_idx, right.track_idx).cmp(&(
            left.effect_idx,
            left.clip_idx,
            left.track_idx,
        ))
    });
    for removal in &sorted_effects {
        ops.push(json!({
            "op": "remove",
            "path": format!(
                "/tracks/{}/clips/{}/effects/{}",
                removal.track_idx, removal.clip_idx, removal.effect_idx
            ),
        }));
    }

    ops.push(json!({
        "op": "remove",
        "path": format!("/tracks/{}", located.idx),
    }));

    let mut warnings = vec![envelope_warning(&data)];
    warnings.extend(keyframes_removed_warnings(&sorted_keyframes));

    Ok((Value::Array(ops), warnings, data))
}

fn locate_track(prior: &Project, track_id: TrackId) -> Option<LocatedTrack<'_>> {
    prior
        .tracks
        .iter()
        .enumerate()
        .find(|(_, track)| track.id == track_id)
        .map(|(idx, track)| LocatedTrack {
            idx,
            id: track.id,
            kind: track.kind,
            locked: track.locked,
            track,
        })
}

fn selector_matches_clip(prior: &Project, raw: &str) -> bool {
    raw.parse::<ClipId>().ok().is_some_and(|clip_id| {
        prior
            .tracks
            .iter()
            .flat_map(|track| track.clips.iter())
            .any(|clip| clip.id == clip_id)
    })
}

fn burned_caption_effect_removals(
    prior: &Project,
    target_track_idx: usize,
    removed_track_id: TrackId,
) -> Vec<EffectRemoval> {
    let removed_track_id = removed_track_id.to_string();
    let mut removals = Vec::new();
    for (track_idx, track) in prior.tracks.iter().enumerate() {
        if track_idx == target_track_idx {
            continue;
        }
        for (clip_idx, clip) in track.clips.iter().enumerate() {
            for (effect_idx, effect) in clip.effects.iter().enumerate() {
                if effect.kind.as_str() == "burned_caption"
                    && effect
                        .params
                        .get("source_text_track_id")
                        .and_then(Value::as_str)
                        == Some(removed_track_id.as_str())
                {
                    removals.push(EffectRemoval {
                        track_idx,
                        clip_idx,
                        effect_idx,
                        effect_id: effect.id,
                    });
                }
            }
        }
    }
    removals
}

fn effect_ids_by_clip(removals: &[EffectRemoval]) -> HashMap<(usize, usize), HashSet<String>> {
    let mut by_clip: HashMap<(usize, usize), HashSet<String>> = HashMap::new();
    for removal in removals {
        by_clip
            .entry((removal.track_idx, removal.clip_idx))
            .or_default()
            .insert(removal.effect_id.to_string());
    }
    by_clip
}

fn dangling_keyframe_removals(
    prior: &Project,
    effect_ids_by_clip: &HashMap<(usize, usize), HashSet<String>>,
) -> Vec<KeyframeRemoval> {
    let mut removals = Vec::new();
    for ((track_idx, clip_idx), effect_ids) in effect_ids_by_clip {
        let clip = &prior.tracks[*track_idx].clips[*clip_idx];
        for (keyframe_idx, keyframe) in clip.keyframes.iter().enumerate() {
            if extract_effect_id_from_property(keyframe.property.as_str())
                .is_some_and(|effect_id| effect_ids.contains(effect_id))
            {
                removals.push(KeyframeRemoval {
                    track_idx: *track_idx,
                    clip_idx: *clip_idx,
                    keyframe_idx,
                    clip_id: clip.id,
                    keyframe_id: keyframe.id,
                });
            }
        }
    }
    removals
}

fn lone_survivor_link_group_clears(
    prior: &Project,
    target_track_idx: usize,
) -> Vec<(usize, usize, ClipId)> {
    let target_groups: HashSet<LinkGroupId> = prior.tracks[target_track_idx]
        .clips
        .iter()
        .filter_map(|clip| clip.link_group)
        .collect();
    if target_groups.is_empty() {
        return Vec::new();
    }

    let mut survivors: HashMap<LinkGroupId, Vec<(usize, usize, ClipId)>> = HashMap::new();
    for (track_idx, track) in prior.tracks.iter().enumerate() {
        if track_idx == target_track_idx {
            continue;
        }
        for (clip_idx, clip) in track.clips.iter().enumerate() {
            let Some(link_group) = clip.link_group else {
                continue;
            };
            if target_groups.contains(&link_group) {
                survivors
                    .entry(link_group)
                    .or_default()
                    .push((track_idx, clip_idx, clip.id));
            }
        }
    }

    let mut clears = Vec::new();
    for members in survivors.into_values() {
        if members.len() == 1 {
            clears.extend(members);
        }
    }
    clears
}

fn keyframes_removed_warnings(sorted_keyframes: &[KeyframeRemoval]) -> Vec<Value> {
    let mut by_clip: HashMap<ClipId, Vec<KeyframeId>> = HashMap::new();
    let mut clip_order: HashMap<ClipId, (usize, usize)> = HashMap::new();
    for removal in sorted_keyframes {
        by_clip
            .entry(removal.clip_id)
            .or_default()
            .push(removal.keyframe_id);
        clip_order
            .entry(removal.clip_id)
            .or_insert((removal.track_idx, removal.clip_idx));
    }

    let mut clips = by_clip.keys().copied().collect::<Vec<_>>();
    clips.sort_by_key(|clip_id| clip_order[clip_id]);

    clips
        .into_iter()
        .map(|clip_id| {
            let ids = sort_ids(
                by_clip
                    .remove(&clip_id)
                    .expect("clip id was collected from by_clip"),
            );
            json!({
                "code": W_KEYFRAMES_REMOVED_CODE,
                "message": "keyframes targeting cascade-removed effects were removed",
                "details": {
                    "clip_id": clip_id.to_string(),
                    "removed_keyframe_ids": ids
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>(),
                }
            })
        })
        .collect()
}

fn envelope_warning(data: &TrackRemoveData) -> Value {
    json!({
        "code": W_TRACK_REMOVE_ENVELOPE_CODE,
        "message": "track.remove envelope",
        "details": {
            "removed_track_id": data.removed_track_id.to_string(),
            "removed_clip_ids": stringify_ids(&data.removed_clip_ids),
            "removed_burned_effect_ids": stringify_ids(&data.removed_burned_effect_ids),
            "removed_keyframe_ids": stringify_ids(&data.removed_keyframe_ids),
            "cleared_link_group_clip_ids": stringify_ids(&data.cleared_link_group_clip_ids),
        }
    })
}

fn stringify_ids<T: ToString>(ids: &[T]) -> Vec<String> {
    ids.iter().map(ToString::to_string).collect()
}

fn sort_ids<T>(ids: impl IntoIterator<Item = T>) -> Vec<T>
where
    T: ToString,
{
    let mut ids: Vec<T> = ids.into_iter().collect();
    ids.sort_by_key(ToString::to_string);
    ids
}

/// Rebuild `TrackRemoveData` from recorded args and warnings.
///
/// # Errors
/// Returns [`ReconstructError`] if the internal envelope warning is
/// missing or malformed.
pub fn data_envelope_from_args_warnings(
    _args: &TrackRemoveArgs,
    warnings: &[Value],
) -> Result<TrackRemoveData, ReconstructError> {
    let details = envelope_details_from_warnings(warnings)?;
    Ok(TrackRemoveData {
        removed_track_id: required_id(details, "removed_track_id")?,
        removed_clip_ids: required_id_list(details, "removed_clip_ids")?,
        removed_burned_effect_ids: required_id_list(details, "removed_burned_effect_ids")?,
        removed_keyframe_ids: required_id_list(details, "removed_keyframe_ids")?,
        cleared_link_group_clip_ids: required_id_list(details, "cleared_link_group_clip_ids")?,
    })
}

fn envelope_details_from_warnings(warnings: &[Value]) -> Result<&Value, ReconstructError> {
    for warning in warnings {
        if warning.get("code").and_then(Value::as_str) != Some(W_TRACK_REMOVE_ENVELOPE_CODE) {
            continue;
        }
        return warning
            .get("details")
            .ok_or(ReconstructError::MissingField {
                name: "warnings[].W_TRACK_REMOVE_ENVELOPE.details",
            });
    }

    Err(ReconstructError::MissingField {
        name: "warnings[].W_TRACK_REMOVE_ENVELOPE",
    })
}

fn required_id<T>(details: &Value, name: &'static str) -> Result<T, ReconstructError>
where
    T: std::str::FromStr,
{
    let raw = details
        .get(name)
        .ok_or(ReconstructError::MissingField { name })?
        .as_str()
        .ok_or(ReconstructError::TypeMismatch {
            name,
            expected: "UUIDv7 string",
        })?;
    raw.parse::<T>()
        .map_err(|_| ReconstructError::TypeMismatch {
            name,
            expected: "UUIDv7 string",
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

impl From<TrackRemoveError> for VerbError {
    fn from(value: TrackRemoveError) -> Self {
        match value {
            TrackRemoveError::BadSelector { .. }
            | TrackRemoveError::SelectorKindMismatch { .. }
            | TrackRemoveError::TrackNotFound { .. }
            | TrackRemoveError::LastInProject
            | TrackRemoveError::Locked { .. } => VerbError::BadArgs {
                detail: value.to_string(),
            },
        }
    }
}

/// The §0.8 verb for `track.remove`.
#[derive(Debug, Default)]
pub struct TrackRemoveVerb;

impl Verb for TrackRemoveVerb {
    fn verb(&self) -> &'static str {
        "track.remove"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: TrackRemoveArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("track.remove: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!("track.remove: patch construction failed: {err}"))
            })?;
        let data = serde_json::to_value(&data)
            .map_err(|err| VerbError::Custom(format!("track.remove: data failed: {err}")))?;

        Ok((patch, data, warnings))
    }

    fn reconstruct(
        &self,
        args: &Value,
        _patch: &Value,
        warnings: &[Value],
        _post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let typed: TrackRemoveArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "TrackRemoveArgs",
            })?;

        let envelope = data_envelope_from_args_warnings(&typed, warnings)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
