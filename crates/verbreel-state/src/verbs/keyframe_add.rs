//! `keyframe.add` (§8.1) — thirty-third production verb in the engine.
//!
//! ## Spec quote (`spec/commands/keyframe.md` §8.1, verbatim)
//!
//! > Adds a keyframe to a clip property.
//! > CLI: `verbreel keyframe add [--project <id>] --clip <id> --property <path>
//! > --time_tk <int> --value '<json>' [--easing <name>] [--bezier '<json>']`
//! > MCP: `keyframe.add`
//! > Args: `project_id: string`, `clip: string`, `property: string`,
//! > `time_tk: integer`, `value: any`, `easing?: string`, `bezier?: number[4]`
//! > Returns (`data`): `{ keyframe_id: string; clip_id: string }`
//! > Errors: `E_NOT_FOUND`, `E_BAD_SELECTOR`, `E_KEYFRAME_BAD_PROPERTY`,
//! > `E_KEYFRAME_BAD_VALUE`, `E_KEYFRAME_DUPLICATE`, `E_BAD_TIME`, `E_LOCKED`.
//!
//! ## Deferred effect-param schema validation
//!
//! Per §8.1 "`E_KEYFRAME_BAD_PROPERTY` vs `E_EFFECT_BAD_PARAMS` precedence",
//! `effects[<uuid>].params.<dotted-path>` first proves that the referenced
//! effect exists on the clip. The per-kind params registry is not available
//! in this slice, so leaf-schema validation is intentionally deferred.

use crate::clip::{Clip, MaskKind};
use crate::invariants::{extract_effect_id_from_property, timeline_duration_tk};
use crate::keyframe::{Easing, Keyframe, KeyframeProperty};
use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::{ClipId, KeyframeId, ProjectId, Tick, TrackId};

/// Default easing literal when omitted from `keyframe.add` args.
pub const DEFAULT_EASING: &str = "linear";

/// Arguments for `keyframe.add`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyframeAddArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Target clip id as bare `UUIDv7`.
    pub clip: String,

    /// Regex-validated keyframe property path.
    pub property: String,

    /// Timeline-relative tick from the parent clip's `track_position_tk`.
    pub time_tk: i64,

    /// Keyframe value. Shape is validated per property at verb layer.
    pub value: Value,

    /// Optional easing literal. Defaults to [`DEFAULT_EASING`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub easing: Option<String>,

    /// Cubic-bezier control points. Required iff `easing == "cubic-bezier"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bezier: Option<[f64; 4]>,
}

/// Envelope returned by `keyframe.add`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyframeAddData {
    /// Newly minted keyframe id.
    pub keyframe_id: KeyframeId,

    /// Parent clip id.
    pub clip_id: ClipId,
}

/// Verb-level validation failures for `keyframe.add`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum KeyframeAddError {
    /// `args.clip` is not parseable as a `ClipId`.
    #[error("keyframe.add: `clip` selector parse failed: {detail}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
    },

    /// No clip exists for `args.clip`.
    #[error("keyframe.add: clip `{clip_id}` not found")]
    ClipNotFound {
        /// Missing clip id string.
        clip_id: String,
    },

    /// Parent clip or track is locked.
    #[error("keyframe.add: {kind} `{id}` is locked")]
    Locked {
        /// Locked entity kind.
        kind: &'static str,
        /// Locked entity id.
        id: String,
    },

    /// Property path fails schema regex or parent-clip existence checks.
    #[error("keyframe.add: bad keyframe property `{got}`")]
    BadProperty {
        /// Offending property string.
        got: String,
    },

    /// Value does not match the target property's accepted shape.
    #[error("keyframe.add: bad value for `{property}`: {detail}")]
    BadValue {
        /// Property being keyed.
        property: String,
        /// Validation detail.
        detail: &'static str,
    },

    /// Time is outside the parent clip window.
    #[error("keyframe.add: time_tk {time_tk} is outside clip window 0..={clip_duration_tk}")]
    BadTime {
        /// Requested keyframe time.
        time_tk: i64,
        /// Parent clip timeline duration.
        clip_duration_tk: i64,
    },

    /// Easing/bezier args violate schema enum or conditional rules.
    #[error("keyframe.add: schema violation: {detail}")]
    SchemaViolation {
        /// Validation detail.
        detail: String,
    },

    /// Existing keyframe already targets `(clip_id, property, time_tk)`.
    #[error("keyframe.add: duplicate keyframe `{existing_keyframe_id}`")]
    Duplicate {
        /// Existing conflicting keyframe id.
        existing_keyframe_id: KeyframeId,
    },
}

/// Build the RFC-6902 patch for `keyframe.add`.
///
/// # Errors
/// Returns [`KeyframeAddError`] for selector, lock, property, value,
/// time, easing, or uniqueness failures.
pub fn compute_patch(
    prior: &Project,
    args: &KeyframeAddArgs,
) -> Result<(Value, Vec<Value>, KeyframeAddData), KeyframeAddError> {
    let clip_id = args
        .clip
        .parse::<ClipId>()
        .map_err(|err| KeyframeAddError::BadSelector {
            detail: err.to_string(),
        })?;

    let (track_idx, clip_idx, track_locked, track_id, clip) = find_clip(prior, clip_id)
        .ok_or_else(|| KeyframeAddError::ClipNotFound {
            clip_id: args.clip.clone(),
        })?;

    if track_locked {
        return Err(KeyframeAddError::Locked {
            kind: "track",
            id: track_id.to_string(),
        });
    }
    if clip.locked {
        return Err(KeyframeAddError::Locked {
            kind: "clip",
            id: clip.id.to_string(),
        });
    }

    let property = KeyframeProperty::try_from(args.property.clone()).map_err(|_| {
        KeyframeAddError::BadProperty {
            got: args.property.clone(),
        }
    })?;
    validate_property_against_clip(clip, property.as_str())?;
    validate_value(property.as_str(), &args.value)?;

    let clip_duration_tk =
        timeline_duration_tk(clip.source_in_tk, clip.source_out_tk, clip.speed).get();
    if args.time_tk < 0 || args.time_tk > clip_duration_tk {
        return Err(KeyframeAddError::BadTime {
            time_tk: args.time_tk,
            clip_duration_tk,
        });
    }

    let easing = parse_easing(args.easing.as_deref(), args.bezier, property.as_str())?;

    if let Some(existing) = clip
        .keyframes
        .iter()
        .find(|keyframe| keyframe.property == property && keyframe.time_tk.get() == args.time_tk)
    {
        return Err(KeyframeAddError::Duplicate {
            existing_keyframe_id: existing.id,
        });
    }

    let keyframe_id = KeyframeId::now();
    let mut keyframe = Keyframe::new(
        keyframe_id,
        property,
        Tick::new(args.time_tk),
        args.value.clone(),
    );
    keyframe.easing = easing;
    let keyframe_value =
        serde_json::to_value(keyframe).map_err(|err| KeyframeAddError::SchemaViolation {
            detail: format!("keyframe serialization failed: {err}"),
        })?;

    let patch = json!([{
        "op": "add",
        "path": format!("/tracks/{track_idx}/clips/{clip_idx}/keyframes/-"),
        "value": keyframe_value,
    }]);

    Ok((
        patch,
        Vec::new(),
        KeyframeAddData {
            keyframe_id,
            clip_id,
        },
    ))
}

fn find_clip(prior: &Project, clip_id: ClipId) -> Option<(usize, usize, bool, TrackId, &Clip)> {
    for (track_idx, track) in prior.tracks.iter().enumerate() {
        for (clip_idx, clip) in track.clips.iter().enumerate() {
            if clip.id == clip_id {
                return Some((track_idx, clip_idx, track.locked, track.id, clip));
            }
        }
    }
    None
}

fn validate_property_against_clip(clip: &Clip, property: &str) -> Result<(), KeyframeAddError> {
    if let Some(effect_id) = extract_effect_id_from_property(property) {
        let exists = clip
            .effects
            .iter()
            .any(|effect| effect.id.to_string() == effect_id);
        if !exists {
            return Err(KeyframeAddError::BadProperty {
                got: property.to_string(),
            });
        }
        // TODO(keyframe.add): validate the params leaf against the effect's
        // registered params schema once that registry exists. Per §8.1
        // "E_KEYFRAME_BAD_PROPERTY vs E_EFFECT_BAD_PARAMS precedence", unknown
        // params leaf names in keyframe.add must remain E_KEYFRAME_BAD_PROPERTY.
        return Ok(());
    }

    if property.starts_with("mask.") {
        return validate_mask_property(clip, property);
    }

    Ok(())
}

fn validate_mask_property(clip: &Clip, property: &str) -> Result<(), KeyframeAddError> {
    let Some(mask) = clip.mask.as_ref() else {
        return Err(KeyframeAddError::BadProperty {
            got: property.to_string(),
        });
    };

    let valid = match mask.kind {
        MaskKind::Rect => matches!(
            property,
            "mask.feather_px"
                | "mask.params.x"
                | "mask.params.y"
                | "mask.params.w"
                | "mask.params.h"
        ),
        MaskKind::Ellipse => matches!(
            property,
            "mask.feather_px"
                | "mask.params.cx"
                | "mask.params.cy"
                | "mask.params.rx"
                | "mask.params.ry"
        ),
        MaskKind::Asset => matches!(property, "mask.feather_px" | "mask.params.threshold"),
        MaskKind::Polygon => false,
    };

    if valid {
        Ok(())
    } else {
        Err(KeyframeAddError::BadProperty {
            got: property.to_string(),
        })
    }
}

fn validate_value(property: &str, value: &Value) -> Result<(), KeyframeAddError> {
    match property {
        "opacity" => validate_number_range(property, value, Some(0.0), Some(1.0)),
        "volume" => validate_number_range(property, value, Some(0.0), Some(4.0)),
        "transform.x"
        | "transform.y"
        | "transform.scale_x"
        | "transform.scale_y"
        | "transform.rotation_deg"
        | "transform.skew_x_deg"
        | "transform.skew_y_deg"
        | "transform.anchor_x"
        | "transform.anchor_y"
        | "mask.params.x"
        | "mask.params.y"
        | "mask.params.w"
        | "mask.params.h"
        | "mask.params.cx"
        | "mask.params.cy"
        | "mask.params.rx"
        | "mask.params.ry"
        | "mask.params.threshold" => validate_number_range(property, value, None, None),
        "mask.feather_px" => validate_number_range(property, value, Some(0.0), None),
        _ if extract_effect_id_from_property(property).is_some() => {
            if value.is_boolean() || value.is_string() {
                Ok(())
            } else if let Some(n) = value.as_f64() {
                if n.is_finite() {
                    Ok(())
                } else {
                    Err(KeyframeAddError::BadValue {
                        property: property.to_string(),
                        detail: "number must be finite",
                    })
                }
            } else {
                Err(KeyframeAddError::BadValue {
                    property: property.to_string(),
                    detail: "effect param keyframes accept only number, boolean, or string",
                })
            }
        }
        _ => Err(KeyframeAddError::BadProperty {
            got: property.to_string(),
        }),
    }
}

fn validate_number_range(
    property: &str,
    value: &Value,
    min: Option<f64>,
    max: Option<f64>,
) -> Result<(), KeyframeAddError> {
    let Some(n) = value.as_f64() else {
        return Err(KeyframeAddError::BadValue {
            property: property.to_string(),
            detail: "value must be a finite number",
        });
    };
    if !n.is_finite() {
        return Err(KeyframeAddError::BadValue {
            property: property.to_string(),
            detail: "number must be finite",
        });
    }
    if let Some(min) = min
        && n < min
    {
        return Err(KeyframeAddError::BadValue {
            property: property.to_string(),
            detail: "number is below the allowed minimum",
        });
    }
    if let Some(max) = max
        && n > max
    {
        return Err(KeyframeAddError::BadValue {
            property: property.to_string(),
            detail: "number is above the allowed maximum",
        });
    }
    Ok(())
}

fn parse_easing(
    easing: Option<&str>,
    bezier: Option<[f64; 4]>,
    property: &str,
) -> Result<Easing, KeyframeAddError> {
    match easing.unwrap_or(DEFAULT_EASING) {
        "linear" => Ok(Easing::Linear),
        "ease-in" => Ok(Easing::EaseIn),
        "ease-out" => Ok(Easing::EaseOut),
        "ease-in-out" => Ok(Easing::EaseInOut),
        "step" => Ok(Easing::Step),
        "cubic-bezier" => {
            let bezier = bezier.ok_or_else(|| KeyframeAddError::SchemaViolation {
                detail: "easing `cubic-bezier` requires bezier: [f64; 4]".to_string(),
            })?;
            if bezier.iter().any(|value| !value.is_finite()) {
                return Err(KeyframeAddError::BadValue {
                    property: property.to_string(),
                    detail: "cubic-bezier control points must be finite",
                });
            }
            Ok(Easing::CubicBezier { bezier })
        }
        other => Err(KeyframeAddError::SchemaViolation {
            detail: format!("unknown easing literal `{other}`"),
        }),
    }
}

/// Rebuild `KeyframeAddData` from recorded args and patch value.
///
/// # Errors
/// Returns [`ReconstructError`] when args or patch shape is invalid.
pub fn data_envelope_from_args_and_patch(
    args: &KeyframeAddArgs,
    patch: &Value,
) -> Result<KeyframeAddData, ReconstructError> {
    let clip_id = args
        .clip
        .parse::<ClipId>()
        .map_err(|_| ReconstructError::TypeMismatch {
            name: "args.clip",
            expected: "UUIDv7 ClipId string",
        })?;
    let keyframe = keyframe_from_patch(patch)?;
    Ok(KeyframeAddData {
        keyframe_id: keyframe.id,
        clip_id,
    })
}

fn keyframe_from_patch(patch_value: &Value) -> Result<Keyframe, ReconstructError> {
    let ops = patch_value
        .as_array()
        .ok_or(ReconstructError::TypeMismatch {
            name: "patch",
            expected: "array",
        })?;
    if ops.len() != 1 {
        return Err(ReconstructError::TypeMismatch {
            name: "patch",
            expected: "single-element array",
        });
    }
    let op = ops
        .first()
        .and_then(Value::as_object)
        .ok_or(ReconstructError::TypeMismatch {
            name: "patch[0]",
            expected: "object",
        })?;
    if op.get("op").and_then(Value::as_str) != Some("add") {
        return Err(ReconstructError::TypeMismatch {
            name: "patch[0].op",
            expected: "add",
        });
    }
    let path = op
        .get("path")
        .and_then(Value::as_str)
        .ok_or(ReconstructError::MissingField {
            name: "patch[0].path",
        })?;
    if !path.starts_with("/tracks/") || !path.ends_with("/keyframes/-") {
        return Err(ReconstructError::TypeMismatch {
            name: "patch[0].path",
            expected: "/tracks/{ti}/clips/{ci}/keyframes/-",
        });
    }
    let value = op
        .get("value")
        .cloned()
        .ok_or(ReconstructError::MissingField {
            name: "patch[0].value",
        })?;
    serde_json::from_value(value).map_err(|err| ReconstructError::Custom(err.to_string()))
}

impl From<KeyframeAddError> for VerbError {
    fn from(value: KeyframeAddError) -> Self {
        match value {
            KeyframeAddError::BadSelector { .. }
            | KeyframeAddError::ClipNotFound { .. }
            | KeyframeAddError::Locked { .. }
            | KeyframeAddError::BadProperty { .. }
            | KeyframeAddError::BadValue { .. }
            | KeyframeAddError::BadTime { .. }
            | KeyframeAddError::SchemaViolation { .. }
            | KeyframeAddError::Duplicate { .. } => VerbError::BadArgs {
                detail: value.to_string(),
            },
        }
    }
}

/// The §0.8 verb for `keyframe.add`.
#[derive(Debug, Default)]
pub struct KeyframeAddVerb;

impl Verb for KeyframeAddVerb {
    fn verb(&self) -> &'static str {
        "keyframe.add"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: KeyframeAddArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("keyframe.add: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!("keyframe.add: patch construction failed: {err}"))
            })?;

        prior
            .apply(&patch)
            .map_err(|err| VerbError::InvariantViolation {
                detail: format!("keyframe.add: post-state validation failed: {err}"),
            })?;

        let data = serde_json::to_value(&data).map_err(|err| {
            VerbError::Custom(format!("keyframe.add: data envelope failed: {err}"))
        })?;

        Ok((patch, data, warnings))
    }

    fn reconstruct(
        &self,
        args: &Value,
        patch: &Value,
        _warnings: &[Value],
        _post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let typed: KeyframeAddArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "KeyframeAddArgs",
            })?;

        let envelope = data_envelope_from_args_and_patch(&typed, patch)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
