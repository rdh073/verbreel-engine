//! `compound.expand` (§20.2) — v1 compound-schema floor.
//!
//! Real `compound.expand` needs compound-asset schema/runtime context
//! outside pure [`Verb::compute_patch`]. This v1 floor validates arg
//! shape and selector form, then returns a spec-coded floor error.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use verbreel_types::{AssetId, ClipId, ProjectId, TrackId};

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// Arguments for `compound.expand`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompoundExpandArgs {
    /// Target project id.
    pub project_id: ProjectId,
    /// Target compound clip selector (`<UUIDv7>` or `clip:<UUIDv7>`).
    pub clip: String,
}

/// Future success envelope for `compound.expand`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompoundExpandData {
    /// Removed compound clip id.
    pub removed_compound_clip_id: ClipId,
    /// Newly materialized constituent clip ids.
    pub new_clip_ids: Vec<ClipId>,
    /// Outer track id receiving constituents.
    pub track_id: TrackId,
    /// Number of constituent clips inserted.
    pub constituent_count: i64,
    /// Compound asset that was expanded.
    pub compound_asset_id: AssetId,
}

/// Verb-level errors for `compound.expand`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CompoundExpandError {
    /// Target clip was not found.
    #[error("compound.expand: E_NOT_FOUND — clip `{clip}` not found")]
    NotFound {
        /// Missing target clip selector/id.
        clip: String,
    },

    /// Structural selector matched no targets.
    #[error("compound.expand: E_NO_MATCH — selector `{selector}` matched nothing")]
    NoMatch {
        /// Missing structural selector.
        selector: String,
    },

    /// Selector parse failed.
    #[error("compound.expand: E_BAD_SELECTOR — {detail}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
    },

    /// Qualified selector prefix resolved to wrong kind.
    #[error("compound.expand: E_SELECTOR_KIND_MISMATCH — actual_kind `{actual_kind}`")]
    SelectorKindMismatch {
        /// Offending selector kind token.
        actual_kind: String,
    },

    /// Target clip is not compound-kind.
    #[error(
        "compound.expand: E_COMPOUND_NOT_A_COMPOUND — clip `{clip_id}` has kind `{actual_kind}`"
    )]
    CompoundNotACompound {
        /// Target clip id.
        clip_id: String,
        /// Actual resolved asset kind detail.
        actual_kind: String,
    },

    /// Expansion would create an overlap.
    #[error("compound.expand: E_CLIP_OVERLAP — failed_clip `{failed_clip}`")]
    ClipOverlap {
        /// First failing clip id during expansion placement.
        failed_clip: String,
    },

    /// Target is locked.
    #[error("compound.expand: E_LOCKED — failed_target `{failed_target}`")]
    Locked {
        /// Locked target identifier.
        failed_target: String,
    },
}

fn parse_clip_selector(raw: &str) -> Result<ClipId, CompoundExpandError> {
    if raw.is_empty() {
        return Err(CompoundExpandError::BadSelector {
            detail: "selector is empty".to_string(),
        });
    }

    if let Some((prefix, body)) = raw.split_once(':') {
        return match prefix {
            "clip" => body
                .parse::<ClipId>()
                .map_err(|err| CompoundExpandError::BadSelector {
                    detail: format!("clip body parse failed: {err}"),
                }),
            other => Err(CompoundExpandError::SelectorKindMismatch {
                actual_kind: other.to_string(),
            }),
        };
    }

    raw.parse::<ClipId>()
        .map_err(|err| CompoundExpandError::BadSelector {
            detail: format!("clip selector parse failed: {err}"),
        })
}

/// Build the RFC 6902 patch for `compound.expand`.
///
/// v1 floor: every accepted bare/`clip:` selector returns
/// `E_COMPOUND_NOT_A_COMPOUND`.
///
/// # Errors
///
/// Returns [`CompoundExpandError`] for selector validation or the v1
/// floor runtime/schema unavailability.
pub fn compute_patch(
    _prior: &Project,
    args: &CompoundExpandArgs,
) -> Result<(Value, Vec<Value>, Value), CompoundExpandError> {
    let clip_id = parse_clip_selector(&args.clip)?;

    Err(CompoundExpandError::CompoundNotACompound {
        clip_id: clip_id.to_string(),
        actual_kind: "compound schema unavailable in v1 floor".to_string(),
    })
}

impl From<CompoundExpandError> for VerbError {
    fn from(value: CompoundExpandError) -> Self {
        match value {
            CompoundExpandError::NotFound { .. }
            | CompoundExpandError::NoMatch { .. }
            | CompoundExpandError::BadSelector { .. }
            | CompoundExpandError::SelectorKindMismatch { .. }
            | CompoundExpandError::CompoundNotACompound { .. }
            | CompoundExpandError::ClipOverlap { .. }
            | CompoundExpandError::Locked { .. } => VerbError::Custom(value.to_string()),
        }
    }
}

/// The §0.8 verb entry for `compound.expand`.
#[derive(Debug, Default)]
pub struct CompoundExpandVerb;

impl Verb for CompoundExpandVerb {
    fn verb(&self) -> &'static str {
        "compound.expand"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: CompoundExpandArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("compound.expand: args deserialize failed: {err}"),
            })?;

        match compute_patch(prior, &typed) {
            Ok((patch_value, warnings, data)) => {
                let patch: json_patch::Patch =
                    serde_json::from_value(patch_value).map_err(|err| {
                        VerbError::Custom(format!(
                            "compound.expand: patch construction failed: {err}"
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
        let _typed: CompoundExpandArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "CompoundExpandArgs",
            })?;
        Ok(Value::Null)
    }
}
