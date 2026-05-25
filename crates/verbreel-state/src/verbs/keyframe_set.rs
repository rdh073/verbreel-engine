//! `keyframe.set` (§8.3) — thirty-fourth production verb in the engine.
//!
//! ## Spec quote (`spec/commands/keyframe.md` §8.3, summarized)
//!
//! `keyframe.set` partially updates an existing keyframe by bare
//! `UUIDv7` id. `property` is immutable; any supplied subset of
//! `time_tk`, `value`, `easing`, and `bezier` replaces the current
//! keyframe fields, with omitted fields preserved.

use crate::invariants::timeline_duration_tk;
use crate::keyframe::{Easing, Keyframe};
use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::{KeyframeId, ProjectId, Tick, TrackId};

/// Warning code emitted when incoming keyframe leaves do not change state.
pub const W_NOOP_CODE: &str = "W_NOOP";

/// Arguments for `keyframe.set`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyframeSetArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Target keyframe id as bare `UUIDv7`.
    pub keyframe: String,

    /// Replacement keyframe time. Absent keeps the current time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_tk: Option<i64>,

    /// Replacement keyframe value. Absent keeps the current value;
    /// explicit JSON `null` is preserved as `Some(Value::Null)` and
    /// then validated against the keyframe property.
    #[serde(
        default,
        deserialize_with = "deserialize_some",
        skip_serializing_if = "Option::is_none"
    )]
    pub value: Option<Value>,

    /// Replacement easing literal. Absent keeps the current easing
    /// unless `bezier` is supplied for an existing cubic-bezier keyframe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub easing: Option<String>,

    /// Replacement cubic-bezier control points. Absent keeps the
    /// current bezier when current/new easing is cubic-bezier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bezier: Option<[f64; 4]>,
}

fn deserialize_some<'de, D>(deserializer: D) -> Result<Option<Value>, D::Error>
where
    D: Deserializer<'de>,
{
    Value::deserialize(deserializer).map(Some)
}

/// Envelope returned by `keyframe.set`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeyframeSetData {
    /// Updated keyframe id.
    pub keyframe_id: KeyframeId,

    /// Full post-state keyframe.
    pub keyframe: Keyframe,
}

/// Verb-level validation failures for `keyframe.set`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum KeyframeSetError {
    /// `args.keyframe` is not parseable as a bare `KeyframeId`.
    #[error("keyframe.set: `keyframe` selector parse failed: {detail}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
    },

    /// No keyframe exists for `args.keyframe`.
    #[error("keyframe.set: keyframe `{keyframe_id}` not found")]
    KeyframeNotFound {
        /// Missing keyframe id string.
        keyframe_id: String,
    },

    /// Parent clip or track is locked.
    #[error("keyframe.set: {kind} `{id}` is locked")]
    Locked {
        /// Locked entity kind.
        kind: &'static str,
        /// Locked entity id.
        id: String,
    },

    /// Value does not match the target property's accepted shape.
    #[error("keyframe.set: bad value for `{property}`: {detail}")]
    BadValue {
        /// Property being keyed.
        property: String,
        /// Validation detail.
        detail: &'static str,
    },

    /// Time is outside the parent clip window.
    #[error("keyframe.set: time_tk {time_tk} is outside clip window 0..={clip_duration_tk}")]
    BadTime {
        /// Requested keyframe time.
        time_tk: i64,
        /// Parent clip timeline duration.
        clip_duration_tk: i64,
    },

    /// Easing/bezier args violate schema enum or conditional rules.
    #[error("keyframe.set: schema violation: {detail}")]
    SchemaViolation {
        /// Validation detail.
        detail: String,
    },

    /// Existing keyframe already targets `(clip_id, property, time_tk)`.
    #[error("keyframe.set: duplicate keyframe `{existing_keyframe_id}`")]
    Duplicate {
        /// Existing conflicting keyframe id.
        existing_keyframe_id: KeyframeId,
    },
}

/// Build the RFC-6902 patch for `keyframe.set`.
///
/// # Errors
/// Returns [`KeyframeSetError`] for selector, lock, value, time, easing,
/// or uniqueness failures.
#[allow(clippy::too_many_lines)]
pub fn compute_patch(
    prior: &Project,
    args: &KeyframeSetArgs,
) -> Result<(Value, Vec<Value>, KeyframeSetData), KeyframeSetError> {
    let keyframe_id =
        args.keyframe
            .parse::<KeyframeId>()
            .map_err(|err| KeyframeSetError::BadSelector {
                detail: err.to_string(),
            })?;

    let (track_idx, clip_idx, keyframe_idx, track_locked, track_id, clip, existing) =
        find_keyframe(prior, keyframe_id).ok_or_else(|| KeyframeSetError::KeyframeNotFound {
            keyframe_id: args.keyframe.clone(),
        })?;

    if track_locked {
        return Err(KeyframeSetError::Locked {
            kind: "track",
            id: track_id.to_string(),
        });
    }
    if clip.locked {
        return Err(KeyframeSetError::Locked {
            kind: "clip",
            id: clip.id.to_string(),
        });
    }

    let property = existing.property.as_str();
    let mut next = existing.clone();

    if let Some(time_tk) = args.time_tk {
        let clip_duration_tk =
            timeline_duration_tk(clip.source_in_tk, clip.source_out_tk, clip.speed).get();
        if time_tk < 0 || time_tk > clip_duration_tk {
            return Err(KeyframeSetError::BadTime {
                time_tk,
                clip_duration_tk,
            });
        }
        next.time_tk = Tick::new(time_tk);
    }

    if let Some(value) = args.value.as_ref() {
        super::keyframe_add::validate_value(property, value).map_err(map_add_error)?;
        next.value = value.clone();
    }

    next.easing = resolve_easing(&existing.easing, args, property)?;

    if let Some(conflict) = clip
        .keyframes
        .iter()
        .enumerate()
        .find_map(|(idx, keyframe)| {
            (idx != keyframe_idx
                && keyframe.property == next.property
                && keyframe.time_tk == next.time_tk)
                .then_some(keyframe.id)
        })
    {
        return Err(KeyframeSetError::Duplicate {
            existing_keyframe_id: conflict,
        });
    }

    let ops = patch_ops(track_idx, clip_idx, keyframe_idx, existing, &next)?;
    if ops.is_empty() {
        return Ok((
            json!([]),
            vec![no_op_warning(keyframe_id)],
            KeyframeSetData {
                keyframe_id,
                keyframe: existing.clone(),
            },
        ));
    }

    Ok((
        Value::Array(ops),
        Vec::new(),
        KeyframeSetData {
            keyframe_id,
            keyframe: next,
        },
    ))
}

fn find_keyframe(
    prior: &Project,
    keyframe_id: KeyframeId,
) -> Option<(
    usize,
    usize,
    usize,
    bool,
    TrackId,
    &crate::clip::Clip,
    &Keyframe,
)> {
    for (track_idx, track) in prior.tracks.iter().enumerate() {
        for (clip_idx, clip) in track.clips.iter().enumerate() {
            for (keyframe_idx, keyframe) in clip.keyframes.iter().enumerate() {
                if keyframe.id == keyframe_id {
                    return Some((
                        track_idx,
                        clip_idx,
                        keyframe_idx,
                        track.locked,
                        track.id,
                        clip,
                        keyframe,
                    ));
                }
            }
        }
    }
    None
}

fn resolve_easing(
    current: &Easing,
    args: &KeyframeSetArgs,
    property: &str,
) -> Result<Easing, KeyframeSetError> {
    match (args.easing.as_deref(), args.bezier) {
        (None, None) => Ok(*current),
        (Some("cubic-bezier"), None) => Err(KeyframeSetError::SchemaViolation {
            detail: "easing `cubic-bezier` requires bezier: [f64; 4]".to_string(),
        }),
        (Some("cubic-bezier"), Some(bezier)) => {
            super::keyframe_add::parse_easing(Some("cubic-bezier"), Some(bezier), property)
                .map_err(map_add_error)
        }
        (Some(easing), Some(_)) => {
            let parsed = super::keyframe_add::parse_easing(Some(easing), None, property)
                .map_err(map_add_error)?;
            match parsed {
                Easing::CubicBezier { .. } => unreachable!("cubic-bezier handled above"),
                _ => Err(KeyframeSetError::SchemaViolation {
                    detail: "bezier requires easing `cubic-bezier`".to_string(),
                }),
            }
        }
        (Some(easing), None) => {
            super::keyframe_add::parse_easing(Some(easing), None, property).map_err(map_add_error)
        }
        (None, Some(bezier)) => match current {
            Easing::CubicBezier { .. } => {
                super::keyframe_add::parse_easing(Some("cubic-bezier"), Some(bezier), property)
                    .map_err(map_add_error)
            }
            _ => Err(KeyframeSetError::SchemaViolation {
                detail: "bezier requires easing `cubic-bezier`".to_string(),
            }),
        },
    }
}

fn map_add_error(error: super::keyframe_add::KeyframeAddError) -> KeyframeSetError {
    match error {
        super::keyframe_add::KeyframeAddError::BadValue { property, detail } => {
            KeyframeSetError::BadValue { property, detail }
        }
        super::keyframe_add::KeyframeAddError::SchemaViolation { detail } => {
            KeyframeSetError::SchemaViolation { detail }
        }
        super::keyframe_add::KeyframeAddError::BadProperty { got } => {
            KeyframeSetError::SchemaViolation {
                detail: format!("existing keyframe property `{got}` is not valid"),
            }
        }
        other => KeyframeSetError::SchemaViolation {
            detail: other.to_string(),
        },
    }
}

fn patch_ops(
    track_idx: usize,
    clip_idx: usize,
    keyframe_idx: usize,
    existing: &Keyframe,
    next: &Keyframe,
) -> Result<Vec<Value>, KeyframeSetError> {
    let base_path = format!("/tracks/{track_idx}/clips/{clip_idx}/keyframes/{keyframe_idx}");
    let mut ops = Vec::new();

    if existing.time_tk != next.time_tk {
        ops.push(json!({
            "op": "replace",
            "path": format!("{base_path}/time_tk"),
            "value": next.time_tk,
        }));
    }
    if existing.value != next.value {
        ops.push(json!({
            "op": "replace",
            "path": format!("{base_path}/value"),
            "value": next.value,
        }));
    }

    let existing_json = keyframe_json(existing)?;
    let next_json = keyframe_json(next)?;
    let existing_obj =
        existing_json
            .as_object()
            .ok_or_else(|| KeyframeSetError::SchemaViolation {
                detail: "existing keyframe did not serialize to an object".to_string(),
            })?;
    let next_obj = next_json
        .as_object()
        .ok_or_else(|| KeyframeSetError::SchemaViolation {
            detail: "next keyframe did not serialize to an object".to_string(),
        })?;

    if existing_obj.get("easing") != next_obj.get("easing") {
        ops.push(json!({
            "op": "replace",
            "path": format!("{base_path}/easing"),
            "value": next_obj
                .get("easing")
                .cloned()
                .ok_or_else(|| KeyframeSetError::SchemaViolation {
                    detail: "next keyframe missing serialized easing".to_string(),
                })?,
        }));
    }

    match (existing_obj.get("bezier"), next_obj.get("bezier")) {
        (None, Some(value)) => ops.push(json!({
            "op": "add",
            "path": format!("{base_path}/bezier"),
            "value": value,
        })),
        (Some(_), None) => ops.push(json!({
            "op": "remove",
            "path": format!("{base_path}/bezier"),
        })),
        (Some(existing_value), Some(next_value)) if existing_value != next_value => {
            ops.push(json!({
                "op": "replace",
                "path": format!("{base_path}/bezier"),
                "value": next_value,
            }));
        }
        (None, None) | (Some(_), Some(_)) => {}
    }

    Ok(ops)
}

fn keyframe_json(keyframe: &Keyframe) -> Result<Value, KeyframeSetError> {
    serde_json::to_value(keyframe).map_err(|err| KeyframeSetError::SchemaViolation {
        detail: format!("keyframe serialization failed: {err}"),
    })
}

fn no_op_warning(keyframe_id: KeyframeId) -> Value {
    json!({
        "code": W_NOOP_CODE,
        "message": "keyframe unchanged",
        "details": {
            "verb": "keyframe.set",
            "keyframe_id": keyframe_id.to_string(),
        }
    })
}

/// Rebuilds the envelope from `(args, post_state)`.
///
/// # Errors
/// Returns [`ReconstructError`] when args are malformed or the keyframe
/// is missing from post-state.
pub fn data_envelope_from_post_state(
    args: &KeyframeSetArgs,
    post_state: &Project,
) -> Result<KeyframeSetData, ReconstructError> {
    let keyframe_id =
        args.keyframe
            .parse::<KeyframeId>()
            .map_err(|_| ReconstructError::TypeMismatch {
                name: "args.keyframe",
                expected: "UUIDv7 KeyframeId string",
            })?;

    let (_, _, _, _, _, _, keyframe) = find_keyframe(post_state, keyframe_id).ok_or_else(|| {
        ReconstructError::PostStateMissing {
            detail: format!("keyframe.set: keyframe {keyframe_id} not found in post_state"),
        }
    })?;

    Ok(KeyframeSetData {
        keyframe_id,
        keyframe: keyframe.clone(),
    })
}

impl From<KeyframeSetError> for VerbError {
    fn from(value: KeyframeSetError) -> Self {
        match value {
            KeyframeSetError::BadSelector { .. }
            | KeyframeSetError::KeyframeNotFound { .. }
            | KeyframeSetError::Locked { .. }
            | KeyframeSetError::BadValue { .. }
            | KeyframeSetError::BadTime { .. }
            | KeyframeSetError::SchemaViolation { .. }
            | KeyframeSetError::Duplicate { .. } => VerbError::BadArgs {
                detail: value.to_string(),
            },
        }
    }
}

/// The §0.8 verb for `keyframe.set`.
#[derive(Debug, Default)]
pub struct KeyframeSetVerb;

impl Verb for KeyframeSetVerb {
    fn verb(&self) -> &'static str {
        "keyframe.set"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: KeyframeSetArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("keyframe.set: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, _data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!("keyframe.set: patch construction failed: {err}"))
            })?;

        let post_state = prior
            .apply(&patch)
            .map_err(|err| VerbError::InvariantViolation {
                detail: format!("keyframe.set: post-state validation failed: {err}"),
            })?;

        let data = data_envelope_from_post_state(&typed, &post_state).map_err(|err| {
            VerbError::Custom(format!(
                "keyframe.set: data envelope reconstruction failed: {err}"
            ))
        })?;

        Ok((
            patch,
            serde_json::to_value(&data).map_err(|err| {
                VerbError::Custom(format!("keyframe.set: data envelope failed: {err}"))
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
        let typed: KeyframeSetArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "KeyframeSetArgs",
            })?;

        let envelope = data_envelope_from_post_state(&typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
