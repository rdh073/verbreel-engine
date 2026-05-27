//! `template.apply` (§16.3) — v1 template not-found floor.
//!
//! ## Spec quote (`spec/commands/template.md` §16.3, abbreviated)
//!
//! > CLI: `verbreel template apply --template_id <id> --slots '<json>'`
//! > MCP: `template.apply`
//! > Args: `project_id`, `template_id`, `slots`, optional `at_tk`,
//! > optional `track_strategy`.
//! > Returns (`data`): insertion summary.
//! > Errors include `E_TEMPLATE_NOT_FOUND` and `E_BAD_TIME`.
//!
//! ## v1 floor
//!
//! This pure verb slice validates only local argument shape, applies
//! the local negative-time rule (`at_tk < 0`), then returns
//! `E_TEMPLATE_NOT_FOUND` for every otherwise well-formed request.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use verbreel_types::ProjectId;

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// Track-placement strategy for `template.apply`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TemplateTrackStrategy {
    /// Always mint new target tracks for template tracks.
    #[default]
    CreateNew,
    /// Reuse existing target tracks by `(kind, name)` when possible.
    UseExisting,
}

impl TemplateTrackStrategy {
    #[allow(clippy::trivially_copy_pass_by_ref)]
    fn is_create_new(&self) -> bool {
        *self == Self::CreateNew
    }
}

/// Arguments for `template.apply`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateApplyArgs {
    /// Target project id required by the `Verb` trait shape.
    pub project_id: ProjectId,
    /// Opaque template id to apply.
    pub template_id: String,
    /// Slot values keyed by opaque slot id.
    pub slots: BTreeMap<String, String>,
    /// Optional insertion tick.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_tk: Option<i64>,
    /// Track placement strategy (default: `create_new`).
    #[serde(default, skip_serializing_if = "TemplateTrackStrategy::is_create_new")]
    pub track_strategy: TemplateTrackStrategy,
}

/// Future success envelope for `template.apply`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateApplyData {
    /// Echoed template id.
    pub template_id: String,
    /// Resolved insertion tick.
    pub at_tk: i64,
    /// Duration of inserted content.
    pub duration_tk: i64,
    /// Freshly created target track ids.
    pub created_track_ids: Vec<String>,
    /// Reused target track ids.
    pub reused_track_ids: Vec<String>,
    /// Freshly inserted media clip ids.
    pub created_clip_ids: Vec<String>,
    /// Freshly inserted text clip ids.
    pub created_text_clip_ids: Vec<String>,
    /// Imported embedded-asset ids.
    pub imported_asset_ids: Vec<String>,
    /// Number of substituted slots.
    pub substituted_slot_count: u64,
    /// Slot ids that used default values.
    pub defaulted_slot_ids: Vec<String>,
}

/// Verb-level failures for `template.apply`.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TemplateApplyError {
    /// Runtime template-catalog miss for a well-formed request.
    #[error(
        "template.apply: E_TEMPLATE_NOT_FOUND — template_id `{template_id}` does not resolve \
         to an installed template"
    )]
    TemplateNotFound {
        /// Template id supplied by caller.
        template_id: String,
    },
    /// Reserved for required/default slot resolution in follow-up slices.
    #[error(
        "template.apply: E_TEMPLATE_SLOT_MISSING — missing required slot ids: \
         {missing_slot_ids:?}"
    )]
    SlotMissing {
        /// Missing required slot ids.
        missing_slot_ids: Vec<String>,
    },
    /// Reserved for slot-kind mismatch validation in follow-up slices.
    #[error(
        "template.apply: E_TEMPLATE_SLOT_KIND_MISMATCH — slot `{slot_id}` expected \
         `{expected_kind}` but got `{actual_value_type}`"
    )]
    SlotKindMismatch {
        /// Offending slot id.
        slot_id: String,
        /// Declared slot kind.
        expected_kind: String,
        /// Runtime value type/kind.
        actual_value_type: String,
    },
    /// Reserved for slot-constraint validation in follow-up slices.
    #[error(
        "template.apply: E_TEMPLATE_SLOT_CONSTRAINT — slot `{slot_id}` violated `{bound}`: \
         {detail}"
    )]
    SlotConstraint {
        /// Offending slot id.
        slot_id: String,
        /// Violated constraint bound.
        bound: String,
        /// Human-readable detail.
        detail: String,
    },
    /// Reserved for template/target schema compatibility checks.
    #[error("template.apply: E_TEMPLATE_SCHEMA_VIOLATION — {detail}")]
    SchemaViolation {
        /// Human-readable schema compatibility detail.
        detail: String,
    },
    /// Reserved for `use_existing` name-match kind conflicts.
    #[error(
        "template.apply: E_TEMPLATE_TRACK_KIND_MISMATCH — template track `{template_track_name}` \
         kind `{template_track_kind}` mismatches target kind `{target_track_kind}`"
    )]
    TrackKindMismatch {
        /// Template track name.
        template_track_name: String,
        /// Template track kind.
        template_track_kind: String,
        /// Target track kind.
        target_track_kind: String,
    },
    /// Reserved for reused-track overlap checks.
    #[error(
        "template.apply: E_CLIP_OVERLAP — template track `{failed_template_track_name}` \
         collides with clips {colliding_clip_ids:?}: {hint}"
    )]
    ClipOverlap {
        /// Template track that failed overlap validation.
        failed_template_track_name: String,
        /// Colliding target clip ids.
        colliding_clip_ids: Vec<String>,
        /// Caller-facing hint.
        hint: String,
    },
    /// Reserved for missing media-slot asset validation.
    #[error(
        "template.apply: E_ASSET_NOT_FOUND — slot `{slot_id}` references missing asset_id \
         `{asset_id}`"
    )]
    AssetNotFound {
        /// Slot id referencing missing asset.
        slot_id: String,
        /// Missing target asset id.
        asset_id: String,
    },
    /// `at_tk` must be greater than or equal to `0`.
    #[error("template.apply: E_BAD_TIME — at_tk {at_tk} must be >= 0")]
    BadTime {
        /// Offending insertion tick.
        at_tk: i64,
    },
    /// Reserved for locked reused-track checks.
    #[error("template.apply: E_LOCKED — target track `{track_id}` is locked")]
    Locked {
        /// Locked target track id.
        track_id: String,
    },
    /// Reserved for runtime busy-state checks.
    #[error("template.apply: E_BUSY — {detail}")]
    Busy {
        /// Runtime busy detail.
        detail: String,
    },
}

/// Build the RFC 6902 patch for `template.apply`.
///
/// v1 floor: validates optional `at_tk >= 0` and then always returns
/// [`TemplateApplyError::TemplateNotFound`].
///
/// # Errors
///
/// Returns [`TemplateApplyError::BadTime`] when `at_tk < 0`.
/// Returns [`TemplateApplyError::TemplateNotFound`] for every
/// otherwise well-formed request.
pub fn compute_patch(
    _prior: &Project,
    args: &TemplateApplyArgs,
) -> Result<(Value, Vec<Value>, Value), TemplateApplyError> {
    if let Some(at_tk) = args.at_tk
        && at_tk < 0
    {
        return Err(TemplateApplyError::BadTime { at_tk });
    }

    Err(TemplateApplyError::TemplateNotFound {
        template_id: args.template_id.clone(),
    })
}

impl From<TemplateApplyError> for VerbError {
    fn from(value: TemplateApplyError) -> Self {
        match value {
            TemplateApplyError::TemplateNotFound { .. }
            | TemplateApplyError::SlotMissing { .. }
            | TemplateApplyError::SlotKindMismatch { .. }
            | TemplateApplyError::SlotConstraint { .. }
            | TemplateApplyError::SchemaViolation { .. }
            | TemplateApplyError::TrackKindMismatch { .. }
            | TemplateApplyError::ClipOverlap { .. }
            | TemplateApplyError::AssetNotFound { .. }
            | TemplateApplyError::BadTime { .. }
            | TemplateApplyError::Locked { .. }
            | TemplateApplyError::Busy { .. } => VerbError::Custom(value.to_string()),
        }
    }
}

/// The §0.8 verb for `template.apply`.
#[derive(Debug, Default)]
pub struct TemplateApplyVerb;

impl Verb for TemplateApplyVerb {
    fn verb(&self) -> &'static str {
        "template.apply"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: TemplateApplyArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("template.apply: args deserialize failed: {err}"),
            })?;

        match compute_patch(prior, &typed) {
            Ok((patch_value, warnings, data)) => {
                let patch: json_patch::Patch =
                    serde_json::from_value(patch_value).map_err(|err| {
                        VerbError::Custom(format!(
                            "template.apply: patch construction failed: {err}"
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
        let _typed: TemplateApplyArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "TemplateApplyArgs",
            })?;

        Ok(Value::Null)
    }
}
