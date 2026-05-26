//! `validate_command` (§1.4) — sixty-fifth production verb in the engine.
//!
//! ## Spec quote (`spec/commands/meta.md` §1.4)
//!
//! > CLI: `verbreel validate --verb <name> --args '<json>'`
//! > MCP: `validate_command`
//! > Args: `verb: string`, `args: object`
//! > Returns (`data`): `{ valid: true } | { valid: false; errors: SchemaError[] }`
//! > Errors: `E_UNKNOWN_VERB`
//!
//! ## Dry-validates a command without applying it.
//!
//! `validate_command` is read-only and does not read or mutate project
//! state. It looks up `args.verb` in
//! [`crate::verbs::default_registry`], constructs a synthetic empty
//! project via [`crate::verbs::synthetic_empty_project`], then dispatches
//! the target verb's `compute_patch` with `args.args` as the JSON Value.
//!
//! Result interpretation:
//!
//! - `Ok(_)` → `Valid` (args parsed and verb succeeded against empty project).
//! - `Err(VerbError::BadArgs)` → `Invalid` (arg-shape rejection — wraps the
//!   detail string in a single [`SchemaError`] with empty `path`).
//! - `Err(VerbError::InvariantViolation | VerbError::Custom)` → `Valid`
//!   (args parsed; verb failed for project-state reasons unrelated to
//!   arg shape).
//!
//! ## `SchemaError` shape (spec gap)
//!
//! Spec §1.4 references `SchemaError[]` but does not formally define
//! the field shape. v1 ships the 2-field shape `{path, message}` with
//! `path` always empty (we surface a single deser-error message, not a
//! per-field validation list) and `message` carrying the
//! `VerbError::BadArgs.detail` text. Multi-error per-field validation
//! requires a JSON-Schema validator wired into the engine and is a
//! follow-up slice.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::ProjectId;

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// Arguments for `validate_command`.
///
/// `project_id` is required for `Verb` trait compatibility; the impl
/// ignores it (the spec defines only `verb` and `args`). See
/// `list_capabilities` and `help` for the same accommodation pattern.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidateCommandArgs {
    /// Required by the `Verb` trait shape; not read by the impl.
    pub project_id: ProjectId,

    /// Verb id to validate (e.g. `"clip.add"`).
    pub verb: String,

    /// The JSON args object to validate against `verb`.
    pub args: Value,
}

/// Per-error entry in the `errors[]` array of an `Invalid` response.
///
/// 2-field shape (`path`, `message`) — see module docs for the spec gap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaError {
    /// JSON-pointer-style path to the offending field. Always `""` in v1.
    pub path: String,
    /// Human-readable error message — sourced from `VerbError::BadArgs.detail`.
    pub message: String,
}

/// Envelope `data` returned by `validate_command`.
///
/// Discriminated by `valid`: `{"valid": true}` (single field) for the
/// Valid branch; `{"valid": false, "errors": [...]}` (two fields) for
/// the Invalid branch. Modeled as a struct with `errors: Option<...>`
/// plus `skip_serializing_if = "Option::is_none"` so the wire shape
/// round-trips cleanly through serde without untagged-enum ordering
/// gymnastics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidateCommandData {
    /// Discriminator: `true` for Valid, `false` for Invalid.
    pub valid: bool,
    /// Populated only when `valid == false`; omitted from the wire when
    /// `valid == true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub errors: Option<Vec<SchemaError>>,
}

impl ValidateCommandData {
    /// Construct the Valid branch (`{"valid": true}`).
    #[must_use]
    pub fn valid() -> Self {
        Self {
            valid: true,
            errors: None,
        }
    }

    /// Construct the Invalid branch with one or more `SchemaError`s.
    #[must_use]
    pub fn invalid(errors: Vec<SchemaError>) -> Self {
        Self {
            valid: false,
            errors: Some(errors),
        }
    }
}

/// Verb-level failures for `validate_command`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ValidateCommandError {
    /// `args.verb` is not registered in `default_registry()`.
    /// Maps to `E_UNKNOWN_VERB`.
    #[error("validate_command: E_UNKNOWN_VERB unknown verb `{verb}`")]
    UnknownVerb {
        /// Verb id that failed lookup.
        verb: String,
    },
}

/// Build the `validate_command` data envelope for `args`.
fn build_data(args: &ValidateCommandArgs) -> Result<ValidateCommandData, ValidateCommandError> {
    let registry = crate::verbs::default_registry();
    let Some(target) = registry.get(&args.verb) else {
        return Err(ValidateCommandError::UnknownVerb {
            verb: args.verb.clone(),
        });
    };

    let stub = crate::verbs::synthetic_empty_project(args.project_id);
    let result = target.compute_patch(&stub, &args.args);

    // `Ok` and `Err(InvariantViolation | Custom)` both map to Valid: the
    // first means the verb accepted args and succeeded against an empty
    // project; the latter means args parsed OK and the verb failed for
    // a project-state / engine-invariant reason that is not an arg-shape
    // issue. Per §1.4 ("dry-validates a command") only `BadArgs` rejects.
    #[allow(clippy::match_same_arms)]
    let data = match result {
        Ok(_) => ValidateCommandData::valid(),
        Err(VerbError::BadArgs { detail }) => ValidateCommandData::invalid(vec![SchemaError {
            path: String::new(),
            message: detail,
        }]),
        Err(_) => ValidateCommandData::valid(),
    };
    Ok(data)
}

/// Build the (empty) RFC-6902 patch and data envelope for
/// `validate_command`.
///
/// The patch is always `[]` and the warnings vec is always empty — this
/// is a read-only verb that ignores its `prior` project.
///
/// # Errors
///
/// Returns [`ValidateCommandError::UnknownVerb`] when `args.verb` is not
/// registered in [`crate::verbs::default_registry`].
pub fn compute_patch(
    _prior: &Project,
    args: &ValidateCommandArgs,
) -> Result<(Value, Vec<Value>, ValidateCommandData), ValidateCommandError> {
    let data = build_data(args)?;
    Ok((json!([]), Vec::new(), data))
}

/// Rebuild the data envelope from `(args, post_state)`.
///
/// `validate_command` is project-agnostic, so the post-state is ignored
/// — the envelope depends solely on `args` and the current build's verb
/// registry.
///
/// # Errors
///
/// Returns [`ReconstructError`] when the inner build path surfaces
/// `UnknownVerb`.
pub fn data_envelope_from_post_state(
    args: &ValidateCommandArgs,
    _post_state: &Project,
) -> Result<ValidateCommandData, ReconstructError> {
    build_data(args).map_err(|err| ReconstructError::Custom(err.to_string()))
}

impl From<ValidateCommandError> for VerbError {
    fn from(value: ValidateCommandError) -> Self {
        match value {
            ValidateCommandError::UnknownVerb { .. } => VerbError::BadArgs {
                detail: value.to_string(),
            },
        }
    }
}

/// The §0.8 verb for `validate_command`.
#[derive(Debug, Default)]
pub struct ValidateCommandVerb;

impl Verb for ValidateCommandVerb {
    fn verb(&self) -> &'static str {
        "validate_command"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: ValidateCommandArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("validate_command: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch = serde_json::from_value(patch_value).map_err(|err| {
            VerbError::Custom(format!(
                "validate_command: patch construction failed: {err}"
            ))
        })?;
        let data = serde_json::to_value(&data).map_err(|err| {
            VerbError::Custom(format!("validate_command: data envelope failed: {err}"))
        })?;

        Ok((patch, data, warnings))
    }

    fn reconstruct(
        &self,
        args: &Value,
        _patch: &Value,
        _warnings: &[Value],
        post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let typed: ValidateCommandArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "ValidateCommandArgs",
            })?;
        let envelope = data_envelope_from_post_state(&typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
