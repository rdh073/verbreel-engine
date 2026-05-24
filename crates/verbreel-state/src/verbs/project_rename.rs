//! `project.rename` (§2.9) — fourth production verb in the engine.
//!
//! ## Spec quote (`spec/commands/project.md` §2.9, verbatim)
//!
//! > Renames a project. Updates `Project.name` (and the `name` field of
//! > the project's entry in `~/.verbreel/projects-index`). Does not
//! > rename the project folder on disk — the folder name is a
//! > user-chosen path independent of the project's display name. To
//! > move/rename the folder, close the project, rename it externally,
//! > then `project.open` the new path.
//! >
//! > **CLI**: `verbreel project rename [--project <id>] --name <str>`
//! > **MCP**: `project.rename`
//! > **Args**: `project_id: string`, `name: string` (1–256 chars
//! > per `Project.name` schema).
//! > **Returns** (`data`): `{ project_id: string; name: string }`
//! > **Errors**: `E_SCHEMA_VIOLATION` (empty or >256-char name).
//!
//! ## Char-count semantics
//!
//! `Project.name` schema caps are character counts (`minLength` /
//! `maxLength`) and not byte lengths. Use `.chars().count()` here,
//! not `.len()`, so multi-byte UTF-8 names like emojis or CJK count as
//! human-visible characters.
//!
//! ## Out of scope (this slice)
//!
//! - No `~/.verbreel/projects-index` update (the name field in the
//!   on-disk project index). That is a separate lifecycle concern.
//! - No warnings / derived data / cross-entity walks.
//! - No schema duplication in Rust — this slice relies on existing
//!   schema contracts in the canonical JSON documents.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::ProjectId;

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// Minimum `Project.name` chars per schema (`minLength: 1`).
pub const PROJECT_NAME_MIN: usize = 1;

/// Maximum `Project.name` chars per schema (`maxLength: 256`).
pub const PROJECT_NAME_MAX: usize = 256;

/// Args for `project.rename`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectRenameArgs {
    /// Target project id.
    pub project_id: ProjectId,
    /// New project name.
    pub name: String,
}

/// Envelope `data` shape returned by `project.rename`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectRenameData {
    /// Target project id (echoed from args).
    pub project_id: ProjectId,
    /// Project name after the mutation (post-state name).
    pub name: String,
}

/// Verb-level errors surfaced by [`compute_patch`]. Both variants map to
/// `E_SCHEMA_VIOLATION` via [`VerbError::BadArgs`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProjectRenameError {
    /// Reject empty names (`Project.name` `minLength: 1`).
    #[error("project.rename: `name` cannot be empty")]
    NameEmpty,

    /// Reject names longer than [`PROJECT_NAME_MAX`] UTF-8 chars.
    #[error("project.rename: `name` has {actual} chars, exceeds max of {max}")]
    NameTooLong {
        /// Actual character count.
        actual: usize,
        /// Maximum allowed characters.
        max: usize,
    },
}

/// Compute the RFC 6902 patch and post-state shape for `project.rename`.
///
/// Pure function — no I/O, no clock, no RNG. Returns:
///
/// - the patch as `serde_json::Value` (`replace` op on `/name`), and
/// - the post-state name string, so the caller can build the in-memory
///   post-state for the data envelope.
///
/// Validation uses character count (`.chars().count()`) so multi-byte
/// Unicode names remain bounded by schema semantics.
///
/// # Errors
///
/// - [`ProjectRenameError::NameEmpty`] when `args.name` is empty.
/// - [`ProjectRenameError::NameTooLong`] when `args.name` exceeds
///   [`PROJECT_NAME_MAX`] UTF-8 characters.
pub fn compute_patch(
    _prior: &Project,
    args: &ProjectRenameArgs,
) -> Result<(Value, String, Vec<Value>), ProjectRenameError> {
    let char_count = args.name.chars().count();
    if char_count < PROJECT_NAME_MIN {
        return Err(ProjectRenameError::NameEmpty);
    }
    if char_count > PROJECT_NAME_MAX {
        return Err(ProjectRenameError::NameTooLong {
            actual: char_count,
            max: PROJECT_NAME_MAX,
        });
    }

    let patch = json!([{
        "op": "replace",
        "path": "/name",
        "value": args.name.clone(),
    }]);

    Ok((patch, args.name.clone(), Vec::new()))
}

/// Build the verb `data` envelope from `(args, post_state)`.
#[must_use]
pub fn data_envelope(args: &ProjectRenameArgs, post_state: &Project) -> ProjectRenameData {
    ProjectRenameData {
        project_id: args.project_id,
        name: post_state.name.clone(),
    }
}

/// Funnel [`ProjectRenameError`] into verb-layer [`VerbError`]. Both
/// variants are schema-validation failures in this slice.
impl From<ProjectRenameError> for VerbError {
    fn from(value: ProjectRenameError) -> Self {
        match value {
            ProjectRenameError::NameEmpty | ProjectRenameError::NameTooLong { .. } => {
                VerbError::BadArgs {
                    detail: value.to_string(),
                }
            }
        }
    }
}

/// The §0.8 verb for `project.rename`.
#[derive(Debug, Default)]
pub struct ProjectRenameVerb;

impl Verb for ProjectRenameVerb {
    fn verb(&self) -> &'static str {
        "project.rename"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: ProjectRenameArgs =
            serde_json::from_value(args.clone()).map_err(|e| VerbError::BadArgs {
                detail: format!("project.rename: args deserialize failed: {e}"),
            })?;

        let (patch_value, new_name, warnings) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch = serde_json::from_value(patch_value).map_err(|e| {
            VerbError::Custom(format!("project.rename: patch construction failed: {e}"))
        })?;

        let mut post_state = prior.clone();
        post_state.name = new_name;
        let envelope = data_envelope(&typed, &post_state);
        let data = serde_json::to_value(&envelope).map_err(|e| {
            VerbError::Custom(format!(
                "project.rename: data envelope serialize failed: {e}"
            ))
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
        let typed: ProjectRenameArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "ProjectRenameArgs",
            })?;

        let envelope = data_envelope(&typed, post_state);
        serde_json::to_value(&envelope).map_err(|e| ReconstructError::Custom(e.to_string()))
    }
}
