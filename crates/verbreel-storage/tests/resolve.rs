//! Integration tests for [`verbreel_storage::layout::resolve_root_for_project_id`].
//!
//! The resolver is the index-lookup half of `verbreel-runtime`'s render
//! resolver lifted into storage (where `register_project` /
//! `projects_index_path` already live). These tests pin the
//! register → resolve round-trip, newest-wins ordering, and the three
//! `ResolveError` variants.

use std::path::Path;

use tempfile::TempDir;
use verbreel_storage::layout::{ResolveError, register_project, resolve_root_for_project_id};

const ID_A: &str = "0192f3a0-0000-7000-8000-000000000001";
const ID_B: &str = "0192f3a0-0000-7000-8000-000000000002";

#[test]
fn register_then_resolve_returns_registered_root() {
    let home = TempDir::new().unwrap();
    let root = home.path().join("projects/alpha");
    register_project(home.path(), ID_A, &root).unwrap();

    let resolved = resolve_root_for_project_id(home.path(), ID_A).unwrap();
    assert_eq!(resolved, root);
}

#[test]
fn unknown_id_yields_not_found() {
    let home = TempDir::new().unwrap();
    register_project(home.path(), ID_A, Path::new("/tmp/alpha")).unwrap();

    let err = resolve_root_for_project_id(home.path(), ID_B).unwrap_err();
    match err {
        ResolveError::NotFound(id) => assert_eq!(id, ID_B),
        other => panic!("expected NotFound, got {other:?}"),
    }
}

#[test]
fn absent_index_is_not_found_not_io() {
    // A home with no `.verbreel/projects-index` must read as "id is not
    // registered", not as an IO error.
    let home = TempDir::new().unwrap();
    let err = resolve_root_for_project_id(home.path(), ID_A).unwrap_err();
    assert!(
        matches!(err, ResolveError::NotFound(_)),
        "absent index must surface NotFound, got {err:?}"
    );
}

#[test]
fn newest_registration_wins() {
    // Re-registering the same id with a new path must shadow the old
    // one (the resolver scans newest-first).
    let home = TempDir::new().unwrap();
    let old = home.path().join("projects/old");
    let new = home.path().join("projects/new");
    register_project(home.path(), ID_A, &old).unwrap();
    register_project(home.path(), ID_A, &new).unwrap();

    let resolved = resolve_root_for_project_id(home.path(), ID_A).unwrap();
    assert_eq!(resolved, new, "the most recent registration must win");
}

#[test]
fn corrupt_index_line_yields_invalid_index() {
    use std::fs;

    let home = TempDir::new().unwrap();
    register_project(home.path(), ID_A, Path::new("/tmp/alpha")).unwrap();

    // Hand-corrupt the index with a non-JSON trailing line.
    let index = home.path().join(".verbreel/projects-index");
    let mut bytes = fs::read(&index).unwrap();
    bytes.extend_from_slice(b"this is not json\n");
    fs::write(&index, &bytes).unwrap();

    // Newest-first scan hits the corrupt line before the valid one, so
    // the lookup aborts rather than silently resolving the stale entry.
    let err = resolve_root_for_project_id(home.path(), ID_A).unwrap_err();
    assert!(
        matches!(err, ResolveError::InvalidIndex { .. }),
        "corrupt index line must surface InvalidIndex, got {err:?}"
    );
}
