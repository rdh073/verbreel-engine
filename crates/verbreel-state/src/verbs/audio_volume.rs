//! `audio.volume` (§9.2) — sixtieth production verb in the engine.
//!
//! ## Behavior
//!
//! Sets the linear gain on a clip or track. Accepts either `gain`
//! (linear, `[0.0, 4.0]`) or `db` (logarithmic, `[-60.0, +12.0]`); the
//! two are mutually exclusive (`oneOf` in the arg schema).
//!
//! Storage is always linear: when `db` is supplied, the value is
//! converted via `gain = 10^(db / 20)` before being persisted. The
//! resolved linear value is what's written to `Clip.volume` /
//! `Track.volume` AND what's returned in `data.volume`. Storing linear
//! keeps `volume == 0` (silence) well-defined without `-Infinity`
//! corner cases; agents that want dB back compute `20 * log10(volume)`
//! client-side.
//!
//! ## Selector handling
//!
//! `target` MUST be qualified per spec §0.4 multi-noun rule:
//! `clip:<UUIDv7>` resolves to a clip and writes `Clip.volume`;
//! `track:<UUIDv7>` resolves to a track and writes `Track.volume`. A
//! bare body (just a UUID with no prefix) is rejected with
//! `E_BAD_SELECTOR`. Unknown prefixes (e.g. `effect:<id>`) are
//! rejected with `E_BAD_SELECTOR`.
//!
//! ## Audio-only guard
//!
//! Both branches reject non-audio targets — `E_CLIP_KIND_MISMATCH` for
//! a clip on a non-audio track, `E_TRACK_KIND_MISMATCH` for a
//! non-audio track. Cross-kind callers should use `clip.set_volume`
//! (§5.11) or `track.set_volume` (§4.8) directly; `audio.volume` is
//! the audio-namespace alias that keeps the `audio.*` discoverability
//! promise.

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use crate::track::TrackKind;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::{ClipId, ProjectId, TrackId};

/// Recovery hint emitted with `E_ARGS_INCOMPATIBLE` for the
/// `gain`/`db` mutual-exclusion failure.
pub const GAIN_DB_HINT: &str = "supply exactly one of gain or db";

/// Recovery hint emitted with `E_BAD_SELECTOR` for bare-body selectors.
pub const SELECTOR_HINT: &str =
    "target must be qualified `clip:<UUIDv7>` or `track:<UUIDv7>` (spec §0.4)";

/// Target kind reflected in `data.target_kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioVolumeTargetKind {
    /// Target is a clip on an audio track; `Clip.volume` was written.
    Clip,
    /// Target is an audio track; `Track.volume` was written.
    Track,
}

/// Args for `audio.volume`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioVolumeArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Qualified `clip:<UUIDv7>` or `track:<UUIDv7>` selector.
    pub target: String,

    /// New linear gain in `[0.0, 4.0]`. Mutually exclusive with `db`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gain: Option<f64>,

    /// New gain in decibels in `[-60.0, +12.0]`. Mutually exclusive
    /// with `gain`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub db: Option<f64>,
}

/// Envelope `data` returned by `audio.volume`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioVolumeData {
    /// Whether the resolved target is a clip or a track.
    pub target_kind: AudioVolumeTargetKind,

    /// Resolved target id (clip UUID or track UUID).
    pub target_id: String,

    /// Resolved linear gain post-write. When `db` was supplied this
    /// is `10^(db / 20)`; otherwise it is `args.gain` verbatim.
    pub volume: f64,
}

/// Verb-level validation failures for `audio.volume`.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum AudioVolumeError {
    /// `args.target` is empty, unqualified, or uses an unknown prefix.
    #[error("audio.volume: `target` selector parse failed: {detail}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
        /// Recovery hint.
        hint: &'static str,
    },

    /// Both `gain` and `db` supplied, or neither was supplied.
    #[error("audio.volume: {detail}")]
    ArgsIncompatible {
        /// Failure detail.
        detail: String,
        /// Recovery hint.
        hint: &'static str,
    },

    /// `gain` or `db` value falls outside the accepted range.
    #[error("audio.volume: `{field}` value {value} out of range [{min}, {max}]")]
    BadRange {
        /// Offending field name (`"gain"` or `"db"`).
        field: &'static str,
        /// Offending value.
        value: f64,
        /// Inclusive lower bound.
        min: f64,
        /// Inclusive upper bound.
        max: f64,
    },

    /// Target clip or track id has no match in the project.
    #[error("audio.volume: {target_kind} `{target_id}` not found")]
    NotFound {
        /// Resolved target kind (`"clip"` or `"track"`).
        target_kind: &'static str,
        /// Missing target id string.
        target_id: String,
    },

    /// Structural selector matched zero targets (reserved for future
    /// non-bare selector forms; bare `<prefix>:<uuid>` uses
    /// `NotFound`).
    #[error("audio.volume: selector `{selector}` matched no {target_kind}")]
    NoMatch {
        /// Target kind (`"clip"` or `"track"`).
        target_kind: &'static str,
        /// Original selector string.
        selector: String,
    },

    /// Target clip's parent track is not `kind: "audio"`.
    #[error(
        "audio.volume: clip `{clip_id}` is on a `{actual_kind}` track; audio.volume is audio-only \
         — use clip.set_volume for cross-kind targets"
    )]
    ClipKindMismatch {
        /// Target clip id.
        clip_id: String,
        /// Actual parent track kind.
        actual_kind: &'static str,
    },

    /// Target track is not `kind: "audio"`.
    #[error(
        "audio.volume: track `{track_id}` is `{actual_kind}` kind; audio.volume is audio-only \
         — use track.set_volume for cross-kind targets"
    )]
    TrackKindMismatch {
        /// Target track id.
        track_id: String,
        /// Actual track kind.
        actual_kind: &'static str,
    },

    /// Target clip, its parent track, or the target track is locked.
    #[error("audio.volume: {kind} `{id}` is locked")]
    Locked {
        /// Locked entity kind (`"clip"` or `"track"`).
        kind: &'static str,
        /// Locked entity id.
        id: String,
    },
}

/// Parsed selector for `audio.volume`'s dual-dispatch path.
#[derive(Debug, Clone)]
enum ParsedTarget {
    Clip(ClipId),
    Track(TrackId),
}

/// Build the RFC-6902 patch for `audio.volume`.
///
/// # Errors
///
/// Returns [`AudioVolumeError`] for incompatible args, out-of-range
/// values, selector parse failures, unknown target ids, non-audio
/// targets, or locked targets.
pub fn compute_patch(
    prior: &Project,
    args: &AudioVolumeArgs,
) -> Result<(Value, Vec<Value>, AudioVolumeData), AudioVolumeError> {
    // Args-shape failures are project-independent; resolve them before
    // touching the project state so callers see "fix your args" errors
    // ahead of "fix your selector" errors.
    let resolved_volume = resolve_volume(args)?;

    let parsed = parse_target(&args.target)?;

    match parsed {
        ParsedTarget::Clip(clip_id) => compute_clip_patch(prior, clip_id, resolved_volume),
        ParsedTarget::Track(track_id) => compute_track_patch(prior, track_id, resolved_volume),
    }
}

fn resolve_volume(args: &AudioVolumeArgs) -> Result<f64, AudioVolumeError> {
    match (args.gain, args.db) {
        (Some(_), Some(_)) => Err(AudioVolumeError::ArgsIncompatible {
            detail: "`gain` and `db` are mutually exclusive".to_string(),
            hint: GAIN_DB_HINT,
        }),
        (None, None) => Err(AudioVolumeError::ArgsIncompatible {
            detail: "one of `gain` or `db` is required".to_string(),
            hint: GAIN_DB_HINT,
        }),
        (Some(gain), None) => {
            if !gain.is_finite() || !(0.0..=4.0).contains(&gain) {
                return Err(AudioVolumeError::BadRange {
                    field: "gain",
                    value: gain,
                    min: 0.0,
                    max: 4.0,
                });
            }
            Ok(gain)
        }
        (None, Some(db)) => {
            if !db.is_finite() || !(-60.0..=12.0).contains(&db) {
                return Err(AudioVolumeError::BadRange {
                    field: "db",
                    value: db,
                    min: -60.0,
                    max: 12.0,
                });
            }
            // Storage is always linear: gain = 10^(db / 20).
            Ok(10.0_f64.powf(db / 20.0))
        }
    }
}

fn parse_target(raw: &str) -> Result<ParsedTarget, AudioVolumeError> {
    if raw.is_empty() {
        return Err(AudioVolumeError::BadSelector {
            detail: "selector is empty".to_string(),
            hint: SELECTOR_HINT,
        });
    }

    let (prefix, body) = raw
        .split_once(':')
        .ok_or_else(|| AudioVolumeError::BadSelector {
            detail: format!("selector `{raw}` is unqualified (missing `clip:` or `track:` prefix)"),
            hint: SELECTOR_HINT,
        })?;

    match prefix {
        "clip" => {
            let clip_id = body
                .parse::<ClipId>()
                .map_err(|err| AudioVolumeError::BadSelector {
                    detail: format!("clip body parse failed: {err}"),
                    hint: SELECTOR_HINT,
                })?;
            Ok(ParsedTarget::Clip(clip_id))
        }
        "track" => {
            let track_id =
                body.parse::<TrackId>()
                    .map_err(|err| AudioVolumeError::BadSelector {
                        detail: format!("track body parse failed: {err}"),
                        hint: SELECTOR_HINT,
                    })?;
            Ok(ParsedTarget::Track(track_id))
        }
        other => Err(AudioVolumeError::BadSelector {
            detail: format!("unknown selector prefix `{other}`"),
            hint: SELECTOR_HINT,
        }),
    }
}

fn compute_clip_patch(
    prior: &Project,
    clip_id: ClipId,
    resolved_volume: f64,
) -> Result<(Value, Vec<Value>, AudioVolumeData), AudioVolumeError> {
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

    let (t_idx, c_idx, track, clip) = location.ok_or_else(|| AudioVolumeError::NotFound {
        target_kind: "clip",
        target_id: clip_id.to_string(),
    })?;

    if track.kind != TrackKind::Audio {
        return Err(AudioVolumeError::ClipKindMismatch {
            clip_id: clip_id.to_string(),
            actual_kind: track_kind_name(track.kind),
        });
    }

    if track.locked {
        return Err(AudioVolumeError::Locked {
            kind: "track",
            id: track.id.to_string(),
        });
    }

    if clip.locked {
        return Err(AudioVolumeError::Locked {
            kind: "clip",
            id: clip_id.to_string(),
        });
    }

    let patch = json!([{
        "op": "replace",
        "path": format!("/tracks/{t_idx}/clips/{c_idx}/volume"),
        "value": resolved_volume,
    }]);

    let data = AudioVolumeData {
        target_kind: AudioVolumeTargetKind::Clip,
        target_id: clip_id.to_string(),
        volume: resolved_volume,
    };

    Ok((patch, Vec::new(), data))
}

fn compute_track_patch(
    prior: &Project,
    track_id: TrackId,
    resolved_volume: f64,
) -> Result<(Value, Vec<Value>, AudioVolumeData), AudioVolumeError> {
    let (t_idx, track) = prior
        .tracks
        .iter()
        .enumerate()
        .find(|(_, track)| track.id == track_id)
        .ok_or_else(|| AudioVolumeError::NotFound {
            target_kind: "track",
            target_id: track_id.to_string(),
        })?;

    if track.kind != TrackKind::Audio {
        return Err(AudioVolumeError::TrackKindMismatch {
            track_id: track_id.to_string(),
            actual_kind: track_kind_name(track.kind),
        });
    }

    if track.locked {
        return Err(AudioVolumeError::Locked {
            kind: "track",
            id: track_id.to_string(),
        });
    }

    let patch = json!([{
        "op": "replace",
        "path": format!("/tracks/{t_idx}/volume"),
        "value": resolved_volume,
    }]);

    let data = AudioVolumeData {
        target_kind: AudioVolumeTargetKind::Track,
        target_id: track_id.to_string(),
        volume: resolved_volume,
    };

    Ok((patch, Vec::new(), data))
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
/// Returns [`ReconstructError::TypeMismatch`] when `args.target` is
/// not a qualified `clip:<UUIDv7>` or `track:<UUIDv7>` selector, or
/// [`ReconstructError::PostStateMissing`] when the post-state does
/// not contain the target clip / track.
pub fn data_envelope_from_post_state(
    args: &AudioVolumeArgs,
    post_state: &Project,
) -> Result<AudioVolumeData, ReconstructError> {
    let parsed = parse_target(&args.target).map_err(|_| ReconstructError::TypeMismatch {
        name: "args.target",
        expected: "qualified `clip:<UUIDv7>` or `track:<UUIDv7>` selector",
    })?;

    match parsed {
        ParsedTarget::Clip(clip_id) => {
            for track in &post_state.tracks {
                for clip in &track.clips {
                    if clip.id == clip_id {
                        return Ok(AudioVolumeData {
                            target_kind: AudioVolumeTargetKind::Clip,
                            target_id: clip_id.to_string(),
                            volume: clip.volume,
                        });
                    }
                }
            }
            Err(ReconstructError::PostStateMissing {
                detail: format!("audio.volume: clip id {clip_id} not found in post_state"),
            })
        }
        ParsedTarget::Track(track_id) => {
            let track = post_state
                .tracks
                .iter()
                .find(|track| track.id == track_id)
                .ok_or_else(|| ReconstructError::PostStateMissing {
                    detail: format!("audio.volume: track id {track_id} not found in post_state"),
                })?;
            Ok(AudioVolumeData {
                target_kind: AudioVolumeTargetKind::Track,
                target_id: track_id.to_string(),
                volume: track.volume,
            })
        }
    }
}

/// `audio.volume` verb registration entry.
#[derive(Debug, Default)]
pub struct AudioVolumeVerb;

impl From<AudioVolumeError> for VerbError {
    fn from(value: AudioVolumeError) -> Self {
        match value {
            AudioVolumeError::BadSelector { .. }
            | AudioVolumeError::ArgsIncompatible { .. }
            | AudioVolumeError::BadRange { .. }
            | AudioVolumeError::NotFound { .. }
            | AudioVolumeError::NoMatch { .. }
            | AudioVolumeError::ClipKindMismatch { .. }
            | AudioVolumeError::TrackKindMismatch { .. }
            | AudioVolumeError::Locked { .. } => VerbError::BadArgs {
                detail: value.to_string(),
            },
        }
    }
}

impl Verb for AudioVolumeVerb {
    fn verb(&self) -> &'static str {
        "audio.volume"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: AudioVolumeArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("audio.volume: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, _data) = compute_patch(prior, &typed)?;

        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!("audio.volume: patch construction failed: {err}"))
            })?;

        let post_state = prior
            .apply(&patch)
            .map_err(|err| VerbError::InvariantViolation {
                detail: format!("audio.volume: post-state validation failed: {err}"),
            })?;

        let envelope = data_envelope_from_post_state(&typed, &post_state).map_err(|err| {
            VerbError::Custom(format!(
                "audio.volume: data envelope reconstruction failed: {err}"
            ))
        })?;

        let data = serde_json::to_value(&envelope).map_err(|err| {
            VerbError::Custom(format!("audio.volume: data serialize failed: {err}"))
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
        let typed: AudioVolumeArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "AudioVolumeArgs",
            })?;

        let envelope = data_envelope_from_post_state(&typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
