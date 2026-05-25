//! `effect.remove` (§6.2) — forty-fourth production verb in the engine.
//!
//! ## Spec quote (`spec/commands/effect.md` §6.2, verbatim excerpt)
//!
//! > Removes an effect from its parent clip or track.
//! >
//! > **Managed effects** (three kinds at v1): all three are owned by a
//! > higher-level verb that maintains the "one-effect-per-target" invariant.
//! > `effect.remove` and `effect.set_param` on a managed effect return
//! > `E_EFFECT_MANAGED` with `details.managing_verb` and a `details.hint`
//! > pointing at the owner verb.
//! >
//! > **Dangling-keyframe cascade**: keyframes whose `property` targets the
//! > removed effect (any path matching `effects[<this_effect_id>].params.*`)
//! > would otherwise become orphaned references. The engine removes those
//! > keyframes in the same patch as the effect removal.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::{ClipId, EffectId, KeyframeId, ProjectId};

use crate::clip::Clip;
use crate::effect::Effect;
use crate::invariants::extract_effect_id_from_property;
use crate::keyframe::Keyframe;
use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// Warning emitted when `effect.remove` cascade-removes effect keyframes.
pub const W_KEYFRAMES_REMOVED_CODE: &str = "W_KEYFRAMES_REMOVED";

/// Arguments for `effect.remove`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectRemoveArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Target effect id as bare `UUIDv7`.
    pub effect: String,
}

/// Envelope `data` returned by `effect.remove`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectRemoveData {
    /// Effect id removed from its parent clip.
    pub removed_effect_id: EffectId,

    /// Keyframes cascade-removed because their property targeted the effect.
    pub removed_keyframe_ids: Vec<KeyframeId>,
}

/// Verb-level validation failures for `effect.remove`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EffectRemoveError {
    /// `args.effect` is not parseable as `UUIDv7`.
    #[error("E_BAD_SELECTOR: effect.remove: `effect` selector parse failed: {detail}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
    },

    /// No clip-attached effect exists for `args.effect`.
    #[error("E_NOT_FOUND: effect.remove: effect `{effect_id}` not found")]
    EffectNotFound {
        /// Missing effect id string.
        effect_id: String,
    },

    /// Parent clip or parent track is locked.
    #[error("E_LOCKED: effect.remove: {kind} `{id}` is locked for effect `{effect_id}`")]
    Locked {
        /// Locked entity kind.
        kind: &'static str,
        /// Locked entity id.
        id: String,
        /// Target effect id string.
        effect_id: String,
    },

    /// The target effect kind is managed by a higher-level verb.
    #[error(
        "E_EFFECT_MANAGED: effect.remove: kind `{kind}` is managed by `{managing_verb}`; hint: {hint}"
    )]
    ManagedEffect {
        /// Managed effect kind.
        kind: &'static str,
        /// Higher-level verb that owns mutation for this effect kind.
        managing_verb: &'static str,
        /// User-facing remediation hint from §6.2.
        hint: &'static str,
    },
}

#[derive(Debug, Clone)]
struct LocatedEffect<'a> {
    track_idx: usize,
    clip_idx: usize,
    track_locked: bool,
    track_id: String,
    clip_locked: bool,
    clip_id: ClipId,
    clip: &'a Clip,
    effect: &'a Effect,
}

/// Build the RFC-6902 patch for `effect.remove`.
///
/// # Errors
///
/// Returns [`EffectRemoveError`] for selector parse failure, missing
/// clip-attached effect, locked parent clip/track, or managed effect kinds.
pub fn compute_patch(
    prior: &Project,
    args: &EffectRemoveArgs,
) -> Result<(Value, Vec<Value>, EffectRemoveData), EffectRemoveError> {
    let effect_id =
        args.effect
            .parse::<EffectId>()
            .map_err(|err| EffectRemoveError::BadSelector {
                detail: err.to_string(),
            })?;

    let located =
        locate_effect(prior, effect_id).ok_or_else(|| EffectRemoveError::EffectNotFound {
            effect_id: args.effect.clone(),
        })?;

    if located.clip_locked {
        return Err(EffectRemoveError::Locked {
            kind: "clip",
            id: located.clip_id.to_string(),
            effect_id: args.effect.clone(),
        });
    }

    if located.track_locked {
        return Err(EffectRemoveError::Locked {
            kind: "track",
            id: located.track_id.clone(),
            effect_id: args.effect.clone(),
        });
    }

    reject_managed_effect(located.effect.kind.as_str())?;

    let filtered_effects = effects_without(located.clip, effect_id);
    let (filtered_keyframes, removed_keyframe_ids) =
        keyframes_without_effect_refs(located.clip, effect_id);

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

    let warnings = if removed_keyframe_ids.is_empty() {
        Vec::new()
    } else {
        vec![keyframes_removed_warning(
            located.clip_id,
            &removed_keyframe_ids,
        )]
    };

    Ok((
        Value::Array(ops),
        warnings,
        EffectRemoveData {
            removed_effect_id: effect_id,
            removed_keyframe_ids,
        },
    ))
}

fn locate_effect(prior: &Project, effect_id: EffectId) -> Option<LocatedEffect<'_>> {
    for (track_idx, track) in prior.tracks.iter().enumerate() {
        for (clip_idx, clip) in track.clips.iter().enumerate() {
            for effect in &clip.effects {
                if effect.id == effect_id {
                    return Some(LocatedEffect {
                        track_idx,
                        clip_idx,
                        track_locked: track.locked,
                        track_id: track.id.to_string(),
                        clip_locked: clip.locked,
                        clip_id: clip.id,
                        clip,
                        effect,
                    });
                }
            }
        }
    }
    None
}

fn reject_managed_effect(kind: &str) -> Result<(), EffectRemoveError> {
    match kind {
        "time_stretch" => Err(EffectRemoveError::ManagedEffect {
            kind: "time_stretch",
            managing_verb: "clip.set_speed",
            hint: "call clip.set_speed --preserve_pitch false on the parent clip",
        }),
        "burned_caption" => Err(EffectRemoveError::ManagedEffect {
            kind: "burned_caption",
            managing_verb: "caption.burn_in",
            hint: "call caption.burn_off to remove the effect while keeping the source text track, or track.remove on the source text track to cascade-remove both, or effect.toggle --enabled false to disable without removing — see §10.5",
        }),
        "denoise" => Err(EffectRemoveError::ManagedEffect {
            kind: "denoise",
            managing_verb: "audio.denoise",
            hint: "call audio.denoise --strength 0 on the target to remove, or effect.toggle --enabled false to disable without removing",
        }),
        _ => Ok(()),
    }
}

fn effects_without(clip: &Clip, effect_id: EffectId) -> Vec<Effect> {
    clip.effects
        .iter()
        .filter(|effect| effect.id != effect_id)
        .cloned()
        .collect()
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

fn keyframes_removed_warning(clip_id: ClipId, removed_keyframe_ids: &[KeyframeId]) -> Value {
    json!({
        "code": W_KEYFRAMES_REMOVED_CODE,
        "message": "effect keyframes targeting the removed effect were removed",
        "details": {
            "clip_id": clip_id.to_string(),
            "removed_keyframe_ids": stringify_ids(removed_keyframe_ids),
        }
    })
}

fn stringify_ids<T: ToString>(ids: &[T]) -> Vec<String> {
    ids.iter().map(ToString::to_string).collect()
}

/// Rebuild `EffectRemoveData` from recorded args and warnings.
///
/// # Errors
///
/// Returns [`ReconstructError`] if recorded args or
/// `W_KEYFRAMES_REMOVED` warning details are malformed.
pub fn data_envelope_from_args_warnings(
    args: &EffectRemoveArgs,
    warnings: &[Value],
) -> Result<EffectRemoveData, ReconstructError> {
    let removed_effect_id =
        args.effect
            .parse::<EffectId>()
            .map_err(|_| ReconstructError::TypeMismatch {
                name: "args.effect",
                expected: "UUIDv7 EffectId string",
            })?;

    Ok(EffectRemoveData {
        removed_effect_id,
        removed_keyframe_ids: removed_keyframe_ids_from_warnings(warnings)?,
    })
}

fn removed_keyframe_ids_from_warnings(
    warnings: &[Value],
) -> Result<Vec<KeyframeId>, ReconstructError> {
    let mut ids = Vec::new();
    for warning in warnings {
        if warning.get("code").and_then(Value::as_str) != Some(W_KEYFRAMES_REMOVED_CODE) {
            continue;
        }

        let details = warning
            .get("details")
            .ok_or(ReconstructError::MissingField {
                name: "warnings[].W_KEYFRAMES_REMOVED.details",
            })?;
        let clip_id = details.get("clip_id").and_then(Value::as_str).ok_or(
            ReconstructError::MissingField {
                name: "warnings[].W_KEYFRAMES_REMOVED.details.clip_id",
            },
        )?;
        clip_id
            .parse::<ClipId>()
            .map_err(|_| ReconstructError::TypeMismatch {
                name: "warnings[].W_KEYFRAMES_REMOVED.details.clip_id",
                expected: "UUIDv7 ClipId string",
            })?;

        let removed = details
            .get("removed_keyframe_ids")
            .and_then(Value::as_array)
            .ok_or(ReconstructError::MissingField {
                name: "warnings[].W_KEYFRAMES_REMOVED.details.removed_keyframe_ids",
            })?;
        for value in removed {
            let raw = value.as_str().ok_or(ReconstructError::TypeMismatch {
                name: "warnings[].W_KEYFRAMES_REMOVED.details.removed_keyframe_ids[]",
                expected: "UUIDv7 KeyframeId string",
            })?;
            ids.push(
                raw.parse::<KeyframeId>()
                    .map_err(|_| ReconstructError::TypeMismatch {
                        name: "warnings[].W_KEYFRAMES_REMOVED.details.removed_keyframe_ids[]",
                        expected: "UUIDv7 KeyframeId string",
                    })?,
            );
        }
    }

    ids.sort_by_key(ToString::to_string);
    Ok(ids)
}

impl From<EffectRemoveError> for VerbError {
    fn from(value: EffectRemoveError) -> Self {
        match value {
            EffectRemoveError::BadSelector { .. }
            | EffectRemoveError::EffectNotFound { .. }
            | EffectRemoveError::Locked { .. }
            | EffectRemoveError::ManagedEffect { .. } => VerbError::BadArgs {
                detail: value.to_string(),
            },
        }
    }
}

/// `effect.remove` verb registration entry.
#[derive(Debug, Default)]
pub struct EffectRemoveVerb;

impl Verb for EffectRemoveVerb {
    fn verb(&self) -> &'static str {
        "effect.remove"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: EffectRemoveArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("effect.remove: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, _data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!("effect.remove: patch construction failed: {err}"))
            })?;

        let _post_state = prior
            .apply(&patch)
            .map_err(|err| VerbError::InvariantViolation {
                detail: format!("effect.remove: post-state validation failed: {err}"),
            })?;

        let envelope = data_envelope_from_args_warnings(&typed, &warnings).map_err(|err| {
            VerbError::Custom(format!(
                "effect.remove: data envelope reconstruction failed: {err}"
            ))
        })?;

        let data = serde_json::to_value(&envelope).map_err(|err| {
            VerbError::Custom(format!("effect.remove: data serialize failed: {err}"))
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
        let typed: EffectRemoveArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "EffectRemoveArgs",
            })?;

        let envelope = data_envelope_from_args_warnings(&typed, warnings)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
