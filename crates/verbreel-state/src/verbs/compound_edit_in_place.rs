//! `compound.edit_in_place` (§20.4) — v1 compound-session floor.
//!
//! Real `compound.edit_in_place` needs compound-asset schema and
//! runtime session-management context outside pure
//! [`Verb::compute_patch`]. This v1 floor validates arg shape and
//! selector form, then returns a spec-coded floor error.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use verbreel_types::{AssetId, ClipId, ProjectId};

use crate::canvas::Canvas;
use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// Arguments for `compound.edit_in_place`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompoundEditInPlaceArgs {
    /// Target project id.
    pub project_id: ProjectId,
    /// Target compound clip selector (`<UUIDv7>` or `clip:<UUIDv7>`).
    pub clip: String,
}

/// Future success envelope for `compound.edit_in_place`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompoundEditInPlaceData {
    /// Allocated edit-session id.
    pub edit_session_id: String,
    /// Allocated child-project id.
    pub child_project_id: ProjectId,
    /// Target compound asset id.
    pub compound_asset_id: AssetId,
    /// Child-project duration in ticks.
    pub child_duration_tk: i64,
    /// Child-project canvas.
    pub child_canvas: Canvas,
    /// Child-project frames-per-second numerator.
    pub child_fps_num: u32,
    /// Child-project frames-per-second denominator.
    pub child_fps_den: u32,
}

/// Verb-level errors for `compound.edit_in_place`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CompoundEditInPlaceError {
    /// Target clip was not found.
    #[error("compound.edit_in_place: E_NOT_FOUND — clip `{clip}` not found")]
    NotFound {
        /// Missing target clip selector/id.
        clip: String,
    },

    /// Structural selector matched no targets.
    #[error("compound.edit_in_place: E_NO_MATCH — selector `{selector}` matched nothing")]
    NoMatch {
        /// Missing structural selector.
        selector: String,
    },

    /// Selector parse failed.
    #[error("compound.edit_in_place: E_BAD_SELECTOR — {detail}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
    },

    /// Qualified selector prefix resolved to wrong kind.
    #[error("compound.edit_in_place: E_SELECTOR_KIND_MISMATCH — actual_kind `{actual_kind}`")]
    SelectorKindMismatch {
        /// Offending selector kind token.
        actual_kind: String,
    },

    /// Target clip is not compound-kind.
    #[error(
        "compound.edit_in_place: E_COMPOUND_NOT_A_COMPOUND — clip `{clip_id}` has kind \
         `{actual_kind}`"
    )]
    CompoundNotACompound {
        /// Target clip id.
        clip_id: String,
        /// Actual resolved asset kind detail.
        actual_kind: String,
    },

    /// Target is locked.
    #[error("compound.edit_in_place: E_LOCKED — failed_target `{failed_target}`")]
    Locked {
        /// Locked target identifier.
        failed_target: String,
    },

    /// Session-capacity limit reached.
    #[error(
        "compound.edit_in_place: E_COMPOUND_SESSION_LIMIT — project_id `{project_id}` cap {cap}"
    )]
    CompoundSessionLimit {
        /// Target project id.
        project_id: String,
        /// Maximum active sessions allowed.
        cap: u32,
    },
}

fn parse_clip_selector(raw: &str) -> Result<ClipId, CompoundEditInPlaceError> {
    if raw.is_empty() {
        return Err(CompoundEditInPlaceError::BadSelector {
            detail: "selector is empty".to_string(),
        });
    }

    if let Some((prefix, body)) = raw.split_once(':') {
        return match prefix {
            "clip" => body
                .parse::<ClipId>()
                .map_err(|err| CompoundEditInPlaceError::BadSelector {
                    detail: format!("clip body parse failed: {err}"),
                }),
            other => Err(CompoundEditInPlaceError::SelectorKindMismatch {
                actual_kind: other.to_string(),
            }),
        };
    }

    raw.parse::<ClipId>()
        .map_err(|err| CompoundEditInPlaceError::BadSelector {
            detail: format!("clip selector parse failed: {err}"),
        })
}

/// Build the RFC 6902 patch for `compound.edit_in_place`.
///
/// v1 floor: every accepted bare/`clip:` selector returns
/// `E_COMPOUND_NOT_A_COMPOUND`.
///
/// # Errors
///
/// Returns [`CompoundEditInPlaceError`] for selector validation or the
/// v1 floor runtime/schema unavailability.
pub fn compute_patch(
    _prior: &Project,
    args: &CompoundEditInPlaceArgs,
) -> Result<(Value, Vec<Value>, Value), CompoundEditInPlaceError> {
    let clip_id = parse_clip_selector(&args.clip)?;

    Err(CompoundEditInPlaceError::CompoundNotACompound {
        clip_id: clip_id.to_string(),
        actual_kind: "compound schema/session runtime unavailable in v1 floor".to_string(),
    })
}

impl From<CompoundEditInPlaceError> for VerbError {
    fn from(value: CompoundEditInPlaceError) -> Self {
        match value {
            CompoundEditInPlaceError::NotFound { .. }
            | CompoundEditInPlaceError::NoMatch { .. }
            | CompoundEditInPlaceError::BadSelector { .. }
            | CompoundEditInPlaceError::SelectorKindMismatch { .. }
            | CompoundEditInPlaceError::CompoundNotACompound { .. }
            | CompoundEditInPlaceError::Locked { .. }
            | CompoundEditInPlaceError::CompoundSessionLimit { .. } => {
                VerbError::Custom(value.to_string())
            }
        }
    }
}

/// The §0.8 verb entry for `compound.edit_in_place`.
#[derive(Debug, Default)]
pub struct CompoundEditInPlaceVerb;

impl Verb for CompoundEditInPlaceVerb {
    fn verb(&self) -> &'static str {
        "compound.edit_in_place"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: CompoundEditInPlaceArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("compound.edit_in_place: args deserialize failed: {err}"),
            })?;

        match compute_patch(prior, &typed) {
            Ok((patch_value, warnings, data)) => {
                let patch: json_patch::Patch =
                    serde_json::from_value(patch_value).map_err(|err| {
                        VerbError::Custom(format!(
                            "compound.edit_in_place: patch construction failed: {err}"
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
        let _typed: CompoundEditInPlaceArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "CompoundEditInPlaceArgs",
            })?;
        Ok(Value::Null)
    }
}
