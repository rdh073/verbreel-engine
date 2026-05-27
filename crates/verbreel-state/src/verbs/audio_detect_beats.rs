//! `audio.detect_beats` (§19.1) — v1 analysis-runtime floor.
//!
//! Real beat detection needs decode/DSP/cache/marker runtime context
//! outside pure [`Verb::compute_patch`]. This v1 floor preserves the
//! published args/data/error surface, validates cheap pure args, and
//! returns `E_ANALYSIS_FAILED` for every accepted well-formed target.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use verbreel_types::{AssetId, ClipId, ProjectId, TrackId};

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// Target kind reflected in `audio.detect_beats` data/errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioAnalysisTargetKind {
    /// Clip target.
    Clip,
    /// Asset target.
    Asset,
    /// Track target.
    Track,
}

/// Supported `audio.detect_beats` algorithms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioDetectBeatsAlgorithm {
    /// Onset-strength autocorrelation (default).
    Onset,
    /// Whole-track tempo estimator with phase locking.
    Tempo,
    /// Librosa reference variant.
    Librosa,
}

impl AudioDetectBeatsAlgorithm {
    /// Return the spec wire literal.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Onset => "onset",
            Self::Tempo => "tempo",
            Self::Librosa => "librosa",
        }
    }
}

impl fmt::Display for AudioDetectBeatsAlgorithm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Runtime stage carried by `E_ANALYSIS_FAILED`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioDetectBeatsStage {
    /// Decoder bootstrap stage.
    DecoderInit,
    /// Audio decode stage.
    AudioDecode,
    /// Algorithm execution stage.
    AlgorithmStep,
    /// Cache write stage.
    CacheWrite,
}

impl fmt::Display for AudioDetectBeatsStage {
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

/// Args for `audio.detect_beats`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioDetectBeatsArgs {
    /// Target project id.
    pub project_id: ProjectId,
    /// Required qualified selector (`clip:<UUIDv7>`, `asset:<UUIDv7>`, `track:<UUIDv7>`).
    pub target: String,
    /// Optional algorithm (`onset`, `tempo`, `librosa`), defaults to `onset`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub algorithm: Option<String>,
    /// Optional confidence threshold in `[0.0, 1.0]`, defaults to `0.5`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_confidence: Option<f64>,
    /// Optional marker emission toggle, defaults to `true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub create_markers: Option<bool>,
    /// Optional lower analysis bound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_tk: Option<i64>,
    /// Optional upper analysis bound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_tk: Option<i64>,
}

/// Future success envelope for `audio.detect_beats`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioDetectBeatsData {
    /// Resolved target id.
    pub target_id: String,
    /// Resolved target kind.
    pub target_kind: AudioAnalysisTargetKind,
    /// Resolved algorithm.
    pub algorithm: AudioDetectBeatsAlgorithm,
    /// Estimated BPM.
    pub tempo_bpm: f64,
    /// Kept beat positions.
    pub beats_tk: Vec<i64>,
    /// Per-beat confidence (parallel to `beats_tk`).
    pub confidence: Vec<f64>,
    /// Pre-filter confidence mean.
    pub mean_confidence_pre_filter: f64,
    /// Number of beats kept after filtering.
    pub kept_beat_count: i64,
    /// Number of beats dropped by filtering.
    pub dropped_beat_count: i64,
    /// Absolute cache path.
    pub cache_path: String,
    /// `true` when result was served from cache.
    pub cache_hit: bool,
    /// Created marker ids when marker emission is enabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_marker_ids: Option<Vec<String>>,
    /// Removed detector-owned marker ids.
    pub removed_marker_ids: Vec<String>,
}

/// Verb-level errors for `audio.detect_beats`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AudioDetectBeatsError {
    /// `algorithm` is not in the registered set.
    #[error(
        "audio.detect_beats: E_ANALYSIS_UNKNOWN_ALGORITHM — requested `{requested}`; \
         registered_algorithms={registered_algorithms:?}"
    )]
    UnknownAlgorithm {
        /// Offending literal.
        requested: String,
        /// Registered algorithm names.
        registered_algorithms: Vec<String>,
    },

    /// Analysis runtime failed.
    #[error(
        "audio.detect_beats: E_ANALYSIS_FAILED — target `{target_id}` algorithm `{algorithm}` \
         failed at stage `{stage}`: {error}"
    )]
    AnalysisFailed {
        /// Resolved target id.
        target_id: String,
        /// Resolved algorithm literal.
        algorithm: AudioDetectBeatsAlgorithm,
        /// Failing stage.
        stage: AudioDetectBeatsStage,
        /// Runtime failure detail.
        error: String,
    },

    /// Resolved target has no audio stream.
    #[error("audio.detect_beats: E_ASSET_NO_AUDIO — target `{target_id}` kind `{target_kind:?}`")]
    AssetNoAudio {
        /// Resolved target id.
        target_id: String,
        /// Resolved target kind.
        target_kind: AudioAnalysisTargetKind,
    },

    /// Asset target kind is unsupported.
    #[error("audio.detect_beats: E_ASSET_UNSUPPORTED_KIND — asset_kind `{asset_kind}`")]
    AssetUnsupportedKind {
        /// Offending asset kind.
        asset_kind: String,
    },

    /// Selector parse failed.
    #[error("audio.detect_beats: E_BAD_SELECTOR — {detail}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
    },

    /// Selector matched no targets.
    #[error("audio.detect_beats: E_NO_MATCH — selector `{selector}` matched nothing")]
    NoMatch {
        /// Selector text.
        selector: String,
    },

    /// Referenced target id does not exist.
    #[error("audio.detect_beats: E_NOT_FOUND — {target_kind} `{target_id}` not found")]
    NotFound {
        /// Target kind literal.
        target_kind: String,
        /// Missing target id.
        target_id: String,
    },

    /// Selector prefix resolved to the wrong noun kind.
    #[error("audio.detect_beats: E_SELECTOR_KIND_MISMATCH — actual_kind `{actual_kind}`")]
    SelectorKindMismatch {
        /// Offending selector kind token.
        actual_kind: String,
    },

    /// Target track is not audio kind.
    #[error(
        "audio.detect_beats: E_TRACK_KIND_MISMATCH — track `{track_id}` has kind `{actual_kind}`"
    )]
    TrackKindMismatch {
        /// Track id.
        track_id: String,
        /// Actual kind.
        actual_kind: String,
    },

    /// Target clip is not audio kind.
    #[error("audio.detect_beats: E_CLIP_KIND_MISMATCH — clip `{clip_id}` has kind `{actual_kind}`")]
    ClipKindMismatch {
        /// Clip id.
        clip_id: String,
        /// Actual kind.
        actual_kind: String,
    },

    /// Numeric arg is out of range.
    #[error("audio.detect_beats: E_BAD_RANGE — field `{field}`: {detail}")]
    BadRange {
        /// Offending field.
        field: String,
        /// Range failure detail.
        detail: String,
    },

    /// Time arg is invalid.
    #[error("audio.detect_beats: E_BAD_TIME — field `{field}` has invalid value `{value}`")]
    BadTime {
        /// Offending field.
        field: String,
        /// Offending value.
        value: i64,
    },

    /// Arg combination is incompatible.
    #[error("audio.detect_beats: E_ARGS_INCOMPATIBLE — {hint}")]
    ArgsIncompatible {
        /// Recovery hint.
        hint: String,
    },

    /// Target is locked.
    #[error("audio.detect_beats: E_LOCKED — {kind} `{id}` is locked")]
    Locked {
        /// Locked entity kind.
        kind: String,
        /// Locked entity id.
        id: String,
    },

    /// Emitted patch violates schema cap.
    #[error(
        "audio.detect_beats: E_SCHEMA_VIOLATION — field `{field}` size_bytes={size_bytes} \
         cap_bytes={cap_bytes}: {hint}"
    )]
    SchemaViolation {
        /// Violating field name.
        field: String,
        /// Actual size.
        size_bytes: usize,
        /// Allowed cap.
        cap_bytes: usize,
        /// Recovery hint.
        hint: String,
    },
}

/// Return the currently registered algorithm literals.
#[must_use]
pub fn registered_algorithms() -> [&'static str; 3] {
    ["onset", "tempo", "librosa"]
}

/// Resolve algorithm default and unknown-algorithm mapping.
///
/// # Errors
///
/// Returns [`AudioDetectBeatsError::UnknownAlgorithm`] when `args.algorithm`
/// is not one of the registered literals.
pub fn resolved_algorithm(
    args: &AudioDetectBeatsArgs,
) -> Result<AudioDetectBeatsAlgorithm, AudioDetectBeatsError> {
    match args.algorithm.as_deref() {
        None | Some("onset") => Ok(AudioDetectBeatsAlgorithm::Onset),
        Some("tempo") => Ok(AudioDetectBeatsAlgorithm::Tempo),
        Some("librosa") => Ok(AudioDetectBeatsAlgorithm::Librosa),
        Some(other) => Err(AudioDetectBeatsError::UnknownAlgorithm {
            requested: other.to_string(),
            registered_algorithms: registered_algorithms()
                .into_iter()
                .map(str::to_string)
                .collect(),
        }),
    }
}

/// Resolve `min_confidence` default and range validation.
///
/// # Errors
///
/// Returns [`AudioDetectBeatsError::BadRange`] for non-finite values or
/// values outside `[0.0, 1.0]`.
pub fn resolved_min_confidence(args: &AudioDetectBeatsArgs) -> Result<f64, AudioDetectBeatsError> {
    let value = args.min_confidence.unwrap_or(0.5);
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(AudioDetectBeatsError::BadRange {
            field: "min_confidence".to_string(),
            detail: format!("value `{value}` must be finite and within [0.0, 1.0]"),
        });
    }
    Ok(value)
}

/// Resolve `create_markers` default.
#[must_use]
pub fn resolved_create_markers(args: &AudioDetectBeatsArgs) -> bool {
    args.create_markers.unwrap_or(true)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedTarget {
    kind: AudioAnalysisTargetKind,
    id: String,
}

fn parse_target(raw: &str) -> Result<ResolvedTarget, AudioDetectBeatsError> {
    if raw.is_empty() {
        return Err(AudioDetectBeatsError::BadSelector {
            detail: "selector is empty".to_string(),
        });
    }

    let (prefix, body) = raw
        .split_once(':')
        .ok_or_else(|| AudioDetectBeatsError::BadSelector {
            detail: format!(
                "selector `{raw}` is unqualified (expected clip:<UUIDv7>, asset:<UUIDv7>, or \
                 track:<UUIDv7>)"
            ),
        })?;

    match prefix {
        "clip" => {
            let clip_id =
                body.parse::<ClipId>()
                    .map_err(|err| AudioDetectBeatsError::BadSelector {
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
                    .map_err(|err| AudioDetectBeatsError::BadSelector {
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
                    .map_err(|err| AudioDetectBeatsError::BadSelector {
                        detail: format!("track body parse failed: {err}"),
                    })?;
            Ok(ResolvedTarget {
                kind: AudioAnalysisTargetKind::Track,
                id: track_id.to_string(),
            })
        }
        other => Err(AudioDetectBeatsError::SelectorKindMismatch {
            actual_kind: other.to_string(),
        }),
    }
}

fn validate_time_window(args: &AudioDetectBeatsArgs) -> Result<(), AudioDetectBeatsError> {
    if let Some(from_tk) = args.from_tk
        && from_tk < 0
    {
        return Err(AudioDetectBeatsError::BadTime {
            field: "from_tk".to_string(),
            value: from_tk,
        });
    }

    if let Some(to_tk) = args.to_tk
        && to_tk < 0
    {
        return Err(AudioDetectBeatsError::BadTime {
            field: "to_tk".to_string(),
            value: to_tk,
        });
    }

    if let (Some(from_tk), Some(to_tk)) = (args.from_tk, args.to_tk)
        && to_tk <= from_tk
    {
        return Err(AudioDetectBeatsError::BadTime {
            field: "to_tk".to_string(),
            value: to_tk,
        });
    }

    Ok(())
}

/// Build the RFC 6902 patch for `audio.detect_beats`.
///
/// v1 floor: every accepted well-formed target returns
/// `E_ANALYSIS_FAILED`.
///
/// # Errors
///
/// Returns [`AudioDetectBeatsError`] for selector/param validation or the
/// v1 floor runtime unavailability.
pub fn compute_patch(
    _prior: &Project,
    args: &AudioDetectBeatsArgs,
) -> Result<(Value, Vec<Value>, Value), AudioDetectBeatsError> {
    let target = parse_target(&args.target)?;
    let algorithm = resolved_algorithm(args)?;
    let _min_confidence = resolved_min_confidence(args)?;
    let create_markers = resolved_create_markers(args);
    validate_time_window(args)?;

    if target.kind == AudioAnalysisTargetKind::Asset && create_markers {
        return Err(AudioDetectBeatsError::ArgsIncompatible {
            hint: "create_markers requires a clip or track target; asset targets return beats_tk \
                   in asset-relative ticks only"
                .to_string(),
        });
    }

    Err(AudioDetectBeatsError::AnalysisFailed {
        target_id: target.id,
        algorithm,
        stage: AudioDetectBeatsStage::AlgorithmStep,
        error: "audio analysis runtime/cache context unavailable in the v1 floor".to_string(),
    })
}

impl From<AudioDetectBeatsError> for VerbError {
    fn from(value: AudioDetectBeatsError) -> Self {
        VerbError::Custom(value.to_string())
    }
}

/// The §0.8 verb entry for `audio.detect_beats`.
#[derive(Debug, Default)]
pub struct AudioDetectBeatsVerb;

impl Verb for AudioDetectBeatsVerb {
    fn verb(&self) -> &'static str {
        "audio.detect_beats"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: AudioDetectBeatsArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("audio.detect_beats: args deserialize failed: {err}"),
            })?;

        match compute_patch(prior, &typed) {
            Ok((patch_value, warnings, data)) => {
                let patch: json_patch::Patch =
                    serde_json::from_value(patch_value).map_err(|err| {
                        VerbError::Custom(format!(
                            "audio.detect_beats: patch construction failed: {err}"
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
        let _typed: AudioDetectBeatsArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "AudioDetectBeatsArgs",
            })?;
        Ok(Value::Null)
    }
}
