//! `audio.detect_silence` (§19.3) — v1 analysis-runtime floor.
//!
//! Real silence detection needs decode/DSP/cache runtime context
//! outside pure [`Verb::compute_patch`]. This v1 floor preserves the
//! published args/data/error surface, validates cheap pure args, and
//! returns `E_ANALYSIS_FAILED` for every accepted well-formed target.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use verbreel_types::{AssetId, ClipId, ProjectId, TrackId};

use super::audio_detect_beats::AudioAnalysisTargetKind;
use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// Runtime stage carried by `E_ANALYSIS_FAILED`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioDetectSilenceStage {
    /// Decoder bootstrap stage.
    DecoderInit,
    /// Audio decode stage.
    AudioDecode,
    /// Algorithm execution stage.
    AlgorithmStep,
    /// Cache write stage.
    CacheWrite,
}

impl fmt::Display for AudioDetectSilenceStage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let literal = match self {
            Self::DecoderInit => "decoder_init",
            Self::AudioDecode => "audio_decode",
            Self::AlgorithmStep => "algorithm_step",
            Self::CacheWrite => "cache_write",
        };
        f.write_str(literal)
    }
}

/// Args for `audio.detect_silence`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioDetectSilenceArgs {
    /// Target project id.
    pub project_id: ProjectId,
    /// Required qualified selector (`clip:<UUIDv7>`, `asset:<UUIDv7>`, `track:<UUIDv7>`).
    pub target: String,
    /// Optional minimum silence duration in ticks; defaults to `120000`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_silence_tk: Option<i64>,
    /// Optional dBFS threshold; defaults to `-40.0`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold_db: Option<f64>,
    /// Optional lower analysis bound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_tk: Option<i64>,
    /// Optional upper analysis bound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_tk: Option<i64>,
}

/// One detected silence interval in the target tick space.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioSilenceInterval {
    /// Inclusive interval start.
    pub start_tk: i64,
    /// Exclusive interval end.
    pub end_tk: i64,
}

/// Future success envelope for `audio.detect_silence`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioDetectSilenceData {
    /// Resolved target id.
    pub target_id: String,
    /// Resolved target kind.
    pub target_kind: AudioAnalysisTargetKind,
    /// Detected silence intervals.
    pub silences: Vec<AudioSilenceInterval>,
    /// Total silence duration in ticks.
    pub total_silence_tk: i64,
    /// Absolute cache path.
    pub cache_path: String,
    /// `true` when result was served from cache.
    pub cache_hit: bool,
}

/// Verb-level errors for `audio.detect_silence`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AudioDetectSilenceError {
    /// Analysis runtime failed.
    #[error(
        "audio.detect_silence: E_ANALYSIS_FAILED — target `{target_id}` failed at stage \
         `{stage}`: {error}"
    )]
    AnalysisFailed {
        /// Resolved target id.
        target_id: String,
        /// Failing stage.
        stage: AudioDetectSilenceStage,
        /// Runtime failure detail.
        error: String,
    },

    /// Resolved target has no audio stream.
    #[error("audio.detect_silence: E_ASSET_NO_AUDIO — target `{target_id}` kind `{target_kind:?}`")]
    AssetNoAudio {
        /// Resolved target id.
        target_id: String,
        /// Resolved target kind.
        target_kind: AudioAnalysisTargetKind,
    },

    /// Asset target kind is unsupported.
    #[error("audio.detect_silence: E_ASSET_UNSUPPORTED_KIND — asset_kind `{asset_kind}`")]
    AssetUnsupportedKind {
        /// Offending asset kind.
        asset_kind: String,
    },

    /// Selector parse failed.
    #[error("audio.detect_silence: E_BAD_SELECTOR — {detail}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
    },

    /// Selector matched no targets.
    #[error("audio.detect_silence: E_NO_MATCH — selector `{selector}` matched nothing")]
    NoMatch {
        /// Selector text.
        selector: String,
    },

    /// Referenced target id does not exist.
    #[error("audio.detect_silence: E_NOT_FOUND — {target_kind} `{target_id}` not found")]
    NotFound {
        /// Target kind literal.
        target_kind: String,
        /// Missing target id.
        target_id: String,
    },

    /// Selector prefix resolved to the wrong noun kind.
    #[error("audio.detect_silence: E_SELECTOR_KIND_MISMATCH — actual_kind `{actual_kind}`")]
    SelectorKindMismatch {
        /// Offending selector kind token.
        actual_kind: String,
    },

    /// Target track is not audio kind.
    #[error(
        "audio.detect_silence: E_TRACK_KIND_MISMATCH — track `{track_id}` has kind \
         `{actual_kind}`"
    )]
    TrackKindMismatch {
        /// Track id.
        track_id: String,
        /// Actual kind.
        actual_kind: String,
    },

    /// Target clip is not audio kind.
    #[error(
        "audio.detect_silence: E_CLIP_KIND_MISMATCH — clip `{clip_id}` has kind `{actual_kind}`"
    )]
    ClipKindMismatch {
        /// Clip id.
        clip_id: String,
        /// Actual kind.
        actual_kind: String,
    },

    /// Numeric arg is out of range.
    #[error("audio.detect_silence: E_BAD_RANGE — field `{field}`: {detail}")]
    BadRange {
        /// Offending field.
        field: String,
        /// Range failure detail.
        detail: String,
    },

    /// Time arg is invalid.
    #[error("audio.detect_silence: E_BAD_TIME — field `{field}` has invalid value `{value}`")]
    BadTime {
        /// Offending field.
        field: String,
        /// Offending value.
        value: i64,
    },
}

/// Resolve `min_silence_tk` default and positivity validation.
///
/// # Errors
///
/// Returns [`AudioDetectSilenceError::BadTime`] when the value is
/// zero or negative.
pub fn resolved_min_silence_tk(
    args: &AudioDetectSilenceArgs,
) -> Result<i64, AudioDetectSilenceError> {
    let value = args.min_silence_tk.unwrap_or(120_000);
    if value <= 0 {
        return Err(AudioDetectSilenceError::BadTime {
            field: "min_silence_tk".to_string(),
            value,
        });
    }
    Ok(value)
}

/// Resolve `threshold_db` default and range validation.
///
/// # Errors
///
/// Returns [`AudioDetectSilenceError::BadRange`] for non-finite values
/// or values outside `[-90.0, 0.0]`.
pub fn resolved_threshold_db(
    args: &AudioDetectSilenceArgs,
) -> Result<f64, AudioDetectSilenceError> {
    let value = args.threshold_db.unwrap_or(-40.0);
    if !value.is_finite() || !(-90.0..=0.0).contains(&value) {
        return Err(AudioDetectSilenceError::BadRange {
            field: "threshold_db".to_string(),
            detail: format!("value `{value}` must be finite and within [-90.0, 0.0]"),
        });
    }
    Ok(value)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedTarget {
    kind: AudioAnalysisTargetKind,
    id: String,
}

fn parse_target(raw: &str) -> Result<ResolvedTarget, AudioDetectSilenceError> {
    if raw.is_empty() {
        return Err(AudioDetectSilenceError::BadSelector {
            detail: "selector is empty".to_string(),
        });
    }

    let (prefix, body) =
        raw.split_once(':')
            .ok_or_else(|| AudioDetectSilenceError::BadSelector {
                detail: format!(
                    "selector `{raw}` is unqualified (expected clip:<UUIDv7>, asset:<UUIDv7>, or \
                 track:<UUIDv7>)"
                ),
            })?;

    match prefix {
        "clip" => {
            let clip_id =
                body.parse::<ClipId>()
                    .map_err(|err| AudioDetectSilenceError::BadSelector {
                        detail: format!("clip body parse failed: {err}"),
                    })?;
            Ok(ResolvedTarget {
                kind: AudioAnalysisTargetKind::Clip,
                id: clip_id.to_string(),
            })
        }
        "asset" => {
            let asset_id =
                body.parse::<AssetId>()
                    .map_err(|err| AudioDetectSilenceError::BadSelector {
                        detail: format!("asset body parse failed: {err}"),
                    })?;
            Ok(ResolvedTarget {
                kind: AudioAnalysisTargetKind::Asset,
                id: asset_id.to_string(),
            })
        }
        "track" => {
            let track_id =
                body.parse::<TrackId>()
                    .map_err(|err| AudioDetectSilenceError::BadSelector {
                        detail: format!("track body parse failed: {err}"),
                    })?;
            Ok(ResolvedTarget {
                kind: AudioAnalysisTargetKind::Track,
                id: track_id.to_string(),
            })
        }
        other => Err(AudioDetectSilenceError::SelectorKindMismatch {
            actual_kind: other.to_string(),
        }),
    }
}

fn validate_time_window(args: &AudioDetectSilenceArgs) -> Result<(), AudioDetectSilenceError> {
    if let Some(from_tk) = args.from_tk
        && from_tk < 0
    {
        return Err(AudioDetectSilenceError::BadTime {
            field: "from_tk".to_string(),
            value: from_tk,
        });
    }

    if let Some(to_tk) = args.to_tk
        && to_tk < 0
    {
        return Err(AudioDetectSilenceError::BadTime {
            field: "to_tk".to_string(),
            value: to_tk,
        });
    }

    if let (Some(from_tk), Some(to_tk)) = (args.from_tk, args.to_tk)
        && to_tk <= from_tk
    {
        return Err(AudioDetectSilenceError::BadTime {
            field: "to_tk".to_string(),
            value: to_tk,
        });
    }

    Ok(())
}

/// Build the RFC 6902 patch for `audio.detect_silence`.
///
/// v1 floor: every accepted well-formed target returns
/// `E_ANALYSIS_FAILED`.
///
/// # Errors
///
/// Returns [`AudioDetectSilenceError`] for selector/param validation or
/// the v1 floor runtime unavailability.
pub fn compute_patch(
    _prior: &Project,
    args: &AudioDetectSilenceArgs,
) -> Result<(Value, Vec<Value>, Value), AudioDetectSilenceError> {
    let target = parse_target(&args.target)?;
    let _min_silence_tk = resolved_min_silence_tk(args)?;
    let _threshold_db = resolved_threshold_db(args)?;
    validate_time_window(args)?;

    Err(AudioDetectSilenceError::AnalysisFailed {
        target_id: target.id,
        stage: AudioDetectSilenceStage::AlgorithmStep,
        error: "audio analysis runtime/cache context unavailable in the v1 floor".to_string(),
    })
}

impl From<AudioDetectSilenceError> for VerbError {
    fn from(value: AudioDetectSilenceError) -> Self {
        VerbError::Custom(value.to_string())
    }
}

/// The §0.8 verb entry for `audio.detect_silence`.
#[derive(Debug, Default)]
pub struct AudioDetectSilenceVerb;

impl Verb for AudioDetectSilenceVerb {
    fn verb(&self) -> &'static str {
        "audio.detect_silence"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: AudioDetectSilenceArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("audio.detect_silence: args deserialize failed: {err}"),
            })?;

        match compute_patch(prior, &typed) {
            Ok((patch_value, warnings, data)) => {
                let patch: json_patch::Patch =
                    serde_json::from_value(patch_value).map_err(|err| {
                        VerbError::Custom(format!(
                            "audio.detect_silence: patch construction failed: {err}"
                        ))
                    })?;
                Ok((patch, data, warnings))
            }
            Err(err) => Err(err.into()),
        }
    }

    fn reconstruct(
        &self,
        args: &Value,
        _patch: &Value,
        _warnings: &[Value],
        _post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let _typed: AudioDetectSilenceArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "AudioDetectSilenceArgs",
            })?;
        Ok(Value::Null)
    }
}
