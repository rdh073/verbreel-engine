//! `effect.reorder` (§6.6) — forty-second production verb in the engine.
//!
//! ## Spec quote (`spec/commands/effect.md` §6.6, verbatim)
//!
//! > Moves an effect to a new index within its parent's `effects` array.
//! > Effect order is render order per §0.13 (index 0 runs first).
//! > Cross-parent moves (e.g. moving an effect from one clip to another,
//! > or from a clip to a track) are not supported by this verb — use
//! > `effect.remove` + `effect.add` for that.
//! >
//! > **Index range**: `to_index` must satisfy
//! > `0 ≤ to_index ≤ effects.length - 1`. Out-of-range values on either
//! > side return `E_BAD_RANGE` (no silent clamping — symmetric handling).
//! > To target the last position discoverably, callers can pass the
//! > sentinel string `"end"` instead of an integer; the engine resolves
//! > it to `effects.length - 1` and returns that value in `data.to_index`.
//! > Other string values are rejected with `E_SCHEMA_VIOLATION`.
//! >
//! > **No-op case** (`to_index == from_index` after `"end"` resolution):
//! > returns `Ok` with `patch: []`, `data.from_index == data.to_index`,
//! > and one `W_NOOP` warning carrying `details.from_index`,
//! > `details.to_index`, and `details.message: "effect already at
//! > requested index"`.

use std::fmt;

use serde::{
    Deserialize, Deserializer, Serialize,
    de::{self, Unexpected, Visitor},
};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::{ClipId, EffectId, ProjectId};

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// Warning code emitted when no actual reorder is performed.
pub const W_NOOP_CODE: &str = "W_NOOP";

/// Internal warning code carrying pre-state reorder indices for replay.
pub const W_EFFECT_REORDER_ENVELOPE_CODE: &str = "W_EFFECT_REORDER_ENVELOPE";

const NOOP_MESSAGE: &str = "effect already at requested index";

/// Target destination for `effect.reorder`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ToIndex {
    /// Explicit zero-based array index.
    Integer(i64),

    /// Sentinel string. Only the literal `"end"` is accepted.
    End(String),
}

impl<'de> Deserialize<'de> for ToIndex {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(ToIndexVisitor)
    }
}

struct ToIndexVisitor;

impl Visitor<'_> for ToIndexVisitor {
    type Value = ToIndex;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("integer or string literal \"end\"")
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(ToIndex::Integer(value))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        let value = i64::try_from(value)
            .map_err(|_| E::invalid_value(Unexpected::Unsigned(value), &self))?;
        Ok(ToIndex::Integer(value))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        if value == "end" {
            Ok(ToIndex::End(value.to_string()))
        } else {
            Err(E::invalid_value(Unexpected::Str(value), &self))
        }
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        if value == "end" {
            Ok(ToIndex::End(value))
        } else {
            Err(E::invalid_value(Unexpected::Str(&value), &self))
        }
    }
}

/// Args for `effect.reorder`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectReorderArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Target effect id as bare `UUIDv7`.
    pub effect: String,

    /// Target destination index or `"end"` sentinel.
    pub to_index: ToIndex,
}

/// Envelope `data` returned by `effect.reorder`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectReorderData {
    /// Target effect id.
    pub effect_id: EffectId,

    /// Parent kind. This slice supports clip-attached effects only.
    pub parent_kind: String,

    /// Parent clip id.
    pub parent_id: ClipId,

    /// Previous index in the parent effects array.
    pub from_index: u32,

    /// New index in the parent effects array.
    pub to_index: u32,
}

/// Verb-level validation failures for `effect.reorder`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EffectReorderError {
    /// `args.effect` is not parseable as `UUIDv7`.
    #[error("effect.reorder: `effect` selector parse failed: {detail}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
    },

    /// No clip-attached effect exists for `args.effect`.
    #[error("effect.reorder: effect `{effect_id}` not found")]
    EffectNotFound {
        /// Missing effect id string.
        effect_id: String,
    },

    /// Parent clip or parent track is locked.
    #[error("effect.reorder: {kind} `{id}` is locked for effect `{effect_id}`")]
    Locked {
        /// Locked entity kind.
        kind: &'static str,
        /// Locked entity id.
        id: String,
        /// Target effect id string.
        effect_id: String,
    },

    /// Target index is outside the parent effects array.
    #[error("effect.reorder: to_index {to_index} out of range for {effects_len} effects")]
    BadRange {
        /// Rejected index.
        to_index: i64,
        /// Current effect count on the parent.
        effects_len: usize,
    },

    /// Args are malformed beyond selector/range errors.
    #[error("E_SCHEMA_VIOLATION: effect.reorder: {detail}")]
    SchemaViolation {
        /// Human-readable schema violation detail.
        detail: String,
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
    clip_id: ClipId,
    effects_len: usize,
    _effect: &'a crate::effect::Effect,
}

/// Build the RFC-6902 patch for `effect.reorder`.
///
/// # Errors
///
/// Returns [`EffectReorderError`] for selector parse failure, missing
/// clip-attached effect, locked parent clip/track, out-of-range destination,
/// or a non-`"end"` string sentinel.
pub fn compute_patch(
    prior: &Project,
    args: &EffectReorderArgs,
) -> Result<(Value, Vec<Value>, EffectReorderData), EffectReorderError> {
    let effect_id =
        args.effect
            .parse::<EffectId>()
            .map_err(|err| EffectReorderError::BadSelector {
                detail: err.to_string(),
            })?;

    let located =
        locate_effect(prior, effect_id).ok_or_else(|| EffectReorderError::EffectNotFound {
            effect_id: args.effect.clone(),
        })?;

    if located.clip_locked {
        return Err(EffectReorderError::Locked {
            kind: "clip",
            id: located.clip_id.to_string(),
            effect_id: args.effect.clone(),
        });
    }

    if located.track_locked {
        return Err(EffectReorderError::Locked {
            kind: "track",
            id: located.track_id.clone(),
            effect_id: args.effect.clone(),
        });
    }

    let resolved_to_index = resolve_to_index(&args.to_index, located.effects_len)?;
    let from_index = to_u32(located.effect_idx)?;
    let to_index = to_u32(resolved_to_index)?;
    let data = EffectReorderData {
        effect_id,
        parent_kind: "clip".to_string(),
        parent_id: located.clip_id,
        from_index,
        to_index,
    };

    if located.effect_idx == resolved_to_index {
        return Ok((
            json!([]),
            vec![json!({
                "code": W_NOOP_CODE,
                "message": NOOP_MESSAGE,
                "details": {
                    "from_index": from_index,
                    "to_index": to_index,
                    "message": NOOP_MESSAGE,
                }
            })],
            data,
        ));
    }

    let patch = json!([{
        "op": "move",
        "from": format!(
            "/tracks/{}/clips/{}/effects/{}",
            located.track_idx, located.clip_idx, located.effect_idx
        ),
        "path": format!(
            "/tracks/{}/clips/{}/effects/{resolved_to_index}",
            located.track_idx, located.clip_idx
        ),
    }]);

    Ok((patch, vec![envelope_warning(&data)], data))
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
                        clip_id: clip.id,
                        effects_len: clip.effects.len(),
                        _effect: effect,
                    });
                }
            }
        }
    }
    None
}

fn resolve_to_index(to_index: &ToIndex, effects_len: usize) -> Result<usize, EffectReorderError> {
    match to_index {
        ToIndex::End(value) if value == "end" => {
            effects_len
                .checked_sub(1)
                .ok_or_else(|| EffectReorderError::SchemaViolation {
                    detail: "cannot resolve `end` in an empty effects array".to_string(),
                })
        }
        ToIndex::End(value) => Err(EffectReorderError::SchemaViolation {
            detail: format!("to_index string `{value}` must be the literal \"end\""),
        }),
        ToIndex::Integer(value) => {
            let Ok(index) = usize::try_from(*value) else {
                return Err(EffectReorderError::BadRange {
                    to_index: *value,
                    effects_len,
                });
            };

            if index >= effects_len {
                return Err(EffectReorderError::BadRange {
                    to_index: *value,
                    effects_len,
                });
            }

            Ok(index)
        }
    }
}

fn to_u32(value: usize) -> Result<u32, EffectReorderError> {
    u32::try_from(value).map_err(|_| EffectReorderError::SchemaViolation {
        detail: "effect index exceeds u32".to_string(),
    })
}

fn envelope_warning(data: &EffectReorderData) -> Value {
    json!({
        "code": W_EFFECT_REORDER_ENVELOPE_CODE,
        "message": "internal: reorder envelope",
        "details": {
            "from_index": data.from_index,
            "to_index": data.to_index,
        }
    })
}

/// Rebuilds the envelope from `(args, patch, warnings, post_state)`.
///
/// # Errors
///
/// Returns [`ReconstructError`] when args, patch, warnings, or post-state
/// do not carry enough information to reproduce the original data envelope.
pub fn data_envelope_from_patch_warnings_post_state(
    args: &EffectReorderArgs,
    patch: &Value,
    warnings: &[Value],
    post_state: &Project,
) -> Result<EffectReorderData, ReconstructError> {
    let effect_id =
        args.effect
            .parse::<EffectId>()
            .map_err(|_| ReconstructError::TypeMismatch {
                name: "args.effect",
                expected: "UUIDv7 EffectId string",
            })?;

    let located =
        locate_effect(post_state, effect_id).ok_or_else(|| ReconstructError::PostStateMissing {
            detail: format!("effect.reorder: effect id {effect_id} not found in post_state"),
        })?;

    let post_index = u32::try_from(located.effect_idx).map_err(|_| {
        ReconstructError::Custom("effect.reorder: post-state index exceeds u32".to_string())
    })?;

    let patch_ops = patch.as_array().ok_or(ReconstructError::TypeMismatch {
        name: "patch",
        expected: "array",
    })?;

    if patch_ops.is_empty() {
        return Ok(EffectReorderData {
            effect_id,
            parent_kind: "clip".to_string(),
            parent_id: located.clip_id,
            from_index: post_index,
            to_index: post_index,
        });
    }

    let details = envelope_details_from_warnings(warnings)?;
    let from_index = required_u32(details, "from_index")?;
    let to_index = required_u32(details, "to_index")?;

    if to_index != post_index {
        return Err(ReconstructError::Custom(format!(
            "effect.reorder: envelope to_index {to_index} does not match post-state index {post_index}"
        )));
    }

    Ok(EffectReorderData {
        effect_id,
        parent_kind: "clip".to_string(),
        parent_id: located.clip_id,
        from_index,
        to_index,
    })
}

fn envelope_details_from_warnings(warnings: &[Value]) -> Result<&Value, ReconstructError> {
    for warning in warnings {
        if warning.get("code").and_then(Value::as_str) != Some(W_EFFECT_REORDER_ENVELOPE_CODE) {
            continue;
        }
        return warning
            .get("details")
            .ok_or(ReconstructError::MissingField {
                name: "warnings[].W_EFFECT_REORDER_ENVELOPE.details",
            });
    }

    Err(ReconstructError::MissingField {
        name: "warnings[].W_EFFECT_REORDER_ENVELOPE",
    })
}

fn required_u32(details: &Value, name: &'static str) -> Result<u32, ReconstructError> {
    let raw = details
        .get(name)
        .ok_or(ReconstructError::MissingField { name })?;
    let value = raw.as_u64().ok_or(ReconstructError::TypeMismatch {
        name,
        expected: "non-negative integer",
    })?;
    u32::try_from(value).map_err(|_| {
        ReconstructError::Custom(format!(
            "effect.reorder: warning field `{name}` exceeds u32"
        ))
    })
}

impl From<EffectReorderError> for VerbError {
    fn from(value: EffectReorderError) -> Self {
        match value {
            EffectReorderError::BadSelector { .. }
            | EffectReorderError::EffectNotFound { .. }
            | EffectReorderError::Locked { .. }
            | EffectReorderError::BadRange { .. }
            | EffectReorderError::SchemaViolation { .. } => VerbError::BadArgs {
                detail: value.to_string(),
            },
        }
    }
}

/// `effect.reorder` verb registration entry.
#[derive(Debug, Default)]
pub struct EffectReorderVerb;

impl Verb for EffectReorderVerb {
    fn verb(&self) -> &'static str {
        "effect.reorder"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: EffectReorderArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("effect.reorder: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, _data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!("effect.reorder: patch construction failed: {err}"))
            })?;

        let post_state = prior
            .apply(&patch)
            .map_err(|err| VerbError::InvariantViolation {
                detail: format!("effect.reorder: post-state validation failed: {err}"),
            })?;

        let envelope = data_envelope_from_patch_warnings_post_state(
            &typed,
            &patch_value,
            &warnings,
            &post_state,
        )
        .map_err(|err| {
            VerbError::Custom(format!(
                "effect.reorder: data envelope reconstruction failed: {err}"
            ))
        })?;

        let data = serde_json::to_value(&envelope).map_err(|err| {
            VerbError::Custom(format!("effect.reorder: data serialize failed: {err}"))
        })?;

        Ok((patch, data, warnings))
    }

    fn reconstruct(
        &self,
        args: &Value,
        patch: &Value,
        warnings: &[Value],
        post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let typed: EffectReorderArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "EffectReorderArgs",
            })?;

        let envelope =
            data_envelope_from_patch_warnings_post_state(&typed, patch, warnings, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
