//! `project.open` (§2.2) — eighty-fifth production verb in the engine.
//! Fifth slice of the project arc; loads an existing on-disk project
//! into the engine.
//!
//! ## Spec quote (`spec/commands/project.md` §2.2, abbreviated)
//!
//! > Loads a project from disk into the engine and acquires its file
//! > lock.
//! >
//! > Acquire `flock(LOCK_EX)` on `.verbreel/lock`. Read `project.json`
//! > as the snapshot. Validate against `project-schema.json`; run
//! > normalization + reconciliation. Read `events.jsonl` and replay
//! > events past `last_saved_event_id`. Run asset integrity check.
//! >
//! > `unverified_asset_ids` lists assets that failed the fast
//! > fingerprint check; they remain in the project but the engine
//! > refuses to decode them until `asset.relink` or `--strict`
//! > reconciles them.
//! >
//! > CLI: `verbreel project open --path <path> [--strict] [--activate]`
//! > MCP: `project.open`
//! > Args: `path: string`, `strict?: boolean` (default `false`).
//! > Returns (`data`): `{ project_id: string; project: Project;
//! >                       unverified_asset_ids: string[] }`.
//! > Errors: `E_PROJECT_NOT_FOUND`, `E_SCHEMA_VIOLATION`,
//! > `E_SCHEMA_VERSION_UNSUPPORTED`, `E_PROJECT_LOCKED`, `E_IO`.
//!
//! ## Why this verb is NOT a [`Verb`](crate::reconstructor::Verb) trait impl
//!
//! Mirrors `project.create` (§2.1), `project.save` (§2.3), and
//! `project.close` (§2.5): the [`crate::reconstructor::Verb`] trait
//! requires a pure `compute_patch(&Project, &args) → (Patch, data,
//! warnings)` that takes a `prior: &Project`. `project.open` has no
//! prior project at call time — the project graph IS the output of
//! the call, not the input. The trait contract cannot model that.
//! `project.open` also mints the [`ProjectStore`] handle that all
//! subsequent verb calls flow through, which is an owned-value return
//! the `Verb` trait does not expose.
//!
//! Free function, same shape as the three siblings shipped earlier in
//! the session.
//!
//! ## Scope (this slice — v1 floor)
//!
//! - Arg parsing + validation (`path` is non-empty, resolved to
//!   absolute if relative).
//! - Pre-flight `fs::metadata` on `path` to distinguish
//!   [`ProjectOpenError::ProjectNotFound`] (path absent or not a
//!   directory) from [`ProjectOpenError::Io`] (other IO failures).
//!   Per the prior iter-2 review on the lost branch, `Path::is_file`
//!   /  `Path::is_dir` swallow non-`NotFound` IO errors (permission
//!   denied, etc.) and would misreport them as `E_PROJECT_NOT_FOUND`;
//!   this verb matches on [`std::io::ErrorKind::NotFound`] explicitly.
//! - Delegate to [`ProjectStore::open_with_registry`] for the §2.2
//!   6-step load workflow: flock acquire, snapshot parse, events.jsonl
//!   replay past `last_saved_event_id`, idempotency-index rebuild. The
//!   lifecycle primitive already implements the spec algorithm.
//! - Run the fast fingerprint integrity check on each tracked asset:
//!   `stat()` the on-disk file at `<root>/<asset.path>`, compare its
//!   mtime + size against the recorded [`crate::FileFingerprint`].
//!   Mismatch → asset id collected into `unverified_asset_ids`. Missing
//!   file → also unverified (the asset is intact in the project graph,
//!   it just can't be decoded until `asset.relink`).
//! - Return the §2.2 envelope `{project_id, project, unverified_asset_ids}`
//!   alongside the live [`ProjectStore`] so the caller retains the
//!   flock and can route subsequent verbs through
//!   [`ProjectStore::mutate_via_verb`].
//!
//! ## Deferred (out of v1 scope)
//!
//! - **`--strict` re-hashes**: §2.2 spec says strict mode re-hashes
//!   every asset file. v1 floor accepts the `strict` flag at the arg
//!   surface but performs only the fast fingerprint check regardless.
//!   The follow-up slice that wires `asset.verify`'s strict path will
//!   activate the re-hash here too.
//! - **§0.13 reconciliation passes** (`W_KEYFRAMES_ORPHANED`,
//!   `W_TRACKS_REORDERED`, `W_LINK_GROUP_CLEARED_ON_LOCKED`): the
//!   lifecycle layer's `open_with_registry` does NOT yet emit these
//!   warnings; they land when the matching reconciliation invariants
//!   land. The verb returns `warnings: vec![]` until then.
//! - **`--activate` write of `~/.verbreel/active-project`**: CLI-front-
//!   end concern, not engine. Out of scope here.
//! - **`~/.verbreel/projects-index` registration**: same deferral as
//!   `project.create` / `project.duplicate`.
//!
//! ## Universal-implicit errors
//!
//! `spec/commands/project.md` §2.2 lists `E_PROJECT_NOT_FOUND`,
//! `E_SCHEMA_VIOLATION`, `E_SCHEMA_VERSION_UNSUPPORTED`,
//! `E_PROJECT_LOCKED`, and `E_IO`. The verb surfaces all five as
//! discrete typed variants.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use verbreel_types::ProjectId;

use crate::asset::Asset;
use crate::asset_meta::FileFingerprint;
use crate::lifecycle::{LifecycleError, ProjectStore};
use crate::project::Project;

/// Arguments for [`open`].
///
/// `deny_unknown_fields` enforces the §2.2 published surface: stray
/// MCP/HTTP payload keys surface as arg-shape errors rather than being
/// silently accepted.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectOpenArgs {
    /// Project root directory on disk. May be relative — when so, it
    /// is resolved against the caller's current working directory.
    /// Must point at a directory containing `project.json`.
    pub path: PathBuf,

    /// Strict-mode flag per §2.2. When `true`, the spec'd verb
    /// re-hashes every asset's bytes; when `false`, fast mode
    /// (mtime + size stat). **v1 floor**: accepted at the surface but
    /// has no observable effect — the verb always runs the fast check.
    /// See module docs for the upgrade path.
    #[serde(default)]
    pub strict: bool,
}

/// Envelope returned by a successful [`open`].
///
/// Mirrors the §2.2 `data` shape exactly. The caller gets back both
/// the typed envelope and (separately, as the first tuple element from
/// [`open`]) the live [`ProjectStore`] handle that owns the flock.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectOpenData {
    /// The loaded project's id, echoed at the envelope top level per
    /// §2.2 for convenience (it is also reachable through
    /// `project.id`).
    pub project_id: ProjectId,

    /// The in-memory [`Project`] graph after `project.json` parse +
    /// `events.jsonl` replay.
    pub project: Project,

    /// Asset ids whose on-disk file fingerprint did not match the
    /// recorded `mtime_ms` + `size_bytes`, or whose file is missing
    /// entirely. The assets remain in the project graph; the engine
    /// refuses to decode them until `asset.relink` reconciles them.
    pub unverified_asset_ids: Vec<String>,
}

/// Errors returned by [`open`].
#[derive(Debug, thiserror::Error)]
pub enum ProjectOpenError {
    /// The supplied path does not exist or is not a directory carrying
    /// `project.json`. Maps to `E_PROJECT_NOT_FOUND`.
    ///
    /// Reachable in three ways: the path itself is absent
    /// ([`std::io::ErrorKind::NotFound`] on stat); the path is a
    /// regular file rather than a directory; the path is a directory
    /// but does not contain `project.json` (the lifecycle layer
    /// surfaces this as [`LifecycleError::NoProjectJson`]).
    #[error("project.open: project not found at {path}")]
    ProjectNotFound {
        /// The path that was probed.
        path: PathBuf,
    },

    /// `project.json` did not deserialize against the project schema.
    /// Maps to `E_SCHEMA_VIOLATION`.
    #[error("project.open: schema violation: {detail}")]
    SchemaViolation {
        /// Underlying serde / validation error message.
        detail: String,
    },

    /// `Project.schema_version` does not match the engine's supported
    /// version. Maps to `E_SCHEMA_VERSION_UNSUPPORTED` — distinct from
    /// `E_SCHEMA_VIOLATION` because a version mismatch is a future
    /// migration concern, not a structural error.
    #[error("project.open: schema_version {found} unsupported (engine supports {expected})")]
    SchemaVersionUnsupported {
        /// The `schema_version` string read from `project.json`.
        found: String,
        /// The `schema_version` this engine build supports.
        expected: String,
    },

    /// Another process holds the `.verbreel/events.jsonl` flock. Maps
    /// to `E_PROJECT_LOCKED`.
    #[error("project.open: project locked at {path}")]
    ProjectLocked {
        /// The project root whose lock is held.
        path: PathBuf,
    },

    /// Filesystem failure during open (stat, read, fsync). Maps to
    /// `E_IO`. Reachable for permission-denied, broken symlinks,
    /// transient `EAGAIN`, etc. — every IO failure that is NOT a clean
    /// `NotFound`.
    #[error("project.open: I/O failure: {0}")]
    Io(#[from] std::io::Error),

    /// Underlying [`ProjectStore::open_with_registry`] failed for a
    /// reason that does not map to one of the spec-aligned variants
    /// above (reconstructor-gate failure, replay apply error, torn-
    /// line recovery returning bytes that don't parse, etc.). Maps to
    /// `E_IO` at the envelope layer but is preserved as a distinct
    /// variant so callers can match on it.
    #[error("project.open: lifecycle failure: {0}")]
    LifecycleFailed(#[from] LifecycleError),
}

/// Open an existing project from disk per §2.2.
///
/// Algorithm:
/// 1. Resolve `args.path` to an absolute path (`std::env::current_dir`
///    join when relative).
/// 2. `fs::metadata(&path)` — distinguishing `NotFound` (→
///    [`ProjectOpenError::ProjectNotFound`]) from other IO errors (→
///    [`ProjectOpenError::Io`]). `Path::is_dir` / `Path::is_file`
///    would silently swallow permission-denied / EACCES into a "not a
///    directory" false negative — the explicit `match` on
///    [`std::io::ErrorKind::NotFound`] is load-bearing per the prior
///    iter-2 review on the lost `feat/project-open` branch.
/// 3. Refuse if the path is not a directory
///    ([`ProjectOpenError::ProjectNotFound`]).
/// 4. Delegate to [`ProjectStore::open_with_registry`] with the
///    engine's `default_registry` + `default_fixtures` so the §0.8
///    reconstructor-purity gate passes on the loaded store. The
///    primitive runs §2.2 steps 1-6: snapshot parse, flock acquire,
///    events.jsonl read + torn-line recovery, replay past
///    `last_saved_event_id`, idempotency-index rebuild.
/// 5. Run the fast fingerprint integrity check on each tracked
///    asset. `<root>/<asset.path>` is `stat`'d; mismatched mtime/size
///    or missing file → asset id added to `unverified_asset_ids`.
/// 6. Return the live store + the §2.2 envelope.
///
/// # Errors
///
/// See variants of [`ProjectOpenError`].
//
// The Ok branch carries `ProjectStore`, which holds the file lock +
// the in-memory project. Boxing would add an allocation for no
// ergonomic gain since every caller destructures the tuple
// immediately; the size signal is the type's contract, not a smell.
// Same precedent as `project_close.rs`'s `Result<_, (ProjectStore, _)>`.
#[allow(clippy::result_large_err)]
pub fn open(args: &ProjectOpenArgs) -> Result<(ProjectStore, ProjectOpenData), ProjectOpenError> {
    // Step 1: resolve to absolute. Relative paths join against the
    // caller's cwd — std semantics, surfaces an IO error if cwd is
    // unreadable (rare; usually fatal at process startup).
    let path = resolve_path(&args.path)?;

    // Step 2: pre-flight existence check via `fs::metadata`. The
    // explicit `NotFound` match is load-bearing — `Path::is_dir`
    // returns `false` on permission-denied which would surface as a
    // misleading `ProjectNotFound` instead of the true `Io` cause.
    let meta = match std::fs::metadata(&path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(ProjectOpenError::ProjectNotFound { path });
        }
        Err(e) => return Err(ProjectOpenError::Io(e)),
    };

    // Step 3: must be a directory. A regular file at the same path is
    // not a project root — surface as ProjectNotFound (the lifecycle
    // layer's project.json existence check would also fail here, but
    // catching it early gives a clearer error site).
    if !meta.is_dir() {
        return Err(ProjectOpenError::ProjectNotFound { path });
    }

    // Step 4: delegate to the lifecycle primitive. The registry +
    // fixtures pair clears the §0.8 reconstructor-purity startup gate
    // by construction (built inline at call time — opens are rare
    // enough that holding a long-lived static is not worth it).
    let registry = crate::verbs::default_registry();
    let fixtures = crate::verbs::default_fixtures();
    let store =
        ProjectStore::open_with_registry(&path, &registry, &fixtures).map_err(|e| match e {
            // Lifecycle's `NoProjectJson` (path is a directory but
            // missing `project.json`) maps to `E_PROJECT_NOT_FOUND`.
            // The engine cannot open a non-project directory; same
            // failure mode as a non-existent path from the caller's
            // perspective.
            LifecycleError::NoProjectJson => {
                ProjectOpenError::ProjectNotFound { path: path.clone() }
            }
            LifecycleError::SnapshotCorrupt { detail } => {
                ProjectOpenError::SchemaViolation { detail }
            }
            LifecycleError::SchemaMismatch { found, expected } => {
                ProjectOpenError::SchemaVersionUnsupported { found, expected }
            }
            LifecycleError::LockHeldByAnotherProcess { .. } => {
                ProjectOpenError::ProjectLocked { path: path.clone() }
            }
            LifecycleError::Io(io) => ProjectOpenError::Io(io),
            other => ProjectOpenError::LifecycleFailed(other),
        })?;

    // Step 5: fast fingerprint check on every asset. Each mismatch
    // produces one entry in `unverified_asset_ids`; missing files
    // count as unverified too (the asset is intact in the graph, the
    // bytes just aren't on disk anymore).
    let unverified_asset_ids = compute_unverified_asset_ids(store.project(), &path);

    // Step 6: build the envelope.
    let project_id = store.project().id;
    let project = store.project().clone();
    let data = ProjectOpenData {
        project_id,
        project,
        unverified_asset_ids,
    };

    Ok((store, data))
}

/// Resolve a possibly-relative path against the caller's cwd. Returns
/// the path unchanged when already absolute; otherwise joins against
/// `std::env::current_dir()`.
fn resolve_path(input: &Path) -> Result<PathBuf, ProjectOpenError> {
    if input.is_absolute() {
        return Ok(input.to_path_buf());
    }
    let cwd = std::env::current_dir().map_err(ProjectOpenError::Io)?;
    Ok(cwd.join(input))
}

/// Run the §2.2 fast fingerprint check across all assets in `project`.
/// Returns the list of asset ids (in tracked order) whose on-disk
/// `(mtime_ms, size_bytes)` does not match the recorded
/// [`FileFingerprint`], or whose file is missing.
fn compute_unverified_asset_ids(project: &Project, project_root: &Path) -> Vec<String> {
    let mut unverified = Vec::new();
    for asset in &project.assets {
        let recorded = asset_fingerprint(asset);
        let asset_file = project_root.join(asset.path().as_str());
        if !fingerprint_matches(&asset_file, &recorded) {
            unverified.push(asset.id().to_string());
        }
    }
    unverified
}

/// Borrow the recorded fingerprint regardless of asset variant. The
/// per-variant metadata structs all carry `fingerprint: FileFingerprint`
/// — there is no shared accessor on the [`Asset`] enum so the match
/// happens here.
fn asset_fingerprint(asset: &Asset) -> FileFingerprint {
    match asset {
        Asset::Video(a) => a.metadata.fingerprint,
        Asset::Audio(a) => a.metadata.fingerprint,
        Asset::Image(a) => a.metadata.fingerprint,
        Asset::Subtitle(a) => a.metadata.fingerprint,
    }
}

/// Compare on-disk `(mtime_ms, size_bytes)` against `recorded`. Returns
/// `true` only when the file exists, both fields read cleanly, and
/// both match. Any IO error (`NotFound`, permission-denied,
/// non-representable mtime) counts as "unverified" — the engine treats
/// integrity-check failures as soft so the project still loads.
fn fingerprint_matches(file: &Path, recorded: &FileFingerprint) -> bool {
    let Ok(meta) = std::fs::metadata(file) else {
        return false;
    };
    let Ok(modified) = meta.modified() else {
        return false;
    };
    let Ok(epoch_dur) = modified.duration_since(std::time::UNIX_EPOCH) else {
        // Pre-1970 mtime — can't be represented as u64 POSIX millis,
        // which is the schema's mtime shape. Treat as unverified
        // rather than silently coercing.
        return false;
    };
    let Ok(mtime_ms) = u64::try_from(epoch_dur.as_millis()) else {
        return false;
    };
    let size_bytes = meta.len();
    mtime_ms == recorded.mtime_ms && size_bytes == recorded.size_bytes
}
