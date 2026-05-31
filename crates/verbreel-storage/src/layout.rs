//! Project-root directory layout helpers.
//!
//! Two responsibilities:
//!
//! 1. [`init_project_root`] — creates the on-disk directory skeleton
//!    a project root needs before `verbreel-state` can save into it
//!    (`<root>/.verbreel/`, an empty `events.jsonl`, and the
//!    `<root>/assets/` content-addressed storage tree).
//!
//! 2. [`projects_index_path`] + [`register_project`] /
//!    [`list_and_prune`] / [`resolve_root_for_project_id`]
//!    — the `<home>/.verbreel/projects-index` file (§2.6). It is a
//!    **single JSON object keyed by `project_id`** (the same shape as
//!    `~/.verbreel/idempotency.json`), maintained via `flock`'d
//!    read-modify-write so concurrent engine instances cannot corrupt
//!    it. Keying by id makes a lookup O(1), makes re-registration a
//!    free in-place upsert (no append + compaction), and makes the
//!    "one corrupt line bricks earlier registrations" failure mode
//!    structurally impossible (#445): the file is one document, parsed
//!    once — it either loads or fails atomically.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{self};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::flock::{self, FlockError};
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

/// Lock file guarding read-modify-write of [`PROJECTS_INDEX`]. A stable
/// dedicated file is used (not the index itself) because the index is
/// replaced via `rename(2)` — a `flock` bound to the index's inode would
/// not serialize across the rename. The lock file is never renamed, so
/// it serializes the whole RMW critical section across processes.
const PROJECTS_INDEX_LOCK: &str = "projects-index.lock";

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

/// One entry in the §2.6 projects index, keyed by `project_id` in the
/// enclosing object. `project_id` is stored redundantly inside the value
/// (matching the spec's object shape) so a single entry round-trips
/// without its map key. Field order is fixed by struct field order so
/// the on-disk shape is stable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexEntry {
    /// `UUIDv7` project id (also the map key).
    pub project_id: String,
    /// Project display name.
    pub name: String,
    /// Absolute path to the project folder.
    pub path: String,
    /// RFC 3339 timestamp the project was last opened.
    pub last_opened_at: String,
}

/// The whole projects index: a JSON object keyed by `project_id`. A
/// [`BTreeMap`] gives a deterministic (sorted-by-key) on-disk order
/// without depending on `serde_json`'s `preserve_order` feature.
pub type ProjectsIndex = BTreeMap<String, IndexEntry>;

/// Acquire the RMW lock and read the current index, returning the
/// parsed map plus the held lock guard. A missing index file reads as
/// an empty map; a present-but-unparseable file is a hard error.
fn lock_and_read(home: &Path) -> io::Result<(flock::ExclusiveFlock, PathBuf, ProjectsIndex)> {
    let dir = home.join(VERBREEL_DIR);
    fs::create_dir_all(&dir)?;

    let lock_file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(dir.join(PROJECTS_INDEX_LOCK))?;
    let guard = flock::acquire_exclusive(lock_file).map_err(flock_to_io)?;

    let index_path = dir.join(PROJECTS_INDEX);
    let index = match fs::read_to_string(&index_path) {
        Ok(contents) if contents.trim().is_empty() => ProjectsIndex::new(),
        Ok(contents) => serde_json::from_str(&contents)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => ProjectsIndex::new(),
        Err(e) => return Err(e),
    };

    Ok((guard, index_path, index))
}

/// Serialize and atomically replace the index file.
fn write_index(index_path: &Path, index: &ProjectsIndex) -> io::Result<()> {
    let bytes =
        serde_json::to_vec(index).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    atomic_write_bytes(index_path, &bytes)
}

/// Map an [`FlockError`] onto an [`io::Error`] so the index helpers keep a
/// single `io::Result` surface. Contention becomes `WouldBlock`.
fn flock_to_io(err: FlockError) -> io::Error {
    match err {
        FlockError::Contended => io::Error::new(
            io::ErrorKind::WouldBlock,
            "projects-index is locked by another process",
        ),
        FlockError::Io(e) => e,
    }
}

/// Read the §2.6 projects index as a parsed map.
///
/// A missing or empty index reads as an empty map. The read takes the
/// RMW lock so it observes a consistent snapshot, never a half-written
/// file mid-`register_project`.
///
/// # Errors
///
/// - [`io::Error`] of kind `InvalidData` if the index is present but not
///   a valid keyed JSON object.
/// - [`io::Error`] of kind `WouldBlock` if another process holds the
///   RMW lock, or any other IO failure creating/reading the files.
pub fn read_index(home: &Path) -> io::Result<ProjectsIndex> {
    let (_guard, _path, index) = lock_and_read(home)?;
    Ok(index)
}

/// Upsert a project entry into `<home>/.verbreel/projects-index` (§2.6).
///
/// The full read-modify-write runs under an exclusive `flock` on a
/// dedicated lock file, so concurrent engine instances serialize. The
/// entry is keyed by `project_id`: a re-registration (e.g. re-opening a
/// moved project) overwrites the existing value in place, setting a
/// fresh `last_opened_at` — no append, no compaction, no duplicate keys.
///
/// `project_path` is rendered through [`Path::display`], which lossily
/// replaces non-UTF-8 bytes with U+FFFD. Project roots on
/// engine-managed media are always UTF-8, so this is acceptable.
///
/// # Errors
///
/// - [`io::Error`] of kind `InvalidData` if the existing index is
///   corrupt (so a bad index is not silently overwritten).
/// - [`io::Error`] of kind `WouldBlock` on lock contention, or any other
///   IO failure creating `<home>/.verbreel/`, reading the existing
///   index, serialising, or writing.
pub fn register_project(
    home: &Path,
    project_id: &str,
    name: &str,
    project_path: &Path,
    last_opened_at: &str,
) -> io::Result<()> {
    let (_guard, index_path, mut index) = lock_and_read(home)?;
    index.insert(
        project_id.to_string(),
        IndexEntry {
            project_id: project_id.to_string(),
            name: name.to_string(),
            path: project_path.display().to_string(),
            last_opened_at: last_opened_at.to_string(),
        },
    );
    write_index(&index_path, &index)
}

/// Remove the entry keyed by `project_id` from the §2.6 projects index,
/// returning the removed entry's path, or `None` if no such entry exists.
///
/// The read-modify-write runs under the same exclusive `flock` as
/// [`register_project`], so a concurrent register/forget cannot tear the
/// file. A missing entry leaves the index untouched (no rewrite) and
/// returns `None` — the caller maps that to `E_PROJECT_NOT_FOUND` (§2.8).
/// The returned path is the index entry's recorded `path`, so the caller
/// can echo it as `removed_path` without re-resolving.
///
/// # Errors
///
/// - [`io::Error`] of kind `InvalidData` if the existing index is corrupt
///   (so a damaged index surfaces, never silently empties).
/// - [`io::Error`] of kind `WouldBlock` on lock contention, or any other
///   IO failure reading/writing the index.
pub fn deregister_project_by_id(home: &Path, project_id: &str) -> io::Result<Option<String>> {
    let (_guard, index_path, mut index) = lock_and_read(home)?;
    match index.remove(project_id) {
        Some(entry) => {
            write_index(&index_path, &index)?;
            Ok(Some(entry.path))
        }
        None => Ok(None),
    }
}

/// Remove every entry whose recorded `path` equals `project_path` from the
/// §2.6 projects index, returning whether any entry was removed.
///
/// The path form has no map key to probe, so this is a linear scan over
/// the (small) index: an entry matches when its stored `path` string is
/// byte-equal to `project_path`. The whole read-scan-write runs under the
/// shared RMW lock. When nothing matches the file is left untouched (no
/// rewrite) and `false` is returned — the path form never errors on a
/// miss (§2.8: `was_in_index: false`).
///
/// Comparison is on the stored string verbatim — no canonicalisation —
/// because the index stores the absolutised root the verb validated at
/// register time, and `project.forget`'s path form is documented to take
/// the same on-disk path string.
///
/// # Errors
///
/// - [`io::Error`] of kind `InvalidData` if the existing index is corrupt.
/// - [`io::Error`] of kind `WouldBlock` on lock contention, or any other
///   IO failure reading/writing the index.
pub fn deregister_project_by_path(home: &Path, project_path: &str) -> io::Result<bool> {
    let (_guard, index_path, mut index) = lock_and_read(home)?;
    let matched: Vec<String> = index
        .iter()
        .filter(|(_, entry)| entry.path == project_path)
        .map(|(id, _)| id.clone())
        .collect();
    if matched.is_empty() {
        return Ok(false);
    }
    for id in &matched {
        index.remove(id);
    }
    write_index(&index_path, &index)?;
    Ok(true)
}

/// Read the index, prune stale entries, and return the surviving map
/// plus the ids removed — all under one RMW lock acquisition (§2.6).
///
/// "Stale" is deliberately narrow: an entry is removed only when its
/// `path` is confirmed gone via [`symlink_metadata`], i.e. the lookup
/// returned `ENOENT`. A `Path::exists()`-style test would also report
/// `false` for a transient `EACCES` or an unmounted network/external
/// volume, which would permanently drop a still-valid registration on
/// a momentary stat failure; those non-`NotFound` errors leave the
/// entry in place. Entries whose id is in `exempt` are never pruned —
/// the engine passes its open-project ids so a project it currently
/// holds open is never removed from its own listing even if the path
/// is transiently unreachable.
///
/// Returning the post-prune index (not just the removed ids) lets the
/// caller list under the same lock that pruned, closing the
/// read-after-prune race a separate `read_index` would open and
/// halving the lock/IO. When nothing is stale the file is left
/// untouched (no rewrite). A missing index reads as empty.
///
/// Removed ids are returned in sorted order (BTreeMap iteration).
///
/// # Errors
///
/// - [`io::Error`] of kind `InvalidData` if the existing index is
///   corrupt (so a damaged index surfaces, never silently empties).
/// - [`io::Error`] of kind `WouldBlock` on lock contention, or any
///   other IO failure reading/writing the index.
pub fn list_and_prune(
    home: &Path,
    exempt: &BTreeSet<String>,
) -> io::Result<(ProjectsIndex, Vec<String>)> {
    let (_guard, index_path, mut index) = lock_and_read(home)?;
    let removed: Vec<String> = index
        .iter()
        .filter(|(id, entry)| !exempt.contains(*id) && path_confirmed_gone(&entry.path))
        .map(|(id, _)| id.clone())
        .collect();
    if removed.is_empty() {
        return Ok((index, removed));
    }
    for id in &removed {
        index.remove(id);
    }
    write_index(&index_path, &index)?;
    Ok((index, removed))
}

/// `true` only when `path` is confirmed absent (`stat` → `ENOENT`).
/// Any other `stat` error (`EACCES`, unmounted volume, `EIO`) returns
/// `false` so a transiently-unreachable path is treated as present and
/// its registration is preserved. Uses `symlink_metadata` to avoid
/// following a dangling symlink into a misleading second error.
fn path_confirmed_gone(path: &str) -> bool {
    match fs::symlink_metadata(Path::new(path)) {
        Ok(_) => false,
        Err(e) => e.kind() == io::ErrorKind::NotFound,
    }
}

/// Failure modes of [`resolve_root_for_project_id`].
///
/// `NotFound` is the spec-load-bearing case: the surfaces map it to
/// `E_PROJECT_NOT_FOUND` (§0.12). `Io` and `InvalidIndex` distinguish a
/// missing/unreadable index from one that is not a valid keyed object —
/// a corrupt index is a hard error, not a silent miss, so the caller can
/// tell "no such project" (`NotFound`) from "index damaged"
/// (`InvalidIndex`).
#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    /// No index entry is keyed by `project_id`. Surfaces map this to
    /// `E_PROJECT_NOT_FOUND` (§0.12). Carries the unresolved id.
    #[error("project not found: {0}")]
    NotFound(String),

    /// The index file could not be read for a reason other than
    /// "does not exist" (a missing index is treated as an empty index,
    /// i.e. `NotFound`, not `Io`).
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// The index file is not a valid keyed JSON object (§2.6). The whole
    /// document failed to parse — keying by id means there is no
    /// per-line scan that could half-succeed and resolve a stale entry.
    #[error("projects index `{path}` has invalid JSON: {detail}")]
    InvalidIndex {
        /// The index path that failed to parse.
        path: String,
        /// The underlying serde error message.
        detail: String,
    },
}

/// Resolve a project root via `<home>/.verbreel/projects-index` (§2.6).
///
/// The index is a single JSON object keyed by `project_id`, so this is
/// an O(1) map lookup, not a newest-first line scan. A corrupt index
/// fails atomically (`InvalidIndex`) — it can never brick the resolution
/// of entries registered before the corruption, because there are no
/// "earlier lines" to be bricked (#445 vanishes structurally).
///
/// A missing index file (the user has never registered a project) is
/// treated as an empty index and yields [`ResolveError::NotFound`], not
/// [`ResolveError::Io`] — "no registrations" and "this id is not
/// registered" are the same outcome for the caller.
///
/// # Errors
///
/// - [`ResolveError::NotFound`] — no entry is keyed by `project_id`
///   (including the "index file absent" case).
/// - [`ResolveError::Io`] — the index exists but could not be read.
/// - [`ResolveError::InvalidIndex`] — the index is not a valid keyed
///   JSON object.
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

    if contents.trim().is_empty() {
        return Err(ResolveError::NotFound(project_id.to_string()));
    }

    let index: ProjectsIndex =
        serde_json::from_str(&contents).map_err(|err| ResolveError::InvalidIndex {
            path: index_path.display().to_string(),
            detail: err.to_string(),
        })?;

    match index.get(project_id) {
        Some(entry) => Ok(PathBuf::from(&entry.path)),
        None => Err(ResolveError::NotFound(project_id.to_string())),
    }
}
