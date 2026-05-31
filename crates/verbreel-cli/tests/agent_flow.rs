//! End-to-end CLI flow over the Agentic-Experience surface.
//!
//! Drives the library [`verbreel_cli::dispatch`] with captured buffers
//! (no subprocess) through the real demo path: create a project, apply a
//! plan, then read the persisted state back. Exercises the `--plan`
//! (offline, no LLM) leg so the whole vertical is verifiable without a
//! network call.

use verbreel_cli::dispatch;

/// Run `dispatch` with the given argv, asserting a 0 exit and returning
/// captured stdout.
fn ok(args: &[&str]) -> String {
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = dispatch(args.iter().copied(), &mut out, &mut err);
    assert_eq!(
        code,
        0,
        "`{}` exited {code}: {}",
        args.join(" "),
        String::from_utf8_lossy(&err)
    );
    String::from_utf8(out).expect("utf8 stdout")
}

#[test]
fn caps_lists_the_full_verb_surface() {
    let out = ok(&["verbreel", "caps", "--by-domain"]);
    assert!(out.contains("clip.trim"), "caps: {out}");
    assert!(out.contains("render.queue.add"));
    assert!(out.contains("verbs across"));
}

#[test]
fn create_then_plan_then_observe_persists() {
    let ws = tempfile::tempdir().expect("tempdir");
    let ws_path = ws.path().to_str().expect("utf8 path");

    // 1. create the project under the workspace.
    let created = ok(&[
        "verbreel",
        "project",
        "create",
        ws_path,
        "--name",
        "demo",
        "--canvas",
        "1920x1080",
    ]);
    assert!(created.contains("created project"), "create: {created}");
    let root = ws.path().join("demo");
    let root = root.to_str().expect("utf8 root");

    // 2. apply a deterministic plan (no LLM).
    let plan = ws.path().join("plan.json");
    std::fs::write(
        &plan,
        r#"{"steps":[
            {"verb":"project.rename","args":{"name":"Highlight Reel"}},
            {"verb":"track.add","args":{"kind":"video","name":"B-roll"}},
            {"verb":"marker.add","args":{"time_tk":240000,"label":"drop"}}
        ],"rationale":"rename, add b-roll track, mark the drop"}"#,
    )
    .expect("write plan");
    let applied = ok(&["verbreel", "edit", root, "--plan", plan.to_str().unwrap()]);
    assert!(applied.contains("3 step(s) applied"), "edit: {applied}");

    // 3. read the persisted state back through a fresh open.
    let info = ok(&["verbreel", "run", root, "project.info"]);
    assert!(info.contains("\"name\": \"Highlight Reel\""), "info: {info}");
    // create seeds Video 1 + Audio 1; track.add added a second video.
    assert!(info.contains("\"video\": 2"), "info: {info}");
}

#[test]
fn edit_dry_run_changes_nothing() {
    let ws = tempfile::tempdir().expect("tempdir");
    let ws_path = ws.path().to_str().expect("utf8 path");
    ok(&[
        "verbreel", "project", "create", ws_path, "--name", "dr", "--canvas", "1280x720",
    ]);
    let root = ws.path().join("dr");
    let root = root.to_str().expect("utf8 root");

    let plan = ws.path().join("plan.json");
    std::fs::write(
        &plan,
        r#"[{"verb":"project.rename","args":{"name":"should-not-stick"}}]"#,
    )
    .expect("write plan");

    let out = ok(&[
        "verbreel",
        "edit",
        root,
        "--plan",
        plan.to_str().unwrap(),
        "--dry-run",
    ]);
    assert!(out.contains("dry-run"), "dry-run output: {out}");

    // Name is still the created one — dry-run applied nothing.
    let info = ok(&["verbreel", "run", root, "project.info"]);
    assert!(info.contains("\"name\": \"dr\""), "info: {info}");
}

#[test]
fn run_rejects_unknown_verb_nonzero() {
    let ws = tempfile::tempdir().expect("tempdir");
    let ws_path = ws.path().to_str().expect("utf8 path");
    ok(&[
        "verbreel", "project", "create", ws_path, "--name", "u", "--canvas", "640x480",
    ]);
    let root = ws.path().join("u");
    let root = root.to_str().expect("utf8 root");

    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = dispatch(
        ["verbreel", "run", root, "clip.teleport"].iter().copied(),
        &mut out,
        &mut err,
    );
    assert_eq!(code, 1, "unknown verb should exit 1");
    assert!(
        String::from_utf8_lossy(&err).contains("unknown verb"),
        "stderr: {}",
        String::from_utf8_lossy(&err)
    );
}
