//! Project-root directory layout helpers.
//!
//! Two responsibilities:
//!
//! 1. [`init_project_root`] — creates the on-disk directory skeleton
//!    a project root needs before `verbreel-state` can save into it
//!    (`<root>/.verbreel/`, an empty `events.jsonl`, and the
//!    `<root>/assets/` content-addressed storage tree).
//!
//! 2. [`projects_index_path`] + [`register_project`] — the
//!    `<home>/.verbreel/projects-index` file that records every
//!    project root the user has ever opened. Each line is a single
//!    JSON object (`{"id":"...","path":"..."}`) so it can be
//!    `grep`-read or replayed line-by-line.

use std::fs::{self, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::fs::atomic_write_bytes;

/// Subdirectory inside a project root that holds engine state files
/// (`events.jsonl`, etc.). Matches `verbreel-state::lifecycle`.
const VERBREEL_DIR: &str = ".verbreel";

/// Name of the append-only event log inside `<root>/.verbreel/`.
const EVENTS_LOG: &str = "events.jsonl";

/// Name of the content-addressed assets tree at the project root.
const ASSETS_DIR: &str = "assets";

/// File name of the user-wide projects registry inside `<home>/.verbreel/`.
const PROJECTS_INDEX: &str = "projects-index";

/// Initialise an empty project root directory.
///
/// Creates (idempotently):
///
/// - `<root>/` itself, if missing.
/// - `<root>/.verbreel/` for engine state.
/// - `<root>/.verbreel/events.jsonl` — touched empty if absent. Open
///   with `create_new` would fail on the second call, which would
///   defeat idempotency, so we use `create(true).append(true)` and
///   immediately drop the handle.
/// - `<root>/assets/` for content-addressed storage.
///
/// Calling this on a root that already has all four artifacts is a
/// no-op and returns `Ok(())` — the function is safe to invoke from
/// `project.create` and from any future "repair my project layout"
/// path without special-casing.
///
/// # Errors
///
/// Returns [`io::Error`] if any `create_dir_all` or `OpenOptions::open`
/// call fails for a non-`AlreadyExists` reason (permission denied,
/// out of disk space, …).
pub fn init_project_root(root: &Path) -> io::Result<()> {
    fs::create_dir_all(root)?;

    let verbreel_dir = root.join(VERBREEL_DIR);
    fs::create_dir_all(&verbreel_dir)?;

    let events_log = verbreel_dir.join(EVENTS_LOG);
    // create(true) is idempotent — it does not truncate an existing
    // file when paired with append(true), so an existing log is left
    // untouched.
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(&events_log)?;

    let assets = root.join(ASSETS_DIR);
    fs::create_dir_all(&assets)?;

    Ok(())
}

/// Return the path of the user-wide projects index file under
/// `<home>/.verbreel/projects-index`.
///
/// This function is pure — it does not touch the filesystem. Use it
/// to resolve where [`register_project`] would write, or to read the
/// index from upstack code.
#[must_use]
pub fn projects_index_path(home: &Path) -> PathBuf {
    home.join(VERBREEL_DIR).join(PROJECTS_INDEX)
}

/// One row in the projects index. Single-line JSON keeps the file
/// grep-friendly. Field order is fixed by struct field order so the
/// on-disk shape is stable across `serde_json` versions.
#[derive(Debug, Serialize)]
struct IndexEntry<'a> {
    id: &'a str,
    path: &'a str,
}

/// Atomically append a `{"id":"…","path":"…"}` line to the projects
/// index at `<home>/.verbreel/projects-index`.
///
/// The implementation reads the full current contents, appends one
/// JSON line + `\n`, and writes the result back via
/// [`atomic_write_bytes`]. This trades O(n) per-write for crash safety
/// — the file is never observed half-updated, even if the process is
/// killed mid-write. The expected n for a single user is small enough
/// (hundreds of projects, not millions) that the trade-off is correct
/// at this layer.
///
/// `project_path` is rendered through [`Path::display`], which lossily
/// replaces non-UTF-8 bytes with U+FFFD. Project roots on
/// engine-managed media are always UTF-8, so this is acceptable.
///
/// # Errors
///
/// - [`io::Error`] if creating `<home>/.verbreel/`, reading the
///   existing index, serialising the entry, or writing the new
///   contents fails.
pub fn register_project(home: &Path, project_id: &str, project_path: &Path) -> io::Result<()> {
    let dir = home.join(VERBREEL_DIR);
    fs::create_dir_all(&dir)?;

    let index_path = dir.join(PROJECTS_INDEX);

    // Read existing contents (empty if the file does not exist yet).
    // The whole-file read is intentional — see the function-level
    // doc comment for the trade-off.
    let mut existing = Vec::new();
    match fs::File::open(&index_path) {
        Ok(mut f) => {
            f.read_to_end(&mut existing)?;
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }

    let entry = IndexEntry {
        id: project_id,
        path: &project_path.display().to_string(),
    };
    let mut line =
        serde_json::to_vec(&entry).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    line.push(b'\n');

    // Guarantee the new line starts on a fresh line even if the
    // previous append crashed before its trailing newline. This costs
    // one byte in the worst case and never produces a torn line.
    if !existing.is_empty() && !existing.ends_with(b"\n") {
        existing.push(b'\n');
    }
    existing.extend_from_slice(&line);

    atomic_write_bytes(&index_path, &existing)
}

/// One row read back from the projects index. Deserialize counterpart of
/// the [`IndexEntry`] write shape — only `id` and `path` are stored, so a
/// borrow-free owned struct is the natural read target.
#[derive(Debug, Deserialize)]
struct IndexRow {
    id: String,
    path: String,
}

/// Failure modes of [`resolve_root_for_project_id`].
///
/// `NotFound` is the spec-load-bearing case: the surfaces map it to
/// `E_PROJECT_NOT_FOUND` (§0.12). `Io` and `InvalidIndex` distinguish a
/// missing/unreadable index from one whose lines are not valid index
/// JSON — a hand-corrupted index is a hard error, not a silent miss,
/// because skipping the bad line could resolve a *stale* registration
/// for the same id and hand back the wrong root.
#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    /// No index entry maps `project_id` to a root. Surfaces map this to
    /// `E_PROJECT_NOT_FOUND` (§0.12). Carries the unresolved id.
    #[error("project not found: {0}")]
    NotFound(String),

    /// The index file could not be read for a reason other than
    /// "does not exist" (a missing index is treated as an empty index,
    /// i.e. `NotFound`, not `Io`).
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// A line in the index is not valid `{"id":...,"path":...}` JSON.
    #[error("projects index `{path}` has invalid JSON: {detail}")]
    InvalidIndex {
        /// The index path that failed to parse.
        path: String,
        /// The underlying serde error message.
        detail: String,
    },
}

/// Resolve a project root via `<home>/.verbreel/projects-index`.
///
/// Lines are scanned **newest-first** (`.rev()`); the most recent
/// registration for an id wins, so re-registering a moved project (a
/// fresh `register_project` append) shadows the older path without a
/// rewrite of the whole file. Mirrors the index-lookup half of
/// `verbreel-runtime`'s render resolver (its `lib.rs:95-116`) minus the
/// explicit candidate-roots scan — that scan is render-delivery-specific
/// and stays in `verbreel-runtime`.
///
/// A missing index file (the user has never registered a project) is
/// treated as an empty index and yields [`ResolveError::NotFound`], not
/// [`ResolveError::Io`] — "no registrations" and "this id is not
/// registered" are the same outcome for the caller.
///
/// # Errors
///
/// - [`ResolveError::NotFound`] — no entry maps `project_id` to a root
///   (including the "index file absent" case).
/// - [`ResolveError::Io`] — the index exists but could not be read.
/// - [`ResolveError::InvalidIndex`] — a non-empty line is not valid
///   index JSON (a hand-corrupted index aborts rather than silently
///   skipping the line, which could resolve a stale registration).
pub fn resolve_root_for_project_id(home: &Path, project_id: &str) -> Result<PathBuf, ResolveError> {
    let index_path = projects_index_path(home);

    let contents = match fs::read_to_string(&index_path) {
        Ok(contents) => contents,
        // A missing index == an empty index: the id is simply not
        // registered. Every other IO failure is surfaced.
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return Err(ResolveError::NotFound(project_id.to_string()));
        }
        Err(err) => return Err(ResolveError::Io(err)),
    };

    for line in contents
        .lines()
        .rev()
        .filter(|line| !line.trim().is_empty())
    {
        let row: IndexRow =
            serde_json::from_str(line).map_err(|err| ResolveError::InvalidIndex {
                path: index_path.display().to_string(),
                detail: err.to_string(),
            })?;
        if row.id == project_id {
            return Ok(PathBuf::from(row.path));
        }
    }

    Err(ResolveError::NotFound(project_id.to_string()))
}
