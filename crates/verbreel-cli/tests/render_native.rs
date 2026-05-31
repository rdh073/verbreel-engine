//! Native `render.start` coverage — `native-render` feature only.
//!
//! Under `native-render` the generic dispatcher special-cases
//! `render.start` and routes it through the injected `verbreel-runtime`
//! instead of the Engine's v1 floor (the runtime lives outside the shared
//! Engine — same engine-vs-runtime split as the HTTP surface). These
//! tests exercise that path through the generic flag→args mapping:
//! `verbreel render start --project … --preset …` parses as the generic
//! tail, builds the args object, and reaches `render::dispatch`.
//!
//! `run_with_runtime` is the injection seam: it points the runtime at a
//! temp project root so the test never touches the user-wide index.

#![cfg(feature = "native-render")]

use clap::Parser;
use serde_json::Value;
use tempfile::TempDir;
use verbreel_cli::{Cli, run_with_runtime};
use verbreel_runtime::RenderRuntimeConfig;
use verbreel_state::{ProjectCreateArgs, ProjectId, project_create};

/// Dispatch `argv` against an injected runtime, returning
/// `(exit, parsed-stdout)`.
fn dispatch_with(runtime: &RenderRuntimeConfig, argv: &[&str]) -> (i32, Value) {
    let cli = Cli::try_parse_from(argv).expect("argv must parse");
    let mut out: Vec<u8> = Vec::new();
    let code = run_with_runtime(cli, &mut out, runtime).expect("run must not fail to write");
    let body = String::from_utf8(out).expect("stdout utf-8");
    let v: Value = serde_json::from_str(&body).unwrap_or_else(|e| panic!("not JSON: {e}\n{body}"));
    (code, v)
}

/// A real project resolved through the injected runtime, fed an unknown
/// preset, must surface the runtime's `E_RENDER_PRESET_UNKNOWN` as the
/// §0.1 envelope `code` on stdout with exit 1. This proves the native
/// path runs the runtime (not the engine floor, which would say
/// `E_RENDER_FAIL`/`E_INTERNAL`) and wraps its `E_*` code into the
/// envelope.
#[test]
fn render_start_routes_to_runtime_and_surfaces_preset_error() {
    let fixture = ProjectFixture::new();
    let runtime = RenderRuntimeConfig::new().with_project_root(&fixture.root);
    let (code, env) = dispatch_with(
        &runtime,
        &[
            "verbreel",
            "render",
            "start",
            "--project",
            &fixture.project_id.to_string(),
            "--preset",
            "definitely-not-a-preset",
            "--out_path",
            "exports/out.mp4",
            "--from_tk",
            "0",
            "--to_tk",
            "8000",
            "--overwrite",
        ],
    );
    assert_eq!(code, 1, "unknown preset must fail: {env}");
    assert_eq!(env.get("ok").and_then(Value::as_bool), Some(false));
    assert_eq!(
        env.get("code").and_then(Value::as_str),
        Some("E_RENDER_PRESET_UNKNOWN"),
        "native path must surface the runtime's preset error, got: {env}"
    );
}

/// An unresolvable project id through the runtime maps to
/// `E_PROJECT_NOT_FOUND` — the runtime resolves the root itself, so an
/// empty runtime config (no roots) yields the not-found envelope.
#[test]
fn render_start_unresolvable_project_is_not_found() {
    let runtime = RenderRuntimeConfig::new();
    let pid = ProjectId::now().to_string();
    let (code, env) = dispatch_with(
        &runtime,
        &[
            "verbreel",
            "render",
            "start",
            "--project",
            &pid,
            "--preset",
            "deterministic",
            "--out_path",
            "exports/out.mp4",
        ],
    );
    assert_eq!(code, 1, "unresolvable project must fail: {env}");
    assert_eq!(
        env.get("code").and_then(Value::as_str),
        Some("E_PROJECT_NOT_FOUND"),
        "got: {env}"
    );
}

/// Codec-enum flags (`--video_codec h264`, `--audio_codec aac`) coerce to
/// strings through the generic flag walk and deserialize into the typed
/// `RenderStartArgs` the runtime expects — the args reach the runtime
/// (which then rejects the unknown preset). A deserialize failure on a
/// bad codec would instead surface `E_SCHEMA_VIOLATION`; here the codecs
/// are valid, so the preset error is what fires, proving the codec flags
/// parsed cleanly.
#[test]
fn render_start_parses_codec_flags() {
    let fixture = ProjectFixture::new();
    let runtime = RenderRuntimeConfig::new().with_project_root(&fixture.root);
    let (code, env) = dispatch_with(
        &runtime,
        &[
            "verbreel",
            "render",
            "start",
            "--project",
            &fixture.project_id.to_string(),
            "--preset",
            "definitely-not-a-preset",
            "--out_path",
            "exports/out.mp4",
            "--video_codec",
            "h264",
            "--audio_codec",
            "aac",
            "--deterministic",
        ],
    );
    assert_eq!(code, 1, "{env}");
    assert_eq!(
        env.get("code").and_then(Value::as_str),
        Some("E_RENDER_PRESET_UNKNOWN"),
        "valid codecs must parse and reach the runtime (preset error, not schema), got: {env}"
    );
}

/// A bad codec value (`--video_codec not-a-codec`) coerces to a plain
/// string through the generic flag walk, then **fails to deserialize**
/// into the typed `RenderStartArgs` (the `RenderVideoCodec` enum rejects
/// the unknown rename). `render::dispatch` must surface that pre-runtime
/// deserialize failure as `E_SCHEMA_VIOLATION` — the value never reaches
/// the runtime. This is the §0.7/AGENTS.md "one test per `E_*` code" floor
/// for the schema-violation branch of the native dispatch.
#[test]
fn render_start_bad_codec_is_schema_violation() {
    let fixture = ProjectFixture::new();
    let runtime = RenderRuntimeConfig::new().with_project_root(&fixture.root);
    let (code, env) = dispatch_with(
        &runtime,
        &[
            "verbreel",
            "render",
            "start",
            "--project",
            &fixture.project_id.to_string(),
            "--preset",
            "deterministic",
            "--out_path",
            "exports/out.mp4",
            "--video_codec",
            "not-a-codec",
        ],
    );
    assert_eq!(code, 1, "bad codec must fail before the runtime: {env}");
    assert_eq!(env.get("ok").and_then(Value::as_bool), Some(false));
    assert_eq!(
        env.get("code").and_then(Value::as_str),
        Some("E_SCHEMA_VIOLATION"),
        "a deserialize failure on the typed args must surface E_SCHEMA_VIOLATION (not a runtime E_* code), got: {env}"
    );
}

/// `render start` with NO resolvable project (no `--project`, no
/// active-project file) must surface `E_PROJECT_NOT_FOUND` — the same code
/// every other project-scoped verb returns for a missing project context
/// (§0.7 uniform error codes; cf. `clip add` → `E_PROJECT_NOT_FOUND`).
///
/// Before the fix the missing `project_id` reached the typed
/// `RenderStartArgs` deserialize and surfaced `E_SCHEMA_VIOLATION` (a
/// required-field error), diverging from the engine path. The CLI now
/// resolves the project-presence guard BEFORE the deserialize.
#[test]
fn render_start_without_project_is_project_not_found() {
    // Empty runtime, hermetic home: no --project flag and no active-project
    // file, so no project_id is injected. With VERBREEL_HOME unset the
    // active-project lookup yields nothing.
    let runtime = RenderRuntimeConfig::new();
    let prev = std::env::var_os("VERBREEL_HOME");
    // SAFETY: this single-threaded test owns the env for its duration.
    unsafe {
        std::env::remove_var("VERBREEL_HOME");
    }
    let (code, env) = dispatch_with(
        &runtime,
        &[
            "verbreel",
            "render",
            "start",
            "--preset",
            "deterministic",
            "--out_path",
            "exports/out.mp4",
        ],
    );
    unsafe {
        if let Some(v) = prev {
            std::env::set_var("VERBREEL_HOME", v);
        }
    }
    assert_eq!(code, 1, "no project context must fail: {env}");
    assert_eq!(env.get("ok").and_then(Value::as_bool), Some(false));
    assert_eq!(
        env.get("code").and_then(Value::as_str),
        Some("E_PROJECT_NOT_FOUND"),
        "no resolvable project must match the engine path's code, not \
         E_SCHEMA_VIOLATION from a missing-field deserialize, got: {env}"
    );
}

struct ProjectFixture {
    _tmp: TempDir,
    root: std::path::PathBuf,
    project_id: ProjectId,
}

impl ProjectFixture {
    fn new() -> Self {
        let tmp = TempDir::new().expect("temp dir");
        let create_data = project_create(&ProjectCreateArgs {
            name: "cli-render-project".to_string(),
            canvas: "64x64".to_string(),
            fps_num: Some(30),
            fps_den: Some(1),
            at: Some(tmp.path().to_path_buf()),
            activate: false,
            metadata: serde_json::Map::new(),
        })
        .expect("project create");
        Self {
            _tmp: tmp,
            root: create_data.path,
            project_id: create_data.project_id,
        }
    }
}
