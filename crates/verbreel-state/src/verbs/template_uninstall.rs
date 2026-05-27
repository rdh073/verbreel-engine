//! `template.uninstall` (§16.6) — v1 template not-found floor.
//!
//! ## v1 floor
//!
//! Real `template.uninstall` requires template-catalog and filesystem
//! runtime context to resolve bundled-vs-user installs and remove a
//! template directory. This v1 state-layer floor validates argument
//! shape and returns `E_TEMPLATE_NOT_FOUND` for every well-formed
//! request.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use verbreel_types::ProjectId;

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// Arguments for `template.uninstall`.
///
/// `project_id` is required by the current `Verb` dispatch shape and
/// is ignored by this v1 floor.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateUninstallArgs {
    /// Required by the `Verb` trait shape; not read by the v1 impl.
    pub project_id: ProjectId,
    /// Opaque template id to uninstall.
    pub template_id: String,
}

/// Future success envelope for `template.uninstall`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateUninstallData {
    /// Echoed template id.
    pub template_id: String,
    /// Removed install path.
    pub removed_path: String,
}

/// Verb-level failures for `template.uninstall`.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TemplateUninstallError {
    /// Runtime template-catalog miss for a well-formed request.
    #[error(
        "template.uninstall: E_TEMPLATE_NOT_FOUND — template_id `{template_id}` does not \
         resolve to an installed template"
    )]
    TemplateNotFound {
        /// Template id supplied by caller.
        template_id: String,
    },
    /// Reserved for bundled immutable templates in follow-up slices.
    #[error(
        "template.uninstall: E_TEMPLATE_BUNDLED_IMMUTABLE — template_id `{template_id}` source \
         `{template_source}` install_path `{install_path}`"
    )]
    TemplateBundledImmutable {
        /// Template id supplied by caller.
        template_id: String,
        /// Template source (`bundled`).
        template_source: String,
        /// Immutable install path.
        install_path: String,
    },
    /// Reserved for filesystem failures in follow-up slices.
    #[error("template.uninstall: E_IO — {detail}")]
    Io {
        /// Human-readable runtime detail.
        detail: String,
    },
}

/// Build the RFC 6902 patch for `template.uninstall`.
///
/// v1 floor: always returns [`TemplateUninstallError::TemplateNotFound`]
/// for every well-formed request.
///
/// # Errors
///
/// Returns [`TemplateUninstallError::TemplateNotFound`] for every
/// well-formed request.
pub fn compute_patch(
    _prior: &Project,
    args: &TemplateUninstallArgs,
) -> Result<(Value, Vec<Value>, Value), TemplateUninstallError> {
    Err(TemplateUninstallError::TemplateNotFound {
        template_id: args.template_id.clone(),
    })
}

impl From<TemplateUninstallError> for VerbError {
    fn from(value: TemplateUninstallError) -> Self {
        match value {
            TemplateUninstallError::TemplateNotFound { .. }
            | TemplateUninstallError::TemplateBundledImmutable { .. }
            | TemplateUninstallError::Io { .. } => VerbError::Custom(value.to_string()),
        }
    }
}

/// The §0.8 verb for `template.uninstall`.
#[derive(Debug, Default)]
pub struct TemplateUninstallVerb;

impl Verb for TemplateUninstallVerb {
    fn verb(&self) -> &'static str {
        "template.uninstall"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: TemplateUninstallArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("template.uninstall: args deserialize failed: {err}"),
            })?;

        match compute_patch(prior, &typed) {
            Ok((patch_value, warnings, data)) => {
                let patch: json_patch::Patch =
                    serde_json::from_value(patch_value).map_err(|err| {
                        VerbError::Custom(format!(
                            "template.uninstall: patch construction failed: {err}"
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
        let _typed: TemplateUninstallArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "TemplateUninstallArgs",
            })?;

        Ok(Value::Null)
    }
}
