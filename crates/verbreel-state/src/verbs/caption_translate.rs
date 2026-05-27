//! `caption.translate` (§10.3) — ninety-second production verb.
//!
//! ## v1 floor — always errors with `E_BUSY`.
//!
//! Real caption translation requires long-running streaming runtime
//! plumbing (audio decode, model invocation, progress emission, and
//! dry-run wiring) that does not belong in the pure
//! [`Verb::compute_patch`] contract yet. Until that runtime lands,
//! every well-formed call returns `E_BUSY` and no patch/warnings/data.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use verbreel_types::{ProjectId, TrackId};

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use crate::verbs::text_add::StyleArg;

/// Arguments for `caption.translate`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptionTranslateArgs {
    /// Target project id.
    pub project_id: ProjectId,
    /// Audio source selector (`track:...` or `clip:...` in the future runtime path).
    #[serde(rename = "from")]
    pub from_selector: String,
    /// Optional style preset string or partial text style object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<StyleArg>,
}

/// Envelope `data` for a future successful `caption.translate` call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptionTranslateData {
    /// Destination text-track id.
    pub text_track_id: TrackId,
    /// Number of translated segments emitted.
    pub segment_count: u32,
    /// Whisper translate target language (always `"en"`).
    pub target_language: String,
}

/// Verb-level runtime failures for `caption.translate`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CaptionTranslateError {
    /// Streaming caption translation runtime is unavailable in this slice.
    #[error("caption.translate: E_BUSY — {detail}")]
    Busy {
        /// Human-readable runtime detail.
        detail: String,
    },
}

/// Build the RFC 6902 patch for `caption.translate`.
///
/// v1 floor: always returns [`CaptionTranslateError::Busy`].
///
/// # Errors
///
/// Always errors with [`CaptionTranslateError::Busy`] in v1 because
/// writer-class streaming/AI runtime plumbing is intentionally deferred.
pub fn compute_patch(
    _prior: &Project,
    args: &CaptionTranslateArgs,
) -> Result<(Value, Vec<Value>, CaptionTranslateData), CaptionTranslateError> {
    Err(CaptionTranslateError::Busy {
        detail: format!(
            "writer-class streaming runtime unavailable in the v1 floor (from `{}`)",
            args.from_selector
        ),
    })
}

impl From<CaptionTranslateError> for VerbError {
    fn from(value: CaptionTranslateError) -> Self {
        match value {
            CaptionTranslateError::Busy { .. } => VerbError::Custom(value.to_string()),
        }
    }
}

/// The §0.8 verb for `caption.translate`.
#[derive(Debug, Default)]
pub struct CaptionTranslateVerb;

impl Verb for CaptionTranslateVerb {
    fn verb(&self) -> &'static str {
        "caption.translate"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: CaptionTranslateArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("caption.translate: args deserialize failed: {err}"),
            })?;

        match compute_patch(prior, &typed) {
            Ok((patch_value, warnings, data)) => {
                let patch: json_patch::Patch =
                    serde_json::from_value(patch_value).map_err(|err| {
                        VerbError::Custom(format!(
                            "caption.translate: patch construction failed: {err}"
                        ))
                    })?;
                Ok((
                    patch,
                    serde_json::to_value(data).map_err(|err| {
                        VerbError::Custom(format!(
                            "caption.translate: data serialization failed: {err}"
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
        let _typed: CaptionTranslateArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "CaptionTranslateArgs",
            })?;

        Ok(Value::Null)
    }
}
