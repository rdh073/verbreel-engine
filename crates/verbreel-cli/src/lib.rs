//! verbreel-cli — generic `<noun> <verb> [--flag value | --args json]`
//! dispatcher over the shared [`verbreel_state::Engine`].
//!
//! Per the crate dependency rule in `CLAUDE.md`:
//!
//! ```text
//! verbreel-cli → verbreel-state, verbreel-storage
//! verbreel-cli/native-render → verbreel-runtime
//! ```
//!
//! ## Composition root
//!
//! This file wires the pieces; it does not implement them. The four
//! concerns live in their own modules:
//!
//! - [`args`] — flag → args-JSON mapping (scalar coercion, `--args`
//!   escape hatch).
//! - [`duration`] — §0.2 duration-string → integer ticks.
//! - [`home`] — `.verbreel` home resolution + the §0.12 active-project
//!   file (read fallback + the `--activate` writer).
//! - [`context`] — verb-id reconstruction + §0.12 project context +
//!   the one-shot pre-open step.
//!
//! Under `native-render` a fifth module, [`render`], owns the one verb
//! the shared Engine cannot execute — `render.start`, whose ffmpeg
//! runtime lives in `verbreel-runtime`, injected at this composition
//! root (mirroring the HTTP surface). Every *other* verb, including the
//! v1 `render.start` floor, flows through `Engine::dispatch`.
//!
//! ## One generic path
//!
//! There are **no enumerated subcommands**. clap captures the whole
//! invocation tail; the dispatcher reconstructs the verb id, builds the
//! args object, resolves project context, and calls
//! [`Engine::dispatch`] once. Adding a verb to the registry requires
//! **zero** edits here — that is the inversion this surface buys.
//!
//! ## Exit codes (§0.1)
//!
//! - `0` — `ok:true`.
//! - `1` — `ok:false` (the §0.1 envelope is printed to **stdout**; the
//!   envelope *is* the error report).
//! - `2` — argument parse failure **before** the engine is invoked
//!   (clap-level: `--help` / `--version` render their own text; a bare
//!   `verbreel` prints help via `arg_required_else_help`).

#![deny(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]

use std::io::Write;

use clap::Parser;
use serde_json::{Map, Value};
use verbreel_state::{Engine, Envelope};

pub mod args;
pub mod context;
pub mod duration;
pub mod home;
#[cfg(feature = "native-render")]
pub mod render;

/// Top-level `clap` parser for the `verbreel` binary.
///
/// `trailing_var_arg` + `allow_hyphen_values` capture the entire
/// invocation tail (`noun verb --flag value …`) as raw strings — a
/// generic dispatcher needs to survive unknown flags, which a typed
/// clap surface cannot. `arg_required_else_help` keeps bare `verbreel`
/// printing help (exit 2) instead of dispatching an empty tail.
#[derive(Debug, Parser)]
#[command(
    name = "verbreel",
    version,
    about = "Verbreel engine CLI — verbreel <noun> <verb> [--flag value | --args json]",
    long_about = None,
    propagate_version = true,
    arg_required_else_help = true
)]
pub struct Cli {
    /// `<noun> <verb> [--flag value ...]` — the full invocation tail.
    #[arg(
        trailing_var_arg = true,
        allow_hyphen_values = true,
        value_name = "ARGS"
    )]
    pub raw: Vec<String>,
}

/// Dispatch a parsed [`Cli`], writing the §0.1 envelope to `out` and
/// returning the process exit code (`0` on `ok:true`, `1` on `ok:false`).
///
/// Tests pass `&mut Vec<u8>` as `out`; the binary passes a locked stdout
/// handle.
///
/// # Errors
///
/// This function does not bubble errors — every failure (bad args,
/// unknown verb, missing project) is rendered as a §0.1 `ok:false`
/// envelope on `out` with exit code 1. The `Result` is retained for
/// signature compatibility with the binary entrypoint and only carries
/// a write failure on `out`.
// `cli` is taken by value: this is the frozen public signature `main.rs`
// and the integration tests call (owned-command hand-off at the
// transport boundary, mirroring `Engine::dispatch`). The body only needs
// to borrow `raw`, so the lint fires, but changing the signature to
// `&Cli` would break the brief-pinned contract.
#[cfg(not(feature = "native-render"))]
#[allow(clippy::needless_pass_by_value)]
pub fn run(cli: Cli, out: &mut dyn Write) -> anyhow::Result<i32> {
    run_inner(&cli.raw, out)
}

/// Dispatch a parsed [`Cli`] with the default process render runtime.
///
/// Under `native-render` the only verb the shared Engine cannot execute
/// is `render.start`; this wires up the default runtime from the
/// environment and hands it to [`run_with_runtime`].
///
/// # Errors
///
/// As [`run_with_runtime`] — only a write failure on `out`.
#[cfg(feature = "native-render")]
#[allow(clippy::needless_pass_by_value)]
pub fn run(cli: Cli, out: &mut dyn Write) -> anyhow::Result<i32> {
    run_with_runtime(cli, out, &verbreel_runtime::RenderRuntimeConfig::from_env())
}

/// Dispatch a parsed [`Cli`] with an injected render runtime
/// (`native-render` only).
///
/// Tests use this to point `render.start` at a temp project root instead
/// of the user-wide index — the same injection seam the HTTP surface
/// exposes via `AppState::with_render_runtime`.
///
/// # Errors
///
/// This function does not bubble dispatch errors — every failure is a
/// §0.1 `ok:false` envelope on `out`. The `Result` only carries a write
/// failure on `out`.
#[cfg(feature = "native-render")]
#[allow(clippy::needless_pass_by_value)]
pub fn run_with_runtime(
    cli: Cli,
    out: &mut dyn Write,
    runtime: &verbreel_runtime::RenderRuntimeConfig,
) -> anyhow::Result<i32> {
    run_inner(&cli.raw, out, runtime)
}

/// Core dispatch: reconstruct id, build args, resolve context, dispatch.
fn run_inner(
    raw: &[String],
    out: &mut dyn Write,
    #[cfg(feature = "native-render")] runtime: &verbreel_runtime::RenderRuntimeConfig,
) -> anyhow::Result<i32> {
    let home = home::resolve_home();
    let mut engine = Engine::new(
        home.clone()
            .unwrap_or_else(|| std::path::PathBuf::from(".")),
    );

    // Positionals: noun is required (the tail is non-empty per
    // arg_required_else_help); verb is optional (flat meta verbs).
    let Some(noun) = raw.first() else {
        // Unreachable through dispatch() (arg_required_else_help guards an
        // empty tail), but run() may be called directly by a test.
        let env = err_envelope("E_BAD_ARGS", "missing <noun>");
        return print_envelope(out, &env, 1);
    };
    // The verb positional is the second token only if it is not a flag.
    let verb = raw
        .get(1)
        .filter(|t| !t.starts_with('-'))
        .map(String::as_str);

    let verb_id = context::reconstruct_verb_id(&engine, noun, verb);

    // The flag tail begins after the positionals actually consumed.
    let tail_start = if verb.is_some() { 2 } else { 1 };
    let tail = &raw[tail_start.min(raw.len())..];

    let parsed = match args::parse_flags(tail) {
        Ok(p) => p,
        Err(e) => {
            let env = err_envelope(&e.code, e.message);
            return print_envelope(out, &env, 1);
        }
    };

    let mut args_map = parsed.args;

    // Native `render.start` — the one verb the shared Engine cannot run.
    // Its ffmpeg runtime resolves the project root itself (its own
    // index-scanning resolver), so it does NOT go through the engine
    // pre-open path below: instead `project_id` (from --project or the
    // active-project file) is injected and the injected runtime executes
    // it. Without `native-render` this block is absent and `render.start`
    // falls through to `Engine::dispatch`'s `E_RENDER_FAIL` v1 floor.
    #[cfg(feature = "native-render")]
    if verb_id == "render.start" {
        let active = home.as_deref().and_then(home::read_active_project);
        let env = match parsed.project.clone().or(active) {
            Some(project_id) => {
                args_map.insert("project_id".to_string(), Value::String(project_id));
                render::dispatch(&args_map, runtime)
            }
            // §0.7 code alignment: no resolvable project must surface
            // E_PROJECT_NOT_FOUND like every other project-scoped verb. The
            // presence guard runs BEFORE the typed RenderStartArgs
            // deserialize — otherwise the missing required `project_id`
            // field surfaces as E_SCHEMA_VIOLATION, diverging from the
            // engine path (`clip add` etc. → E_PROJECT_NOT_FOUND).
            None => err_envelope(
                "E_PROJECT_NOT_FOUND",
                "no project context: pass --project <id> or set an active project",
            ),
        };
        let code = i32::from(!env.is_ok());
        return print_envelope(out, &env, code);
    }

    // §0.12 project context — only for *known* project-scoped verbs.
    // An unknown id skips context resolution and goes straight to the
    // engine, which returns the §0.1 `E_UNKNOWN_VERB` envelope — so an
    // unknown verb never masquerades as `E_PROJECT_NOT_FOUND`. Lifecycle
    // globals (project.create/open/list) and flat meta verbs also take no
    // injected project_id.
    let known = engine.verb_ids().iter().any(|v| v == &verb_id);
    if known && context::is_project_scoped(&verb_id) {
        let active = home.as_deref().and_then(home::read_active_project);
        if let Err(env) = context::ensure_project_context(
            &mut engine,
            home.as_deref(),
            parsed.project.as_deref(),
            active,
            &mut args_map,
        ) {
            return print_envelope(out, &env, 1);
        }
    } else if let Some(project) = &parsed.project {
        // A --project flag on a global verb (e.g. project.open passes a
        // path, not an id) is still forwarded verbatim as project_id so
        // verbs that read it (e.g. project.save) see it. The engine
        // ignores it where irrelevant.
        args_map.insert("project_id".to_string(), Value::String(project.clone()));
    }

    let mut env = engine.dispatch(&verb_id, Value::Object(args_map.clone()));
    hint_help_topic(&verb_id, noun, verb, &mut env);

    // §0.12 active-project writer: after a successful create/open that
    // carried `activate:true`, persist the returned id so a later
    // invocation can resolve an omitted --project. CLI-front-end concern
    // the engine explicitly defers (project_open.rs:81).
    maybe_write_active_project(home.as_deref(), &env, &args_map);

    let code = i32::from(!env.is_ok());
    print_envelope(out, &env, code)
}

/// Persist the active project after a successful `--activate`d
/// create/open. A write failure is non-fatal — the edit already
/// succeeded and is persisted; only the next-run ergonomic fallback is
/// unavailable. No-op when the verb failed, `--activate` was absent, the
/// home is unknown, or the envelope carries no `project_id`.
fn maybe_write_active_project(
    home: Option<&std::path::Path>,
    env: &Envelope,
    args: &Map<String, Value>,
) {
    if !env.is_ok() || !activate_requested(args) {
        return;
    }
    let (Some(home), Some(id)) = (home, project_id_from_data(env)) else {
        return;
    };
    let _ = home::write_active_project(home, &id);
}

/// `true` if the args requested `--activate` (a bare boolean flag
/// coerces to JSON `true`).
fn activate_requested(args: &Map<String, Value>) -> bool {
    args.get("activate")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Pull `data.project_id` (a string) from a successful envelope, if
/// present — used by the active-project writer.
fn project_id_from_data(env: &Envelope) -> Option<String> {
    match env {
        Envelope::Ok { data, .. } => data
            .get("project_id")
            .and_then(Value::as_str)
            .map(str::to_string),
        Envelope::Err { .. } => None,
    }
}

/// Build an `ok:false` envelope from a code + message. The engine's own
/// `Envelope::err` constructor is private, but the variant fields are
/// public, so the CLI constructs the §0.1 shape directly.
fn err_envelope(code: &str, message: impl Into<String>) -> Envelope {
    Envelope::Err {
        code: code.to_string(),
        message: message.into(),
        hint: None,
        details: None,
    }
}

/// Attach a `--topic` hint when `verbreel help <topic>` is mis-parsed.
///
/// `help` is a flat meta verb; a trailing positional (`verbreel help clip`)
/// is read as a verb token, so [`context::reconstruct_verb_id`] produces
/// the id `help.clip`, which the engine rejects with `E_UNKNOWN_VERB`. The
/// user meant the `topic?` arg, spelled as a flag on this generic surface
/// (`verbreel help --topic clip`). The engine cannot know the CLI's flag
/// spelling, so the hint is a surface concern grafted onto the existing
/// `E_UNKNOWN_VERB` envelope (no new code, no swallowed error).
fn hint_help_topic(verb_id: &str, noun: &str, verb: Option<&str>, env: &mut Envelope) {
    let (Some(topic), Envelope::Err { code, hint, .. }) = (verb, &mut *env) else {
        return;
    };
    if noun == "help" && verb_id.starts_with("help.") && code == "E_UNKNOWN_VERB" {
        *hint = Some(format!(
            "did you mean a help topic? try `verbreel help --topic {topic}`"
        ));
    }
}

/// Print the §0.1 envelope pretty + newline-terminated, returning `code`.
fn print_envelope(out: &mut dyn Write, env: &Envelope, code: i32) -> anyhow::Result<i32> {
    let rendered = serde_json::to_string_pretty(&env.to_json())?;
    writeln!(out, "{rendered}")?;
    Ok(code)
}

/// Full parse + dispatch + error-formatting loop. The binary calls this
/// with the process's argv + stdout + stderr; tests call it with
/// captured buffers.
///
/// Returns the process exit code:
/// - `0` on `ok:true`, `1` on `ok:false` (envelope on `out`).
/// - The clap-reported exit code (**2**, or `--help`/`--version`'s own
///   code) on a parse failure, with clap's formatted text on `err`.
/// - `1` on a write failure bubbled from [`run`] (`error: <detail>` on
///   `err`).
pub fn dispatch<I, T>(argv: I, out: &mut dyn Write, err: &mut dyn Write) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    let cli = match Cli::try_parse_from(argv) {
        Ok(c) => c,
        Err(parse_err) => {
            // Clap errors carry their own pre-formatted help/error text
            // (including the bare-`verbreel` → print-help path enabled by
            // arg_required_else_help). Render to `err`; tests assert on
            // the real stderr emission.
            let _ = write!(err, "{parse_err}");
            return parse_err.exit_code();
        }
    };
    match run(cli, out) {
        Ok(code) => code,
        Err(e) => {
            let _ = writeln!(err, "error: {e}");
            1
        }
    }
}
