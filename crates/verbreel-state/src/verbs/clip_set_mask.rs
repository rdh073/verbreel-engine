//! `clip.set_mask` (§5.19) — forty-first production verb in the engine.
//!
//! Sets or clears [`crate::clip::Clip::mask`]. When clearing a mask, or
//! switching mask kind, incompatible `mask.*` keyframes are cascade-removed
//! in the same patch and reported via `W_KEYFRAMES_REMOVED`.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::{AssetId, ClipId, KeyframeId, ProjectId};

use crate::asset::Asset;
use crate::clip::{Clip, ClipMask, MaskKind};
use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// Warning emitted when `clip.set_mask` cascade-removes mask keyframes.
pub const W_KEYFRAMES_REMOVED_CODE: &str = "W_KEYFRAMES_REMOVED";

/// Arguments for `clip.set_mask`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipSetMaskArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Target clip id as bare `UUIDv7`.
    pub clip: String,

    /// New mask value. `None` serializes as JSON `null` and removes the mask.
    pub mask: Option<ClipMask>,
}

/// Envelope returned by `clip.set_mask`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipSetMaskData {
    /// Target clip id.
    pub clip_id: ClipId,

    /// Post-state mask value.
    pub mask: Option<ClipMask>,
}

/// Verb-level validation failures for `clip.set_mask`.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum ClipSetMaskError {
    /// `args.clip` is not parseable as `UUIDv7`.
    #[error("E_BAD_SELECTOR: clip.set_mask: `clip` selector parse failed: {detail}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
    },

    /// No clip exists for `clip`.
    #[error("E_NOT_FOUND: clip.set_mask: clip `{clip_id}` not found")]
    ClipNotFound {
        /// Missing clip id string.
        clip_id: String,
    },

    /// The target clip or its parent track is locked.
    #[error("E_LOCKED: clip.set_mask: {kind} `{id}` is locked for clip `{clip_id}`")]
    Locked {
        /// Locked entity kind.
        kind: &'static str,
        /// Locked entity id.
        id: String,
        /// Target clip id string.
        clip_id: String,
    },

    /// `mask` is shape-valid JSON but violates a schema-layer scalar bound.
    #[error("E_SCHEMA_VIOLATION: clip.set_mask: `{field}` value {value} must satisfy {bound}")]
    SchemaViolation {
        /// Invalid field.
        field: &'static str,
        /// Invalid value.
        value: f64,
        /// Human-readable bound.
        bound: &'static str,
    },

    /// `mask.params` violates per-kind engine bounds.
    #[error(
        "E_MASK_INVALID_PARAMS: clip.set_mask: {kind} mask field `{field}` value {value} must satisfy {bound}"
    )]
    MaskInvalidParams {
        /// Mask kind.
        kind: &'static str,
        /// Invalid field.
        field: &'static str,
        /// Invalid value.
        value: Value,
        /// Human-readable bound.
        bound: &'static str,
    },

    /// Asset mask references a missing asset.
    #[error("E_ASSET_NOT_FOUND: clip.set_mask: mask asset `{asset_id}` not found")]
    AssetNotFound {
        /// Missing asset id.
        asset_id: String,
    },

    /// Asset mask references a non-image asset.
    #[error(
        "E_TRACK_KIND_MISMATCH: clip.set_mask: expected asset kind `image`, got `{actual_kind}` for `{asset_id}`"
    )]
    TrackKindMismatch {
        /// Referenced asset id.
        asset_id: AssetId,
        /// Expected asset kind.
        expected_kind: &'static str,
        /// Actual asset kind.
        actual_kind: &'static str,
    },
}

#[derive(Debug, Clone)]
struct LocatedClip<'a> {
    track_idx: usize,
    clip_idx: usize,
    track_locked: bool,
    track_id: String,
    clip: &'a Clip,
}

/// Build the RFC-6902 patch for `clip.set_mask`.
///
/// # Errors
/// Returns [`ClipSetMaskError`] for bad selector, missing clip, locked
/// target, invalid per-kind mask params, missing mask asset, or non-image
/// mask asset.
pub fn compute_patch(
    prior: &Project,
    args: &ClipSetMaskArgs,
) -> Result<(Value, Vec<Value>, ClipSetMaskData), ClipSetMaskError> {
    let clip_id = args
        .clip
        .parse::<ClipId>()
        .map_err(|err| ClipSetMaskError::BadSelector {
            detail: err.to_string(),
        })?;

    let located = locate_clip(prior, clip_id).ok_or_else(|| ClipSetMaskError::ClipNotFound {
        clip_id: args.clip.clone(),
    })?;

    if located.track_locked {
        return Err(ClipSetMaskError::Locked {
            kind: "track",
            id: located.track_id.clone(),
            clip_id: args.clip.clone(),
        });
    }

    if located.clip.locked {
        return Err(ClipSetMaskError::Locked {
            kind: "clip",
            id: located.clip.id.to_string(),
            clip_id: args.clip.clone(),
        });
    }

    if let Some(mask) = args.mask.as_ref() {
        validate_mask(prior, mask)?;
    }

    let (filtered_keyframes, removed_keyframe_ids) =
        filtered_keyframes_after_mask_change(located.clip, args.mask.as_ref());

    let mut ops = vec![json!({
        "op": "replace",
        "path": format!("/tracks/{}/clips/{}/mask", located.track_idx, located.clip_idx),
        "value": args.mask,
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
        vec![keyframes_removed_warning(clip_id, &removed_keyframe_ids)]
    };

    Ok((
        Value::Array(ops),
        warnings,
        ClipSetMaskData {
            clip_id,
            mask: args.mask.clone(),
        },
    ))
}

fn locate_clip(prior: &Project, clip_id: ClipId) -> Option<LocatedClip<'_>> {
    for (track_idx, track) in prior.tracks.iter().enumerate() {
        for (clip_idx, clip) in track.clips.iter().enumerate() {
            if clip.id == clip_id {
                return Some(LocatedClip {
                    track_idx,
                    clip_idx,
                    track_locked: track.locked,
                    track_id: track.id.to_string(),
                    clip,
                });
            }
        }
    }
    None
}

fn validate_mask(prior: &Project, mask: &ClipMask) -> Result<(), ClipSetMaskError> {
    if !mask.feather_px.is_finite() || mask.feather_px < 0.0 {
        return Err(ClipSetMaskError::SchemaViolation {
            field: "feather_px",
            value: mask.feather_px,
            bound: ">= 0",
        });
    }

    match mask.kind {
        MaskKind::Rect => {
            validate_positive(mask, "w")?;
            validate_positive(mask, "h")?;
        }
        MaskKind::Ellipse => {
            validate_positive(mask, "rx")?;
            validate_positive(mask, "ry")?;
        }
        MaskKind::Polygon => {
            let count = mask
                .params
                .get("points")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            if !(3..=256).contains(&count) {
                return Err(ClipSetMaskError::MaskInvalidParams {
                    kind: mask_kind_name(mask.kind),
                    field: "points",
                    value: json!(count),
                    bound: "3 <= points.len() <= 256",
                });
            }
        }
        MaskKind::Asset => {
            if let Some(threshold_value) = mask.params.get("threshold") {
                let Some(threshold) = threshold_value.as_f64() else {
                    return Err(ClipSetMaskError::MaskInvalidParams {
                        kind: mask_kind_name(mask.kind),
                        field: "threshold",
                        value: threshold_value.clone(),
                        bound: "0 <= threshold <= 1",
                    });
                };
                if !(0.0..=1.0).contains(&threshold) {
                    return Err(ClipSetMaskError::MaskInvalidParams {
                        kind: mask_kind_name(mask.kind),
                        field: "threshold",
                        value: threshold_value.clone(),
                        bound: "0 <= threshold <= 1",
                    });
                }
            }

            let asset_id = mask
                .params
                .get("asset_id")
                .and_then(Value::as_str)
                .ok_or_else(|| ClipSetMaskError::MaskInvalidParams {
                    kind: mask_kind_name(mask.kind),
                    field: "asset_id",
                    value: mask.params.get("asset_id").cloned().unwrap_or(Value::Null),
                    bound: "valid UUIDv7 asset_id",
                })?;
            let parsed_asset_id =
                asset_id
                    .parse::<AssetId>()
                    .map_err(|_| ClipSetMaskError::MaskInvalidParams {
                        kind: mask_kind_name(mask.kind),
                        field: "asset_id",
                        value: json!(asset_id),
                        bound: "valid UUIDv7 asset_id",
                    })?;
            let Some(asset) = prior
                .assets
                .iter()
                .find(|asset| *asset.id() == parsed_asset_id)
            else {
                return Err(ClipSetMaskError::AssetNotFound {
                    asset_id: asset_id.to_string(),
                });
            };
            let actual_kind = asset_kind_name(asset);
            if actual_kind != "image" {
                return Err(ClipSetMaskError::TrackKindMismatch {
                    asset_id: parsed_asset_id,
                    expected_kind: "image",
                    actual_kind,
                });
            }
        }
    }

    Ok(())
}

fn validate_positive(mask: &ClipMask, field: &'static str) -> Result<(), ClipSetMaskError> {
    let value = mask.params.get(field).cloned().unwrap_or(Value::Null);
    let valid = value.as_f64().is_some_and(|number| number > 0.0);
    if valid {
        Ok(())
    } else {
        Err(ClipSetMaskError::MaskInvalidParams {
            kind: mask_kind_name(mask.kind),
            field,
            value,
            bound: "> 0",
        })
    }
}

fn filtered_keyframes_after_mask_change(
    clip: &Clip,
    new_mask: Option<&ClipMask>,
) -> (Vec<crate::keyframe::Keyframe>, Vec<KeyframeId>) {
    let should_remove = |property: &str| match (clip.mask.as_ref(), new_mask) {
        (_, None) => property.starts_with("mask."),
        (Some(prior), Some(next)) if prior.kind != next.kind && property.starts_with("mask.") => {
            !valid_mask_property_for_kind(next.kind, property)
        }
        _ => false,
    };

    let mut filtered = Vec::with_capacity(clip.keyframes.len());
    let mut removed = Vec::new();
    for keyframe in &clip.keyframes {
        if should_remove(keyframe.property.as_str()) {
            removed.push(keyframe.id);
        } else {
            filtered.push(keyframe.clone());
        }
    }
    removed.sort_by_key(ToString::to_string);
    (filtered, removed)
}

fn valid_mask_property_for_kind(kind: MaskKind, property: &str) -> bool {
    match kind {
        MaskKind::Rect => matches!(
            property,
            "mask.params.x"
                | "mask.params.y"
                | "mask.params.w"
                | "mask.params.h"
                | "mask.feather_px"
        ),
        MaskKind::Ellipse => matches!(
            property,
            "mask.params.cx"
                | "mask.params.cy"
                | "mask.params.rx"
                | "mask.params.ry"
                | "mask.feather_px"
        ),
        MaskKind::Asset => matches!(property, "mask.params.threshold" | "mask.feather_px"),
        MaskKind::Polygon => matches!(property, "mask.feather_px"),
    }
}

fn keyframes_removed_warning(clip_id: ClipId, removed_keyframe_ids: &[KeyframeId]) -> Value {
    json!({
        "code": W_KEYFRAMES_REMOVED_CODE,
        "message": "mask keyframes incompatible with the new mask were removed",
        "details": {
            "clip_id": clip_id.to_string(),
            "removed_keyframe_ids": stringify_ids(removed_keyframe_ids),
        }
    })
}

fn stringify_ids<T: ToString>(ids: &[T]) -> Vec<String> {
    ids.iter().map(ToString::to_string).collect()
}

fn mask_kind_name(kind: MaskKind) -> &'static str {
    match kind {
        MaskKind::Rect => "rect",
        MaskKind::Ellipse => "ellipse",
        MaskKind::Polygon => "polygon",
        MaskKind::Asset => "asset",
    }
}

fn asset_kind_name(asset: &Asset) -> &'static str {
    match asset {
        Asset::Video(_) => "video",
        Asset::Audio(_) => "audio",
        Asset::Image(_) => "image",
        Asset::Subtitle(_) => "subtitle",
    }
}

/// Rebuilds the envelope from `(args, warnings, post_state)`.
///
/// # Errors
/// Returns [`ReconstructError`] when `args.clip` is invalid, the target clip
/// is missing in post-state, or any `W_KEYFRAMES_REMOVED` warning has
/// malformed details.
pub fn data_envelope_from_post_state_warnings(
    args: &ClipSetMaskArgs,
    warnings: &[Value],
    post_state: &Project,
) -> Result<ClipSetMaskData, ReconstructError> {
    validate_keyframes_removed_warnings(warnings)?;
    data_envelope_from_post_state(args, post_state)
}

/// Rebuilds the envelope from `(args, post_state)`.
///
/// # Errors
/// Returns [`ReconstructError::TypeMismatch`] when `args.clip` is not
/// a valid `UUIDv7`, or [`ReconstructError::PostStateMissing`] when the
/// post-state does not contain the target clip.
pub fn data_envelope_from_post_state(
    args: &ClipSetMaskArgs,
    post_state: &Project,
) -> Result<ClipSetMaskData, ReconstructError> {
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
                return Ok(ClipSetMaskData {
                    clip_id,
                    mask: clip.mask.clone(),
                });
            }
        }
    }

    Err(ReconstructError::PostStateMissing {
        detail: format!("clip.set_mask: clip id {clip_id} not found in post_state"),
    })
}

fn validate_keyframes_removed_warnings(warnings: &[Value]) -> Result<(), ReconstructError> {
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

        let ids = details
            .get("removed_keyframe_ids")
            .and_then(Value::as_array)
            .ok_or(ReconstructError::MissingField {
                name: "warnings[].W_KEYFRAMES_REMOVED.details.removed_keyframe_ids",
            })?;
        for id in ids {
            let raw = id.as_str().ok_or(ReconstructError::TypeMismatch {
                name: "warnings[].W_KEYFRAMES_REMOVED.details.removed_keyframe_ids[]",
                expected: "UUIDv7 KeyframeId string",
            })?;
            raw.parse::<KeyframeId>()
                .map_err(|_| ReconstructError::TypeMismatch {
                    name: "warnings[].W_KEYFRAMES_REMOVED.details.removed_keyframe_ids[]",
                    expected: "UUIDv7 KeyframeId string",
                })?;
        }
    }
    Ok(())
}

impl From<ClipSetMaskError> for VerbError {
    fn from(value: ClipSetMaskError) -> Self {
        VerbError::BadArgs {
            detail: value.to_string(),
        }
    }
}

/// `clip.set_mask` verb registration entry.
#[derive(Debug, Default)]
pub struct ClipSetMaskVerb;

impl Verb for ClipSetMaskVerb {
    fn verb(&self) -> &'static str {
        "clip.set_mask"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: ClipSetMaskArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("clip.set_mask: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, _data) = compute_patch(prior, &typed)?;

        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!("clip.set_mask: patch construction failed: {err}"))
            })?;

        let post_state = prior
            .apply(&patch)
            .map_err(|err| VerbError::InvariantViolation {
                detail: format!("clip.set_mask: post-state validation failed: {err}"),
            })?;

        let envelope = data_envelope_from_post_state_warnings(&typed, &warnings, &post_state)
            .map_err(|err| {
                VerbError::Custom(format!(
                    "clip.set_mask: data envelope reconstruction failed: {err}"
                ))
            })?;

        let data = serde_json::to_value(&envelope)
            .map_err(|err| VerbError::Custom(format!("clip.set_mask: data failed: {err}")))?;

        Ok((patch, data, warnings))
    }

    fn reconstruct(
        &self,
        args: &Value,
        _patch: &Value,
        warnings: &[Value],
        post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let typed: ClipSetMaskArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "ClipSetMaskArgs",
            })?;

        let envelope = data_envelope_from_post_state_warnings(&typed, warnings, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
