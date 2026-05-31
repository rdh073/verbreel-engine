//! Shared test scaffolding: a temp `.verbreel` home injected via
//! `VERBREEL_HOME`, plus a registered project so the resolver can map an
//! id to a root.
//!
//! `VERBREEL_HOME` is process-global, so every test that depends on it
//! runs under a single serializing mutex via [`with_home`]. The closure
//! receives the temp home path; the env var is set for the closure's
//! duration and restored afterward.
//!
//! Each integration-test binary that declares `mod common;` compiles its
//! own copy, so a helper used by only one binary reads as dead code in
//! the others — `allow(dead_code)` on the module is the standard shared-
//! test-toolkit shape, not a masked unused item.

#![allow(dead_code)]

use std::path::Path;
use std::sync::{Mutex, MutexGuard, OnceLock};

use tempfile::TempDir;

/// One global lock serializing every `VERBREEL_HOME` mutation. `OnceLock`
/// + `Mutex` rather than a crate dep keeps the test tree dependency-light.
fn env_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    // A poisoned lock from a panicking test must not cascade-fail the
    // rest; recover the guard so the remaining tests still run.
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Run `f` with `VERBREEL_HOME` pointing at a fresh temp dir, serialized
/// against every other `with_home` caller. Returns whatever `f` returns.
pub fn with_home<R>(f: impl FnOnce(&Path) -> R) -> R {
    let _guard = env_lock();
    let tmp = TempDir::new().expect("temp dir");
    let prev = std::env::var_os("VERBREEL_HOME");
    // SAFETY-equivalent: env mutation is serialized by `env_lock`, so no
    // other thread reads `VERBREEL_HOME` concurrently.
    unsafe {
        std::env::set_var("VERBREEL_HOME", tmp.path());
    }
    let result = f(tmp.path());
    unsafe {
        match prev {
            Some(v) => std::env::set_var("VERBREEL_HOME", v),
            None => std::env::remove_var("VERBREEL_HOME"),
        }
    }
    result
}

/// Create a project on disk under `home`, register it in the projects
/// index (so `resolve_root_for_project_id` finds it — the engine's
/// `project.create` does not register), and return its id.
///
/// This is the test-only bridge over the PR1-contract gap: nothing in the
/// merged engine populates the index, so a project-scoped CLI call cannot
/// resolve a root in production yet. Tests register explicitly to drive
/// the full open→dispatch path.
pub fn create_registered_project(home: &Path) -> String {
    let create_data = verbreel_state::project_create(&verbreel_state::ProjectCreateArgs {
        name: "cli-test-project".to_string(),
        canvas: "64x64".to_string(),
        fps_num: Some(30),
        fps_den: Some(1),
        at: Some(home.join("projects")),
        activate: false,
        metadata: serde_json::Map::new(),
    })
    .expect("project create");
    let id = create_data.project_id.to_string();
    verbreel_storage::layout::register_project(home, &id, &create_data.path)
        .expect("register project in index");
    id
}
