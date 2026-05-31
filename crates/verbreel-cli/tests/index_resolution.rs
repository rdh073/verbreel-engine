//! Production-path coverage closing the review's [P2-1] / [P2-2] /
//! [P2-3] gaps. None of these use the test-only `register_project`
//! bridge — they exercise the real id→root resolution that R1 (engine
//! registers a project in the projects-index on `create`/`open`) now
//! makes work end to end.
//!
//! - [P2-1] `real_create_then_fresh_engine_resolves_scoped_verb` — a
//!   project created through one `Engine`/dispatch resolves from a
//!   *separate* fresh `Engine` (a new one-shot CLI process, same
//!   `VERBREEL_HOME`), proving the headline dispatch path is functional
//!   in production now that `project.create` populates the index.
//! - [P2-2] `render_start_floor_errors_deterministically` — base
//!   (no-feature) `render.start` through `Engine::dispatch` errors
//!   deterministically at the v1 floor; this is the contract the PR's
//!   `render.rs` deletion was justified by.
//! - [P2-3] `corrupt_index_line_yields_e_io` — a garbage projects-index
//!   line makes `resolve_root_for_project_id` return
//!   `ResolveError::InvalidIndex`, which the CLI maps to `E_IO`.

mod common;

use clap::Parser;
use serde_json::Value;
use verbreel_cli::{Cli, run};

/// Parse argv and dispatch under the *current* `VERBREEL_HOME`, returning
/// `(exit, parsed-stdout)`. Unlike `project.rs`'s helper this does NOT
/// install its own temp home — callers wrap the whole flow in a single
/// `with_home` so multiple dispatches share one home (a real one-shot CLI
/// runs each invocation as a fresh process against the same `.verbreel`).
fn dispatch(argv: &[&str]) -> (i32, Value) {
    let cli = Cli::try_parse_from(argv).expect("argv must parse");
    let mut out: Vec<u8> = Vec::new();
    let code = run(cli, &mut out).expect("run must not fail to write");
    let body = String::from_utf8(out).expect("stdout utf-8");
    let v: Value = serde_json::from_str(&body).unwrap_or_else(|e| panic!("not JSON: {e}\n{body}"));
    (code, v)
}

/// [P2-1] The headline production path: create a project (which now
/// registers it in the index, per R1), then in a *separate* dispatch —
/// modeling a fresh one-shot CLI process with an empty engine open-map
/// but the same `VERBREEL_HOME` — target a project-scoped verb by id and
/// assert it resolves the root from the index and succeeds.
///
/// This is the real flow `verbreel project create` → `verbreel clip list
/// --project <id>` with NO test-only `register_project` bridge: each
/// `dispatch` call builds its own `Engine`, so the second one starts cold
/// and must resolve the project entirely from the on-disk index that the
/// first dispatch wrote.
#[test]
fn real_create_then_fresh_engine_resolves_scoped_verb() {
    common::with_home(|home| {
        let dest = home.join("real-project");
        // First one-shot process: create the project. R1 makes the engine
        // append the id→root entry to <home>/.verbreel/projects-index.
        let (create_code, create_env) = dispatch(&[
            "verbreel",
            "project",
            "create",
            "--name",
            "real",
            "--canvas",
            "64x64",
            "--at",
            dest.to_str().unwrap(),
        ]);
        assert_eq!(create_code, 0, "create must succeed: {create_env}");
        let id = create_env
            .pointer("/data/project_id")
            .and_then(Value::as_str)
            .expect("create envelope must carry data.project_id")
            .to_string();

        // Second one-shot process: a *fresh* engine (its open-map is
        // empty) must resolve `id` from the index alone. A project-scoped
        // read (`clip list`) succeeding proves index resolution works in
        // production — no `register_project` bridge involved.
        let (read_code, read_env) = dispatch(&["verbreel", "clip", "list", "--project", &id]);
        assert_eq!(
            read_code, 0,
            "fresh-engine scoped read must resolve from the index and succeed: {read_env}"
        );
        assert_eq!(read_env.get("ok").and_then(Value::as_bool), Some(true));
        // It must NOT be the "could not resolve" failure that this whole
        // test exists to rule out.
        assert_ne!(
            read_env.get("code").and_then(Value::as_str),
            Some("E_PROJECT_NOT_FOUND"),
            "scoped verb must resolve the project from the index, got: {read_env}"
        );
    });
}

/// [P2-2] Base-build `render.start` through the Engine floor.
///
/// Without `native-render` the runtime is absent and `render.start`
/// dispatches to the engine's v1 floor, which always errors. The floor
/// verb returns `RenderStartError::RenderFail` (its message embeds the
/// spec code `E_RENDER_FAIL`), and the engine maps a `VerbError::Custom`
/// to the envelope `code` `E_INTERNAL` (`verb_error_code`). So the stable
/// public contract the PR's `render.rs` deletion rests on is: exit 1,
/// `ok:false`, the `E_RENDER_FAIL` string surfaced in `message`. Asserting
/// both pins the behavior so a registry change to `render.start` cannot
/// silently break it.
///
/// `render.start` is project-scoped, so the project is created+resolved
/// first; the floor error is reached only *after* a real project resolves.
///
/// Gated to the **base** build: under `native-render` `render.start`
/// routes through the runtime (not the Engine floor), so the floor
/// contract this asserts no longer applies — `render_native.rs` covers
/// the native path instead.
#[cfg(not(feature = "native-render"))]
#[test]
fn render_start_floor_errors_deterministically() {
    common::with_home(|home| {
        let dest = home.join("render-floor-project");
        let (create_code, create_env) = dispatch(&[
            "verbreel",
            "project",
            "create",
            "--name",
            "render-floor",
            "--canvas",
            "64x64",
            "--at",
            dest.to_str().unwrap(),
        ]);
        assert_eq!(create_code, 0, "create must succeed: {create_env}");
        let id = create_env
            .pointer("/data/project_id")
            .and_then(Value::as_str)
            .expect("data.project_id")
            .to_string();

        let (code, env) = dispatch(&[
            "verbreel",
            "render",
            "start",
            "--project",
            &id,
            "--preset",
            "any-preset",
            "--out_path",
            "exports/out.mp4",
        ]);
        assert_eq!(code, 1, "v1-floor render.start must fail: {env}");
        assert_eq!(env.get("ok").and_then(Value::as_bool), Some(false));
        // The envelope code is `E_INTERNAL` (the floor's `VerbError::Custom`
        // mapping), and the `E_RENDER_FAIL` spec code is carried in the
        // message — the stable, observable v1 contract.
        assert_eq!(
            env.get("code").and_then(Value::as_str),
            Some("E_INTERNAL"),
            "v1 floor maps render.start's Custom error to E_INTERNAL, got: {env}"
        );
        let message = env.get("message").and_then(Value::as_str).unwrap_or("");
        assert!(
            message.contains("E_RENDER_FAIL"),
            "floor message must surface the E_RENDER_FAIL spec code, got: {env}"
        );
    });
}

/// [P2-3] A corrupt projects-index line surfaces as `E_IO`.
///
/// `resolve_root_for_project_id` aborts on a non-JSON index line with
/// `ResolveError::InvalidIndex` (a hand-corrupted index is a hard error,
/// not a silent miss). `context::resolve_error_to_envelope` maps both
/// `Io` and `InvalidIndex` to `E_IO`. Writing garbage into the index and
/// then dispatching a project-scoped verb with `--project` exercises that
/// branch — the only `E_*` code in the CLI that previously had no test.
#[test]
fn corrupt_index_line_yields_e_io() {
    common::with_home(|home| {
        // Write a garbage (non-JSON) line into the projects index. The
        // index lives at <home>/.verbreel/projects-index.
        let dir = home.join(".verbreel");
        std::fs::create_dir_all(&dir).expect("create .verbreel");
        std::fs::write(dir.join("projects-index"), "this is not json\n")
            .expect("write corrupt index");

        // Any project id triggers a resolve scan; the scan hits the
        // garbage line and returns InvalidIndex before NotFound.
        let (code, env) = dispatch(&[
            "verbreel",
            "clip",
            "list",
            "--project",
            "00000000-0000-0000-0000-000000000000",
        ]);
        assert_eq!(code, 1, "corrupt index must fail dispatch: {env}");
        assert_eq!(env.get("ok").and_then(Value::as_bool), Some(false));
        assert_eq!(
            env.get("code").and_then(Value::as_str),
            Some("E_IO"),
            "InvalidIndex must map to E_IO, got: {env}"
        );
    });
}
