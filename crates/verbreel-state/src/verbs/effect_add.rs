//! `effect.add` (§6.1) -- fiftieth production verb in the engine.
//!
//! Clip-attached effects only. Track-attached effects are rejected until
//! `Track.effects` is typed.

use crate::clip::Clip;
use crate::effect::{Effect, EffectKind, EffectWindow};
use crate::invariants::{EFFECT_PARAMS_MAX_BYTES, EFFECT_PARAMS_MAX_KEYS, timeline_duration_tk};
use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use crate::track::TrackKind;
use crate::verbs::effect_list_available::V1_EFFECT_KINDS;
use crate::verbs::project_set_fps::is_off_frame;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use thiserror::Error;
use verbreel_types::{ClipId, EffectId, ProjectId, TICK_RATE_HZ, Tick};

/// Warning code emitted when a requested effect window edge is snapped.
pub const W_TIME_SNAPPED_CODE: &str = "W_TIME_SNAPPED";

/// Internal warning code carrying the minted effect id for replay.
pub const W_EFFECT_ADD_ENVELOPE_CODE: &str = "W_EFFECT_ADD_ENVELOPE";

const QUALIFIED_TARGET_HINT: &str = "target must be qualified form, e.g. clip:<uuid>";
const TRACK_EFFECTS_DEFERRED_HINT: &str =
    "track-attached effects deferred; track.effects is not yet typed";

/// Arguments for `effect.add`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectAddArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Qualified target selector. This slice accepts only `clip:<uuidv7>`.
    pub target: String,

    /// Effect kind identifier.
    pub kind: String,

    /// Optional params object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Map<String, Value>>,

    /// Optional parent-relative window start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_tk: Option<i64>,

    /// Optional parent-relative window end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out_tk: Option<i64>,
}

/// Envelope `data` returned by `effect.add`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectAddData {
    /// Freshly minted effect id.
    pub effect_id: EffectId,

    /// Target kind. This slice always returns `clip`.
    pub target_kind: String,

    /// Target clip id.
    pub target_id: ClipId,
}

/// Verb-level validation failures for `effect.add`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EffectAddError {
    /// `args.target` is not parseable as a supported qualified selector.
    #[error("E_BAD_SELECTOR: effect.add: `target` selector parse failed: {detail}; hint: {hint}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
        /// User-facing selector hint.
        hint: &'static str,
    },

    /// `args.target` resolves to a noun kind this implementation does not accept.
    #[error(
        "E_SELECTOR_KIND_MISMATCH: effect.add: target prefix `{actual_prefix}` is not supported; hint: {hint}"
    )]
    SelectorKindMismatch {
        /// Prefix found before `:`.
        actual_prefix: String,
        /// User-facing remediation hint.
        hint: &'static str,
    },

    /// No clip exists for `args.target`.
    #[error("E_NOT_FOUND: effect.add: clip `{clip_id}` not found")]
    ClipNotFound {
        /// Missing clip id string.
        clip_id: String,
    },

    /// Exactly one of `in_tk` / `out_tk` was supplied.
    #[error("E_SCHEMA_VIOLATION: effect.add: `{field}`: {message}")]
    SchemaViolation {
        /// Details field name.
        field: &'static str,
        /// Details message.
        message: &'static str,
    },

    /// Effect kind is not registered in the v1.0 set.
    #[error("E_EFFECT_UNKNOWN_KIND: effect.add: kind `{kind}` is not registered")]
    UnknownKind {
        /// Requested kind.
        kind: String,
        /// Allowed v1.0 kinds.
        allowed: Vec<&'static str>,
    },

    /// The target effect kind is managed by a higher-level verb.
    #[error(
        "E_EFFECT_MANAGED: effect.add: kind `{kind}` is managed by `{managing_verb}`; hint: {hint}"
    )]
    ManagedEffect {
        /// Managed effect kind.
        kind: &'static str,
        /// Higher-level verb that owns mutation for this effect kind.
        managing_verb: &'static str,
        /// User-facing remediation hint from §6.2.
        hint: &'static str,
    },

    /// Target clip or parent track is locked.
    #[error("E_LOCKED: effect.add: clip `{failed_clip}` or its parent track is locked")]
    Locked {
        /// First failed clip id.
        failed_clip: String,
    },

    /// Effect window is invalid for the parent clip.
    #[error("E_BAD_TIME: effect.add: `{field}` value {value} is outside bound {bound:?}")]
    BadTime {
        /// Invalid field name.
        field: &'static str,
        /// Invalid value.
        value: i64,
        /// Parent-relative allowed bound.
        bound: [i64; 2],
    },

    /// Params exceed the §0.13 params caps.
    #[error("E_EFFECT_BAD_PARAMS: effect.add: `{field}` value {value} exceeds bound {bound}")]
    BadParams {
        /// Field path reported in `details.field`.
        field: &'static str,
        /// Spec bound.
        bound: usize,
        /// Actual value.
        value: usize,
    },
}

#[derive(Debug, Clone, Copy)]
struct LocatedClip<'a> {
    track_idx: usize,
    clip_idx: usize,
    track_kind: TrackKind,
    track_locked: bool,
    clip: &'a Clip,
}

/// Build the RFC-6902 patch for `effect.add`.
///
/// # Errors
///
/// Returns [`EffectAddError`] for selector, kind, lock, window, and
/// params-cap validation failures.
pub fn compute_patch(
    prior: &Project,
    args: &EffectAddArgs,
) -> Result<(Value, Vec<Value>, EffectAddData), EffectAddError> {
    let clip_id = parse_clip_target(args.target.as_str())?;
    let located = locate_clip(prior, clip_id).ok_or_else(|| EffectAddError::ClipNotFound {
        clip_id: clip_id.to_string(),
    })?;

    validate_window_pair(args)?;
    validate_kind(args.kind.as_str())?;
    reject_managed_effect(args.kind.as_str())?;

    if located.track_locked || located.clip.locked {
        return Err(EffectAddError::Locked {
            failed_clip: located.clip.id.to_string(),
        });
    }

    let parent_duration_tk = timeline_duration_tk(
        located.clip.source_in_tk,
        located.clip.source_out_tk,
        located.clip.speed,
    )
    .get();
    let mut warnings = Vec::new();
    let window = build_window(
        prior,
        located.track_kind,
        args,
        parent_duration_tk,
        &mut warnings,
    )?;

    let params = args.params.clone().unwrap_or_default();
    check_params_caps(&params)?;

    let effect_id = EffectId::now();
    let effect = Effect {
        id: effect_id,
        kind: EffectKind::new(args.kind.clone()).map_err(|_| EffectAddError::UnknownKind {
            kind: args.kind.clone(),
            allowed: V1_EFFECT_KINDS.to_vec(),
        })?,
        enabled: true,
        params,
        window,
    };

    let mut effects = located.clip.effects.clone();
    effects.push(effect);

    let data = EffectAddData {
        effect_id,
        target_kind: "clip".to_string(),
        target_id: clip_id,
    };
    warnings.push(envelope_warning(&data));

    Ok((
        json!([{
            "op": "replace",
            "path": format!("/tracks/{}/clips/{}/effects", located.track_idx, located.clip_idx),
            "value": effects,
        }]),
        warnings,
        data,
    ))
}

fn parse_clip_target(target: &str) -> Result<ClipId, EffectAddError> {
    let Some((prefix, body)) = target.split_once(':') else {
        return Err(EffectAddError::BadSelector {
            detail: "missing target prefix".to_string(),
            hint: QUALIFIED_TARGET_HINT,
        });
    };

    match prefix {
        "clip" => body
            .parse::<ClipId>()
            .map_err(|err| EffectAddError::BadSelector {
                detail: err.to_string(),
                hint: QUALIFIED_TARGET_HINT,
            }),
        "track" => Err(EffectAddError::SelectorKindMismatch {
            actual_prefix: prefix.to_string(),
            hint: TRACK_EFFECTS_DEFERRED_HINT,
        }),
        other => Err(EffectAddError::SelectorKindMismatch {
            actual_prefix: other.to_string(),
            hint: QUALIFIED_TARGET_HINT,
        }),
    }
}

fn locate_clip(project: &Project, clip_id: ClipId) -> Option<LocatedClip<'_>> {
    for (track_idx, track) in project.tracks.iter().enumerate() {
        for (clip_idx, clip) in track.clips.iter().enumerate() {
            if clip.id == clip_id {
                return Some(LocatedClip {
                    track_idx,
                    clip_idx,
                    track_kind: track.kind,
                    track_locked: track.locked,
                    clip,
                });
            }
        }
    }
    None
}

fn validate_window_pair(args: &EffectAddArgs) -> Result<(), EffectAddError> {
    if args.in_tk.is_some() ^ args.out_tk.is_some() {
        return Err(EffectAddError::SchemaViolation {
            field: "in_tk_out_tk_pair",
            message: "supply both or neither",
        });
    }
    Ok(())
}

fn validate_kind(kind: &str) -> Result<(), EffectAddError> {
    if V1_EFFECT_KINDS.contains(&kind) {
        Ok(())
    } else {
        Err(EffectAddError::UnknownKind {
            kind: kind.to_string(),
            allowed: V1_EFFECT_KINDS.to_vec(),
        })
    }
}

fn reject_managed_effect(kind: &str) -> Result<(), EffectAddError> {
    match kind {
        "time_stretch" => Err(EffectAddError::ManagedEffect {
            kind: "time_stretch",
            managing_verb: "clip.set_speed",
            hint: "call clip.set_speed --preserve_pitch false on the parent clip",
        }),
        "burned_caption" => Err(EffectAddError::ManagedEffect {
            kind: "burned_caption",
            managing_verb: "caption.burn_in",
            hint: "call caption.burn_off to remove the effect while keeping the source text track, or track.remove on the source text track to cascade-remove both, or effect.toggle --enabled false to disable without removing — see §10.5",
        }),
        "denoise" => Err(EffectAddError::ManagedEffect {
            kind: "denoise",
            managing_verb: "audio.denoise",
            hint: "call audio.denoise --strength 0 on the target to remove, or effect.toggle --enabled false to disable without removing",
        }),
        _ => Ok(()),
    }
}

fn build_window(
    prior: &Project,
    track_kind: TrackKind,
    args: &EffectAddArgs,
    parent_duration_tk: i64,
    warnings: &mut Vec<Value>,
) -> Result<Option<EffectWindow>, EffectAddError> {
    let (Some(mut in_tk), Some(mut out_tk)) = (args.in_tk, args.out_tk) else {
        return Ok(None);
    };

    validate_window_bounds(in_tk, out_tk, parent_duration_tk)?;

    if matches!(track_kind, TrackKind::Video | TrackKind::Text) {
        in_tk = snap_window_field(prior, in_tk, "in_tk", warnings);
        out_tk = snap_window_field(prior, out_tk, "out_tk", warnings);
    }

    validate_window_bounds(in_tk, out_tk, parent_duration_tk)?;

    Ok(Some(EffectWindow {
        in_tk: Tick::new(in_tk),
        out_tk: Tick::new(out_tk),
    }))
}

fn validate_window_bounds(
    in_tk: i64,
    out_tk: i64,
    parent_duration_tk: i64,
) -> Result<(), EffectAddError> {
    let bound = [0, parent_duration_tk];
    if in_tk < 0 {
        return Err(EffectAddError::BadTime {
            field: "in_tk",
            value: in_tk,
            bound,
        });
    }
    if out_tk <= in_tk {
        return Err(EffectAddError::BadTime {
            field: "out_tk",
            value: out_tk,
            bound,
        });
    }
    if out_tk > parent_duration_tk {
        return Err(EffectAddError::BadTime {
            field: "out_tk",
            value: out_tk,
            bound,
        });
    }
    Ok(())
}

fn snap_window_field(
    prior: &Project,
    value_tk: i64,
    field: &'static str,
    warnings: &mut Vec<Value>,
) -> i64 {
    if !is_off_frame(Tick::new(value_tk), prior.fps_num, prior.fps_den) {
        return value_tk;
    }

    let snapped_tk = nearest_frame_tick(value_tk, prior.fps_num, prior.fps_den);
    if snapped_tk != value_tk {
        warnings.push(json!({
            "code": W_TIME_SNAPPED_CODE,
            "message": "time value snapped to frame boundary",
            "details": {
                "field": field,
                "from_tk": value_tk,
                "to_tk": snapped_tk,
            }
        }));
    }
    snapped_tk
}

fn nearest_frame_tick(value_tk: i64, fps_num: u32, fps_den: u32) -> i64 {
    if fps_num == 0 {
        return value_tk;
    }

    let frame_clock = u64::from(TICK_RATE_HZ) * u64::from(fps_den);
    let step_tk = frame_clock / gcd_u64(frame_clock, u64::from(fps_num));
    if step_tk == 0 {
        return value_tk;
    }

    let value = i128::from(value_tk.max(0));
    let step = i128::from(step_tk);
    let snapped = ((value + (step / 2)) / step) * step;
    i64::try_from(snapped).unwrap_or(i64::MAX)
}

fn gcd_u64(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        let rem = a % b;
        a = b;
        b = rem;
    }
    a
}

fn check_params_caps(params: &Map<String, Value>) -> Result<(), EffectAddError> {
    let key_count = params.len();
    if key_count > EFFECT_PARAMS_MAX_KEYS {
        return Err(EffectAddError::BadParams {
            field: "params.keys",
            bound: EFFECT_PARAMS_MAX_KEYS,
            value: key_count,
        });
    }

    let bytes = verbreel_canon::jcs::canonicalize(&Value::Object(params.clone()))
        .map_or(usize::MAX, |v| v.len());
    if bytes > EFFECT_PARAMS_MAX_BYTES {
        return Err(EffectAddError::BadParams {
            field: "params.bytes",
            bound: EFFECT_PARAMS_MAX_BYTES,
            value: bytes,
        });
    }

    Ok(())
}

fn envelope_warning(data: &EffectAddData) -> Value {
    json!({
        "code": W_EFFECT_ADD_ENVELOPE_CODE,
        "message": "effect.add envelope",
        "details": data,
    })
}

/// Rebuild `EffectAddData` from recorded warnings.
///
/// # Errors
///
/// Returns [`ReconstructError`] if the internal envelope warning is
/// missing or malformed.
pub fn data_envelope_from_args_warnings(
    _args: &EffectAddArgs,
    warnings: &[Value],
) -> Result<EffectAddData, ReconstructError> {
    for warning in warnings {
        if warning.get("code").and_then(Value::as_str) != Some(W_EFFECT_ADD_ENVELOPE_CODE) {
            continue;
        }
        let details = warning
            .get("details")
            .ok_or(ReconstructError::MissingField {
                name: "warnings[].W_EFFECT_ADD_ENVELOPE.details",
            })?;
        return serde_json::from_value(details.clone()).map_err(|_| {
            ReconstructError::TypeMismatch {
                name: "warnings[].W_EFFECT_ADD_ENVELOPE.details",
                expected: "EffectAddData",
            }
        });
    }

    Err(ReconstructError::MissingField {
        name: "warnings[].W_EFFECT_ADD_ENVELOPE",
    })
}

impl From<EffectAddError> for VerbError {
    fn from(value: EffectAddError) -> Self {
        VerbError::BadArgs {
            detail: value.to_string(),
        }
    }
}

/// `effect.add` verb registration entry.
#[derive(Debug, Default)]
pub struct EffectAddVerb;

impl Verb for EffectAddVerb {
    fn verb(&self) -> &'static str {
        "effect.add"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: EffectAddArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("effect.add: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, _data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!("effect.add: patch construction failed: {err}"))
            })?;

        let post_state = prior
            .apply(&patch)
            .map_err(|err| VerbError::InvariantViolation {
                detail: format!("effect.add: post-state validation failed: {err}"),
            })?;
        drop(post_state);

        let envelope = data_envelope_from_args_warnings(&typed, &warnings).map_err(|err| {
            VerbError::Custom(format!(
                "effect.add: data envelope reconstruction failed: {err}"
            ))
        })?;
        let data = serde_json::to_value(&envelope).map_err(|err| {
            VerbError::Custom(format!("effect.add: data serialize failed: {err}"))
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
        let typed: EffectAddArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "EffectAddArgs",
            })?;

        let envelope = data_envelope_from_args_warnings(&typed, warnings)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
