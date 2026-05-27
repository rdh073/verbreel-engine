//! `preview.waveform` (§14.2) — v1 waveform/cache floor.
//!
//! ## Spec quote (`spec/commands/preview.md` §14.2, abbreviated)
//!
//! > CLI: `verbreel preview waveform [--project <id>] --target <selector> ...`
//! > MCP: `preview.waveform`
//! > Args: `project_id`, `target`, optional `samples`, `out_path`.
//! > Returns (`data`): `{ peaks, rms, samples, channels, cache_path, out_path? }`.
//! > Errors: `E_BAD_SELECTOR`, `E_NOT_FOUND`, `E_NO_MATCH`,
//! > `E_TRACK_KIND_MISMATCH`, `E_CLIP_KIND_MISMATCH`,
//! > `E_ASSET_NO_AUDIO`, `E_ASSET_UNSUPPORTED_KIND`, `E_BAD_RANGE`,
//! > `E_PATH_ESCAPE`, `E_IO`.
//!
//! ## v1 floor
//!
//! This pure verb slice validates only local argument shape/range
//! pinned by the spec (`target` must be qualified with `asset:`,
//! `clip:`, or `track:` and `samples ∈ [1, 100000]` with default
//! `1024`) and then always returns `E_IO` because waveform
//! rendering/cache/file-writing runtime is intentionally unavailable at
//! this layer.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use verbreel_types::ProjectId;

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

const DEFAULT_SAMPLES: i64 = 1024;
const SAMPLES_ALLOWED: &str = "[1, 100000]";
const TARGET_HINT: &str = "target must be qualified `asset:<...>`, `clip:<...>`, or `track:<...>`";

/// Arguments for `preview.waveform`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewWaveformArgs {
    /// Target project id.
    pub project_id: ProjectId,
    /// Qualified multi-noun selector.
    pub target: String,
    /// Requested waveform bucket count.
    ///
    /// Signed by design so out-of-range values map to `E_BAD_RANGE`
    /// (`VerbError::BadArgs`) instead of serde-level type errors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub samples: Option<i64>,
    /// Optional caller-supplied output path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out_path: Option<String>,
}

/// Resolve `samples` with the §14.2 default (`1024`).
#[must_use]
pub fn resolved_samples(args: &PreviewWaveformArgs) -> i64 {
    args.samples.unwrap_or(DEFAULT_SAMPLES)
}

/// Envelope `data` for a future successful `preview.waveform`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PreviewWaveformData {
    /// Peak amplitude per bucket, normalized to `[0.0, 1.0]`.
    pub peaks: Vec<f64>,
    /// RMS amplitude per bucket, normalized to `[0.0, 1.0]`.
    pub rms: Vec<f64>,
    /// Effective bucket count.
    pub samples: u32,
    /// Channel mode (`"mono"` in v1).
    pub channels: String,
    /// Resolved cache path under the project root.
    pub cache_path: String,
    /// Optional caller-supplied output path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out_path: Option<String>,
}

/// Verb-level failures for `preview.waveform`.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PreviewWaveformError {
    /// `target` is empty, bare, malformed, or has an unsupported prefix.
    #[error("preview.waveform: E_BAD_SELECTOR — {detail}; hint: {hint}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
        /// Recovery hint.
        hint: String,
    },
    /// `samples` is outside the allowed `[1, 100000]` range.
    #[error("preview.waveform: E_BAD_RANGE — {field} {value} out of range {allowed}")]
    BadRange {
        /// Field name.
        field: String,
        /// Offending value.
        value: i64,
        /// Allowed range string.
        allowed: String,
    },
    /// Runtime waveform/cache/file-writing unavailable in v1 floor.
    #[error("preview.waveform: E_IO — {detail}")]
    Io {
        /// Runtime detail for operators.
        detail: String,
    },
    /// Reserved for selector resolution against project graph.
    #[error("preview.waveform: E_NOT_FOUND — target `{target}` not found")]
    NotFound {
        /// Resolved target the caller supplied.
        target: String,
    },
    /// Reserved for selector forms that match no entity.
    #[error("preview.waveform: E_NO_MATCH — selector `{selector}` matched no target")]
    NoMatch {
        /// Original selector string.
        selector: String,
    },
    /// Reserved for selectors resolving to non-audio tracks.
    #[error(
        "preview.waveform: E_TRACK_KIND_MISMATCH — track `{target}` is not audio (actual kind `{actual_kind}`)"
    )]
    TrackKindMismatch {
        /// Target track selector/id.
        target: String,
        /// Actual track kind.
        actual_kind: String,
    },
    /// Reserved for selectors resolving to non-audio clips.
    #[error(
        "preview.waveform: E_CLIP_KIND_MISMATCH — clip `{target}` is not audio (actual kind `{actual_kind}`)"
    )]
    ClipKindMismatch {
        /// Target clip selector/id.
        target: String,
        /// Actual clip kind.
        actual_kind: String,
    },
    /// Reserved for video assets with no audio stream.
    #[error("preview.waveform: E_ASSET_NO_AUDIO — asset `{target}` has no audio stream")]
    AssetNoAudio {
        /// Target asset selector/id.
        target: String,
    },
    /// Reserved for non-waveform-able asset kinds.
    #[error(
        "preview.waveform: E_ASSET_UNSUPPORTED_KIND — asset `{target}` kind `{actual_kind}` is unsupported for waveform"
    )]
    AssetUnsupportedKind {
        /// Target asset selector/id.
        target: String,
        /// Actual asset kind.
        actual_kind: String,
    },
    /// Reserved for §0.11 path-safety checks in a follow-up slice.
    #[error("preview.waveform: E_PATH_ESCAPE — path `{path}` escapes project root")]
    PathEscape {
        /// Offending path.
        path: String,
    },
}

fn validate_target_shape(target: &str) -> Result<(), PreviewWaveformError> {
    if target.is_empty() {
        return Err(PreviewWaveformError::BadSelector {
            detail: "selector is empty".to_string(),
            hint: TARGET_HINT.to_string(),
        });
    }

    let (prefix, _body) =
        target
            .split_once(':')
            .ok_or_else(|| PreviewWaveformError::BadSelector {
                detail: format!("selector `{target}` is unqualified"),
                hint: TARGET_HINT.to_string(),
            })?;

    if matches!(prefix, "asset" | "clip" | "track") {
        return Ok(());
    }

    Err(PreviewWaveformError::BadSelector {
        detail: format!("unknown selector prefix `{prefix}`"),
        hint: TARGET_HINT.to_string(),
    })
}

/// Build the RFC 6902 patch for `preview.waveform`.
///
/// v1 floor: validates local selector/range constraints and then always
/// returns [`PreviewWaveformError::Io`].
///
/// # Errors
///
/// Returns [`PreviewWaveformError::BadSelector`] when `target` is
/// empty, unqualified, or has an unsupported prefix.
/// Returns [`PreviewWaveformError::BadRange`] when resolved `samples`
/// is outside `[1, 100000]`.
/// Returns [`PreviewWaveformError::Io`] for every otherwise
/// well-formed request.
pub fn compute_patch(
    _prior: &Project,
    args: &PreviewWaveformArgs,
) -> Result<(Value, Vec<Value>, Value), PreviewWaveformError> {
    validate_target_shape(&args.target)?;

    let samples = resolved_samples(args);
    if !(1..=100_000).contains(&samples) {
        return Err(PreviewWaveformError::BadRange {
            field: "samples".to_string(),
            value: samples,
            allowed: SAMPLES_ALLOWED.to_string(),
        });
    }

    let out_path = args.out_path.as_deref().unwrap_or("<cache/waveforms>");
    Err(PreviewWaveformError::Io {
        detail: format!(
            "preview waveform renderer/cache unavailable in v1 floor (target `{}`, samples {samples}, out_path `{out_path}`)",
            args.target
        ),
    })
}

impl From<PreviewWaveformError> for VerbError {
    fn from(value: PreviewWaveformError) -> Self {
        match value {
            PreviewWaveformError::BadSelector { .. } | PreviewWaveformError::BadRange { .. } => {
                VerbError::BadArgs {
                    detail: value.to_string(),
                }
            }
            PreviewWaveformError::Io { .. }
            | PreviewWaveformError::NotFound { .. }
            | PreviewWaveformError::NoMatch { .. }
            | PreviewWaveformError::TrackKindMismatch { .. }
            | PreviewWaveformError::ClipKindMismatch { .. }
            | PreviewWaveformError::AssetNoAudio { .. }
            | PreviewWaveformError::AssetUnsupportedKind { .. }
            | PreviewWaveformError::PathEscape { .. } => VerbError::Custom(value.to_string()),
        }
    }
}

/// The §0.8 verb for `preview.waveform`.
#[derive(Debug, Default)]
pub struct PreviewWaveformVerb;

impl Verb for PreviewWaveformVerb {
    fn verb(&self) -> &'static str {
        "preview.waveform"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: PreviewWaveformArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("preview.waveform: args deserialize failed: {err}"),
            })?;

        match compute_patch(prior, &typed) {
            Ok((patch_value, warnings, data)) => {
                let patch: json_patch::Patch =
                    serde_json::from_value(patch_value).map_err(|err| {
                        VerbError::Custom(format!(
                            "preview.waveform: patch construction failed: {err}"
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
        let _typed: PreviewWaveformArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "PreviewWaveformArgs",
            })?;

        Ok(Value::Null)
    }
}
