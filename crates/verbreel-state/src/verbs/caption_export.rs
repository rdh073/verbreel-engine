//! `caption.export` (§10.6) — ninety-third production verb.
//!
//! ## v1 floor — always errors with `E_IO`.
//!
//! Real subtitle export writes UTF-8 sidecar files (SRT/VTT/ASS),
//! performs path-safety checks, applies overwrite semantics, and
//! derives warnings/data from text clips. Those operations need
//! filesystem/runtime context outside pure [`Verb::compute_patch`].
//! Until that runtime lands, every well-formed call returns `E_IO` and
//! no patch/warnings/data are produced.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use verbreel_types::ProjectId;

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// Output subtitle format for `caption.export`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CaptionExportFormat {
    /// `SubRip` (`.srt`) format.
    Srt,
    /// `WebVTT` (`.vtt`) format.
    Vtt,
    /// Advanced `SubStation` Alpha (`.ass`) format.
    Ass,
}

impl CaptionExportFormat {
    /// Return the exact spec wire literal for this format.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Srt => "srt",
            Self::Vtt => "vtt",
            Self::Ass => "ass",
        }
    }
}

/// Arguments for `caption.export`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptionExportArgs {
    /// Target project id.
    pub project_id: ProjectId,
    /// Source text track selector.
    pub text_track: String,
    /// Destination subtitle path.
    pub out_path: String,
    /// Explicit format override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<CaptionExportFormat>,
    /// Overwrite existing destination path (future runtime path).
    #[serde(default)]
    pub overwrite: bool,
}

/// Envelope `data` returned by a future successful `caption.export`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptionExportData {
    /// Resolved output path.
    pub out_path: String,
    /// Resolved subtitle format.
    pub format: CaptionExportFormat,
    /// Count of subtitle segments emitted.
    pub segment_count: u32,
    /// Style fields that were dropped during format conversion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dropped_style_fields: Option<Vec<String>>,
}

/// Verb-level failures for `caption.export`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CaptionExportError {
    /// Runtime sidecar writer is unavailable in this pure v1 floor.
    #[error("caption.export: E_IO — {detail}")]
    Io {
        /// Human-readable runtime detail.
        detail: String,
    },
}

/// Resolve the output format according to §10.6 precedence.
///
/// Explicit `format` wins. Otherwise the `out_path` extension is used
/// for `.srt` / `.vtt` / `.ass` (case-insensitive). Unknown or missing
/// extensions default to `srt`.
#[must_use]
pub fn resolved_format(args: &CaptionExportArgs) -> CaptionExportFormat {
    if let Some(format) = args.format {
        return format;
    }

    let ext = Path::new(&args.out_path)
        .extension()
        .and_then(std::ffi::OsStr::to_str);

    match ext {
        Some(ext) if ext.eq_ignore_ascii_case("srt") => CaptionExportFormat::Srt,
        Some(ext) if ext.eq_ignore_ascii_case("vtt") => CaptionExportFormat::Vtt,
        Some(ext) if ext.eq_ignore_ascii_case("ass") => CaptionExportFormat::Ass,
        _ => CaptionExportFormat::Srt,
    }
}

/// Build the RFC 6902 patch for `caption.export`.
///
/// v1 floor: always returns [`CaptionExportError::Io`].
///
/// # Errors
///
/// Always errors with [`CaptionExportError::Io`] in v1 because subtitle
/// sidecar writing is intentionally deferred.
pub fn compute_patch(
    _prior: &Project,
    args: &CaptionExportArgs,
) -> Result<(Value, Vec<Value>, CaptionExportData), CaptionExportError> {
    let format = resolved_format(args);
    Err(CaptionExportError::Io {
        detail: format!(
            "subtitle sidecar writer unavailable in the v1 floor (out_path `{}`, format `{}`)",
            args.out_path,
            format.as_str()
        ),
    })
}

impl From<CaptionExportError> for VerbError {
    fn from(value: CaptionExportError) -> Self {
        match value {
            CaptionExportError::Io { .. } => VerbError::Custom(value.to_string()),
        }
    }
}

/// The §0.8 verb for `caption.export`.
#[derive(Debug, Default)]
pub struct CaptionExportVerb;

impl Verb for CaptionExportVerb {
    fn verb(&self) -> &'static str {
        "caption.export"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: CaptionExportArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("caption.export: args deserialize failed: {err}"),
            })?;

        match compute_patch(prior, &typed) {
            Ok((patch_value, warnings, data)) => {
                let patch: json_patch::Patch =
                    serde_json::from_value(patch_value).map_err(|err| {
                        VerbError::Custom(format!(
                            "caption.export: patch construction failed: {err}"
                        ))
                    })?;
                Ok((
                    patch,
                    serde_json::to_value(data).map_err(|err| {
                        VerbError::Custom(format!(
                            "caption.export: data serialization failed: {err}"
                        ))
                    })?,
                    warnings,
                ))
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
        let _typed: CaptionExportArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "CaptionExportArgs",
            })?;

        Ok(Value::Null)
    }
}
