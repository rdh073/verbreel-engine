//! `caption.burn_in` (§10.4) — fifty-fifth production verb.
//!
//! Walks every video track in the project and emits one
//! `burned_caption` managed effect per video clip whose timeline range
//! intersects the union of text-clip ranges on the source text track.
//! Idempotent: re-running with the same text track updates the
//! existing matched effect rather than appending a duplicate; if hand-
//! edited state left two or more matches on a single clip the verb
//! dedups them and surfaces `W_CAPTION_BURN_DEDUP`.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use thiserror::Error;
use verbreel_types::{ClipId, EffectId, ProjectId, Tick, TrackId};

use crate::clip::Clip;
use crate::effect::{Effect, EffectKind, EffectWindow};
use crate::invariants::timeline_duration_tk;
use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use crate::track::TrackKind;
use crate::verbs::text_add::StyleArg;

/// Warning code emitted when no video clips overlap any text-clip range.
pub const W_NOOP_CODE: &str = "W_NOOP";

/// Warning code emitted per clip dedup-recovered from >=2 matches.
pub const W_CAPTION_BURN_DEDUP_CODE: &str = "W_CAPTION_BURN_DEDUP";

/// Internal warning code carrying minted/affected ids for replay.
pub const W_CAPTION_BURN_IN_ENVELOPE_CODE: &str = "W_CAPTION_BURN_IN_ENVELOPE";

const BURNED_CAPTION_KIND: &str = "burned_caption";

const PRESET_REGISTRY_DEFERRED_HINT: &str =
    "preset registry not implemented this slice; supply a literal style object";

const TEXT_STYLE_FIELDS: &[&str] = &[
    "content",
    "font_family",
    "font_size_px",
    "font_weight",
    "italic",
    "color",
    "bg_color",
    "stroke_color",
    "stroke_px",
    "align",
    "letter_spacing",
    "line_height",
    "shadow",
    "padding_px",
];

/// Arguments for `caption.burn_in`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptionBurnInArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Source text track selector: bare `UUIDv7` or `track:<uuid>`.
    pub text_track: String,

    /// Optional preset name or partial `TextElement` style object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<StyleArg>,
}

/// Envelope returned by `caption.burn_in`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptionBurnInData {
    /// Resolved source text track id.
    pub text_track_id: TrackId,

    /// Newly minted `burned_caption` effect ids, sorted by UUID string.
    pub effect_ids: Vec<EffectId>,

    /// Pre-existing matched effect ids that were updated in place.
    pub updated_effect_ids: Vec<EffectId>,

    /// Effect ids removed during multi-match dedup recovery.
    pub deduped_effect_ids: Vec<EffectId>,

    /// Clip ids whose `effects` array changed, sorted by UUID string.
    pub affected_clip_ids: Vec<ClipId>,
}

/// Verb-level validation failures for `caption.burn_in`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CaptionBurnInError {
    /// `text_track` failed to parse as a supported selector form.
    #[error("E_BAD_SELECTOR: caption.burn_in: `text_track` selector parse failed: {detail}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
    },

    /// `text_track` parsed but did not resolve to any track.
    #[error("E_TRACK_NOT_FOUND: caption.burn_in: text track `{track_id}` not found")]
    TrackNotFound {
        /// Missing track id string.
        track_id: String,
    },

    /// `text_track` resolved to a non-text track.
    #[error(
        "E_TRACK_KIND_MISMATCH: caption.burn_in: expected {expected_kind} track, got {actual_kind}"
    )]
    TrackKindMismatch {
        /// Expected track kind.
        expected_kind: &'static str,
        /// Actual track kind.
        actual_kind: &'static str,
    },

    /// `style` was supplied as a preset name; preset registry is deferred.
    #[error("E_PRESET_UNKNOWN: caption.burn_in: preset `{preset}` unknown; {hint}")]
    PresetUnknown {
        /// Requested preset name.
        preset: String,
        /// Recovery hint.
        hint: &'static str,
    },

    /// `style` object schema violation.
    #[error("E_SCHEMA_VIOLATION: caption.burn_in: style schema violation: {detail}")]
    StyleSchemaViolation {
        /// Human-readable detail.
        detail: String,
    },

    /// An affected video clip or its parent track is locked.
    #[error("E_LOCKED: caption.burn_in: clip `{failed_clip}` or its parent track is locked")]
    Locked {
        /// First locked affected clip in deterministic order.
        failed_clip: String,
    },
}

/// Intersection segment on a video clip's timeline in clip-relative ticks.
#[derive(Debug, Clone, Copy)]
struct WindowRange {
    in_tk: i64,
    out_tk: i64,
}

#[derive(Debug, Clone)]
struct AffectedClip<'a> {
    track_idx: usize,
    clip_idx: usize,
    track_locked: bool,
    clip: &'a Clip,
    window: WindowRange,
}

#[derive(Debug, Clone, Copy)]
enum Disposition {
    Create,
    Update,
    Dedup,
}

#[derive(Debug, Clone)]
struct ClipDispatch<'a> {
    affected: AffectedClip<'a>,
    disposition: Disposition,
    /// Existing matched effect ids, in `effects[]` declared order.
    matched_effect_ids: Vec<EffectId>,
    /// Newly minted effect id (Create path) or kept-effect id (Update / Dedup).
    primary_effect_id: EffectId,
    /// Effects removed during dedup; empty otherwise.
    removed_effect_ids: Vec<EffectId>,
}

/// Build the RFC-6902 patch for `caption.burn_in`.
///
/// # Errors
/// Returns [`CaptionBurnInError`] for selector parse, missing track,
/// track kind mismatch, preset registry miss, style schema violation,
/// or locked affected clip / parent track.
pub fn compute_patch(
    prior: &Project,
    args: &CaptionBurnInArgs,
) -> Result<(Value, Vec<Value>, CaptionBurnInData), CaptionBurnInError> {
    let text_track_id = resolve_text_track(prior, &args.text_track)?;
    let style_value = resolve_style(args.style.as_ref())?;

    let union_segments = text_clip_union_segments(prior, text_track_id);
    let affected_clips = collect_affected_clips(prior, &union_segments);

    if affected_clips.is_empty() {
        let data = CaptionBurnInData {
            text_track_id,
            effect_ids: Vec::new(),
            updated_effect_ids: Vec::new(),
            deduped_effect_ids: Vec::new(),
            affected_clip_ids: Vec::new(),
        };
        let warnings = vec![no_op_warning(text_track_id), envelope_warning(&data)];
        return Ok((json!([]), warnings, data));
    }

    for affected in &affected_clips {
        if affected.clip.locked || affected.track_locked {
            return Err(CaptionBurnInError::Locked {
                failed_clip: affected.clip.id.to_string(),
            });
        }
    }

    let dispatches = build_dispatches(text_track_id, affected_clips);

    let mut ops = Vec::new();
    let mut warnings = Vec::new();
    let mut effect_ids = Vec::new();
    let mut updated_effect_ids = Vec::new();
    let mut deduped_effect_ids = Vec::new();
    let mut affected_clip_ids = Vec::new();

    for dispatch in &dispatches {
        let new_effects = build_effects_for_clip(dispatch, style_value.as_ref(), text_track_id);
        ops.push(json!({
            "op": "replace",
            "path": format!(
                "/tracks/{}/clips/{}/effects",
                dispatch.affected.track_idx, dispatch.affected.clip_idx
            ),
            "value": new_effects,
        }));

        if !dispatch.removed_effect_ids.is_empty() {
            let filtered_keyframes =
                keyframes_without_effects(dispatch.affected.clip, &dispatch.removed_effect_ids);
            // Only replace keyframes when something was filtered out so
            // we don't emit redundant ops for the common no-keyframes
            // path.
            if filtered_keyframes.len() != dispatch.affected.clip.keyframes.len() {
                ops.push(json!({
                    "op": "replace",
                    "path": format!(
                        "/tracks/{}/clips/{}/keyframes",
                        dispatch.affected.track_idx, dispatch.affected.clip_idx
                    ),
                    "value": filtered_keyframes,
                }));
            }
            warnings.push(dedup_warning(
                dispatch.affected.clip.id,
                &dispatch.removed_effect_ids,
            ));
            deduped_effect_ids.extend(dispatch.removed_effect_ids.iter().copied());
        }

        match dispatch.disposition {
            Disposition::Create => effect_ids.push(dispatch.primary_effect_id),
            Disposition::Update | Disposition::Dedup => {
                updated_effect_ids.push(dispatch.primary_effect_id);
            }
        }
        affected_clip_ids.push(dispatch.affected.clip.id);
    }

    let data = CaptionBurnInData {
        text_track_id,
        effect_ids: sort_ids(effect_ids),
        updated_effect_ids: sort_ids(updated_effect_ids),
        deduped_effect_ids: sort_ids(deduped_effect_ids),
        affected_clip_ids: sort_ids(affected_clip_ids),
    };
    warnings.push(envelope_warning(&data));

    Ok((Value::Array(ops), warnings, data))
}

fn resolve_text_track(prior: &Project, raw: &str) -> Result<TrackId, CaptionBurnInError> {
    let body = parse_selector(raw)?;
    let track_id = body
        .parse::<TrackId>()
        .map_err(|err| CaptionBurnInError::BadSelector {
            detail: err.to_string(),
        })?;

    let track = prior
        .tracks
        .iter()
        .find(|track| track.id == track_id)
        .ok_or_else(|| CaptionBurnInError::TrackNotFound {
            track_id: raw.to_string(),
        })?;

    if track.kind != TrackKind::Text {
        return Err(CaptionBurnInError::TrackKindMismatch {
            expected_kind: "text",
            actual_kind: track_kind_name(track.kind),
        });
    }

    Ok(track_id)
}

fn parse_selector(raw: &str) -> Result<&str, CaptionBurnInError> {
    if let Some((prefix, body)) = raw.split_once(':') {
        if prefix != "track" {
            return Err(CaptionBurnInError::BadSelector {
                detail: format!("unsupported selector prefix `{prefix}`"),
            });
        }
        return Ok(body);
    }
    Ok(raw)
}

fn resolve_style(style: Option<&StyleArg>) -> Result<Option<Value>, CaptionBurnInError> {
    match style {
        None => Ok(None),
        Some(StyleArg::Preset(preset)) => Err(CaptionBurnInError::PresetUnknown {
            preset: preset.clone(),
            hint: PRESET_REGISTRY_DEFERRED_HINT,
        }),
        Some(StyleArg::Object(map)) => {
            validate_style_object(map)?;
            Ok(Some(Value::Object(map.clone())))
        }
    }
}

fn validate_style_object(map: &Map<String, Value>) -> Result<(), CaptionBurnInError> {
    for field in map.keys() {
        if !TEXT_STYLE_FIELDS.contains(&field.as_str()) {
            return Err(CaptionBurnInError::StyleSchemaViolation {
                detail: format!("unknown style field `{field}`"),
            });
        }
    }
    Ok(())
}

fn track_kind_name(kind: TrackKind) -> &'static str {
    match kind {
        TrackKind::Video => "video",
        TrackKind::Audio => "audio",
        TrackKind::Text => "text",
        TrackKind::Effect => "effect",
    }
}

/// Collect the union of every text clip's timeline range on `track_id`
/// as sorted, non-overlapping `[start, end)` segments.
fn text_clip_union_segments(prior: &Project, track_id: TrackId) -> Vec<(i64, i64)> {
    let Some(track) = prior.tracks.iter().find(|track| track.id == track_id) else {
        return Vec::new();
    };

    let mut ranges = track
        .clips
        .iter()
        .map(|clip| {
            let start = clip.track_position_tk.get();
            let duration =
                timeline_duration_tk(clip.source_in_tk, clip.source_out_tk, clip.speed).get();
            (start, start.saturating_add(duration))
        })
        .filter(|(start, end)| end > start)
        .collect::<Vec<_>>();
    ranges.sort_by_key(|(start, _)| *start);

    let mut merged: Vec<(i64, i64)> = Vec::new();
    for (start, end) in ranges {
        if let Some(last) = merged.last_mut()
            && start <= last.1
        {
            last.1 = last.1.max(end);
            continue;
        }
        merged.push((start, end));
    }
    merged
}

fn collect_affected_clips<'a>(
    prior: &'a Project,
    union_segments: &[(i64, i64)],
) -> Vec<AffectedClip<'a>> {
    let mut affected = Vec::new();
    if union_segments.is_empty() {
        return affected;
    }

    for (track_idx, track) in prior.tracks.iter().enumerate() {
        if track.kind != TrackKind::Video {
            continue;
        }
        for (clip_idx, clip) in track.clips.iter().enumerate() {
            let clip_start = clip.track_position_tk.get();
            let clip_duration =
                timeline_duration_tk(clip.source_in_tk, clip.source_out_tk, clip.speed).get();
            if clip_duration <= 0 {
                continue;
            }
            let clip_end = clip_start.saturating_add(clip_duration);

            let mut intersection_start: Option<i64> = None;
            let mut intersection_end: Option<i64> = None;
            for &(seg_start, seg_end) in union_segments {
                let lo = seg_start.max(clip_start);
                let hi = seg_end.min(clip_end);
                if lo < hi {
                    intersection_start = Some(match intersection_start {
                        Some(prev) => prev.min(lo),
                        None => lo,
                    });
                    intersection_end = Some(match intersection_end {
                        Some(prev) => prev.max(hi),
                        None => hi,
                    });
                }
            }

            let (Some(start_abs), Some(end_abs)) = (intersection_start, intersection_end) else {
                continue;
            };

            let in_tk = (start_abs - clip_start).max(0);
            let out_tk = (end_abs - clip_start).min(clip_duration);
            if out_tk <= in_tk {
                continue;
            }

            affected.push(AffectedClip {
                track_idx,
                clip_idx,
                track_locked: track.locked,
                clip,
                window: WindowRange { in_tk, out_tk },
            });
        }
    }
    affected
}

fn build_dispatches(
    text_track_id: TrackId,
    affected: Vec<AffectedClip<'_>>,
) -> Vec<ClipDispatch<'_>> {
    let track_id_str = text_track_id.to_string();
    affected
        .into_iter()
        .map(|affected| {
            let matched_effect_ids = affected
                .clip
                .effects
                .iter()
                .filter(|effect| {
                    effect.kind.as_str() == BURNED_CAPTION_KIND
                        && effect
                            .params
                            .get("source_text_track_id")
                            .and_then(Value::as_str)
                            == Some(track_id_str.as_str())
                })
                .map(|effect| effect.id)
                .collect::<Vec<_>>();

            let (disposition, primary_effect_id, removed_effect_ids) =
                match matched_effect_ids.len() {
                    0 => (Disposition::Create, EffectId::now(), Vec::new()),
                    1 => (Disposition::Update, matched_effect_ids[0], Vec::new()),
                    _ => {
                        let primary = matched_effect_ids[0];
                        let removed = matched_effect_ids[1..].to_vec();
                        (Disposition::Dedup, primary, removed)
                    }
                };

            ClipDispatch {
                affected,
                disposition,
                matched_effect_ids,
                primary_effect_id,
                removed_effect_ids,
            }
        })
        .collect()
}

fn build_effects_for_clip(
    dispatch: &ClipDispatch<'_>,
    style_value: Option<&Value>,
    text_track_id: TrackId,
) -> Vec<Effect> {
    let mut params = Map::new();
    params.insert(
        "source_text_track_id".to_string(),
        Value::String(text_track_id.to_string()),
    );
    if let Some(style) = style_value {
        params.insert("style".to_string(), style.clone());
    }

    let window = Some(EffectWindow {
        in_tk: Tick::new(dispatch.affected.window.in_tk),
        out_tk: Tick::new(dispatch.affected.window.out_tk),
    });

    let new_effect = Effect {
        id: dispatch.primary_effect_id,
        kind: EffectKind::new(BURNED_CAPTION_KIND.to_string())
            .expect("burned_caption is non-empty"),
        enabled: true,
        params: params.clone(),
        window,
    };

    match dispatch.disposition {
        Disposition::Create => {
            let mut effects = dispatch.affected.clip.effects.clone();
            effects.push(new_effect);
            effects
        }
        Disposition::Update | Disposition::Dedup => {
            let removed_set: std::collections::HashSet<EffectId> =
                dispatch.removed_effect_ids.iter().copied().collect();
            let mut out = Vec::with_capacity(dispatch.affected.clip.effects.len());
            // Look up only the FIRST matched id (we keep that in place).
            let kept_match = dispatch.matched_effect_ids.first().copied();
            for effect in &dispatch.affected.clip.effects {
                if removed_set.contains(&effect.id) {
                    continue;
                }
                if Some(effect.id) == kept_match {
                    let mut updated = effect.clone();
                    updated.enabled = true;
                    updated.window = window;
                    updated.params.insert(
                        "source_text_track_id".to_string(),
                        Value::String(text_track_id.to_string()),
                    );
                    if let Some(style) = style_value {
                        updated.params.insert("style".to_string(), style.clone());
                    }
                    out.push(updated);
                } else {
                    out.push(effect.clone());
                }
            }
            out
        }
    }
}

fn keyframes_without_effects(
    clip: &Clip,
    removed_effect_ids: &[EffectId],
) -> Vec<crate::keyframe::Keyframe> {
    use crate::invariants::extract_effect_id_from_property;
    let removed_set: std::collections::HashSet<String> =
        removed_effect_ids.iter().map(EffectId::to_string).collect();
    clip.keyframes
        .iter()
        .filter(|keyframe| {
            extract_effect_id_from_property(keyframe.property.as_str())
                .is_none_or(|effect_id| !removed_set.contains(effect_id))
        })
        .cloned()
        .collect()
}

fn no_op_warning(text_track_id: TrackId) -> Value {
    json!({
        "code": W_NOOP_CODE,
        "message": "no video clips overlap any text-clip range on the source track",
        "details": {
            "message": "no video clips overlap any text-clip range on the source track",
            "text_track_id": text_track_id.to_string(),
        }
    })
}

fn dedup_warning(clip_id: ClipId, removed_effect_ids: &[EffectId]) -> Value {
    json!({
        "code": W_CAPTION_BURN_DEDUP_CODE,
        "message": "multiple burned_caption effects for the same source text track collapsed to one",
        "details": {
            "clip_id": clip_id.to_string(),
            "removed_effect_ids": removed_effect_ids.iter().map(EffectId::to_string).collect::<Vec<_>>(),
        }
    })
}

fn envelope_warning(data: &CaptionBurnInData) -> Value {
    json!({
        "code": W_CAPTION_BURN_IN_ENVELOPE_CODE,
        "message": "caption.burn_in envelope",
        "details": {
            "text_track_id": data.text_track_id.to_string(),
            "effect_ids": data.effect_ids.iter().map(EffectId::to_string).collect::<Vec<_>>(),
            "updated_effect_ids": data.updated_effect_ids.iter().map(EffectId::to_string).collect::<Vec<_>>(),
            "deduped_effect_ids": data.deduped_effect_ids.iter().map(EffectId::to_string).collect::<Vec<_>>(),
            "affected_clip_ids": data.affected_clip_ids.iter().map(ClipId::to_string).collect::<Vec<_>>(),
        }
    })
}

fn sort_ids<T>(ids: impl IntoIterator<Item = T>) -> Vec<T>
where
    T: ToString,
{
    let mut out = ids.into_iter().collect::<Vec<_>>();
    out.sort_by_key(ToString::to_string);
    out
}

/// Rebuild `CaptionBurnInData` from recorded warnings.
///
/// # Errors
/// Returns [`ReconstructError`] if the internal envelope warning is
/// absent or malformed.
pub fn data_envelope_from_warnings(
    warnings: &[Value],
) -> Result<CaptionBurnInData, ReconstructError> {
    let details = envelope_details(warnings)?;
    Ok(CaptionBurnInData {
        text_track_id: required_id(details, "text_track_id")?,
        effect_ids: required_id_list(details, "effect_ids")?,
        updated_effect_ids: required_id_list(details, "updated_effect_ids")?,
        deduped_effect_ids: required_id_list(details, "deduped_effect_ids")?,
        affected_clip_ids: required_id_list(details, "affected_clip_ids")?,
    })
}

fn envelope_details(warnings: &[Value]) -> Result<&Value, ReconstructError> {
    for warning in warnings {
        if warning.get("code").and_then(Value::as_str) != Some(W_CAPTION_BURN_IN_ENVELOPE_CODE) {
            continue;
        }
        return warning
            .get("details")
            .ok_or(ReconstructError::MissingField {
                name: "warnings[].W_CAPTION_BURN_IN_ENVELOPE.details",
            });
    }
    Err(ReconstructError::MissingField {
        name: "warnings[].W_CAPTION_BURN_IN_ENVELOPE",
    })
}

fn required_id<T>(details: &Value, name: &'static str) -> Result<T, ReconstructError>
where
    T: std::str::FromStr,
{
    let raw = details
        .get(name)
        .ok_or(ReconstructError::MissingField { name })?
        .as_str()
        .ok_or(ReconstructError::TypeMismatch {
            name,
            expected: "UUIDv7 string",
        })?;
    raw.parse::<T>()
        .map_err(|_| ReconstructError::TypeMismatch {
            name,
            expected: "UUIDv7 string",
        })
}

fn required_id_list<T>(details: &Value, name: &'static str) -> Result<Vec<T>, ReconstructError>
where
    T: std::str::FromStr + ToString,
{
    let values = details
        .get(name)
        .ok_or(ReconstructError::MissingField { name })?
        .as_array()
        .ok_or(ReconstructError::TypeMismatch {
            name,
            expected: "array of UUIDv7 strings",
        })?;
    let ids = values
        .iter()
        .map(|value| {
            let raw = value.as_str().ok_or(ReconstructError::TypeMismatch {
                name,
                expected: "array of UUIDv7 strings",
            })?;
            raw.parse::<T>()
                .map_err(|_| ReconstructError::TypeMismatch {
                    name,
                    expected: "array of UUIDv7 strings",
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(sort_ids(ids))
}

impl From<CaptionBurnInError> for VerbError {
    fn from(value: CaptionBurnInError) -> Self {
        VerbError::BadArgs {
            detail: value.to_string(),
        }
    }
}

/// `caption.burn_in` verb registration entry.
#[derive(Debug, Default)]
pub struct CaptionBurnInVerb;

impl Verb for CaptionBurnInVerb {
    fn verb(&self) -> &'static str {
        "caption.burn_in"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: CaptionBurnInArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("caption.burn_in: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, _data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!("caption.burn_in: patch construction failed: {err}"))
            })?;

        let post_state = prior
            .apply(&patch)
            .map_err(|err| VerbError::InvariantViolation {
                detail: format!("caption.burn_in: post-state validation failed: {err}"),
            })?;
        drop(post_state);

        let envelope = data_envelope_from_warnings(&warnings).map_err(|err| {
            VerbError::Custom(format!(
                "caption.burn_in: data envelope reconstruction failed: {err}"
            ))
        })?;
        let data = serde_json::to_value(&envelope).map_err(|err| {
            VerbError::Custom(format!("caption.burn_in: data serialize failed: {err}"))
        })?;

        Ok((patch, data, warnings))
    }

    fn reconstruct(
        &self,
        _args: &Value,
        _patch: &Value,
        warnings: &[Value],
        _post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let envelope = data_envelope_from_warnings(warnings)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
