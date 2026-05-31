//! Integration tests for [`verbreel_storage::layout::resolve_root_for_project_id`]
//! and the §2.6 keyed-object index lifecycle helpers
//! ([`register_project`], [`list_and_prune`]).
//!
//! The index is a single JSON object keyed by `project_id` (§2.6), so a
//! lookup is an O(1) map probe — not a newest-first line scan. These
//! tests pin the register → resolve round-trip, upsert-by-id, the three
//! `ResolveError` variants, and the three corruption regressions folded
//! in from #445 (a corrupt single document fails atomically; the
//! per-line shadow bug vanishes with the format).

use std::collections::BTreeSet;
use std::path::Path;

use tempfile::TempDir;
use verbreel_storage::layout::{
    IndexEntry, ResolveError, list_and_prune, read_index, register_project,
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
    register_project(
        home.path(),
        ID_A,
        "alpha-moved",
        &new,
        "2025-02-02T00:00:00Z",
    )
    .unwrap();

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

// --- list_and_prune ------------------------------------------------------

fn no_exempt() -> BTreeSet<String> {
    BTreeSet::new()
}

#[test]
fn list_and_prune_removes_entries_whose_path_is_gone() {
    let home = TempDir::new().unwrap();
    let live = TempDir::new().unwrap(); // exists on disk
    register_project(home.path(), ID_A, "alpha", live.path(), AT).unwrap();
    register_project(
        home.path(),
        ID_B,
        "beta",
        Path::new("/no/such/path/ever"),
        AT,
    )
    .unwrap();

    let (index, removed) = list_and_prune(home.path(), &no_exempt()).unwrap();
    assert_eq!(
        removed,
        vec![ID_B.to_string()],
        "only the gone-path id is pruned"
    );
    assert_eq!(index.len(), 1, "returned map already reflects the prune");
    assert!(index.contains_key(ID_A), "live entry survives the prune");

    // The on-disk rewrite matches the returned map.
    let reread = read_index(home.path()).unwrap();
    assert_eq!(reread.len(), 1);
    assert!(reread.contains_key(ID_A));
}

#[test]
fn list_and_prune_no_op_when_all_live_returns_empty() {
    let home = TempDir::new().unwrap();
    let live = TempDir::new().unwrap();
    register_project(home.path(), ID_A, "alpha", live.path(), AT).unwrap();

    let (index, removed) = list_and_prune(home.path(), &no_exempt()).unwrap();
    assert!(removed.is_empty(), "no gone-path entries -> empty removed");
    assert_eq!(index.len(), 1);
}

#[test]
fn list_and_prune_on_absent_index_is_empty() {
    let home = TempDir::new().unwrap();
    let (index, removed) = list_and_prune(home.path(), &no_exempt()).unwrap();
    assert!(removed.is_empty());
    assert!(index.is_empty());
}

#[test]
fn list_and_prune_never_removes_an_exempt_id_even_when_path_is_gone() {
    // An open project whose path is transiently unreachable must NOT be
    // dropped from the durable index by a read-shaped verb. The engine
    // passes its open-project ids as `exempt`.
    let home = TempDir::new().unwrap();
    register_project(
        home.path(),
        ID_A,
        "alpha",
        Path::new("/no/such/path/ever"),
        AT,
    )
    .unwrap();

    let exempt: BTreeSet<String> = [ID_A.to_string()].into_iter().collect();
    let (index, removed) = list_and_prune(home.path(), &exempt).unwrap();
    assert!(
        removed.is_empty(),
        "an exempt (open) id is preserved despite a gone path"
    );
    assert!(index.contains_key(ID_A));
    // Untouched index when only an exempt entry would have been pruned.
    assert_eq!(read_index(home.path()).unwrap().len(), 1);
}

#[test]
fn list_and_prune_corrupt_index_surfaces_invalid_data_not_empty() {
    // A damaged index must surface an InvalidData IO error, never be
    // silently swallowed to an empty map (the caller emits
    // W_INDEX_UNREADABLE on this).
    use std::fs;

    let home = TempDir::new().unwrap();
    let index = home.path().join(".verbreel/projects-index");
    fs::create_dir_all(index.parent().unwrap()).unwrap();
    fs::write(&index, b"{ not a json object").unwrap();

    let err = list_and_prune(home.path(), &no_exempt()).unwrap_err();
    assert_eq!(
        err.kind(),
        std::io::ErrorKind::InvalidData,
        "corrupt index must surface InvalidData, got {err:?}"
    );
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
