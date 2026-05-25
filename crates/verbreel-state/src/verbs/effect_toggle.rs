//! `effect.toggle` (§6.4) — twenty-sixth production verb in the engine.
//!
//! ## Spec quote (`spec/commands/effect.md` §6.4, verbatim)
//!
//! > Toggles the `enabled` flag. Managed effects (e.g. `time_stretch`) can be
//! > be toggled by `effect.toggle`; disabling reverts behavior temporarily
//! > without removing the effect record.
//! > CLI: `verbreel effect toggle [--project <id>] --effect <id> [--enabled <bool>]`
//! > MCP: `effect.toggle`
//! > Args: `project_id: string`, `effect: string`, `enabled?: boolean` (default `true`)
//! > Returns (`data`): `{ effect_id: string; enabled: boolean }`
//! > Errors: `E_NOT_FOUND`, `E_BAD_SELECTOR`, `E_SELECTOR_KIND_MISMATCH`,
//! > `E_LOCKED`
//! > Warnings: `W_NOOP` per §0.6.
//!
//! ## Selector handling (v1)
//!
//! `effect` is accepted as bare `UUIDv7` only. Structural selectors are
//! deferred to a future verb slice.

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::{EffectId, ProjectId};

/// Warning code emitted when incoming state already matches target.
pub const W_NOOP_CODE: &str = "W_NOOP";

/// Default `enabled` value when omitted from `effect.toggle` args.
pub const DEFAULT_ENABLED: bool = true;

/// Args for `effect.toggle`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectToggleArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Target effect id as bare `UUIDv7`.
    pub effect: String,

    /// Desired `enabled` state. Omitted values default to `DEFAULT_ENABLED`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

/// Envelope data returned by `effect.toggle`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectToggleData {
    /// Target effect id.
    pub effect_id: EffectId,

    /// New enabled state in post-state.
    pub enabled: bool,
}

/// Verb-level validation failures for `effect.toggle`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EffectToggleError {
    /// `args.effect` is not parseable as `UUIDv7`.
    #[error("effect.toggle: `effect` selector parse failed: {detail}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
    },

    /// No effect exists for `args.effect`.
    #[error("effect.toggle: effect `{effect_id}` not found")]
    EffectNotFound {
        /// Missing effect id string.
        effect_id: String,
    },

    /// Parent clip is locked.
    #[error("effect.toggle: parent clip `{clip_id}` is locked")]
    Locked {
        /// Locked parent clip id.
        clip_id: String,
    },
}

/// Build the RFC-6902 patch for `effect.toggle`.
///
/// # Errors
///
/// - [`EffectToggleError::BadSelector`] for non-UUIDv7 `args.effect`.
/// - [`EffectToggleError::EffectNotFound`] if `args.effect` resolves to no effect.
/// - [`EffectToggleError::Locked`] if the parent clip is locked.
/// - Idempotent no-op path: empty patch + [`W_NOOP_CODE`] warning when
///   `args.enabled` equals the current effect enabled state.
pub fn compute_patch(
    prior: &Project,
    args: &EffectToggleArgs,
) -> Result<(Value, Vec<Value>, EffectToggleData), EffectToggleError> {
    let effect_id =
        args.effect
            .parse::<EffectId>()
            .map_err(|err| EffectToggleError::BadSelector {
                detail: err.to_string(),
            })?;

    let mut location: Option<(
        usize,
        usize,
        usize,
        &crate::clip::Clip,
        &crate::effect::Effect,
    )> = None;
    'outer: for (t_idx, track) in prior.tracks.iter().enumerate() {
        for (c_idx, clip) in track.clips.iter().enumerate() {
            for (e_idx, effect) in clip.effects.iter().enumerate() {
                if effect.id == effect_id {
                    location = Some((t_idx, c_idx, e_idx, clip, effect));
                    break 'outer;
                }
            }
        }
    }

    let (t_idx, c_idx, e_idx, clip, effect) =
        location.ok_or_else(|| EffectToggleError::EffectNotFound {
            effect_id: args.effect.clone(),
        })?;

    if clip.locked {
        return Err(EffectToggleError::Locked {
            clip_id: clip.id.to_string(),
        });
    }

    let target_enabled = args.enabled.unwrap_or(DEFAULT_ENABLED);
    let current_enabled = effect.enabled;

    if target_enabled == current_enabled {
        return Ok((
            json!([]),
            vec![json!({
                "code": W_NOOP_CODE,
                "message": "effect enabled state unchanged",
                "details": {
                    "effect_id": effect_id.to_string(),
                    "enabled": current_enabled,
                }
            })],
            EffectToggleData {
                effect_id,
                enabled: current_enabled,
            },
        ));
    }

    let patch = json!([{
        "op": "replace",
        "path": format!("/tracks/{t_idx}/clips/{c_idx}/effects/{e_idx}/enabled"),
        "value": target_enabled,
    }]);

    Ok((
        patch,
        Vec::new(),
        EffectToggleData {
            effect_id,
            enabled: target_enabled,
        },
    ))
}

/// Rebuilds the envelope from `(args, post_state)`.
///
/// # Errors
///
/// Returns [`ReconstructError::TypeMismatch`] when `args.effect` is not a `UUIDv7`,
/// or [`ReconstructError::PostStateMissing`] when the post-state does not contain
/// the target effect.
pub fn data_envelope_from_post_state(
    args: &EffectToggleArgs,
    post_state: &Project,
) -> Result<EffectToggleData, ReconstructError> {
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
                    return Ok(EffectToggleData {
                        effect_id,
                        enabled: effect.enabled,
                    });
                }
            }
        }
    }

    Err(ReconstructError::PostStateMissing {
        detail: format!("effect.toggle: effect id {effect_id} not found in post_state"),
    })
}

/// `effect.toggle` verb registration entry.
#[derive(Debug, Default)]
pub struct EffectToggleVerb;

impl From<EffectToggleError> for VerbError {
    fn from(value: EffectToggleError) -> Self {
        match value {
            EffectToggleError::BadSelector { .. }
            | EffectToggleError::EffectNotFound { .. }
            | EffectToggleError::Locked { .. } => VerbError::BadArgs {
                detail: value.to_string(),
            },
        }
    }
}

impl Verb for EffectToggleVerb {
    fn verb(&self) -> &'static str {
        "effect.toggle"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: EffectToggleArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("effect.toggle: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, _data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!("effect.toggle: patch construction failed: {err}"))
            })?;

        let post_state = prior
            .apply(&patch)
            .map_err(|err| VerbError::InvariantViolation {
                detail: format!("effect.toggle: post-state validation failed: {err}"),
            })?;

        let envelope = data_envelope_from_post_state(&typed, &post_state).map_err(|err| {
            VerbError::Custom(format!(
                "effect.toggle: data envelope reconstruction failed: {err}"
            ))
        })?;

        let data = serde_json::to_value(&envelope).map_err(|err| {
            VerbError::Custom(format!("effect.toggle: data serialize failed: {err}"))
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
        let typed: EffectToggleArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "EffectToggleArgs",
            })?;

        let envelope = data_envelope_from_post_state(&typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| {
            ReconstructError::Custom(format!("unable to serialize EffectToggleData: {err}"))
        })
    }
}
