//! `project.close` (§2.5) — eighty-first production verb in the engine.
//! Fourth slice of the project arc that touches storage.
//!
//! ## Spec quote (`spec/commands/project.md` §2.5, abbreviated)
//!
//! > Releases the project from memory and drops its file lock. Does not
//! > delete anything on disk.
//! >
//! > `save: true` is applied **before** the busy-check — a save against
//! > the in-memory snapshot is safe even while a render is reading the
//! > same graph.
//! >
//! > CLI: `verbreel project close [--project <id>] [--save] [--cancel_jobs]`
//! > MCP: `project.close`
//! > Args: `project_id: string`, `save?: boolean` (default `false`),
//! > `cancel_jobs?: boolean` (default `false`).
//! > Returns (`data`): `{ closed: true; canceled_job_ids: string[];
//! > cancel_failures: [] }`.
//! > Errors: `E_BUSY` (active jobs without `cancel_jobs`; or
//! > `cancel_jobs` with one or more cancel failures), `E_IO` (save
//! > failure).
//!
//! ## Why this verb is NOT a [`Verb`](crate::reconstructor::Verb) trait impl
//!
//! Mirrors `project.save` (§2.3) and `project.open` (§2.2): the
//! [`crate::reconstructor::Verb`] trait requires a pure
//! `compute_patch(&Project, &args) → (Patch, data, warnings)` that
//! cannot reach the [`ProjectStore`] that owns the on-disk root and the
//! file lock. `project.close` must consume the store by value so the
//! `Drop` impl on the underlying `NativeBackend` releases the
//! `fs4::flock` on `events.jsonl` — this is an `&mut ProjectStore`-or-
//! by-value operation that does not fit the trait contract. The verb is
//! also read-only with respect to `events.jsonl` (`patch: []`, no event
//! line written), so routing it through the event-emitting registry
//! would require a synthetic patch the spec forbids.
//!
//! This matches the design from `project.create` (§2.1), `project.open`
//! (§2.2), and `project.save` (§2.3): a free function that consumes
//! `ProjectStore` plus `&ProjectCloseArgs` and returns a typed
//! envelope. The registry stays responsible for the event-emitting
//! verbs only.
//!
//! ## Scope (this slice — v1 floor)
//!
//! - Arg parsing + validation (`project_id` matches the loaded
//!   project's id).
//! - Optional pre-close save (`save: true`) via [`super::project_save`]
//!   delegating to [`ProjectStore::save`] for the atomic
//!   `tempfile + rename + parent-dir fsync` write per §2.3.
//! - Consume `store` by value so the `Drop` impl releases the flock.
//! - Returns `{closed: true, canceled_job_ids: [], cancel_failures: []}`
//!   — both arrays empty in v1 because no long-running job verbs exist
//!   yet.
//!
//! ## Why `canceled_job_ids` and `cancel_failures` are empty in v1
//!
//! The spec's busy-check fires only when a `render.start`,
//! `caption.auto_generate`, or `caption.translate` job is `queued` or
//! `running` against this project at call time. None of those verbs
//! exist in this engine build yet — the render queue is still a stub
//! (§21.x verbs operate against an empty queue, `caption.auto_generate`
//! / `caption.translate` are unshipped). So there are no active jobs to
//! reject against, no jobs to cancel, no cancel failures to report.
//!
//! When the long-running job verbs land, this verb upgrades to:
//!
//! 1. Iterate the project's active jobs (filtered by `project_id`).
//! 2. If non-empty and `cancel_jobs == false` → return `E_BUSY` with
//!    `details.active_jobs`.
//! 3. If `cancel_jobs == true` → enter "draining" state, run each
//!    job's verb-specific cancel path serially, collect terminal-state
//!    job ids into `canceled_job_ids` and any failures into
//!    `cancel_failures`. If any failure, return `E_BUSY` and retain the
//!    flock; otherwise proceed to close.
//!
//! Until then both arrays are unconditionally empty on the Ok path.
//!
//! ## Universal-implicit `E_PROJECT_NOT_FOUND`
//!
//! `spec/commands/project.md` §2.5 lists `E_BUSY` and `E_IO` under
//! Errors but the verb takes `project_id` as a required arg. Per
//! `spec/commands/conventions.md` §0.12 "Universal-implicit error" rule
//! (`appendix-a-errors.md` line 23), every verb that declares
//! `project_id` can return `E_PROJECT_NOT_FOUND` when the supplied UUID
//! does not name an open project — a per-verb list that omits an
//! arg-shape-reachable code is a documentation bug, not a behavioral
//! exception. The verb-layer surfaces
//! [`ProjectCloseError::ProjectNotFound`] when the caller supplies a
//! `project_id` that does not match the store's loaded project id.

use serde::{Deserialize, Serialize};
use verbreel_types::ProjectId;

use crate::lifecycle::{LifecycleError, ProjectStore};
use crate::verbs::project_save::{self, ProjectSaveArgs, ProjectSaveError};

/// Arguments for [`close`].
//
// `deny_unknown_fields` enforces the §2.5 published surface: stray
// MCP/HTTP payload keys surface as arg-shape errors rather than being
// silently accepted.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectCloseArgs {
    /// Target project id. Must match the store's loaded project id; a
    /// mismatch surfaces as [`ProjectCloseError::ProjectNotFound`] per
    /// the universal-implicit error rule.
    pub project_id: ProjectId,

    /// When `true`, persist the in-memory snapshot via [`super::project_save`]
    /// before releasing the project. Default `false`.
    ///
    /// The save runs **before** the busy-check per §2.5 — a save
    /// against the in-memory snapshot is safe even while a render is
    /// reading the same graph.
    #[serde(default)]
    pub save: bool,

    /// When `true`, cancel any active long-running jobs against this
    /// project before releasing it. Default `false`. **v1 floor**: no
    /// long-running job verbs exist yet, so this flag is accepted but
    /// has no observable effect — the v1 cancel pass always returns an
    /// empty `canceled_job_ids` list. See module docs for the upgrade
    /// path.
    #[serde(default)]
    pub cancel_jobs: bool,
}

/// Envelope returned by a successful [`close`].
///
/// Mirrors the §2.5 `data` shape exactly. `closed` is always `true` on
/// the Ok path (the `Err` path surfaces the failure to close).
/// `canceled_job_ids` and `cancel_failures` are always empty in v1
/// because no long-running job verbs exist yet; the field shape is
/// retained so the upgrade to real cancel logic is non-breaking.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectCloseData {
    /// Always `true` on the Ok path — the project was released and its
    /// flock dropped. Encoded as a constant field rather than a unit
    /// type so the JSON shape matches the spec verbatim.
    pub closed: bool,

    /// Ids of long-running jobs cancelled during this close call. v1:
    /// always empty (no long-running job verbs exist yet). When
    /// `render.start` / `caption.auto_generate` / `caption.translate`
    /// land, this will list every job whose cancel returned a terminal
    /// state.
    pub canceled_job_ids: Vec<String>,

    /// Per §2.5: "`cancel_failures` is always empty on the Ok path;
    /// failures land in `details.cancel_failures` on the Err path." v1:
    /// always empty for the same reason as [`Self::canceled_job_ids`].
    /// Typed as `Vec<()>` because no failure shape is reachable yet —
    /// the upgrade slice will replace `()` with a `CancelFailure`
    /// struct.
    pub cancel_failures: Vec<()>,
}

/// Errors returned by [`close`].
#[derive(Debug, thiserror::Error)]
pub enum ProjectCloseError {
    /// The supplied `project_id` does not match the store's loaded
    /// project id. Maps to `E_PROJECT_NOT_FOUND` per the
    /// universal-implicit rule (§0.12 / `appendix-a-errors.md`).
    #[error(
        "project.close: project not found (args.project_id={requested}, \
         store.project_id={loaded})"
    )]
    ProjectNotFound {
        /// The id the caller passed in `args.project_id`.
        requested: ProjectId,
        /// The id of the project the store actually has loaded.
        loaded: ProjectId,
    },

    /// Pre-close `save: true` raised an `E_IO`-class failure during the
    /// atomic write. Surfaces the inner `std::io::Error` so callers can
    /// inspect the underlying syscall failure. Maps to `E_IO`.
    #[error("project.close: pre-close save failed (I/O): {0}")]
    SaveIo(std::io::Error),

    /// Pre-close `save: true` raised a non-IO lifecycle failure (e.g.
    /// the [`std::io::ErrorKind::InvalidData`] branch of the serialize
    /// step). Maps to `E_IO` at the envelope layer but is preserved as
    /// a distinct variant so callers can match on it.
    #[error("project.close: pre-close save lifecycle failure: {0}")]
    SaveLifecycle(#[from] LifecycleError),
}

impl From<ProjectSaveError> for ProjectCloseError {
    fn from(err: ProjectSaveError) -> Self {
        match err {
            ProjectSaveError::ProjectNotFound { requested, loaded } => {
                // Can't actually be reached on the close path because
                // close validates the id before delegating to save —
                // but keep the mapping non-lossy for future-proofing.
                ProjectCloseError::ProjectNotFound { requested, loaded }
            }
            ProjectSaveError::Io(io) => ProjectCloseError::SaveIo(io),
            ProjectSaveError::LifecycleFailed(other) => ProjectCloseError::SaveLifecycle(other),
        }
    }
}

/// Close the project and release its file lock per §2.5.
///
/// Validates `args.project_id` against `store.project().id`, then:
///
/// 1. If `args.save` is `true`, runs the §2.3 atomic save before
///    proceeding. A save failure aborts the close — the project is
///    **not** released, the flock is retained, and the caller can
///    retry after addressing the IO issue.
/// 2. **v1 floor**: skips the busy-check / cancel pass because no
///    long-running job verbs exist yet. The spec's `cancel_jobs` arg
///    is accepted to keep the surface stable, but has no observable
///    effect.
/// 3. On `Ok`, the function consumes `store` and drops it. The `Drop`
///    impl on the inner `NativeBackend` releases the `fs4::flock` on
///    `events.jsonl`. On `Err`, the store is returned to the caller
///    inside the error tuple so the flock is **retained** — matching
///    the §2.5 requirement that a close failure leaves the project
///    open and re-tryable.
///
/// The returned envelope carries `closed: true` and empty `canceled_job_ids`
/// / `cancel_failures` arrays per §2.5 v1 semantics.
///
/// # Errors
///
/// Returns `(store, err)` on failure so the caller can retry without
/// re-opening the project — the flock is still held by `store`.
///
/// - [`ProjectCloseError::ProjectNotFound`] — `args.project_id` does
///   not match the store's loaded project id (maps to
///   `E_PROJECT_NOT_FOUND` per the universal-implicit rule).
/// - [`ProjectCloseError::SaveIo`] — `args.save == true` and the
///   atomic write raised a filesystem failure (maps to `E_IO`).
/// - [`ProjectCloseError::SaveLifecycle`] — `args.save == true` and
///   the lifecycle layer raised a non-IO failure (maps to `E_IO` at
///   the envelope layer).
// The Err carries `ProjectStore` — the live file lock + project
// snapshot — so the caller can retry without re-opening the project.
// Boxing would add an allocation for no ergonomic gain since every
// caller destructures the tuple immediately; the size signal is the
// type's contract, not a smell.
#[allow(clippy::result_large_err)]
pub fn close(
    mut store: ProjectStore,
    args: &ProjectCloseArgs,
) -> Result<ProjectCloseData, (ProjectStore, ProjectCloseError)> {
    let loaded = store.project().id;
    if args.project_id != loaded {
        return Err((
            store,
            ProjectCloseError::ProjectNotFound {
                requested: args.project_id,
                loaded,
            },
        ));
    }

    if args.save {
        let save_args = ProjectSaveArgs {
            project_id: args.project_id,
        };
        // Per §2.5: a save failure aborts the close. Return the store
        // back to the caller inside the error tuple so its flock
        // stays held — a `?` here would drop the store and release
        // the flock prematurely, defeating the retry contract.
        if let Err(e) = project_save::save(&mut store, &save_args) {
            return Err((store, ProjectCloseError::from(e)));
        }
    }

    // v1 floor: no long-running jobs exist. `args.cancel_jobs` is
    // accepted at the arg surface but has no observable effect; the
    // busy-check and cancel sequence land in a follow-up slice when
    // render.start / caption.auto_generate / caption.translate ship.

    // Drop the store. The `NativeBackend` inside drops its file handle,
    // which releases the `fs4::flock` on `<root>/.verbreel/events.jsonl`
    // — a subsequent `ProjectStore::open` against the same root
    // succeeds without waiting.
    drop(store);

    Ok(ProjectCloseData {
        closed: true,
        canceled_job_ids: Vec::new(),
        cancel_failures: Vec::new(),
    })
}
