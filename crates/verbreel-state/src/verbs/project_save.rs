//! `project.save` (§2.3) — eightieth production verb in the engine.
//! Third slice of the project arc that touches storage.
//!
//! ## Spec quote (`spec/commands/project.md` §2.3, abbreviated)
//!
//! > Persists the in-memory project to `project.json`. Atomic: writes to
//! > `project.json.tmp` then renames. Also updates
//! > `Project.last_saved_event_id` (set to the most recent applied
//! > event's `event_id`, or kept at `null` for a freshly-created project
//! > with no mutations yet) before the write, so a subsequent
//! > `project.open` knows where the snapshot's effective state ended.
//! >
//! > `project.save` is read-only with respect to `events.jsonl`: the
//! > verb returns `patch: []`, `event_id: ""`, and writes nothing to the
//! > event log.
//! >
//! > CLI: `verbreel project save [--project <id>]`
//! > MCP: `project.save`
//! > Args: `project_id: string`.
//! > Returns (`data`): `{ path: string; bytes_written: integer }`.
//! > Errors: `E_IO`.
//!
//! ## Why this verb is NOT a [`Verb`](crate::reconstructor::Verb) trait impl
//!
//! The [`crate::reconstructor::Verb`] trait requires a pure
//! `compute_patch(&Project, &args) → (Patch, data, warnings)`. It cannot
//! reach the [`ProjectStore`] that owns the on-disk root and the
//! `last_applied_event_id` bookkeeping, both of which `project.save`
//! must touch. The atomic-rename + parent-dir fsync also live outside
//! the `Verb` contract because they are an `&mut ProjectStore`-only
//! operation: re-routing the call through the registry would force a
//! synthetic patch that does not exist (the spec says the patch is `[]`
//! and no event line is written).
//!
//! This mirrors the design from `project.create` (§2.1) and
//! `project.open` (§2.2): a free function over `&mut ProjectStore` plus
//! `&ProjectSaveArgs` that returns a typed envelope. The registry stays
//! responsible for the event-emitting verbs only.
//!
//! ## Scope (this slice)
//!
//! - Arg parsing + validation (`project_id` matches the loaded
//!   project's id).
//! - Delegation to [`crate::lifecycle::ProjectStore::save`] for the
//!   atomic `tempfile + rename + parent-dir fsync` write per §2.3.
//! - Error remap from [`LifecycleError`] onto the spec-aligned
//!   [`ProjectSaveError`] variants.
//!
//! ## Universal-implicit `E_PROJECT_NOT_FOUND`
//!
//! `spec/commands/project.md` §2.3 lists only `E_IO` under Errors, but
//! the verb takes `project_id` as a required arg. Per
//! `spec/commands/conventions.md` §0.12 "Universal-implicit error" rule
//! (`appendix-a-errors.md` line 23), every verb that declares
//! `project_id` can return `E_PROJECT_NOT_FOUND` when the supplied UUID
//! does not name an open project — and per the conventions escape
//! clause, a per-verb list that omits an arg-shape-reachable code is a
//! documentation bug, not a behavioral exception. The verb-layer
//! surfaces [`ProjectSaveError::ProjectNotFound`] when the caller
//! supplies a `project_id` that does not match the store's loaded
//! project id.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use verbreel_types::ProjectId;

use crate::lifecycle::{LifecycleError, ProjectStore};

/// Arguments for [`save`].
//
// `deny_unknown_fields` enforces the §2.3 published surface: stray
// MCP/HTTP payload keys surface as arg-shape errors rather than being
// silently accepted.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectSaveArgs {
    /// Target project id. Must match the store's loaded project id; a
    /// mismatch surfaces as [`ProjectSaveError::ProjectNotFound`] per
    /// the universal-implicit error rule.
    pub project_id: ProjectId,
}

/// Envelope returned by a successful [`save`].
///
/// Mirrors the §2.3 `data` shape exactly: `path` is the on-disk
/// `project.json` absolute path, `bytes_written` is the serialized
/// snapshot's byte length.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectSaveData {
    /// Absolute path of the written `project.json`.
    pub path: PathBuf,

    /// Number of bytes written to `project.json` (the serialized
    /// `Project` JSON).
    pub bytes_written: u64,
}

/// Errors returned by [`save`].
#[derive(Debug, thiserror::Error)]
pub enum ProjectSaveError {
    /// The supplied `project_id` does not match the store's loaded
    /// project id. Maps to `E_PROJECT_NOT_FOUND` per the
    /// universal-implicit rule (§0.12 / `appendix-a-errors.md`).
    #[error(
        "project.save: project not found (args.project_id={requested}, \
         store.project_id={loaded})"
    )]
    ProjectNotFound {
        /// The id the caller passed in `args.project_id`.
        requested: ProjectId,
        /// The id of the project the store actually has loaded.
        loaded: ProjectId,
    },

    /// Filesystem failure during the atomic write (temp file create,
    /// serialize, write, fsync, rename, or parent-dir fsync). Maps to
    /// `E_IO`.
    #[error("project.save: I/O failure: {0}")]
    Io(#[from] std::io::Error),

    /// Underlying [`ProjectStore::save`] failed for a reason that does
    /// not map to one of the spec-aligned variants above (e.g. an
    /// internal serialize error that hit the
    /// [`std::io::ErrorKind::InvalidData`] branch). Maps to `E_IO` at
    /// the envelope layer but is preserved as a distinct variant so
    /// callers can match on it.
    #[error("project.save: lifecycle failure: {0}")]
    LifecycleFailed(#[from] LifecycleError),
}

/// Persist the store's in-memory project to `<root>/project.json` per
/// §2.3.
///
/// Validates `args.project_id` against `store.project().id`, then
/// delegates to [`ProjectStore::save`] for the atomic
/// `tempfile + rename + parent-dir fsync` write. The store's
/// `last_saved_event_id` is bumped to `last_applied_event_id` (or kept
/// at `None` for a freshly-created project with no mutations) by the
/// lifecycle layer before the write — this verb does not duplicate that
/// bookkeeping.
///
/// # Errors
///
/// - [`ProjectSaveError::ProjectNotFound`] — `args.project_id` does not
///   match the store's loaded project id (maps to `E_PROJECT_NOT_FOUND`
///   per the universal-implicit rule).
/// - [`ProjectSaveError::Io`] — filesystem failure during the atomic
///   write (maps to `E_IO`).
/// - [`ProjectSaveError::LifecycleFailed`] — any non-IO failure from
///   [`ProjectStore::save`] (maps to `E_IO` at the envelope layer).
pub fn save(
    store: &mut ProjectStore,
    args: &ProjectSaveArgs,
) -> Result<ProjectSaveData, ProjectSaveError> {
    let loaded = store.project().id;
    if args.project_id != loaded {
        return Err(ProjectSaveError::ProjectNotFound {
            requested: args.project_id,
            loaded,
        });
    }

    let info = store.save().map_err(|e| match e {
        LifecycleError::Io(io) => ProjectSaveError::Io(io),
        other => ProjectSaveError::LifecycleFailed(other),
    })?;

    Ok(ProjectSaveData {
        path: info.path,
        bytes_written: info.bytes_written,
    })
}
