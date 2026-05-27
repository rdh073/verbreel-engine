//! `audio.analyze` (§19.2) — v1 analysis-runtime floor.
//!
//! Real multi-feature analysis needs decode/DSP/cache runtime context
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

/// Supported `audio.analyze` features.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioAnalyzeFeature {
    /// Global BPM estimate.
    Tempo,
    /// Musical key estimate.
    Key,
    /// RMS energy envelope.
    Energy,
    /// Structural segmentation.
    Sections,
    /// Long-term average spectral centroid.
    SpectralCentroid,
}

impl AudioAnalyzeFeature {
    /// Return the spec wire literal.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Tempo => "tempo",
            Self::Key => "key",
            Self::Energy => "energy",
            Self::Sections => "sections",
            Self::SpectralCentroid => "spectral_centroid",
        }
    }
}

impl fmt::Display for AudioAnalyzeFeature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Runtime stage carried by `E_ANALYSIS_FAILED`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioAnalyzeStage {
    /// Decoder bootstrap stage.
    DecoderInit,
    /// Audio decode stage.
    AudioDecode,
    /// Algorithm execution stage.
    AlgorithmStep,
    /// Cache write stage.
    CacheWrite,
}

impl fmt::Display for AudioAnalyzeStage {
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

/// Args for `audio.analyze`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioAnalyzeArgs {
    /// Target project id.
    pub project_id: ProjectId,
    /// Required qualified selector (`clip:<UUIDv7>`, `asset:<UUIDv7>`, `track:<UUIDv7>`).
    pub target: String,
    /// Optional feature list; omitted means all registered features in spec order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub features: Option<Vec<String>>,
    /// Optional lower analysis bound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_tk: Option<i64>,
    /// Optional upper analysis bound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_tk: Option<i64>,
}

/// One structural section entry in future `audio.analyze` data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioAnalyzeSection {
    /// Inclusive section start in the target tick space.
    pub start_tk: i64,
    /// Exclusive section end in the target tick space.
    pub end_tk: i64,
    /// Best-effort section label.
    pub label: String,
}

/// Future success envelope for `audio.analyze`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioAnalyzeData {
    /// Resolved target id.
    pub target_id: String,
    /// Resolved target kind.
    pub target_kind: AudioAnalysisTargetKind,
    /// Returned feature literals in spec order.
    pub features_returned: Vec<String>,
    /// BPM estimate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tempo_bpm: Option<f64>,
    /// Estimated musical key (`"unknown"` on non-tonal sources).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// Envelope samples as `[tick, value]` pairs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub energy_envelope_tk_value_pairs: Option<Vec<(i64, f64)>>,
    /// Structural sections.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sections: Option<Vec<AudioAnalyzeSection>>,
    /// Long-term average spectral centroid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spectral_centroid_hz_avg: Option<f64>,
    /// Absolute cache path.
    pub cache_path: String,
    /// `true` when result was served from cache.
    pub cache_hit: bool,
}

/// Verb-level errors for `audio.analyze`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AudioAnalyzeError {
    /// `features` contains a literal outside the registered set.
    #[error(
        "audio.analyze: E_ANALYSIS_UNKNOWN_FEATURE — requested `{requested}`; \
         registered_features={registered_features:?}"
    )]
    UnknownFeature {
        /// Offending literal.
        requested: String,
        /// Registered feature names.
        registered_features: Vec<String>,
    },

    /// Analysis runtime failed.
    #[error(
        "audio.analyze: E_ANALYSIS_FAILED — target `{target_id}` failed at stage `{stage}`: \
         {error}; failed_features={failed_features:?}"
    )]
    AnalysisFailed {
        /// Resolved target id.
        target_id: String,
        /// Failing stage.
        stage: AudioAnalyzeStage,
        /// Runtime failure detail.
        error: String,
        /// Feature literals that failed.
        failed_features: Vec<String>,
    },

    /// Resolved target has no audio stream.
    #[error("audio.analyze: E_ASSET_NO_AUDIO — target `{target_id}` kind `{target_kind:?}`")]
    AssetNoAudio {
        /// Resolved target id.
        target_id: String,
        /// Resolved target kind.
        target_kind: AudioAnalysisTargetKind,
    },

    /// Asset target kind is unsupported.
    #[error("audio.analyze: E_ASSET_UNSUPPORTED_KIND — asset_kind `{asset_kind}`")]
    AssetUnsupportedKind {
        /// Offending asset kind.
        asset_kind: String,
    },

    /// Selector parse failed.
    #[error("audio.analyze: E_BAD_SELECTOR — {detail}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
    },

    /// Selector matched no targets.
    #[error("audio.analyze: E_NO_MATCH — selector `{selector}` matched nothing")]
    NoMatch {
        /// Selector text.
        selector: String,
    },

    /// Referenced target id does not exist.
    #[error("audio.analyze: E_NOT_FOUND — {target_kind} `{target_id}` not found")]
    NotFound {
        /// Target kind literal.
        target_kind: String,
        /// Missing target id.
        target_id: String,
    },

    /// Selector prefix resolved to the wrong noun kind.
    #[error("audio.analyze: E_SELECTOR_KIND_MISMATCH — actual_kind `{actual_kind}`")]
    SelectorKindMismatch {
        /// Offending selector kind token.
        actual_kind: String,
    },

    /// Target track is not audio kind.
    #[error("audio.analyze: E_TRACK_KIND_MISMATCH — track `{track_id}` has kind `{actual_kind}`")]
    TrackKindMismatch {
        /// Track id.
        track_id: String,
        /// Actual kind.
        actual_kind: String,
    },

    /// Target clip is not audio kind.
    #[error("audio.analyze: E_CLIP_KIND_MISMATCH — clip `{clip_id}` has kind `{actual_kind}`")]
    ClipKindMismatch {
        /// Clip id.
        clip_id: String,
        /// Actual kind.
        actual_kind: String,
    },

    /// Numeric arg is out of range.
    #[error("audio.analyze: E_BAD_RANGE — field `{field}`: {detail}")]
    BadRange {
        /// Offending field.
        field: String,
        /// Range failure detail.
        detail: String,
    },

    /// Time arg is invalid.
    #[error("audio.analyze: E_BAD_TIME — field `{field}` has invalid value `{value}`")]
    BadTime {
        /// Offending field.
        field: String,
        /// Offending value.
        value: i64,
    },
}

/// Return the currently registered feature literals.
#[must_use]
pub fn registered_features() -> [&'static str; 5] {
    ["tempo", "key", "energy", "sections", "spectral_centroid"]
}

/// Resolve feature defaults, unknown-feature mapping, and canonical order.
///
/// # Errors
///
/// Returns [`AudioAnalyzeError::UnknownFeature`] when `args.features`
/// contains a literal outside the registered set.
pub fn resolved_features(
    args: &AudioAnalyzeArgs,
) -> Result<Vec<AudioAnalyzeFeature>, AudioAnalyzeError> {
    let mut want_tempo = false;
    let mut want_key = false;
    let mut want_energy = false;
    let mut want_sections = false;
    let mut want_spectral_centroid = false;

    if let Some(requested) = args.features.as_deref() {
        for feature in requested {
            match feature.as_str() {
                "tempo" => want_tempo = true,
                "key" => want_key = true,
                "energy" => want_energy = true,
                "sections" => want_sections = true,
                "spectral_centroid" => want_spectral_centroid = true,
                other => {
                    return Err(AudioAnalyzeError::UnknownFeature {
                        requested: other.to_string(),
                        registered_features: registered_features()
                            .into_iter()
                            .map(str::to_string)
                            .collect(),
                    });
                }
            }
        }
    } else {
        want_tempo = true;
        want_key = true;
        want_energy = true;
        want_sections = true;
        want_spectral_centroid = true;
    }

    let mut resolved = Vec::with_capacity(5);
    if want_tempo {
        resolved.push(AudioAnalyzeFeature::Tempo);
    }
    if want_key {
        resolved.push(AudioAnalyzeFeature::Key);
    }
    if want_energy {
        resolved.push(AudioAnalyzeFeature::Energy);
    }
    if want_sections {
        resolved.push(AudioAnalyzeFeature::Sections);
    }
    if want_spectral_centroid {
        resolved.push(AudioAnalyzeFeature::SpectralCentroid);
    }

    Ok(resolved)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedTarget {
    kind: AudioAnalysisTargetKind,
    id: String,
}

fn parse_target(raw: &str) -> Result<ResolvedTarget, AudioAnalyzeError> {
    if raw.is_empty() {
        return Err(AudioAnalyzeError::BadSelector {
            detail: "selector is empty".to_string(),
        });
    }

    let (prefix, body) = raw
        .split_once(':')
        .ok_or_else(|| AudioAnalyzeError::BadSelector {
            detail: format!(
                "selector `{raw}` is unqualified (expected clip:<UUIDv7>, asset:<UUIDv7>, or \
                 track:<UUIDv7>)"
            ),
        })?;

    match prefix {
        "clip" => {
            let clip_id = body
                .parse::<ClipId>()
                .map_err(|err| AudioAnalyzeError::BadSelector {
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
                    .map_err(|err| AudioAnalyzeError::BadSelector {
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
                    .map_err(|err| AudioAnalyzeError::BadSelector {
                        detail: format!("track body parse failed: {err}"),
                    })?;
            Ok(ResolvedTarget {
                kind: AudioAnalysisTargetKind::Track,
                id: track_id.to_string(),
            })
        }
        other => Err(AudioAnalyzeError::SelectorKindMismatch {
            actual_kind: other.to_string(),
        }),
    }
}

fn validate_time_window(args: &AudioAnalyzeArgs) -> Result<(), AudioAnalyzeError> {
    if let Some(from_tk) = args.from_tk
        && from_tk < 0
    {
        return Err(AudioAnalyzeError::BadTime {
            field: "from_tk".to_string(),
            value: from_tk,
        });
    }

    if let Some(to_tk) = args.to_tk
        && to_tk < 0
    {
        return Err(AudioAnalyzeError::BadTime {
            field: "to_tk".to_string(),
            value: to_tk,
        });
    }

    if let (Some(from_tk), Some(to_tk)) = (args.from_tk, args.to_tk)
        && to_tk <= from_tk
    {
        return Err(AudioAnalyzeError::BadTime {
            field: "to_tk".to_string(),
            value: to_tk,
        });
    }

    Ok(())
}

/// Build the RFC 6902 patch for `audio.analyze`.
///
/// v1 floor: every accepted well-formed target returns
/// `E_ANALYSIS_FAILED`.
///
/// # Errors
///
/// Returns [`AudioAnalyzeError`] for selector/param validation or the
/// v1 floor runtime unavailability.
pub fn compute_patch(
    _prior: &Project,
    args: &AudioAnalyzeArgs,
) -> Result<(Value, Vec<Value>, Value), AudioAnalyzeError> {
    let target = parse_target(&args.target)?;
    let features = resolved_features(args)?;
    validate_time_window(args)?;

    Err(AudioAnalyzeError::AnalysisFailed {
        target_id: target.id,
        stage: AudioAnalyzeStage::AlgorithmStep,
        error: "audio analysis runtime/cache context unavailable in the v1 floor".to_string(),
        failed_features: features
            .into_iter()
            .map(|feature| feature.to_string())
            .collect(),
    })
}

impl From<AudioAnalyzeError> for VerbError {
    fn from(value: AudioAnalyzeError) -> Self {
        VerbError::Custom(value.to_string())
    }
}

/// The §0.8 verb entry for `audio.analyze`.
#[derive(Debug, Default)]
pub struct AudioAnalyzeVerb;

impl Verb for AudioAnalyzeVerb {
    fn verb(&self) -> &'static str {
        "audio.analyze"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: AudioAnalyzeArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("audio.analyze: args deserialize failed: {err}"),
            })?;

        match compute_patch(prior, &typed) {
            Ok((patch_value, warnings, data)) => {
                let patch: json_patch::Patch =
                    serde_json::from_value(patch_value).map_err(|err| {
                        VerbError::Custom(format!(
                            "audio.analyze: patch construction failed: {err}"
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
        let _typed: AudioAnalyzeArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "AudioAnalyzeArgs",
            })?;
        Ok(Value::Null)
    }
}
