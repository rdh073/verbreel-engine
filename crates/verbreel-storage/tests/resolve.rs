//! Integration tests for [`verbreel_storage::layout::resolve_root_for_project_id`]
//! and the §2.6 keyed-object index lifecycle helpers
//! ([`register_project`], [`deregister_project`], [`prune_stale`]).
//!
//! The index is a single JSON object keyed by `project_id` (§2.6), so a
//! lookup is an O(1) map probe — not a newest-first line scan. These
//! tests pin the register → resolve round-trip, upsert-by-id, the three
//! `ResolveError` variants, and the three corruption regressions folded
//! in from #445 (a corrupt single document fails atomically; the
//! per-line shadow bug vanishes with the format).

use std::path::Path;

use tempfile::TempDir;
use verbreel_storage::layout::{
    IndexEntry, ResolveError, deregister_project, prune_stale, read_index, register_project,
    resolve_root_for_project_id,
};

const ID_A: &str = "0192f3a0-0000-7000-8000-000000000001";
const ID_B: &str = "0192f3a0-0000-7000-8000-000000000002";
const AT: &str = "2025-01-01T00:00:00Z";

#[test]
fn register_then_resolve_returns_registered_root() {
    let home = TempDir::new().unwrap();
    let root = home.path().join("projects/alpha");
    register_project(home.path(), ID_A, "alpha", &root, AT).unwrap();

    let resolved = resolve_root_for_project_id(home.path(), ID_A).unwrap();
    assert_eq!(resolved, root);
}

#[test]
fn unknown_id_yields_not_found() {
    let home = TempDir::new().unwrap();
    register_project(home.path(), ID_A, "alpha", Path::new("/tmp/alpha"), AT).unwrap();

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
fn reregister_same_id_upserts_in_place() {
    // Re-registering the same id with a new path must overwrite the old
    // entry (upsert-by-id), not accumulate a second one.
    let home = TempDir::new().unwrap();
    let old = home.path().join("projects/old");
    let new = home.path().join("projects/new");
    register_project(home.path(), ID_A, "alpha", &old, AT).unwrap();
    register_project(home.path(), ID_A, "alpha-moved", &new, "2025-02-02T00:00:00Z").unwrap();

    let resolved = resolve_root_for_project_id(home.path(), ID_A).unwrap();
    assert_eq!(resolved, new, "the latest registration must win");

    let index = read_index(home.path()).unwrap();
    assert_eq!(index.len(), 1, "upsert must not accumulate duplicate keys");
    let entry = &index[ID_A];
    assert_eq!(entry.name, "alpha-moved");
    assert_eq!(entry.last_opened_at, "2025-02-02T00:00:00Z");
}

#[test]
fn register_stores_all_section_2_6_fields() {
    let home = TempDir::new().unwrap();
    let root = home.path().join("projects/alpha");
    register_project(home.path(), ID_A, "Alpha Project", &root, AT).unwrap();

    let index = read_index(home.path()).unwrap();
    let entry: &IndexEntry = &index[ID_A];
    assert_eq!(entry.project_id, ID_A);
    assert_eq!(entry.name, "Alpha Project");
    assert_eq!(entry.path, root.display().to_string());
    assert_eq!(entry.last_opened_at, AT);
}

// --- deregister_project --------------------------------------------------

#[test]
fn deregister_removes_entry_and_reports_presence() {
    let home = TempDir::new().unwrap();
    register_project(home.path(), ID_A, "alpha", Path::new("/tmp/alpha"), AT).unwrap();
    register_project(home.path(), ID_B, "beta", Path::new("/tmp/beta"), AT).unwrap();

    assert!(deregister_project(home.path(), ID_A).unwrap(), "was present");
    assert!(
        matches!(
            resolve_root_for_project_id(home.path(), ID_A),
            Err(ResolveError::NotFound(_))
        ),
        "deregistered id must no longer resolve"
    );
    // The untouched sibling still resolves.
    assert_eq!(
        resolve_root_for_project_id(home.path(), ID_B).unwrap(),
        Path::new("/tmp/beta"),
    );
}

#[test]
fn deregister_absent_id_returns_false() {
    let home = TempDir::new().unwrap();
    register_project(home.path(), ID_A, "alpha", Path::new("/tmp/alpha"), AT).unwrap();
    assert!(
        !deregister_project(home.path(), ID_B).unwrap(),
        "absent id reports was_in_index=false, not an error"
    );
}

#[test]
fn deregister_on_absent_index_returns_false() {
    let home = TempDir::new().unwrap();
    assert!(!deregister_project(home.path(), ID_A).unwrap());
}

// --- prune_stale ---------------------------------------------------------

#[test]
fn prune_stale_removes_entries_whose_path_is_gone() {
    let home = TempDir::new().unwrap();
    let live = TempDir::new().unwrap(); // exists on disk
    register_project(home.path(), ID_A, "alpha", live.path(), AT).unwrap();
    register_project(home.path(), ID_B, "beta", Path::new("/no/such/path/ever"), AT).unwrap();

    let removed = prune_stale(home.path()).unwrap();
    assert_eq!(removed, vec![ID_B.to_string()], "only the stale id is pruned");

    let index = read_index(home.path()).unwrap();
    assert_eq!(index.len(), 1);
    assert!(index.contains_key(ID_A), "live entry survives the prune");
}

#[test]
fn prune_stale_no_op_when_all_live_returns_empty() {
    let home = TempDir::new().unwrap();
    let live = TempDir::new().unwrap();
    register_project(home.path(), ID_A, "alpha", live.path(), AT).unwrap();

    let removed = prune_stale(home.path()).unwrap();
    assert!(removed.is_empty(), "no stale entries -> empty removed list");
    assert_eq!(read_index(home.path()).unwrap().len(), 1);
}

#[test]
fn prune_stale_on_absent_index_is_empty() {
    let home = TempDir::new().unwrap();
    assert!(prune_stale(home.path()).unwrap().is_empty());
}

// --- #445 corruption regressions (folded; format makes them structural) --

#[test]
fn corrupt_index_document_yields_invalid_index_atomically() {
    // (a) Trailing junk after a valid map breaks the SINGLE JSON
    // document, so the whole index fails to parse — there is no
    // per-line scan to half-succeed. The keyed-map format makes the
    // "one bad line bricks earlier registrations" failure mode
    // impossible: the file is one document, parsed once.
    use std::fs;

    let home = TempDir::new().unwrap();
    register_project(home.path(), ID_A, "alpha", Path::new("/tmp/alpha"), AT).unwrap();

    let index = home.path().join(".verbreel/projects-index");
    let mut bytes = fs::read(&index).unwrap();
    bytes.extend_from_slice(b"this is not json");
    fs::write(&index, &bytes).unwrap();

    let err = resolve_root_for_project_id(home.path(), ID_A).unwrap_err();
    assert!(
        matches!(err, ResolveError::InvalidIndex { .. }),
        "corrupt document must surface InvalidIndex, got {err:?}"
    );
}

#[test]
fn valid_sibling_resolves_regardless_of_other_entry_contents() {
    // (b) With a map, one entry's value cannot shadow another's: each
    // id is an independent key. A lookup for ID_B succeeds even though
    // ID_A's entry has an unusual (but still valid) path — there is no
    // newest-first ordering that could let A's line block B.
    let home = TempDir::new().unwrap();
    register_project(home.path(), ID_A, "alpha", Path::new("/weird/../path"), AT).unwrap();
    register_project(home.path(), ID_B, "beta", Path::new("/tmp/beta"), AT).unwrap();

    let resolved = resolve_root_for_project_id(home.path(), ID_B).unwrap();
    assert_eq!(resolved, Path::new("/tmp/beta"));
}

#[test]
fn corrupt_document_with_no_match_is_invalid_index_not_not_found() {
    // (c) A damaged index where the requested id is absent must surface
    // InvalidIndex (index damaged), NOT NotFound (no such project), so
    // the caller can tell "project unknown" from "index unreadable".
    use std::fs;

    let home = TempDir::new().unwrap();
    let index = home.path().join(".verbreel/projects-index");
    fs::create_dir_all(index.parent().unwrap()).unwrap();
    fs::write(&index, b"{ this is not a json object").unwrap();

    let err = resolve_root_for_project_id(home.path(), ID_A).unwrap_err();
    assert!(
        matches!(err, ResolveError::InvalidIndex { .. }),
        "damaged index + absent id must surface InvalidIndex, got {err:?}"
    );
}
