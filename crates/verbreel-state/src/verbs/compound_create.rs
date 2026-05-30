//! `compound.create` (§20.1) — v1 storage/schema floor.
//!
//! Real `compound.create` needs compound-asset schema/storage context
//! outside pure [`Verb::compute_patch`]. This v1 floor validates arg
//! shape and cheap limits, then returns spec-coded floor errors.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use verbreel_types::{AssetId, ClipId, ProjectId, TrackId};

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// Per-verb upper bound on `clips` (`maxItems` in §20.1).
pub const COMPOUND_CREATE_CLIPS_MAX: usize = 1000;

/// Arguments for `compound.create`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompoundCreateArgs {
    /// Target project id.
    pub project_id: ProjectId,
    /// Selected clip ids to compact into one compound clip.
    pub clips: Vec<ClipId>,
    /// Optional output compound clip name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Optional non-contiguous selection override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_gaps: Option<bool>,
}

/// Resolve `allow_gaps` default.
#[must_use]
pub fn resolved_allow_gaps(args: &CompoundCreateArgs) -> bool {
    args.allow_gaps.unwrap_or(false)
}

/// Future success envelope for `compound.create`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompoundCreateData {
    /// New outer compound clip id.
    pub compound_clip_id: ClipId,
    /// Compound asset id (new or deduped).
    pub compound_asset_id: AssetId,
    /// Removed source clip ids in input order.
    pub removed_clip_ids: Vec<ClipId>,
    /// Track containing the replacement clip.
    pub track_id: TrackId,
    /// Compound clip position in ticks.
    pub track_position_tk: i64,
    /// Compound source duration in ticks.
    pub duration_tk: i64,
    /// Outer clips whose singleton link groups were cleared.
    pub cleared_link_group_clip_ids: Vec<ClipId>,
    /// `true` when an existing compound asset hash was reused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deduped_existing_asset: Option<bool>,
}

/// Verb-level errors for `compound.create`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CompoundCreateError {
    /// No clips were selected.
    #[error(
        "compound.create: E_COMPOUND_EMPTY — clips selection is empty; provide at least one clip id"
    )]
    CompoundEmpty,

    /// Selected clips span multiple tracks.
    #[error("compound.create: E_COMPOUND_MIXED_TRACKS — member_track_ids={member_track_ids:?}")]
    CompoundMixedTracks {
        /// Distinct offending track ids.
        member_track_ids: Vec<String>,
    },

    /// Selection is not contiguous while `allow_gaps=false`.
    #[error(
        "compound.create: E_COMPOUND_NON_CONTIGUOUS — first_gap_after_clip_id \
         `{first_gap_after_clip_id}` first_gap_size_tk={first_gap_size_tk}"
    )]
    CompoundNonContiguous {
        /// Clip id immediately before first gap.
        first_gap_after_clip_id: String,
        /// First gap size in ticks.
        first_gap_size_tk: i64,
    },

    /// One requested clip target was missing.
    #[error(
        "compound.create: E_NOT_FOUND — failed_index={failed_index} failed_target \
         `{failed_target}`"
    )]
    NotFound {
        /// Requested array index that failed.
        failed_index: usize,
        /// Missing target identifier.
        failed_target: String,
    },

    /// One requested target was locked.
    #[error("compound.create: E_LOCKED — failed_target `{failed_target}`")]
    Locked {
        /// Locked target identifier.
        failed_target: String,
    },

    /// Arg-schema cap violation (`clips` maxItems).
    #[error(
        "compound.create: E_SCHEMA_VIOLATION — field `{field}` exceeds maxItems \
         (actual: {actual}, max: {max}); {hint}"
    )]
    SchemaViolation {
        /// Offending field.
        field: &'static str,
        /// Recovery hint.
        hint: &'static str,
        /// Caller-supplied count.
        actual: usize,
        /// Allowed cap.
        max: usize,
    },

    /// v1 floor for accepted non-empty requests.
    #[error("compound.create: E_SCHEMA_VIOLATION — {detail}")]
    StorageSchemaUnavailable {
        /// Human-readable storage/schema floor detail.
        detail: String,
    },
}

/// Build the RFC 6902 patch for `compound.create`.
///
/// v1 floor:
/// - `clips.len() > 1000` => `E_SCHEMA_VIOLATION` (arg cap)
/// - `clips.is_empty()` => `E_COMPOUND_EMPTY`
/// - otherwise => `E_SCHEMA_VIOLATION` storage/schema floor
///
/// # Errors
///
/// Returns [`CompoundCreateError`] for cap/empty/floor paths.
pub fn compute_patch(
    _prior: &Project,
    args: &CompoundCreateArgs,
) -> Result<(Value, Vec<Value>, Value), CompoundCreateError> {
    if args.clips.len() > COMPOUND_CREATE_CLIPS_MAX {
        return Err(CompoundCreateError::SchemaViolation {
            field: "clips",
            hint: "split the selection into smaller compounds",
            actual: args.clips.len(),
            max: COMPOUND_CREATE_CLIPS_MAX,
        });
    }

    if args.clips.is_empty() {
        return Err(CompoundCreateError::CompoundEmpty);
    }

    let _allow_gaps = resolved_allow_gaps(args);

    Err(CompoundCreateError::StorageSchemaUnavailable {
        detail: "compound asset schema/storage context unavailable in the pure state-layer v1 \
                 floor"
            .to_string(),
    })
}

impl From<CompoundCreateError> for VerbError {
    fn from(value: CompoundCreateError) -> Self {
        match value {
            CompoundCreateError::SchemaViolation { .. } => VerbError::BadArgs {
                detail: value.to_string(),
            },
            CompoundCreateError::CompoundEmpty
            | CompoundCreateError::CompoundMixedTracks { .. }
            | CompoundCreateError::CompoundNonContiguous { .. }
            | CompoundCreateError::NotFound { .. }
            | CompoundCreateError::Locked { .. }
            | CompoundCreateError::StorageSchemaUnavailable { .. } => {
                VerbError::Custom(value.to_string())
            }
        }
    }
}

/// The §0.8 verb entry for `compound.create`.
#[derive(Debug, Default)]
pub struct CompoundCreateVerb;

impl Verb for CompoundCreateVerb {
    fn verb(&self) -> &'static str {
        "compound.create"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: CompoundCreateArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("compound.create: args deserialize failed: {err}"),
            })?;

        match compute_patch(prior, &typed) {
            Ok((patch_value, warnings, data)) => {
                let patch: json_patch::Patch =
                    serde_json::from_value(patch_value).map_err(|err| {
                        VerbError::Custom(format!(
                            "compound.create: patch construction failed: {err}"
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
        let _typed: CompoundCreateArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "CompoundCreateArgs",
            })?;
        Ok(Value::Null)
    }
}
