//! `audio.fade` (§9.3) — fifty-ninth production verb in the engine.
//!
//! ## Behavior
//!
//! Alias for `clip.set_fade` (§5.12) scoped to **audio clips only**.
//! Writes `Clip.fade_in_tk`, `Clip.fade_out_tk`, `Clip.fade_in_curve`,
//! and `Clip.fade_out_curve` on the resolved audio clip.
//!
//! `curve` is a convenience that writes both `fade_in_curve` and
//! `fade_out_curve` to the same value. Supplying `curve` together with
//! either `curve_in` or `curve_out` is `E_ARGS_INCOMPATIBLE`. For
//! cross-kind callers (video/text/image) use `clip.set_fade` directly —
//! this verb rejects non-audio clips with `E_CLIP_KIND_MISMATCH` to keep
//! the `audio.*` namespace discoverability promise.
//!
//! ## Selector handling
//!
//! `clip` accepts a bare `UUIDv7` (resolved against `Project.tracks`)
//! OR a qualified `clip:<UUIDv7>` selector. A qualified selector with a
//! different prefix (e.g. `track:<UUIDv7>`) is `E_SELECTOR_KIND_MISMATCH`.
//! A bare UUID with no match is `E_NOT_FOUND`; a qualified selector with
//! no match is `E_NO_MATCH` — matching the §0.4 selector-resolution
//! taxonomy used by `clip.set_blend_mode` / `clip.lock` and the new-style
//! `parse_selector` precedent in `caption_burn_in` (§10.4).

use crate::clip::FadeCurve;
use crate::invariants::timeline_duration_tk;
use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use crate::track::TrackKind;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::{ClipId, ProjectId};

/// Recovery hint emitted with `E_ARGS_INCOMPATIBLE` when `curve` is
/// combined with `curve_in` or `curve_out`.
pub const CURVE_COMBO_HINT: &str =
    "use curve_in / curve_out for per-direction control, or curve alone for both";

/// Args for `audio.fade`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioFadeArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Target clip id — bare `UUIDv7` or qualified `clip:<UUIDv7>` selector.
    pub clip: String,

    /// Optional fade-in duration in ticks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade_in_tk: Option<i64>,

    /// Optional fade-out duration in ticks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade_out_tk: Option<i64>,

    /// Convenience: writes both `fade_in_curve` and `fade_out_curve`.
    /// Mutually exclusive with `curve_in` / `curve_out`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curve: Option<FadeCurve>,

    /// Per-direction fade-in curve.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curve_in: Option<FadeCurve>,

    /// Per-direction fade-out curve.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curve_out: Option<FadeCurve>,
}

/// Envelope `data` returned by `audio.fade`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioFadeData {
    /// Target clip id.
    pub clip_id: ClipId,

    /// Post-state fade-in duration in ticks.
    pub fade_in_tk: i64,

    /// Post-state fade-out duration in ticks.
    pub fade_out_tk: i64,

    /// Post-state fade-in curve.
    pub fade_in_curve: FadeCurve,

    /// Post-state fade-out curve.
    pub fade_out_curve: FadeCurve,
}

/// Verb-level validation failures for `audio.fade`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AudioFadeError {
    /// `args.clip` is malformed (bad UUID or bad prefix).
    #[error("audio.fade: `clip` selector parse failed: {detail}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
    },

    /// Bare-UUID `args.clip` does not match any clip.
    #[error("audio.fade: clip `{clip_id}` not found")]
    NotFound {
        /// Missing clip id string.
        clip_id: String,
    },

    /// Qualified-selector `args.clip` matched zero clips.
    #[error("audio.fade: clip selector `{selector}` matched no clip")]
    NoMatch {
        /// Original selector string.
        selector: String,
    },

    /// Qualified selector used a non-clip prefix (e.g. `track:`).
    #[error("audio.fade: selector prefix `{actual_prefix}` is not clip-kind")]
    SelectorKindMismatch {
        /// Offending prefix token.
        actual_prefix: String,
    },

    /// Target clip's parent track is not `kind: "audio"`.
    #[error(
        "audio.fade: clip `{clip_id}` is on a `{actual_kind}` track; audio.fade is audio-only \
         — use clip.set_fade for cross-kind targets"
    )]
    ClipKindMismatch {
        /// Target clip id.
        clip_id: String,
        /// Actual parent track kind.
        actual_kind: &'static str,
    },

    /// A provided fade duration is negative.
    #[error("audio.fade: `{field}` value {value} must be >= 0")]
    BadTime {
        /// Invalid field name.
        field: &'static str,
        /// Invalid value.
        value: i64,
    },

    /// A provided fade duration exceeds the clip's timeline duration,
    /// or the fade sum overflows it.
    #[error(
        "audio.fade: fade sum {fade_sum_tk} exceeds timeline_duration_tk {timeline_duration_tk}"
    )]
    BadRange {
        /// Sum of fade-in and fade-out ticks.
        fade_sum_tk: i64,
        /// Clip timeline duration in ticks.
        timeline_duration_tk: i64,
    },

    /// `curve` was combined with `curve_in` or `curve_out`, or no fade
    /// field was supplied at all.
    #[error("audio.fade: {detail}")]
    ArgsIncompatible {
        /// Failure detail.
        detail: String,
        /// Recovery hint.
        hint: &'static str,
    },

    /// Target clip or its parent track is locked.
    #[error("audio.fade: {kind} `{id}` is locked for clip `{clip_id}`")]
    Locked {
        /// Locked entity kind (`"clip"` or `"track"`).
        kind: &'static str,
        /// Locked entity id.
        id: String,
        /// Target clip id string.
        clip_id: String,
    },
}

#[derive(Debug, Clone)]
struct LocatedClip<'a> {
    track_idx: usize,
    clip_idx: usize,
    track_kind: TrackKind,
    track_locked: bool,
    track_id: String,
    clip: &'a crate::clip::Clip,
}

/// Build the RFC-6902 patch for `audio.fade`.
///
/// # Errors
///
/// Returns [`AudioFadeError`] for selector parse failure, missing clip,
/// non-audio target, locked target, negative or out-of-range fade
/// durations, or incompatible curve-arg combinations.
pub fn compute_patch(
    prior: &Project,
    args: &AudioFadeArgs,
) -> Result<(Value, Vec<Value>, AudioFadeData), AudioFadeError> {
    validate_curve_combo(args)?;

    let (clip_id, was_qualified) = resolve_clip_selector(&args.clip)?;

    let located = locate_clip(prior, clip_id).ok_or_else(|| {
        if was_qualified {
            AudioFadeError::NoMatch {
                selector: args.clip.clone(),
            }
        } else {
            AudioFadeError::NotFound {
                clip_id: args.clip.clone(),
            }
        }
    })?;

    // §9.3 audio-only guard: rejection happens BEFORE locked / time /
    // range checks so callers get the discoverability error first
    // ("you're calling the wrong verb"), not a downstream symptom.
    if located.track_kind != TrackKind::Audio {
        return Err(AudioFadeError::ClipKindMismatch {
            clip_id: args.clip.clone(),
            actual_kind: track_kind_name(located.track_kind),
        });
    }

    if located.track_locked {
        return Err(AudioFadeError::Locked {
            kind: "track",
            id: located.track_id,
            clip_id: args.clip.clone(),
        });
    }

    if located.clip.locked {
        return Err(AudioFadeError::Locked {
            kind: "clip",
            id: located.clip.id.to_string(),
            clip_id: args.clip.clone(),
        });
    }

    validate_non_negative(args.fade_in_tk, "fade_in_tk")?;
    validate_non_negative(args.fade_out_tk, "fade_out_tk")?;

    let fade_in_tk = args
        .fade_in_tk
        .unwrap_or_else(|| located.clip.fade_in_tk.get());
    let fade_out_tk = args
        .fade_out_tk
        .unwrap_or_else(|| located.clip.fade_out_tk.get());

    let (fade_in_curve, fade_out_curve) = resolve_curves(args, located.clip);

    let duration_tk = timeline_duration_tk(
        located.clip.source_in_tk,
        located.clip.source_out_tk,
        located.clip.speed,
    )
    .get();
    check_fade_sum(fade_in_tk, fade_out_tk, duration_tk)?;

    let data = AudioFadeData {
        clip_id,
        fade_in_tk,
        fade_out_tk,
        fade_in_curve,
        fade_out_curve,
    };

    let patch = json!([
        {
            "op": "replace",
            "path": format!("/tracks/{}/clips/{}/fade_in_tk", located.track_idx, located.clip_idx),
            "value": fade_in_tk,
        },
        {
            "op": "replace",
            "path": format!("/tracks/{}/clips/{}/fade_out_tk", located.track_idx, located.clip_idx),
            "value": fade_out_tk,
        },
        {
            "op": "replace",
            "path": format!("/tracks/{}/clips/{}/fade_in_curve", located.track_idx, located.clip_idx),
            "value": fade_in_curve,
        },
        {
            "op": "replace",
            "path": format!("/tracks/{}/clips/{}/fade_out_curve", located.track_idx, located.clip_idx),
            "value": fade_out_curve,
        },
    ]);

    Ok((patch, Vec::new(), data))
}

fn validate_curve_combo(args: &AudioFadeArgs) -> Result<(), AudioFadeError> {
    if args.curve.is_some() && (args.curve_in.is_some() || args.curve_out.is_some()) {
        return Err(AudioFadeError::ArgsIncompatible {
            detail: "`curve` cannot be combined with `curve_in` / `curve_out`".to_string(),
            hint: CURVE_COMBO_HINT,
        });
    }

    if args.fade_in_tk.is_none()
        && args.fade_out_tk.is_none()
        && args.curve.is_none()
        && args.curve_in.is_none()
        && args.curve_out.is_none()
    {
        return Err(AudioFadeError::ArgsIncompatible {
            detail: "at least one fade field must be supplied".to_string(),
            hint: CURVE_COMBO_HINT,
        });
    }

    Ok(())
}

fn resolve_clip_selector(raw: &str) -> Result<(ClipId, bool), AudioFadeError> {
    if let Some((prefix, body)) = raw.split_once(':') {
        match prefix {
            "clip" => {
                let clip_id =
                    body.parse::<ClipId>()
                        .map_err(|err| AudioFadeError::BadSelector {
                            detail: err.to_string(),
                        })?;
                Ok((clip_id, true))
            }
            other => Err(AudioFadeError::SelectorKindMismatch {
                actual_prefix: other.to_string(),
            }),
        }
    } else {
        let clip_id = raw
            .parse::<ClipId>()
            .map_err(|err| AudioFadeError::BadSelector {
                detail: err.to_string(),
            })?;
        Ok((clip_id, false))
    }
}

fn resolve_curves(args: &AudioFadeArgs, clip: &crate::clip::Clip) -> (FadeCurve, FadeCurve) {
    if let Some(shared) = args.curve {
        return (shared, shared);
    }
    let fade_in_curve = args.curve_in.unwrap_or(clip.fade_in_curve);
    let fade_out_curve = args.curve_out.unwrap_or(clip.fade_out_curve);
    (fade_in_curve, fade_out_curve)
}

fn locate_clip(prior: &Project, clip_id: ClipId) -> Option<LocatedClip<'_>> {
    for (track_idx, track) in prior.tracks.iter().enumerate() {
        for (clip_idx, clip) in track.clips.iter().enumerate() {
            if clip.id == clip_id {
                return Some(LocatedClip {
                    track_idx,
                    clip_idx,
                    track_kind: track.kind,
                    track_locked: track.locked,
                    track_id: track.id.to_string(),
                    clip,
                });
            }
        }
    }
    None
}

fn validate_non_negative(value: Option<i64>, field: &'static str) -> Result<(), AudioFadeError> {
    if let Some(value) = value
        && value < 0
    {
        return Err(AudioFadeError::BadTime { field, value });
    }
    Ok(())
}

fn check_fade_sum(
    fade_in_tk: i64,
    fade_out_tk: i64,
    duration_tk: i64,
) -> Result<(), AudioFadeError> {
    let fade_sum_tk = fade_in_tk.saturating_add(fade_out_tk);
    if fade_sum_tk > duration_tk {
        return Err(AudioFadeError::BadRange {
            fade_sum_tk,
            timeline_duration_tk: duration_tk,
        });
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

/// Rebuilds the envelope from `(args, post_state)`.
///
/// # Errors
///
/// Returns [`ReconstructError::TypeMismatch`] when `args.clip` is not a
/// valid clip selector, or [`ReconstructError::PostStateMissing`] when
/// the post-state does not contain the target clip.
pub fn data_envelope_from_post_state(
    args: &AudioFadeArgs,
    post_state: &Project,
) -> Result<AudioFadeData, ReconstructError> {
    let (clip_id, _was_qualified) =
        resolve_clip_selector(&args.clip).map_err(|_| ReconstructError::TypeMismatch {
            name: "args.clip",
            expected: "UUIDv7 ClipId string or qualified `clip:<UUIDv7>` selector",
        })?;

    for track in &post_state.tracks {
        for clip in &track.clips {
            if clip.id == clip_id {
                return Ok(AudioFadeData {
                    clip_id,
                    fade_in_tk: clip.fade_in_tk.get(),
                    fade_out_tk: clip.fade_out_tk.get(),
                    fade_in_curve: clip.fade_in_curve,
                    fade_out_curve: clip.fade_out_curve,
                });
            }
        }
    }

    Err(ReconstructError::PostStateMissing {
        detail: format!("audio.fade: clip id {clip_id} not found in post_state"),
    })
}

/// `audio.fade` verb registration entry.
#[derive(Debug, Default)]
pub struct AudioFadeVerb;

impl From<AudioFadeError> for VerbError {
    fn from(value: AudioFadeError) -> Self {
        match value {
            AudioFadeError::BadSelector { .. }
            | AudioFadeError::NotFound { .. }
            | AudioFadeError::NoMatch { .. }
            | AudioFadeError::SelectorKindMismatch { .. }
            | AudioFadeError::ClipKindMismatch { .. }
            | AudioFadeError::BadTime { .. }
            | AudioFadeError::BadRange { .. }
            | AudioFadeError::ArgsIncompatible { .. }
            | AudioFadeError::Locked { .. } => VerbError::BadArgs {
                detail: value.to_string(),
            },
        }
    }
}

impl Verb for AudioFadeVerb {
    fn verb(&self) -> &'static str {
        "audio.fade"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: AudioFadeArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("audio.fade: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, _data) = compute_patch(prior, &typed)?;

        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!("audio.fade: patch construction failed: {err}"))
            })?;

        let post_state = prior
            .apply(&patch)
            .map_err(|err| VerbError::InvariantViolation {
                detail: format!("audio.fade: post-state validation failed: {err}"),
            })?;

        let envelope = data_envelope_from_post_state(&typed, &post_state).map_err(|err| {
            VerbError::Custom(format!(
                "audio.fade: data envelope reconstruction failed: {err}"
            ))
        })?;

        let data = serde_json::to_value(&envelope).map_err(|err| {
            VerbError::Custom(format!("audio.fade: data serialize failed: {err}"))
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
        let typed: AudioFadeArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "AudioFadeArgs",
            })?;

        let envelope = data_envelope_from_post_state(&typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
