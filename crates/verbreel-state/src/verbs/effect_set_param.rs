//! `effect.set_param` (§6.3) — forty-third production verb in the engine.
//!
//! ## Spec quote (`spec/commands/effect.md` §6.3, verbatim)
//!
//! > **CLI**: `verbreel effect set_param [--project <id>] --effect <id> --params '<json>'`
//! > **MCP**: `effect.set_param`
//! > **Args**: `project_id: string`, `effect: string`, `params: object` (partial)
//! > **Returns** (`data`): `{ effect_id: string; params: object }`
//! > **Errors**: `E_NOT_FOUND`, `E_BAD_SELECTOR` (the `effect` arg failed to parse),
//! > `E_SELECTOR_KIND_MISMATCH` (selector resolved to a non-effect noun), `E_LOCKED`,
//! > `E_EFFECT_BAD_PARAMS`, `E_EFFECT_MANAGED` (params of any managed effect —
//! > `time_stretch`, `burned_caption`, **`denoise`** — cannot be edited directly;
//! > mutate via the managing verb per §6.2's managed-effect table).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use thiserror::Error;
use verbreel_types::{EffectId, ProjectId};

use crate::effect::Effect;
use crate::invariants::{EFFECT_PARAMS_MAX_BYTES, EFFECT_PARAMS_MAX_KEYS};
use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// Warning code emitted when the post-merge params already match the current
/// params.
pub const W_NOOP_CODE: &str = "W_NOOP";

const NOOP_MESSAGE: &str = "effect.set_param no-op";

/// Args for `effect.set_param`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectSetParamArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Target effect id as bare `UUIDv7`.
    pub effect: String,

    /// Partial params object. Supplied keys overwrite existing keys;
    /// absent keys are preserved.
    pub params: Map<String, Value>,
}

/// Envelope `data` returned by `effect.set_param`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EffectSetParamData {
    /// Target effect id.
    pub effect_id: EffectId,

    /// Full post-state params object.
    pub params: Map<String, Value>,
}

/// Verb-level validation failures for `effect.set_param`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EffectSetParamError {
    /// `args.effect` is not parseable as `UUIDv7`.
    #[error("effect.set_param: `effect` selector parse failed: {detail}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
    },

    /// No clip-attached effect exists for `args.effect`.
    #[error("effect.set_param: effect `{effect_id}` not found")]
    EffectNotFound {
        /// Missing effect id string.
        effect_id: String,
    },

    /// Parent clip or parent track is locked.
    #[error("effect.set_param: {kind} `{id}` is locked for effect `{effect_id}`")]
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
        "E_EFFECT_MANAGED: effect.set_param: kind `{kind}` is managed by `{managing_verb}`; hint: {hint}"
    )]
    ManagedEffect {
        /// Managed effect kind.
        kind: &'static str,
        /// Higher-level verb that owns mutation for this effect kind.
        managing_verb: &'static str,
        /// User-facing remediation hint from §6.2.
        hint: &'static str,
    },

    /// Post-merge params exceed the §0.13 params caps.
    #[error("E_EFFECT_BAD_PARAMS: effect.set_param: `{field}` value {value} exceeds bound {bound}")]
    BadParams {
        /// Field path reported in `details.field`.
        field: &'static str,
        /// Spec bound.
        bound: usize,
        /// Actual post-merge value.
        value: usize,
    },
}

#[derive(Debug, Clone)]
struct LocatedEffect<'a> {
    track_idx: usize,
    clip_idx: usize,
    effect_idx: usize,
    track_locked: bool,
    track_id: String,
    clip_locked: bool,
    clip_id: String,
    effect: &'a Effect,
}

/// Build the RFC-6902 patch for `effect.set_param`.
///
/// # Errors
///
/// Returns [`EffectSetParamError`] for selector parse failure, missing
/// clip-attached effect, locked parent clip/track, managed effect kinds,
/// or post-merge params that exceed the §0.13 caps.
pub fn compute_patch(
    prior: &Project,
    args: &EffectSetParamArgs,
) -> Result<(Value, Vec<Value>, EffectSetParamData), EffectSetParamError> {
    let effect_id =
        args.effect
            .parse::<EffectId>()
            .map_err(|err| EffectSetParamError::BadSelector {
                detail: err.to_string(),
            })?;

    let located =
        locate_effect(prior, effect_id).ok_or_else(|| EffectSetParamError::EffectNotFound {
            effect_id: args.effect.clone(),
        })?;

    if located.clip_locked {
        return Err(EffectSetParamError::Locked {
            kind: "clip",
            id: located.clip_id.clone(),
            effect_id: args.effect.clone(),
        });
    }

    if located.track_locked {
        return Err(EffectSetParamError::Locked {
            kind: "track",
            id: located.track_id.clone(),
            effect_id: args.effect.clone(),
        });
    }

    reject_managed_effect(located.effect.kind.as_str())?;

    let current_params = &located.effect.params;
    let mut proposed_params = current_params.clone();
    for (key, value) in &args.params {
        proposed_params.insert(key.clone(), value.clone());
    }

    check_params_caps(&proposed_params)?;

    let data = EffectSetParamData {
        effect_id,
        params: proposed_params.clone(),
    };

    if proposed_params == *current_params {
        return Ok((
            json!([]),
            vec![json!({
                "code": W_NOOP_CODE,
                "message": NOOP_MESSAGE,
                "details": {
                    "effect_id": effect_id.to_string(),
                    "message": NOOP_MESSAGE,
                }
            })],
            data,
        ));
    }

    let patch = json!([{
        "op": "replace",
        "path": format!(
            "/tracks/{}/clips/{}/effects/{}/params",
            located.track_idx, located.clip_idx, located.effect_idx
        ),
        "value": Value::Object(proposed_params),
    }]);

    Ok((patch, Vec::new(), data))
}

fn locate_effect(prior: &Project, effect_id: EffectId) -> Option<LocatedEffect<'_>> {
    for (track_idx, track) in prior.tracks.iter().enumerate() {
        for (clip_idx, clip) in track.clips.iter().enumerate() {
            for (effect_idx, effect) in clip.effects.iter().enumerate() {
                if effect.id == effect_id {
                    return Some(LocatedEffect {
                        track_idx,
                        clip_idx,
                        effect_idx,
                        track_locked: track.locked,
                        track_id: track.id.to_string(),
                        clip_locked: clip.locked,
                        clip_id: clip.id.to_string(),
                        effect,
                    });
                }
            }
        }
    }
    None
}

fn reject_managed_effect(kind: &str) -> Result<(), EffectSetParamError> {
    match kind {
        "time_stretch" => Err(EffectSetParamError::ManagedEffect {
            kind: "time_stretch",
            managing_verb: "clip.set_speed",
            hint: "call clip.set_speed --preserve_pitch false on the parent clip",
        }),
        "burned_caption" => Err(EffectSetParamError::ManagedEffect {
            kind: "burned_caption",
            managing_verb: "caption.burn_in",
            hint: "call caption.burn_off to remove the effect while keeping the source text track, or track.remove on the source text track to cascade-remove both, or effect.toggle --enabled false to disable without removing — see §10.5",
        }),
        "denoise" => Err(EffectSetParamError::ManagedEffect {
            kind: "denoise",
            managing_verb: "audio.denoise",
            hint: "call audio.denoise --strength 0 on the target to remove, or effect.toggle --enabled false to disable without removing",
        }),
        _ => Ok(()),
    }
}

fn check_params_caps(params: &Map<String, Value>) -> Result<(), EffectSetParamError> {
    let key_count = params.len();
    if key_count > EFFECT_PARAMS_MAX_KEYS {
        return Err(EffectSetParamError::BadParams {
            field: "params.keys",
            bound: EFFECT_PARAMS_MAX_KEYS,
            value: key_count,
        });
    }

    let bytes = serde_json::to_vec(params).map_or(usize::MAX, |value| value.len());
    if bytes > EFFECT_PARAMS_MAX_BYTES {
        return Err(EffectSetParamError::BadParams {
            field: "params.bytes",
            bound: EFFECT_PARAMS_MAX_BYTES,
            value: bytes,
        });
    }

    Ok(())
}

/// Rebuilds the envelope from `(args, post_state)`.
///
/// # Errors
///
/// Returns [`ReconstructError::TypeMismatch`] when `args.effect` is not a
/// `UUIDv7`, or [`ReconstructError::PostStateMissing`] when the post-state
/// does not contain the target effect.
pub fn data_envelope_from_post_state(
    args: &EffectSetParamArgs,
    post_state: &Project,
) -> Result<EffectSetParamData, ReconstructError> {
    let effect_id =
        args.effect
            .parse::<EffectId>()
            .map_err(|_| ReconstructError::TypeMismatch {
                name: "args.effect",
                expected: "UUIDv7 EffectId string",
            })?;

    for track in &post_state.tracks {
        for clip in &track.clips {
            for effect in &clip.effects {
                if effect.id == effect_id {
                    return Ok(EffectSetParamData {
                        effect_id,
                        params: effect.params.clone(),
                    });
                }
            }
        }
    }

    Err(ReconstructError::PostStateMissing {
        detail: format!("effect.set_param: effect id {effect_id} not found in post_state"),
    })
}

impl From<EffectSetParamError> for VerbError {
    fn from(value: EffectSetParamError) -> Self {
        match value {
            EffectSetParamError::BadSelector { .. }
            | EffectSetParamError::EffectNotFound { .. }
            | EffectSetParamError::Locked { .. }
            | EffectSetParamError::ManagedEffect { .. }
            | EffectSetParamError::BadParams { .. } => VerbError::BadArgs {
                detail: value.to_string(),
            },
        }
    }
}

/// `effect.set_param` verb registration entry.
#[derive(Debug, Default)]
pub struct EffectSetParamVerb;

impl Verb for EffectSetParamVerb {
    fn verb(&self) -> &'static str {
        "effect.set_param"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: EffectSetParamArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("effect.set_param: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, _data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!(
                    "effect.set_param: patch construction failed: {err}"
                ))
            })?;

        let post_state = prior
            .apply(&patch)
            .map_err(|err| VerbError::InvariantViolation {
                detail: format!("effect.set_param: post-state validation failed: {err}"),
            })?;

        let envelope = data_envelope_from_post_state(&typed, &post_state).map_err(|err| {
            VerbError::Custom(format!(
                "effect.set_param: data envelope reconstruction failed: {err}"
            ))
        })?;

        let data = serde_json::to_value(&envelope).map_err(|err| {
            VerbError::Custom(format!("effect.set_param: data serialize failed: {err}"))
        })?;

        Ok((patch, data, warnings))
    }

    fn reconstruct(
        &self,
        args: &Value,
        _patch: &Value,
        _warnings: &[Value],
        post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let typed: EffectSetParamArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "EffectSetParamArgs",
            })?;

        let envelope = data_envelope_from_post_state(&typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| {
            ReconstructError::Custom(format!("unable to serialize EffectSetParamData: {err}"))
        })
    }
}
