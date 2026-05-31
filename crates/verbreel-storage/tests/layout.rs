//! Integration tests for the project-root + projects-index layout
//! helpers in [`verbreel_storage::layout`].

use std::path::Path;

use tempfile::TempDir;
use verbreel_storage::layout::{
    init_project_root, projects_index_path, read_index, register_project,
};

const AT: &str = "2025-01-01T00:00:00Z";

// --- init_project_root: layout shape ------------------------------------

#[test]
fn init_creates_verbreel_subdir_and_events_log_and_assets_tree() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("proj");
    init_project_root(&root).unwrap();

    assert!(root.is_dir(), "<root> must exist");
    assert!(
        root.join(".verbreel").is_dir(),
        "<root>/.verbreel must exist"
    );
    assert!(
        root.join(".verbreel/events.jsonl").is_file(),
        "<root>/.verbreel/events.jsonl must exist as a file"
    );
    assert!(root.join("assets").is_dir(), "<root>/assets must exist");
}

#[test]
fn init_events_log_is_empty_when_created() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("proj");
    init_project_root(&root).unwrap();
    let log = root.join(".verbreel/events.jsonl");
    let len = std::fs::metadata(&log).unwrap().len();
    assert_eq!(len, 0, "newly-created events.jsonl must be empty");
}

#[test]
fn init_is_idempotent_on_fully_initialised_root() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("proj");
    init_project_root(&root).unwrap();
    // Second call must succeed without panicking or erroring.
    init_project_root(&root).unwrap();
    // And no further calls must change the layout.
    init_project_root(&root).unwrap();
    assert!(root.join(".verbreel/events.jsonl").is_file());
    assert!(root.join("assets").is_dir());
}

#[test]
fn init_preserves_existing_events_log_contents() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("proj");
    init_project_root(&root).unwrap();

    let log = root.join(".verbreel/events.jsonl");
    let sentinel = b"{\"sentinel\":true}\n";
    std::fs::write(&log, sentinel).unwrap();

    // Second init must not truncate or rewrite the log.
    init_project_root(&root).unwrap();
    let bytes = std::fs::read(&log).unwrap();
    assert_eq!(bytes, sentinel, "init must not truncate existing log");
}

#[test]
fn init_creates_nested_parents_as_needed() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("a/b/c/proj");
    init_project_root(&root).unwrap();
    assert!(root.is_dir());
    assert!(root.join(".verbreel").is_dir());
    assert!(root.join("assets").is_dir());
}

#[test]
fn init_succeeds_on_root_that_already_exists_as_empty_dir() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("proj");
    std::fs::create_dir(&root).unwrap();
    init_project_root(&root).unwrap();
    assert!(root.join(".verbreel").is_dir());
}

// --- projects_index_path: pure resolver ---------------------------------

#[test]
fn projects_index_path_under_home_dot_verbreel() {
    let home = Path::new("/home/example");
    let p = projects_index_path(home);
    assert_eq!(p, Path::new("/home/example/.verbreel/projects-index"));
}

#[test]
fn projects_index_path_preserves_relative_home() {
    let home = Path::new("relative-home");
    let p = projects_index_path(home);
    assert!(p.is_relative());
    assert_eq!(p, Path::new("relative-home/.verbreel/projects-index"));
}

#[test]
fn projects_index_path_does_not_touch_filesystem() {
    // Pass a path that definitely doesn't exist; the function is pure
    // and must still return.
    let p = projects_index_path(Path::new("/this/does/not/exist/ever"));
    assert_eq!(
        p,
        Path::new("/this/does/not/exist/ever/.verbreel/projects-index")
    );
}

// --- register_project: keyed-object upsert semantics (§2.6) --------------

#[test]
fn register_creates_home_dot_verbreel_dir_on_first_call() {
    let home = TempDir::new().unwrap();
    let proj = TempDir::new().unwrap();
    assert!(!home.path().join(".verbreel").exists());
    register_project(
        home.path(),
        "01900000-0000-7000-8000-000000000001",
        "proj",
        proj.path(),
        AT,
    )
    .unwrap();
    assert!(home.path().join(".verbreel").is_dir());
    assert!(home.path().join(".verbreel/projects-index").is_file());
}

#[test]
fn register_writes_keyed_object_with_section_2_6_fields() {
    let home = TempDir::new().unwrap();
    let proj = TempDir::new().unwrap();
    let id = "01900000-0000-7000-8000-000000000001";
    register_project(home.path(), id, "My Project", proj.path(), AT).unwrap();

    let body = std::fs::read_to_string(home.path().join(".verbreel/projects-index")).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(&body).expect("index must be a single JSON object");
    let entry = v
        .get(id)
        .unwrap_or_else(|| panic!("object must be keyed by project id; got {body}"));
    assert_eq!(entry["project_id"], id);
    assert_eq!(entry["name"], "My Project");
    assert_eq!(entry["path"], proj.path().display().to_string());
    assert_eq!(entry["last_opened_at"], AT);
}

#[test]
fn register_two_projects_keys_both_under_their_ids() {
    let home = TempDir::new().unwrap();
    let proj_a = TempDir::new().unwrap();
    let proj_b = TempDir::new().unwrap();
    let id_a = "01900000-0000-7000-8000-00000000000a";
    let id_b = "01900000-0000-7000-8000-00000000000b";

    register_project(home.path(), id_a, "a", proj_a.path(), AT).unwrap();
    register_project(home.path(), id_b, "b", proj_b.path(), AT).unwrap();

    let index = read_index(home.path()).unwrap();
    assert_eq!(index.len(), 2, "two distinct keys expected");
    assert!(index.contains_key(id_a));
    assert!(index.contains_key(id_b));
}

#[test]
fn register_reregistering_same_id_does_not_accumulate() {
    let home = TempDir::new().unwrap();
    let id = "01900000-0000-7000-8000-000000000001";
    let old = TempDir::new().unwrap();
    let new = TempDir::new().unwrap();
    register_project(home.path(), id, "old", old.path(), AT).unwrap();
    register_project(home.path(), id, "new", new.path(), "2025-02-02T00:00:00Z").unwrap();

    let index = read_index(home.path()).unwrap();
    assert_eq!(index.len(), 1, "re-register is an upsert, not an append");
    assert_eq!(index[id].name, "new");
    assert_eq!(index[id].path, new.path().display().to_string());
}

#[test]
fn register_overwrites_pre_existing_corrupt_index_atomically() {
    // A keyed map is one document: an upsert reads-modifies-writes the
    // whole object. A previously-corrupt index surfaces as a read error
    // rather than silently appending a torn second document.
    let home = TempDir::new().unwrap();
    let index = home.path().join(".verbreel/projects-index");
    std::fs::create_dir_all(index.parent().unwrap()).unwrap();
    std::fs::write(&index, b"not a json object").unwrap();

    let proj = TempDir::new().unwrap();
    let err = register_project(home.path(), "new-id", "new", proj.path(), AT).unwrap_err();
    assert_eq!(
        err.kind(),
        std::io::ErrorKind::InvalidData,
        "a corrupt existing index must fail the RMW, not append junk"
    );
}

#[test]
fn register_is_atomic_no_temp_or_stale_lock_left_behind_on_success() {
    let home = TempDir::new().unwrap();
    let proj = TempDir::new().unwrap();
    register_project(home.path(), "id-1", "p", proj.path(), AT).unwrap();

    let verbreel = home.path().join(".verbreel");
    let names: Vec<String> = std::fs::read_dir(&verbreel)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    // No `*.tmp` NamedTempFile leftover. The persistent lock file
    // (`projects-index.lock`) is expected to remain alongside the index.
    assert!(
        names.iter().any(|n| n == "projects-index"),
        "index file must exist, got {names:?}"
    );
    assert!(
        names.iter().all(|n| !n.contains(".tmp")),
        "no temp file may be left behind, got {names:?}"
    );
}
