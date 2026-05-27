//! `template.install` (§16.5) — v1 file-installer unavailable floor.
//!
//! ## v1 floor
//!
//! Real `template.install` validates and explodes a portable
//! `.verbreel-template` file into the user template directory, applying
//! overwrite policy and writing installed template files. Those
//! operations require filesystem/runtime context outside pure
//! [`Verb::compute_patch`]. This v1 state-layer floor validates only
//! argument shape and returns `E_IO` for every well-formed request.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use verbreel_types::ProjectId;

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// Arguments for `template.install`.
///
/// `project_id` is required by the current `Verb` dispatch shape and is
/// ignored by this v1 floor.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateInstallArgs {
    /// Required by the `Verb` trait shape; not read by the v1 impl.
    pub project_id: ProjectId,
    /// Portable template file path (`.verbreel-template`).
    pub path: String,
    /// Whether existing installs for the same template id may be replaced.
    #[serde(default)]
    pub overwrite: bool,
}

/// Future success envelope for `template.install`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateInstallData {
    /// Installed template id.
    pub template_id: String,
    /// Absolute destination install path.
    pub install_path: String,
    /// Present only for dry-run style success paths.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub would_overwrite: Option<bool>,
}

/// Verb-level failures for `template.install`.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TemplateInstallError {
    /// Reserved for path-safety escapes per §0.11.
    #[error("template.install: E_PATH_ESCAPE — {detail}")]
    PathEscape {
        /// Human-readable path-safety detail.
        detail: String,
    },
    /// Reserved for schema/validation failures, including id collisions.
    #[error("template.install: E_TEMPLATE_SCHEMA_VIOLATION — {detail}")]
    TemplateSchemaViolation {
        /// Human-readable validation detail.
        detail: String,
    },
    /// Runtime installer/runtime context is unavailable in this pure v1 floor.
    #[error("template.install: E_IO — {detail}")]
    Io {
        /// Human-readable runtime detail.
        detail: String,
    },
}

/// Build the RFC 6902 patch for `template.install`.
///
/// v1 floor: always returns [`TemplateInstallError::Io`].
///
/// # Errors
///
/// Always errors with [`TemplateInstallError::Io`] in v1 because
/// template-file install/write operations are intentionally deferred.
pub fn compute_patch(
    _prior: &Project,
    args: &TemplateInstallArgs,
) -> Result<(Value, Vec<Value>, TemplateInstallData), TemplateInstallError> {
    Err(TemplateInstallError::Io {
        detail: format!(
            "template file installer unavailable in the v1 floor (path `{}`)",
            args.path
        ),
    })
}

impl From<TemplateInstallError> for VerbError {
    fn from(value: TemplateInstallError) -> Self {
        match value {
            TemplateInstallError::PathEscape { .. }
            | TemplateInstallError::TemplateSchemaViolation { .. }
            | TemplateInstallError::Io { .. } => VerbError::Custom(value.to_string()),
        }
    }
}

/// The §0.8 verb for `template.install`.
#[derive(Debug, Default)]
pub struct TemplateInstallVerb;

impl Verb for TemplateInstallVerb {
    fn verb(&self) -> &'static str {
        "template.install"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: TemplateInstallArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("template.install: args deserialize failed: {err}"),
            })?;

        match compute_patch(prior, &typed) {
            Ok((patch_value, warnings, data)) => {
                let patch: json_patch::Patch =
                    serde_json::from_value(patch_value).map_err(|err| {
                        VerbError::Custom(format!(
                            "template.install: patch construction failed: {err}"
                        ))
                    })?;
                Ok((
                    patch,
                    serde_json::to_value(data).map_err(|err| {
                        VerbError::Custom(format!(
                            "template.install: data serialization failed: {err}"
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
        let _typed: TemplateInstallArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "TemplateInstallArgs",
            })?;

        Ok(Value::Null)
    }
}
