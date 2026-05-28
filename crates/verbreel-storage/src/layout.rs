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

use serde::Serialize;

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
