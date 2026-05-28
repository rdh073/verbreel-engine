//! Integration tests for the project-root + projects-index layout
//! helpers in [`verbreel_storage::layout`].

use std::path::Path;

use tempfile::TempDir;
use verbreel_storage::layout::{init_project_root, projects_index_path, register_project};

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

// --- register_project: append semantics ---------------------------------

#[test]
fn register_creates_home_dot_verbreel_dir_on_first_call() {
    let home = TempDir::new().unwrap();
    let proj = TempDir::new().unwrap();
    assert!(!home.path().join(".verbreel").exists());
    register_project(
        home.path(),
        "01900000-0000-7000-8000-000000000001",
        proj.path(),
    )
    .unwrap();
    assert!(home.path().join(".verbreel").is_dir());
    assert!(home.path().join(".verbreel/projects-index").is_file());
}

#[test]
fn register_writes_single_json_line_with_id_and_path() {
    let home = TempDir::new().unwrap();
    let proj = TempDir::new().unwrap();
    let id = "01900000-0000-7000-8000-000000000001";
    register_project(home.path(), id, proj.path()).unwrap();

    let body = std::fs::read_to_string(home.path().join(".verbreel/projects-index")).unwrap();
    let line = body.trim_end_matches('\n');
    assert!(
        line.contains(id),
        "line must mention the project id: {line}"
    );
    assert!(
        line.contains(&proj.path().display().to_string()),
        "line must mention the project path: {line}"
    );
    assert!(body.ends_with('\n'), "line must end with newline");
    assert_eq!(body.lines().count(), 1, "exactly one entry");
}

#[test]
fn register_two_projects_appends_both_lines() {
    let home = TempDir::new().unwrap();
    let proj_a = TempDir::new().unwrap();
    let proj_b = TempDir::new().unwrap();
    let id_a = "01900000-0000-7000-8000-00000000000a";
    let id_b = "01900000-0000-7000-8000-00000000000b";

    register_project(home.path(), id_a, proj_a.path()).unwrap();
    register_project(home.path(), id_b, proj_b.path()).unwrap();

    let body = std::fs::read_to_string(home.path().join(".verbreel/projects-index")).unwrap();
    let lines: Vec<&str> = body.lines().collect();
    assert_eq!(lines.len(), 2, "two entries expected; body was: {body}");
    assert!(lines[0].contains(id_a), "first line must hold id_a");
    assert!(lines[1].contains(id_b), "second line must hold id_b");
}

#[test]
fn register_lines_are_valid_json() {
    let home = TempDir::new().unwrap();
    let proj = TempDir::new().unwrap();
    register_project(
        home.path(),
        "01900000-0000-7000-8000-000000000001",
        proj.path(),
    )
    .unwrap();

    let body = std::fs::read_to_string(home.path().join(".verbreel/projects-index")).unwrap();
    for line in body.lines() {
        let v: serde_json::Value = serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("line must parse as JSON, got error: {e} for: {line}"));
        assert!(v.get("id").is_some(), "json line must have 'id' field");
        assert!(v.get("path").is_some(), "json line must have 'path' field");
    }
}

#[test]
fn register_recovers_when_existing_file_lacks_trailing_newline() {
    let home = TempDir::new().unwrap();
    let index = home.path().join(".verbreel/projects-index");
    std::fs::create_dir_all(index.parent().unwrap()).unwrap();
    // Simulate a crash mid-write — last line has no trailing newline.
    std::fs::write(&index, b"{\"id\":\"old\",\"path\":\"/old\"}").unwrap();

    let proj = TempDir::new().unwrap();
    register_project(home.path(), "new-id", proj.path()).unwrap();

    let body = std::fs::read_to_string(&index).unwrap();
    let lines: Vec<&str> = body.lines().collect();
    assert_eq!(
        lines.len(),
        2,
        "must produce 2 well-formed lines; body: {body}"
    );
    assert!(lines[0].contains("old"));
    assert!(lines[1].contains("new-id"));
}

#[test]
fn register_is_atomic_no_temp_file_left_behind_on_success() {
    let home = TempDir::new().unwrap();
    let proj = TempDir::new().unwrap();
    register_project(home.path(), "id-1", proj.path()).unwrap();

    let verbreel = home.path().join(".verbreel");
    let entries: Vec<_> = std::fs::read_dir(&verbreel)
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    // After atomic_write_bytes persists, only `projects-index` should
    // remain — no `*.tmp` or NamedTempFile leftover.
    assert_eq!(
        entries.len(),
        1,
        "exactly one file expected in .verbreel/, got {entries:?}"
    );
    let name = entries[0].file_name();
    assert_eq!(name.to_string_lossy(), "projects-index");
}
