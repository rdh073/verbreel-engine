//! Home-directory resolution and the §0.12 active-project file.
//!
//! `home` is the directory that *contains* `.verbreel/` — the same root
//! [`Engine::new`](verbreel_state::Engine::new) and
//! [`verbreel_storage::layout::projects_index_path`] are handed. It is
//! resolved from `VERBREEL_HOME` then `HOME`, mirroring the existing
//! convention in `verbreel-runtime` (`from_env` at `lib.rs:55`) rather
//! than inventing a new one. Tests inject a temp dir via `VERBREEL_HOME`.
//!
//! The active-project file at `<home>/.verbreel/active-project` is a
//! one-line `project_id`. §0.12 makes it a **CLI-only** ergonomic: an
//! omitted `--project` falls back to its trimmed contents. The matching
//! writer (after `project.open --activate` / `project.create --activate`)
//! is a CLI-front-end concern the engine explicitly defers
//! (`project_open.rs:81`, `project_create.rs:67`), so it lives here.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// `.verbreel` subdirectory name — mirrors
/// `verbreel_storage::layout`'s private constant so the active-project
/// file lands next to the projects index.
const VERBREEL_DIR: &str = ".verbreel";

/// File holding the single active `project_id` (§0.12).
const ACTIVE_PROJECT: &str = "active-project";

/// Resolve the `.verbreel` home root from the environment.
///
/// Order: `VERBREEL_HOME`, then `HOME`. Returns `None` only when neither
/// is set (a headless environment with no home) — the caller maps that
/// to a project-less engine rooted at the current directory so
/// project-less reads still work.
#[must_use]
pub fn resolve_home() -> Option<PathBuf> {
    std::env::var_os("VERBREEL_HOME")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

/// Path to the active-project file under `home`.
#[must_use]
pub fn active_project_path(home: &Path) -> PathBuf {
    home.join(VERBREEL_DIR).join(ACTIVE_PROJECT)
}

/// Read the active `project_id` from `<home>/.verbreel/active-project`.
///
/// A missing file, an empty file, or a read error all yield `None` — a
/// missing active project is "no fallback", never a hard error (the
/// caller then leaves `project_id` unset and lets the engine decide).
#[must_use]
pub fn read_active_project(home: &Path) -> Option<String> {
    let contents = fs::read_to_string(active_project_path(home)).ok()?;
    let trimmed = contents.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Write `project_id` to `<home>/.verbreel/active-project` atomically.
///
/// Atomic = write a sibling temp file, then rename over the target, so a
/// concurrent reader never observes a half-written id. Creates the
/// `.verbreel` directory if absent.
///
/// # Errors
///
/// Propagates any I/O failure creating the directory, writing the temp
/// file, or renaming it into place.
pub fn write_active_project(home: &Path, project_id: &str) -> io::Result<()> {
    let dir = home.join(VERBREEL_DIR);
    fs::create_dir_all(&dir)?;
    let target = dir.join(ACTIVE_PROJECT);
    let tmp = dir.join(format!("{ACTIVE_PROJECT}.tmp.{}", std::process::id()));
    fs::write(&tmp, format!("{project_id}\n"))?;
    fs::rename(&tmp, &target)
}
