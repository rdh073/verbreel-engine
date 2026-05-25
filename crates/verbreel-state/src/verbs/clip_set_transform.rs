//! `clip.set_transform` (§5.9) — twenty-third production verb.
//!
//! ## Spec quote (`spec/commands/clip.md` §5.9, verbatim)
//!
//! > Updates transform fields. Only the fields passed are updated.
//! >
//! > CLI: `verbreel clip set_transform [--project <id>] --clip <id> [--x <n>] [--y <n>] [--scale_x <n>] [--scale_y <n>] [--rotation_deg <deg>] [--anchor_x <n>] [--anchor_y <n>] [--skew_x_deg <deg>] [--skew_y_deg <deg>] [--flip_h <bool>] [--flip_v <bool>]`
//! > MCP: `clip.set_transform`
//! > Args: `project_id: string`, `clip: string`, `transform: Partial<Transform>`
//! > Returns (`data`): `{ clip_id: string; transform: Transform }`
//! > Errors: `E_LOCKED`
//!
//! plus standard clip selector errors and §0.6 `W_NOOP`.

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use crate::transform::Transform;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::{ClipId, ProjectId};

/// Warning code emitted when the incoming transform equals current.
pub const W_NOOP_CODE: &str = "W_NOOP";

/// Partial update payload for clip transform fields.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PartialTransform {
    /// X translation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    /// Y translation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
    /// Scale in X.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale_x: Option<f64>,
    /// Scale in Y.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale_y: Option<f64>,
    /// Rotation in degrees.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation_deg: Option<f64>,
    /// Anchor in X.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor_x: Option<f64>,
    /// Anchor in Y.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor_y: Option<f64>,
    /// X skew in degrees.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skew_x_deg: Option<f64>,
    /// Y skew in degrees.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skew_y_deg: Option<f64>,
    /// Flip horizontally.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flip_h: Option<bool>,
    /// Flip vertically.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flip_v: Option<bool>,
}

/// Args for `clip.set_transform`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipSetTransformArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Target clip id as bare `UUIDv7`.
    pub clip: String,

    /// New partial transform fields.
    pub transform: PartialTransform,
}

/// Envelope `data` returned by `clip.set_transform`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipSetTransformData {
    /// Target clip id.
    pub clip_id: ClipId,

    /// New full transform in post-state.
    pub transform: Transform,
}

fn clip_transform_value(transform: &Transform) -> Value {
    json!({
        "x": transform.x,
        "y": transform.y,
        "scale_x": transform.scale_x,
        "scale_y": transform.scale_y,
        "rotation_deg": transform.rotation_deg,
        "anchor_x": transform.anchor_x,
        "anchor_y": transform.anchor_y,
        "skew_x_deg": transform.skew_x_deg,
        "skew_y_deg": transform.skew_y_deg,
        "flip_h": transform.flip_h,
        "flip_v": transform.flip_v
    })
}

fn transform_diff(
    requested: &PartialTransform,
    current: &Transform,
) -> Result<Vec<(String, Value)>, ClipSetTransformError> {
    let mut diff: Vec<(String, Value)> = Vec::new();

    macro_rules! check_f64 {
        ($field:ident) => {
            if let Some(v) = requested.$field {
                if !v.is_finite() {
                    return Err(ClipSetTransformError::BadValue {
                        field: stringify!($field),
                        value: v,
                    });
                }
                #[allow(clippy::float_cmp)]
                if v != current.$field {
                    diff.push((stringify!($field).to_string(), json!(v)));
                }
            }
        };
    }

    check_f64!(x);
    check_f64!(y);
    check_f64!(scale_x);
    check_f64!(scale_y);
    check_f64!(rotation_deg);
    check_f64!(anchor_x);
    check_f64!(anchor_y);
    check_f64!(skew_x_deg);
    check_f64!(skew_y_deg);

    if let Some(v) = requested.flip_h
        && v != current.flip_h
    {
        diff.push(("flip_h".to_string(), json!(v)));
    }

    if let Some(v) = requested.flip_v
        && v != current.flip_v
    {
        diff.push(("flip_v".to_string(), json!(v)));
    }

    Ok(diff)
}

fn next_transform(requested: &PartialTransform, current: &Transform) -> Transform {
    Transform {
        x: requested.x.unwrap_or(current.x),
        y: requested.y.unwrap_or(current.y),
        scale_x: requested.scale_x.unwrap_or(current.scale_x),
        scale_y: requested.scale_y.unwrap_or(current.scale_y),
        rotation_deg: requested.rotation_deg.unwrap_or(current.rotation_deg),
        anchor_x: requested.anchor_x.unwrap_or(current.anchor_x),
        anchor_y: requested.anchor_y.unwrap_or(current.anchor_y),
        skew_x_deg: requested.skew_x_deg.unwrap_or(current.skew_x_deg),
        skew_y_deg: requested.skew_y_deg.unwrap_or(current.skew_y_deg),
        flip_h: requested.flip_h.unwrap_or(current.flip_h),
        flip_v: requested.flip_v.unwrap_or(current.flip_v),
    }
}

/// Verb-level validation failures for `clip.set_transform`.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum ClipSetTransformError {
    /// `args.clip` is not parseable as `UUIDv7`.
    #[error("clip.set_transform: `clip` selector parse failed: {detail}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
    },

    /// No clip exists for `clip`.
    #[error("clip.set_transform: clip `{clip_id}` not found")]
    ClipNotFound {
        /// Missing clip id.
        clip_id: String,
    },

    /// The target clip is locked.
    #[error("clip.set_transform: clip `{clip_id}` is locked")]
    Locked {
        /// Locked clip id.
        clip_id: String,
    },

    /// A supplied float field is non-finite.
    #[error("clip.set_transform: field `{field}` value {value} is not finite")]
    BadValue {
        /// Field name.
        field: &'static str,
        /// Rejected value.
        value: f64,
    },
}

/// Build the RFC-6902 patch for `clip.set_transform`.
///
/// # Errors
///
/// - [`ClipSetTransformError::BadSelector`] for non-UUIDv7 `args.clip`.
/// - [`ClipSetTransformError::ClipNotFound`] if `args.clip` resolves to no clip.
/// - [`ClipSetTransformError::Locked`] if the target clip is locked.
/// - [`ClipSetTransformError::BadValue`] if any supplied floating-point value is non-finite.
/// - Idempotent no-op path: empty patch + [`W_NOOP_CODE`] warning when
///   every supplied field matches current state.
pub fn compute_patch(
    prior: &Project,
    args: &ClipSetTransformArgs,
) -> Result<(Value, Vec<Value>, ClipSetTransformData), ClipSetTransformError> {
    let clip_id =
        args.clip
            .parse::<ClipId>()
            .map_err(|err| ClipSetTransformError::BadSelector {
                detail: err.to_string(),
            })?;

    let mut location: Option<(usize, usize, &crate::track::Track, &crate::clip::Clip)> = None;
    for (t_idx, track) in prior.tracks.iter().enumerate() {
        for (c_idx, clip) in track.clips.iter().enumerate() {
            if clip.id == clip_id {
                location = Some((t_idx, c_idx, track, clip));
                break;
            }
        }
        if location.is_some() {
            break;
        }
    }

    let (t_idx, c_idx, _track, clip) =
        location.ok_or_else(|| ClipSetTransformError::ClipNotFound {
            clip_id: args.clip.clone(),
        })?;

    if clip.locked {
        return Err(ClipSetTransformError::Locked {
            clip_id: args.clip.clone(),
        });
    }

    let diff = transform_diff(&args.transform, &clip.transform)?;

    if diff.is_empty() {
        return Ok((
            json!([]),
            vec![json!({
                "code": W_NOOP_CODE,
                "message": "clip transform unchanged",
                "details": {
                    "clip_id": clip_id.to_string(),
                    "transform": clip_transform_value(&clip.transform),
                }
            })],
            ClipSetTransformData {
                clip_id,
                transform: clip.transform,
            },
        ));
    }

    let mut ops = Vec::with_capacity(diff.len());
    for (field, value) in diff {
        ops.push(json!({
            "op": "replace",
            "path": format!("/tracks/{t_idx}/clips/{c_idx}/transform/{field}"),
            "value": value,
        }));
    }

    Ok((
        Value::Array(ops),
        Vec::new(),
        ClipSetTransformData {
            clip_id,
            transform: next_transform(&args.transform, &clip.transform),
        },
    ))
}

/// Rebuilds the envelope from `(args, post_state)`.
///
/// # Errors
///
/// Returns [`ReconstructError::TypeMismatch`] when `args.clip` is not
/// a valid `UUIDv7`, or [`ReconstructError::PostStateMissing`] when the
/// post-state does not contain the target clip.
pub fn data_envelope_from_post_state(
    args: &ClipSetTransformArgs,
    post_state: &Project,
) -> Result<ClipSetTransformData, ReconstructError> {
    let clip_id = args
        .clip
        .parse::<ClipId>()
        .map_err(|_| ReconstructError::TypeMismatch {
            name: "args.clip",
            expected: "UUIDv7 ClipId string",
        })?;

    for track in &post_state.tracks {
        for clip in &track.clips {
            if clip.id == clip_id {
                return Ok(ClipSetTransformData {
                    clip_id,
                    transform: clip.transform,
                });
            }
        }
    }

    Err(ReconstructError::PostStateMissing {
        detail: format!("clip set_transform: clip id {clip_id} not found in post_state"),
    })
}

/// `clip.set_transform` verb registration entry.
#[derive(Debug, Default)]
pub struct ClipSetTransformVerb;

impl From<ClipSetTransformError> for VerbError {
    fn from(value: ClipSetTransformError) -> Self {
        match value {
            ClipSetTransformError::BadSelector { .. }
            | ClipSetTransformError::ClipNotFound { .. }
            | ClipSetTransformError::Locked { .. }
            | ClipSetTransformError::BadValue { .. } => VerbError::BadArgs {
                detail: value.to_string(),
            },
        }
    }
}

impl Verb for ClipSetTransformVerb {
    fn verb(&self) -> &'static str {
        "clip.set_transform"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: ClipSetTransformArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("clip.set_transform: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, _data) = compute_patch(prior, &typed)?;

        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!(
                    "clip.set_transform: patch construction failed: {err}"
                ))
            })?;

        let post_state = prior
            .apply(&patch)
            .map_err(|err| VerbError::InvariantViolation {
                detail: format!("clip.set_transform: post-state validation failed: {err}"),
            })?;

        let envelope = data_envelope_from_post_state(&typed, &post_state).map_err(|err| {
            VerbError::Custom(format!(
                "clip.set_transform: data envelope reconstruction failed: {err}"
            ))
        })?;

        let data = serde_json::to_value(&envelope).map_err(|err| {
            VerbError::Custom(format!("clip.set_transform: data serialize failed: {err}"))
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
        let typed: ClipSetTransformArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "ClipSetTransformArgs",
            })?;

        let envelope = data_envelope_from_post_state(&typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
