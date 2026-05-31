//! `project.list` (§2.6) — eighty-first production verb in the engine.
//!
//! ## Spec quote (`spec/commands/project.md` §2.6, abbreviated)
//!
//! > CLI: `verbreel project list`
//! > MCP: `project.list`
//! > Args: none
//! > Returns (`data`): `{ projects: { id: string; name: string;
//! >   path: string; state: "open"|"closed";
//! >   last_opened_at: string }[] }`
//!
//! ## Pure verb path vs live engine read.
//!
//! Per §2.6, the source of truth is `~/.verbreel/projects-index` — a
//! user-scoped JSON object keyed by `project_id`, with `flock`'d
//! read-modify-write semantics shared across every Verbreel engine
//! instance on the host. The `Verb` trait's purity contract forbids
//! file I/O in `compute_patch`, so the registry verb here always
//! reports an **empty** list — that empty result is the deterministic
//! replay/reconstruct contract the conformance fixtures pin.
//!
//! The **live** `project.list` data is produced by the engine's
//! `Engine::handle_project_list`, which has the `home` + open-map this
//! pure path cannot see: it reads the index, prunes stale entries
//! (emitting `W_INDEX_STALE`), and marks each project `open`/`closed`.
//! Keeping the index read in the engine (not the verb) honors the
//! purity contract without a full `VerbContext` refactor.
//!
//! ## Bundle metadata, not project state.
//!
//! `project.list` is read-only and does not read or mutate project
//! state. The spec lists no args; the `Verb` trait's dispatcher always
//! carries a `project_id`, so the verb accepts `project_id` for shape
//! compatibility and ignores it in v1 (same accommodation pattern as
//! `render.queue.list`, `stock.list_providers`, `font.list`, and
//! `list_capabilities`).
//!
//! ## Long-running-engine vs one-shot-CLI semantics collapse at v1.
//!
//! Per §2.6, a long-running engine (MCP server, HTTP daemon) reports
//! currently-open projects with `state: "open"` plus index-discovered
//! projects with `state: "closed"`; a one-shot CLI invocation reports
//! every index entry as `state: "closed"`. v1 returns no entries, so
//! both surfaces collapse to the same empty output and the
//! `ProjectListState` discriminator is exercised only by the
//! lowercase-serialization tests.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::ProjectId;

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// State of a single project entry — the closed set from §2.6.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProjectListState {
    /// The project is currently open in this engine instance.
    Open,
    /// The project is known via `~/.verbreel/projects-index` but not
    /// currently held open by this engine instance.
    Closed,
}

/// Single project entry returned by `project.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectEntry {
    /// `UUIDv7` project id.
    pub id: String,
    /// Project display name.
    pub name: String,
    /// Absolute path to the project folder.
    pub path: String,
    /// Whether this engine holds the project open.
    pub state: ProjectListState,
    /// RFC 3339 timestamp the project was last opened (from the index).
    pub last_opened_at: String,
}

/// Arguments for `project.list`.
///
/// `project_id` is required by the `Verb` trait shape; the spec
/// nominally accepts no args, but the engine's verb dispatcher always
/// carries a `project_id` per §0.8, so we accept it and ignore it in
/// v1.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectListArgs {
    /// Required by the `Verb` trait shape; not read by the v1 impl.
    pub project_id: ProjectId,
}

/// Envelope returned by `project.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectListData {
    /// Project entries discoverable via `~/.verbreel/projects-index`
    /// plus any open in this engine instance. Empty in v1.
    pub projects: Vec<ProjectEntry>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Error)]
/// Verb-level error type for `project.list`.
pub enum ProjectListError {
    /// No verb-level runtime errors in v1 (empty list always succeeds).
    #[error("project.list: unreachable (no error variants)")]
    Unreachable,
}

/// Build the canonical `project.list` data envelope.
fn build_data() -> ProjectListData {
    ProjectListData {
        projects: Vec::new(),
    }
}

/// Build the RFC 6902 patch for `project.list`.
///
/// # Errors
///
/// No runtime errors are produced by this verb; the returned `Result`
/// exists for parity with the broader compute-patch API.
pub fn compute_patch(
    _prior: &Project,
    _args: &ProjectListArgs,
) -> Result<(Value, Vec<Value>, ProjectListData), ProjectListError> {
    Ok((json!([]), Vec::new(), build_data()))
}

/// Build the data envelope from `(args, post_state)`.
///
/// # Errors
///
/// Reuses [`compute_patch`], so this can only return reconstruction
/// errors introduced while rebuilding the deterministic envelope.
pub fn data_envelope_from_args(
    args: &ProjectListArgs,
    post_state: &Project,
) -> Result<ProjectListData, ReconstructError> {
    let (_, _, data) =
        compute_patch(post_state, args).map_err(|e| ReconstructError::Custom(e.to_string()))?;
    Ok(data)
}

impl From<ProjectListError> for VerbError {
    fn from(value: ProjectListError) -> Self {
        match value {
            ProjectListError::Unreachable => VerbError::BadArgs {
                detail: value.to_string(),
            },
        }
    }
}

/// The §0.8 verb for `project.list`.
#[derive(Debug, Default)]
pub struct ProjectListVerb;

impl Verb for ProjectListVerb {
    fn verb(&self) -> &'static str {
        "project.list"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: ProjectListArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("project.list: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch = serde_json::from_value(patch_value).map_err(|err| {
            VerbError::Custom(format!("project.list: patch construction failed: {err}"))
        })?;

        let data = serde_json::to_value(&data).map_err(|err| {
            VerbError::Custom(format!("project.list: data envelope failed: {err}"))
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
        let typed: ProjectListArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "ProjectListArgs",
            })?;

        let envelope = data_envelope_from_args(&typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
