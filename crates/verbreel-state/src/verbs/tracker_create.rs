//! `tracker.create` (§18.1) — seventy-first production verb in the engine.
//!
//! Allocates a tracker resource. No compute happens at create time — the
//! verb only mints a clock-derived `tracker_id`, validates that the
//! resolved `clip` is video kind, validates the algorithm-specific
//! `params` against §18.6, and appends a placeholder Map to
//! `Project.trackers[]`. The heavy analysis pass happens in
//! `tracker.run` (not landed at this slice).
//!
//! ## Algorithm registry at v1.1
//!
//! Three algorithms are accepted: `object`, `face`, `optical_flow`.
//! `hands` is on the v1.x roadmap and is rejected here with
//! [`TrackerCreateError::UnknownAlgorithm`] per §18.6. Per-algorithm
//! params shapes:
//!
//! - `object`: requires `object_bbox_at_tk: { x, y, w, h, at_tk }`.
//!   `at_tk` must land within
//!   `[clip.track_position_tk, clip.track_position_tk + clip.timeline_duration_tk)`.
//! - `face`: params optional. If supplied, `min_face_size_px >= 1`
//!   (default `48`) and `confidence_threshold in [0.0, 1.0]`
//!   (default `0.5`).
//! - `optical_flow`: requires `point_at_tk: { x, y, at_tk }`. Same
//!   project-time window for `at_tk` as `object`.
//!
//! ## Reconstructor compatibility
//!
//! The minted `tracker_id` is clock-derived via `Uuid::now_v7()`.
//! Post-state alone cannot disambiguate which of multiple trackers a
//! given create call produced, so the forward path emits one internal
//! warning ([`W_TRACKER_CREATE_ENVELOPE_CODE`]) carrying `tracker_id`,
//! `source_clip_id`, and `algorithm`. The reconstructor reads that
//! warning back into [`TrackerCreateData`], mirroring the impure-mint
//! envelope pattern used by `clip.add` and the destructive-envelope
//! pattern used by `asset.remove` / `tracker.remove`.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use uuid::Uuid;
use verbreel_types::ProjectId;

use crate::clip::Clip;
use crate::invariants::timeline_duration_tk;
use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use crate::track::TrackKind;
use crate::tracker::Tracker;

/// Internal warning code carrying the clock-derived data envelope.
pub const W_TRACKER_CREATE_ENVELOPE_CODE: &str = "W_TRACKER_CREATE_ENVELOPE";

/// Default `min_face_size_px` per §18.6 when omitted from `face` params.
/// Documented at the constant level; not consumed at create time
/// because no analysis runs here (tracker.run owns the compute).
pub const FACE_DEFAULT_MIN_FACE_SIZE_PX: i64 = 48;

/// Default `confidence_threshold` per §18.6 when omitted from `face`
/// params. Same deferral as [`FACE_DEFAULT_MIN_FACE_SIZE_PX`].
pub const FACE_DEFAULT_CONFIDENCE_THRESHOLD: f64 = 0.5;

/// Algorithm enum for `tracker.create.algorithm`. v1.1 accepts three;
/// `Hands` is on the roadmap and is rejected by the verb with
/// [`TrackerCreateError::UnknownAlgorithm`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackerAlgorithm {
    /// Object-bbox follower. Requires `params.object_bbox_at_tk`.
    Object,
    /// Face detector / tracker. `params` optional.
    Face,
    /// Lucas-Kanade single-point optical flow. Requires
    /// `params.point_at_tk`.
    OpticalFlow,
    /// Hands tracker. v1.x roadmap — rejected at v1.1.
    Hands,
}

impl TrackerAlgorithm {
    fn as_str(self) -> &'static str {
        match self {
            TrackerAlgorithm::Object => "object",
            TrackerAlgorithm::Face => "face",
            TrackerAlgorithm::OpticalFlow => "optical_flow",
            TrackerAlgorithm::Hands => "hands",
        }
    }
}

/// Args for `tracker.create`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackerCreateArgs {
    /// Target project id.
    pub project_id: ProjectId,
    /// Source clip selector — bare `UUIDv7` or qualified `clip:<UUIDv7>`.
    pub clip: String,
    /// Tracker algorithm.
    pub algorithm: TrackerAlgorithm,
    /// Algorithm-specific params. Required for `object` /
    /// `optical_flow`; optional for `face` (treated as empty `{}` when
    /// absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

/// Envelope returned by `tracker.create`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackerCreateData {
    /// Freshly minted tracker id (`UUIDv7` string).
    pub tracker_id: String,
    /// Resolved source clip id (echoed back for round-trip clarity).
    pub source_clip_id: String,
    /// Algorithm name in `snake_case` (`object` / `face` /
    /// `optical_flow`).
    pub algorithm: String,
}

/// Verb-level validation failures for `tracker.create`.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum TrackerCreateError {
    /// `algorithm` is not in the engine's registered set (e.g. `hands`
    /// at v1.1).
    #[error("tracker.create: algorithm `{requested}` is not registered at v1.1")]
    UnknownAlgorithm {
        /// The rejected algorithm name (lowercase `snake_case`).
        requested: String,
    },

    /// Params failed the algorithm's schema (missing required field,
    /// out-of-range value, malformed shape, `at_tk` outside the source
    /// clip's project-time window).
    #[error("tracker.create: bad params `{field}`: {error}")]
    BadParams {
        /// JSON-pointer-style field name within `params`.
        field: String,
        /// Failure detail.
        error: String,
    },

    /// `clip` selector parse failure (bad UUID or bad prefix).
    #[error("tracker.create: `clip` selector parse failed: {detail}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
    },

    /// Qualified selector used a non-clip prefix (e.g. `track:`).
    #[error("tracker.create: selector prefix is not clip-kind (actual: `{actual_kind}`)")]
    SelectorKindMismatch {
        /// Offending prefix token.
        actual_kind: String,
    },

    /// Bare-UUID `clip` does not resolve to any clip.
    #[error("tracker.create: clip `{clip_id}` not found")]
    NotFound {
        /// Missing clip id string.
        clip_id: String,
    },

    /// Qualified-selector `clip` matched zero clips (structural selectors
    /// reserved for future v1.x — currently unreachable, kept for §0.4
    /// selector-taxonomy parity).
    #[error("tracker.create: clip selector `{selector}` matched no clip")]
    NoMatch {
        /// Original selector string.
        selector: String,
    },

    /// Resolved clip's parent track is not `kind: "video"`. Audio,
    /// image, and text clips have no per-frame motion data.
    #[error(
        "tracker.create: source clip is on a `{actual_kind}` track; trackers operate on video frames only"
    )]
    ClipKindMismatch {
        /// Actual parent track kind.
        actual_kind: String,
    },
}

impl From<TrackerCreateError> for VerbError {
    fn from(value: TrackerCreateError) -> Self {
        VerbError::BadArgs {
            detail: value.to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct LocatedClip<'a> {
    track_kind: TrackKind,
    clip: &'a Clip,
}

/// Build the RFC-6902 patch and envelope for `tracker.create`.
///
/// # Errors
///
/// Returns [`TrackerCreateError`] for unknown algorithm, selector parse
/// failures, missing / wrong-kind clip, or per-algorithm params failures.
pub fn compute_patch(
    prior: &Project,
    args: &TrackerCreateArgs,
) -> Result<(Value, Vec<Value>, TrackerCreateData), TrackerCreateError> {
    if args.algorithm == TrackerAlgorithm::Hands {
        return Err(TrackerCreateError::UnknownAlgorithm {
            requested: TrackerAlgorithm::Hands.as_str().to_string(),
        });
    }

    let (clip_id_str, was_qualified) = resolve_clip_selector(&args.clip)?;

    let located = locate_clip(prior, &clip_id_str).ok_or_else(|| {
        if was_qualified {
            TrackerCreateError::NoMatch {
                selector: args.clip.clone(),
            }
        } else {
            TrackerCreateError::NotFound {
                clip_id: args.clip.clone(),
            }
        }
    })?;

    if located.track_kind != TrackKind::Video {
        return Err(TrackerCreateError::ClipKindMismatch {
            actual_kind: track_kind_name(located.track_kind).to_string(),
        });
    }

    validate_params(args.algorithm, args.params.as_ref(), located.clip)?;

    let tracker_id = Uuid::now_v7().to_string();
    let algorithm_str = args.algorithm.as_str().to_string();
    let resolved_clip_id = located.clip.id.to_string();

    let mut tracker_map = serde_json::Map::new();
    tracker_map.insert("tracker_id".to_string(), json!(tracker_id));
    tracker_map.insert("source_clip_id".to_string(), json!(resolved_clip_id));
    tracker_map.insert("algorithm".to_string(), json!(algorithm_str));
    tracker_map.insert(
        "params".to_string(),
        args.params.clone().unwrap_or_else(|| json!({})),
    );
    tracker_map.insert("applied_to_clip_ids".to_string(), json!([]));
    tracker_map.insert("sample_count".to_string(), json!(-1));
    tracker_map.insert("cache_hash".to_string(), json!(""));
    tracker_map.insert("cache_path".to_string(), json!(""));

    let tracker = Tracker(tracker_map);

    let data = TrackerCreateData {
        tracker_id,
        source_clip_id: resolved_clip_id,
        algorithm: algorithm_str,
    };

    let patch = json!([{
        "op": "add",
        "path": "/trackers/-",
        "value": tracker,
    }]);

    let warnings = vec![envelope_warning(&data)];
    Ok((patch, warnings, data))
}

fn resolve_clip_selector(raw: &str) -> Result<(String, bool), TrackerCreateError> {
    if let Some((prefix, body)) = raw.split_once(':') {
        match prefix {
            "clip" => {
                if body.parse::<uuid::Uuid>().is_err() {
                    return Err(TrackerCreateError::BadSelector {
                        detail: format!("`{body}` is not a valid UUID"),
                    });
                }
                Ok((body.to_string(), true))
            }
            other => Err(TrackerCreateError::SelectorKindMismatch {
                actual_kind: other.to_string(),
            }),
        }
    } else {
        if raw.parse::<uuid::Uuid>().is_err() {
            return Err(TrackerCreateError::BadSelector {
                detail: format!("`{raw}` is not a valid UUID"),
            });
        }
        Ok((raw.to_string(), false))
    }
}

fn locate_clip<'a>(prior: &'a Project, clip_id_str: &str) -> Option<LocatedClip<'a>> {
    for track in &prior.tracks {
        for clip in &track.clips {
            if clip.id.to_string() == clip_id_str {
                return Some(LocatedClip {
                    track_kind: track.kind,
                    clip,
                });
            }
        }
    }
    None
}

fn track_kind_name(kind: TrackKind) -> &'static str {
    match kind {
        TrackKind::Video => "video",
        TrackKind::Audio => "audio",
        TrackKind::Text => "text",
        TrackKind::Effect => "effect",
    }
}

fn validate_params(
    algorithm: TrackerAlgorithm,
    params: Option<&Value>,
    clip: &Clip,
) -> Result<(), TrackerCreateError> {
    match algorithm {
        TrackerAlgorithm::Object => {
            let params = params.ok_or_else(|| TrackerCreateError::BadParams {
                field: "params".to_string(),
                error: "missing required params for `object`".to_string(),
            })?;
            let bbox = require_object_field(params, "object_bbox_at_tk")?;
            require_number(bbox, "object_bbox_at_tk.x")?;
            require_number(bbox, "object_bbox_at_tk.y")?;
            require_number(bbox, "object_bbox_at_tk.w")?;
            require_number(bbox, "object_bbox_at_tk.h")?;
            let at_tk = require_integer(bbox, "object_bbox_at_tk.at_tk")?;
            check_at_tk_in_clip_window(at_tk, clip, "object_bbox_at_tk.at_tk")?;
            Ok(())
        }
        TrackerAlgorithm::Face => {
            let Some(params) = params else {
                return Ok(());
            };
            if let Some(value) = params.get("min_face_size_px") {
                let min_face = value
                    .as_i64()
                    .ok_or_else(|| TrackerCreateError::BadParams {
                        field: "min_face_size_px".to_string(),
                        error: "expected integer".to_string(),
                    })?;
                if min_face < 1 {
                    return Err(TrackerCreateError::BadParams {
                        field: "min_face_size_px".to_string(),
                        error: format!("value {min_face} must be >= 1"),
                    });
                }
            }
            if let Some(value) = params.get("confidence_threshold") {
                let confidence = value
                    .as_f64()
                    .ok_or_else(|| TrackerCreateError::BadParams {
                        field: "confidence_threshold".to_string(),
                        error: "expected number".to_string(),
                    })?;
                if !(0.0..=1.0).contains(&confidence) {
                    return Err(TrackerCreateError::BadParams {
                        field: "confidence_threshold".to_string(),
                        error: format!("value {confidence} must be in [0.0, 1.0]"),
                    });
                }
            }
            Ok(())
        }
        TrackerAlgorithm::OpticalFlow => {
            let params = params.ok_or_else(|| TrackerCreateError::BadParams {
                field: "params".to_string(),
                error: "missing required params for `optical_flow`".to_string(),
            })?;
            let point = require_object_field(params, "point_at_tk")?;
            require_number(point, "point_at_tk.x")?;
            require_number(point, "point_at_tk.y")?;
            let at_tk = require_integer(point, "point_at_tk.at_tk")?;
            check_at_tk_in_clip_window(at_tk, clip, "point_at_tk.at_tk")?;
            Ok(())
        }
        TrackerAlgorithm::Hands => unreachable!("hands is rejected before validate_params"),
    }
}

fn require_object_field<'a>(
    parent: &'a Value,
    field: &str,
) -> Result<&'a Value, TrackerCreateError> {
    let value = parent
        .get(field)
        .ok_or_else(|| TrackerCreateError::BadParams {
            field: field.to_string(),
            error: "missing required field".to_string(),
        })?;
    if !value.is_object() {
        return Err(TrackerCreateError::BadParams {
            field: field.to_string(),
            error: "expected object".to_string(),
        });
    }
    Ok(value)
}

fn require_number(parent: &Value, dotted_field: &str) -> Result<f64, TrackerCreateError> {
    let leaf = dotted_field
        .rsplit_once('.')
        .map_or(dotted_field, |(_, leaf)| leaf);
    parent
        .get(leaf)
        .ok_or_else(|| TrackerCreateError::BadParams {
            field: dotted_field.to_string(),
            error: "missing required field".to_string(),
        })?
        .as_f64()
        .ok_or_else(|| TrackerCreateError::BadParams {
            field: dotted_field.to_string(),
            error: "expected number".to_string(),
        })
}

fn require_integer(parent: &Value, dotted_field: &str) -> Result<i64, TrackerCreateError> {
    let leaf = dotted_field
        .rsplit_once('.')
        .map_or(dotted_field, |(_, leaf)| leaf);
    parent
        .get(leaf)
        .ok_or_else(|| TrackerCreateError::BadParams {
            field: dotted_field.to_string(),
            error: "missing required field".to_string(),
        })?
        .as_i64()
        .ok_or_else(|| TrackerCreateError::BadParams {
            field: dotted_field.to_string(),
            error: "expected integer".to_string(),
        })
}

fn check_at_tk_in_clip_window(
    at_tk: i64,
    clip: &Clip,
    field: &str,
) -> Result<(), TrackerCreateError> {
    let start_tk = clip.track_position_tk.get();
    let duration_tk = timeline_duration_tk(clip.source_in_tk, clip.source_out_tk, clip.speed).get();
    let end_tk = start_tk.saturating_add(duration_tk);
    if at_tk < start_tk || at_tk >= end_tk {
        return Err(TrackerCreateError::BadParams {
            field: field.to_string(),
            error: "outside source clip window".to_string(),
        });
    }
    Ok(())
}

fn envelope_warning(data: &TrackerCreateData) -> Value {
    json!({
        "code": W_TRACKER_CREATE_ENVELOPE_CODE,
        "message": format!("tracker.create envelope for {}", data.tracker_id),
        "details": {
            "tracker_id": data.tracker_id,
            "source_clip_id": data.source_clip_id,
            "algorithm": data.algorithm,
        }
    })
}

/// Rebuild [`TrackerCreateData`] from recorded args and warnings.
///
/// # Errors
///
/// Returns [`ReconstructError`] if the internal envelope warning is
/// missing or malformed.
pub fn data_envelope_from_args_warnings(
    _args: &TrackerCreateArgs,
    warnings: &[Value],
) -> Result<TrackerCreateData, ReconstructError> {
    let details = envelope_details_from_warnings(warnings)?;
    Ok(TrackerCreateData {
        tracker_id: required_string(details, "tracker_id")?,
        source_clip_id: required_string(details, "source_clip_id")?,
        algorithm: required_string(details, "algorithm")?,
    })
}

fn envelope_details_from_warnings(warnings: &[Value]) -> Result<&Value, ReconstructError> {
    for warning in warnings {
        if warning.get("code").and_then(Value::as_str) != Some(W_TRACKER_CREATE_ENVELOPE_CODE) {
            continue;
        }
        return warning
            .get("details")
            .ok_or(ReconstructError::MissingField {
                name: "warnings[].W_TRACKER_CREATE_ENVELOPE.details",
            });
    }

    Err(ReconstructError::MissingField {
        name: "warnings[].W_TRACKER_CREATE_ENVELOPE",
    })
}

fn required_string(details: &Value, name: &'static str) -> Result<String, ReconstructError> {
    details
        .get(name)
        .ok_or(ReconstructError::MissingField { name })?
        .as_str()
        .map(String::from)
        .ok_or(ReconstructError::TypeMismatch {
            name,
            expected: "string",
        })
}

/// The §0.8 verb for `tracker.create`.
#[derive(Debug, Default)]
pub struct TrackerCreateVerb;

impl Verb for TrackerCreateVerb {
    fn verb(&self) -> &'static str {
        "tracker.create"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: TrackerCreateArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("tracker.create: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!("tracker.create: patch construction failed: {err}"))
            })?;
        let data = serde_json::to_value(&data)
            .map_err(|err| VerbError::Custom(format!("tracker.create: data failed: {err}")))?;

        Ok((patch, data, warnings))
    }

    fn reconstruct(
        &self,
        args: &Value,
        _patch: &Value,
        warnings: &[Value],
        _post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let typed: TrackerCreateArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "TrackerCreateArgs",
            })?;

        let envelope = data_envelope_from_args_warnings(&typed, warnings)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
