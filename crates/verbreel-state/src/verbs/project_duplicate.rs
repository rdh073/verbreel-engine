//! `project.duplicate` (§2.7) — eighty-second production verb. Seventh
//! slice of the project arc; storage-heavy.
//!
//! ## Spec quote (`spec/commands/project.md` §2.7, abbreviated)
//!
//! > Clones a project folder. Assets are hard-linked when possible
//! > (same filesystem), copied otherwise. Transactional: the engine
//! > refuses to start work if the destination folder already exists
//! > (returns `E_PROJECT_EXISTS` before any disk writes). On any
//! > mid-call failure, the engine recursively deletes the partial
//! > destination folder before returning `Err`.
//! >
//! > `project.json` is copied, then mutated in place: fresh
//! > `Project.id` (UUIDv7), refreshed `Project.created_at` /
//! > `Project.updated_at`, `Project.last_saved_event_id` reset to
//! > `null`. `assets/` is hard-linked when same-filesystem, copied
//! > otherwise. `events.jsonl` is NOT copied (the duplicate starts
//! > with an empty event log).
//! >
//! > CLI: `verbreel project duplicate [--project <id>] --name <str> [--at <path>]`
//! > MCP: `project.duplicate`
//! > Args: `project_id: string, name: string, at?: string`.
//! > Returns (`data`): `{ project_id: string; path: string }`.
//! > Errors: `E_PROJECT_EXISTS`, `E_IDEMPOTENCY_CONFLICT`, `E_IO`.
//!
//! ## Why this verb is NOT a [`Verb`](crate::reconstructor::Verb) trait impl
//!
//! Same shape as `project.create` / `project.open` / `project.save`:
//! the duplicate writes a **second** on-disk project at a different
//! path, and the result envelope returns the new project's id, not
//! the source's. The `Verb` trait's pure `compute_patch(&Project,
//! &args) → (Patch, data, warnings)` contract cannot model a
//! multi-root filesystem-level transaction.
//!
//! ## v1 accommodation: `source_path` arg
//!
//! Per §2.7 the verb takes `project_id` and resolves the source root
//! via `~/.verbreel/projects-index`. v1 ships without the index
//! (same deferral as `project.list` §2.6 — see
//! `verbs::project_list`). The v1 surface therefore accepts an
//! explicit `source_path: PathBuf` arg that the caller (CLI / MCP
//! shim layer) supplies after looking up the path it already knows.
//! When the projects-index lands, the same slice that wires the
//! file read for `project.list` will also resolve `project_id →
//! source_path` here, and the `source_path` arg goes away.
//!
//! ## Deferred (out of v1 scope)
//!
//! - `~/.verbreel/projects-index` registration of the new entry.
//! - `E_IDEMPOTENCY_CONFLICT`: §0.8 idempotency is wired at the
//!   `ProjectStore::mutate` layer for verbs that emit events; this
//!   verb writes no event into the source's log, so there's nothing
//!   to key on at v1. Mirrors the `project.save` deferral.
//! - `.verbreel/config.json` copy: the engine does not yet emit this
//!   file. When it lands, add the copy here.

use std::fs::{self, File};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;
use verbreel_events::timestamp_rfc3339_now;
use verbreel_types::ProjectId;

use crate::project::Project;
use crate::verbs::project_rename::{PROJECT_NAME_MAX, PROJECT_NAME_MIN};

/// Arguments for [`duplicate`].
///
/// `deny_unknown_fields` enforces the §2.7 published surface: stray
/// MCP/HTTP payload keys surface as arg-shape errors rather than
/// being silently accepted.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectDuplicateArgs {
    /// Source project id. v1 informational only — the actual source
    /// is resolved from [`Self::source_path`]; the id is echoed back
    /// for diagnostic purposes.
    pub project_id: ProjectId,

    /// Display name for the new project. Must satisfy the schema's
    /// `Project.name` constraints (1–256 UTF-8 chars).
    pub name: String,

    /// Optional explicit destination root. When `None`, the
    /// destination is `<source_path parent>/<name>` (sibling of the
    /// source named by `name`). Must be absolute when supplied —
    /// relative paths are rejected with
    /// [`ProjectDuplicateError::RelativeAt`] because the returned
    /// envelope's `path` field is contractually absolute and a
    /// process-cwd-relative resolution would make destination
    /// placement depend on runtime cwd.
    #[serde(default)]
    pub at: Option<PathBuf>,

    /// v1 accommodation: the absolute on-disk path of the source
    /// project root. A future slice that wires
    /// `~/.verbreel/projects-index` will resolve this from
    /// [`Self::project_id`].
    pub source_path: PathBuf,
}

/// Envelope returned by a successful [`duplicate`].
///
/// Mirrors the §2.7 `data` shape: `project_id` is the **new** id
/// (minted `UUIDv7` for the duplicate), `path` is the absolute path of
/// the duplicate's root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectDuplicateData {
    /// Newly minted [`ProjectId`] for the duplicate.
    pub project_id: ProjectId,
    /// Absolute path of the duplicate's root.
    pub path: PathBuf,
}

/// Errors returned by [`duplicate`].
#[derive(Debug, thiserror::Error)]
pub enum ProjectDuplicateError {
    /// `args.name` is empty (`Project.name` `minLength: 1`). Maps to
    /// `E_SCHEMA_VIOLATION` per §0.12 universal-implicit (reachable
    /// via the `name` arg shape).
    #[error("project.duplicate: `name` cannot be empty")]
    NameEmpty,

    /// `args.name` exceeds [`PROJECT_NAME_MAX`] UTF-8 chars. Maps to
    /// `E_SCHEMA_VIOLATION`.
    #[error("project.duplicate: `name` has {actual} chars, exceeds max of {max}")]
    NameTooLong {
        /// Actual character count.
        actual: usize,
        /// Maximum allowed characters.
        max: usize,
    },

    /// Destination root already exists. Maps to `E_PROJECT_EXISTS`.
    /// Surfaces BEFORE any writes per the §2.7 transactional contract.
    #[error("project.duplicate: destination already exists: {path}")]
    DestinationExists {
        /// The destination path that exists.
        path: PathBuf,
    },

    /// `args.at` was supplied as a relative path. Maps to
    /// `E_SCHEMA_VIOLATION` (universal-implicit per §0.12; reachable
    /// via the `at` arg shape). Relative paths are rejected because
    /// the returned envelope contractually carries the absolute
    /// destination, and process-cwd-relative resolution would couple
    /// destination placement to the caller's runtime cwd.
    #[error("project.duplicate: `at` must be absolute, got: {path}")]
    RelativeAt {
        /// The relative path the caller supplied.
        path: PathBuf,
    },

    /// Source `project.json` is missing or unreadable. Maps to
    /// `E_PROJECT_NOT_FOUND` per §0.12 universal-implicit (reachable
    /// via the `project_id` arg shape).
    #[error("project.duplicate: source project not found at {path}")]
    SourceNotFound {
        /// The source path that was probed.
        path: PathBuf,
    },

    /// Source `project.json` exists but does not deserialize. Maps
    /// to `E_IO` — at the v1 surface the source path is the caller's
    /// responsibility, so a malformed snapshot surfaces as an I/O
    /// class failure rather than a separate code.
    #[error("project.duplicate: source project.json is corrupt: {detail}")]
    SourceCorrupt {
        /// Underlying serde error message.
        detail: String,
    },

    /// Filesystem failure during the clone. Maps to `E_IO`. The
    /// destination has been rolled back (recursively deleted) before
    /// this is returned, so the caller sees a clean fail.
    #[error("project.duplicate: I/O failure: {0}")]
    Io(#[from] std::io::Error),
}

/// Clone the project rooted at `args.source_path` into a new folder
/// per §2.7.
///
/// Algorithm:
/// 1. Validate `name` per [`PROJECT_NAME_MIN`] / [`PROJECT_NAME_MAX`].
/// 2. Resolve `dest = args.at.unwrap_or(<source parent>/<name>)`.
/// 3. Refuse if `dest` exists (`E_PROJECT_EXISTS`) before any writes.
/// 4. Read + deserialize source `project.json`.
/// 5. Mint a new [`ProjectId`], refresh `created_at`/`updated_at`,
///    reset `last_saved_event_id` to `None`.
/// 6. Create destination tree (`<dest>/` + `<dest>/.verbreel/`).
/// 7. Walk `<source>/assets/` recursively. Each file: `hard_link`,
///    fall back to `copy` on `EXDEV`. Subdirs are mirrored.
/// 8. Atomic-write the mutated `Project` to `<dest>/project.json`
///    via `NamedTempFile::new_in(<dest>) + persist + parent fsync`.
/// 9. Create empty `<dest>/.verbreel/events.jsonl` (touched +
///    fsync'd; no flock — duplicate is not opened by this call).
/// 10. On any failure in steps 6-9, recursively delete `<dest>`
///     before returning the error. Rollback failures are logged
///     (`tracing::error!`) but the original failure is surfaced.
///
/// # Errors
///
/// See variants of [`ProjectDuplicateError`].
pub fn duplicate(
    args: &ProjectDuplicateArgs,
) -> Result<ProjectDuplicateData, ProjectDuplicateError> {
    // Step 1: name validation (mirror project_rename).
    let char_count = args.name.chars().count();
    if char_count < PROJECT_NAME_MIN {
        return Err(ProjectDuplicateError::NameEmpty);
    }
    if char_count > PROJECT_NAME_MAX {
        return Err(ProjectDuplicateError::NameTooLong {
            actual: char_count,
            max: PROJECT_NAME_MAX,
        });
    }

    // Step 2: enforce `at` is absolute (the returned envelope's
    // `path` is contractually absolute), then resolve destination.
    if let Some(at) = args.at.as_deref()
        && !at.is_absolute()
    {
        return Err(ProjectDuplicateError::RelativeAt {
            path: at.to_path_buf(),
        });
    }
    let dest = resolve_destination(&args.source_path, args.at.as_deref(), &args.name);

    // Step 3: refuse if dest exists. Symlink_metadata avoids
    // following dangling symlinks into a misleading "doesn't exist"
    // verdict.
    if dest.symlink_metadata().is_ok() {
        return Err(ProjectDuplicateError::DestinationExists { path: dest });
    }

    // Step 4: read source project.json.
    let source_pj = args.source_path.join("project.json");
    let bytes = match fs::read(&source_pj) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(ProjectDuplicateError::SourceNotFound {
                path: args.source_path.clone(),
            });
        }
        Err(e) => return Err(ProjectDuplicateError::Io(e)),
    };
    let mut project: Project =
        serde_json::from_slice(&bytes).map_err(|e| ProjectDuplicateError::SourceCorrupt {
            detail: e.to_string(),
        })?;

    // Step 5: mint new id + refresh timestamps + reset bookkeeping.
    // The reset of last_saved_event_id is load-bearing per §2.7: a
    // non-null id pointing at a source event would fail the
    // duplicate's first project.open consistency check.
    let new_id = ProjectId::now();
    let now = timestamp_rfc3339_now();
    project.id = new_id;
    project.created_at.clone_from(&now);
    project.updated_at = now;
    project.last_saved_event_id = None;

    // Steps 6-9 run inside a closure so any failure can trigger the
    // rollback (step 10) without a tangled drop-guard.
    match write_destination(&dest, &project, &args.source_path) {
        Ok(()) => Ok(ProjectDuplicateData {
            project_id: new_id,
            path: dest,
        }),
        Err(e) => {
            // Step 10: rollback. The destination may be partially
            // populated; remove the whole subtree. A rollback
            // failure is logged but the ORIGINAL error is surfaced
            // — the caller's failure mode is "the clone didn't
            // happen", not "the rollback didn't happen".
            //
            // Avoid `dest.exists()` here: it follows symlinks, so a
            // dangling-symlink rollback path would silently skip the
            // `remove_dir_all` and leak a partial destination. Call
            // `remove_dir_all` unconditionally and treat `NotFound`
            // as the no-op success case.
            if let Err(rollback_err) = fs::remove_dir_all(&dest)
                && !matches!(rollback_err.kind(), std::io::ErrorKind::NotFound)
            {
                tracing::error!(
                    dest = %dest.display(),
                    rollback_error = %rollback_err,
                    original_error = %e,
                    "project.duplicate: rollback failed; destination may be partial"
                );
            }
            Err(e)
        }
    }
}

/// Resolve the destination root per the §2.7 `at` rules.
fn resolve_destination(source: &Path, at: Option<&Path>, name: &str) -> PathBuf {
    if let Some(p) = at {
        return p.to_path_buf();
    }
    match source.parent() {
        Some(parent) => parent.join(name),
        // Source has no parent (e.g. root "/"); fall back to using
        // `name` as a relative path. The destination collision check
        // will catch any nonsense before any write.
        None => PathBuf::from(name),
    }
}

/// Steps 6-9 from [`duplicate`] — creates the destination tree,
/// mirrors `assets/`, atomic-writes `project.json`, and initializes
/// the empty `events.jsonl`. All-or-nothing: any error here triggers
/// the rollback at the caller.
fn write_destination(
    dest: &Path,
    project: &Project,
    source_root: &Path,
) -> Result<(), ProjectDuplicateError> {
    // Step 6: destination tree.
    fs::create_dir_all(dest)?;
    fs::create_dir_all(dest.join(".verbreel"))?;

    // Step 7: mirror assets/ if present. Empty / missing assets dir
    // is fine — projects without imports are valid.
    let source_assets = source_root.join("assets");
    if source_assets.is_dir() {
        mirror_assets(&source_assets, &dest.join("assets"))?;
    }

    // Step 8: atomic write of the mutated project.json. The
    // serialization path is the §0.5.2 canonical JSON (vr-jcs) form
    // per CLAUDE.md ("use vr-jcs for project.json serialization"),
    // not serde_json::to_vec_pretty — a fresh project should land on
    // disk in the canonical shape so its first project_hash matches
    // the on-disk bytes byte-for-byte.
    let project_value = serde_json::to_value(project)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let bytes = verbreel_canon::canonicalize(&project_value)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let mut tmp = NamedTempFile::new_in(dest)?;
    tmp.as_file_mut().write_all(&bytes)?;
    tmp.as_file_mut().sync_data()?;
    let target = dest.join("project.json");
    tmp.persist(&target).map_err(|e| e.error)?;
    let dir = File::open(dest)?;
    dir.sync_data()?;

    // Step 9: empty events.jsonl. Mirrors NativeBackend::open's
    // file shape (zero-byte file) without taking the flock — the
    // duplicate is not opened by this call.
    let events_path = dest.join(".verbreel").join("events.jsonl");
    let events = File::create(&events_path)?;
    events.sync_data()?;

    Ok(())
}

/// Recursively mirror `src` into `dst`. Each regular file is
/// hard-linked when same-filesystem; on `EXDEV` (cross-device) the
/// file is byte-copied instead. Directories are created with
/// `create_dir_all` on the destination side. Symlinks are followed
/// (mirrors `fs::copy` semantics — content-addressed asset paths
/// don't ship symlinks under v1).
fn mirror_assets(src: &Path, dst: &Path) -> Result<(), ProjectDuplicateError> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if kind.is_dir() {
            mirror_assets(&src_path, &dst_path)?;
        } else {
            link_or_copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

/// Try `hard_link`; on cross-device error fall back to `fs::copy`.
/// `ErrorKind::CrossesDevices` is the portable form (stable since
/// Rust 1.85, well within the MSRV) and maps to Linux/macOS `EXDEV`
/// + Windows `ERROR_NOT_SAME_DEVICE` uniformly.
fn link_or_copy(src: &Path, dst: &Path) -> Result<(), ProjectDuplicateError> {
    match fs::hard_link(src, dst) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::CrossesDevices => {
            fs::copy(src, dst)?;
            Ok(())
        }
        Err(e) => Err(ProjectDuplicateError::Io(e)),
    }
}
