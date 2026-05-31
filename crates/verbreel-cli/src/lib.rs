//! verbreel-cli — `clap`-derived command-line surface.
//!
//! Per the crate dependency rule in `CLAUDE.md`:
//!
//! ```text
//! verbreel-cli → verbreel-state, verbreel-storage
//! verbreel-cli/native-render → verbreel-runtime
//! ```
//!
//! ## Lib + bin split
//!
//! The crate ships a thin `main.rs` that delegates to [`run`]. All
//! routing, argument parsing, and verb dispatch live here in the
//! library so integration tests can call them directly without
//! spawning subprocesses.
//!
//! The pattern: [`Cli`] is the top-level `clap` parser; [`Command`]
//! enumerates the noun-level subcommands (`project`, …); each noun
//! carries its own sub-`Args` struct (e.g. [`ProjectCmd`]) with an
//! inner action enum ([`ProjectAction`]). [`run`] matches on the parsed
//! tree and dispatches to the per-noun module.
//!
//! ## Adding a subcommand
//!
//! Every new subcommand after this first slice is additive:
//!
//! 1. Add a variant to [`Command`] (or to the noun's action enum if it
//!    belongs under an existing noun).
//! 2. Add a module under `src/` exposing the verb wrapper.
//! 3. Add a match arm in [`run`].
//!
//! No edits to existing wrappers required — the dispatch shape
//! inverts dependency on concrete verbs through
//! [`verbreel_state::default_registry`].

#![deny(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]

use std::io::Write;
use std::path::PathBuf;

use clap::{Parser, Subcommand};

pub mod agent;
pub mod project;
#[cfg(feature = "native-render")]
pub mod render;

/// Top-level `clap` parser for the `verbreel` binary.
///
/// `arg_required_else_help = true` pins the missing-subcommand behavior
/// so bare `verbreel` prints help instead of relying on a clap default
/// that could shift across major versions.
#[derive(Debug, Parser)]
#[command(
    name = "verbreel",
    version,
    about = "Verbreel engine CLI",
    long_about = None,
    propagate_version = true,
    arg_required_else_help = true
)]
pub struct Cli {
    /// Which noun-level subcommand was invoked.
    #[command(subcommand)]
    pub command: Command,
}

/// Noun-level subcommands recognised by [`Cli`].
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Project lifecycle commands (`verbreel project ...`).
    Project(ProjectCmd),
    /// Print the engine capability catalog (agent discovery surface).
    Caps(CapsCmd),
    /// Apply one verb to a project (`verbreel run <root> <verb>`).
    Run(RunCmd),
    /// Plan + apply an editing intent (`verbreel edit <root> ...`).
    Edit(EditCmd),
    /// Native render commands (`verbreel render ...`).
    #[cfg(feature = "native-render")]
    Render(RenderCmd),
}

/// `verbreel caps` — dump the capability catalog.
#[derive(Debug, clap::Args)]
pub struct CapsCmd {
    /// Print a compact human view grouped by domain instead of the full
    /// JSON catalog.
    #[arg(long)]
    pub by_domain: bool,
}

/// `verbreel run <root> <verb>` — apply one verb to a project.
#[derive(Debug, clap::Args)]
pub struct RunCmd {
    /// Project root (`<root>/project.json`).
    pub root: PathBuf,
    /// Verb id, e.g. `clip.trim`.
    pub verb: String,
    /// Verb args as a JSON object. `project_id` is injected when omitted.
    #[arg(long)]
    pub args: Option<String>,
    /// §0.8 idempotency key — a repeat call with the same key + args is
    /// deduplicated (replayed) instead of re-applied.
    #[arg(long)]
    pub key: Option<String>,
}

/// `verbreel edit <root>` — plan an intent into verbs and apply them.
#[derive(Debug, clap::Args)]
pub struct EditCmd {
    /// Project root to edit.
    pub root: PathBuf,
    /// Natural-language editing goal (Claude planner; needs the `claude`
    /// build feature + `ANTHROPIC_API_KEY`).
    #[arg(long, conflicts_with = "plan")]
    pub intent: Option<String>,
    /// A ready-made JSON plan file (works in every build; no LLM).
    #[arg(long, conflicts_with = "intent")]
    pub plan: Option<PathBuf>,
    /// Print the plan without applying it.
    #[arg(long)]
    pub dry_run: bool,
}

/// `verbreel project ...` argument group.
#[derive(Debug, clap::Args)]
#[command(arg_required_else_help = true, long_about = None)]
pub struct ProjectCmd {
    /// Which project-scoped action was invoked.
    #[command(subcommand)]
    pub action: ProjectAction,
}

/// Project-scoped actions wired by this crate.
#[derive(Debug, Subcommand)]
pub enum ProjectAction {
    /// List all known projects (v1 floor — emits an empty array).
    List,
    /// Create a fresh project on disk (`verbreel project create ...`).
    Create(ProjectCreateCmd),
}

/// `verbreel project create <workspace> --name <n>` — create a project.
#[derive(Debug, clap::Args)]
pub struct ProjectCreateCmd {
    /// Workspace directory; the project is created at `<workspace>/<name>`.
    pub workspace: PathBuf,
    /// Project display name (also the created directory name).
    #[arg(long)]
    pub name: String,
    /// Canvas size as `<W>x<H>`.
    #[arg(long, default_value = "1920x1080")]
    pub canvas: String,
    /// Frame rate as `<num>/<den>` (default 30/1).
    #[arg(long)]
    pub fps: Option<String>,
}

/// `verbreel render ...` argument group.
#[cfg(feature = "native-render")]
#[derive(Debug, clap::Args)]
#[command(arg_required_else_help = true, long_about = None)]
pub struct RenderCmd {
    /// Which render-scoped action was invoked.
    #[command(subcommand)]
    pub action: RenderAction,
}

/// Render-scoped actions wired by this crate.
#[cfg(feature = "native-render")]
#[derive(Debug, Subcommand)]
pub enum RenderAction {
    /// Start a native render job and write an MP4 output.
    Start(render::RenderStartCmd),
}

/// Dispatch a parsed [`Cli`] to the appropriate verb wrapper, writing
/// human-facing output to `out` and returning the process exit code.
///
/// Tests pass `&mut Vec<u8>` as `out`; the binary passes a locked
/// stdout handle.
///
/// # Errors
///
/// Surfaces any error bubbled up by the wrapped verb (verb dispatch,
/// argument validation, JSON serialization).
#[cfg(not(feature = "native-render"))]
pub fn run(cli: Cli, out: &mut dyn Write) -> anyhow::Result<i32> {
    match cli.command {
        Command::Project(p) => match p.action {
            ProjectAction::List => project::list(out),
            ProjectAction::Create(c) => project::create(out, &c),
        },
        Command::Caps(c) => agent::caps(out, c.by_domain),
        Command::Run(r) => agent::run(out, &r.root, &r.verb, r.args.as_deref(), r.key),
        Command::Edit(e) => agent::edit(
            out,
            &e.root,
            e.intent.as_deref(),
            e.plan.as_deref(),
            e.dry_run,
        ),
    }
}

/// Dispatch a parsed [`Cli`] with the default process runtime.
///
/// # Errors
///
/// Surfaces any error bubbled up by the wrapped verb or runtime.
#[cfg(feature = "native-render")]
pub fn run(cli: Cli, out: &mut dyn Write) -> anyhow::Result<i32> {
    run_with_runtime(cli, out, &verbreel_runtime::RenderRuntimeConfig::from_env())
}

/// Dispatch a parsed [`Cli`] with an injected render runtime.
///
/// Tests use this to avoid relying on the process current directory or
/// user-wide projects index.
///
/// # Errors
///
/// Surfaces any error bubbled up by the wrapped verb or runtime.
#[cfg(feature = "native-render")]
pub fn run_with_runtime(
    cli: Cli,
    out: &mut dyn Write,
    runtime: &verbreel_runtime::RenderRuntimeConfig,
) -> anyhow::Result<i32> {
    match cli.command {
        Command::Project(p) => match p.action {
            ProjectAction::List => project::list(out),
            ProjectAction::Create(c) => project::create(out, &c),
        },
        Command::Caps(c) => agent::caps(out, c.by_domain),
        Command::Run(r) => agent::run(out, &r.root, &r.verb, r.args.as_deref(), r.key),
        Command::Edit(e) => agent::edit(
            out,
            &e.root,
            e.intent.as_deref(),
            e.plan.as_deref(),
            e.dry_run,
        ),
        Command::Render(r) => match r.action {
            RenderAction::Start(args) => render::start(&args, out, runtime),
        },
    }
}

/// Full parse + dispatch + error-formatting loop. The binary calls this
/// with the process's argv + stdout + stderr; tests call it with
/// captured buffers to exercise the real stderr emission path.
///
/// Returns the process exit code:
/// - `0` on success.
/// - The clap-reported exit code on parse failure (writes the formatted
///   clap error to `err`).
/// - `1` on any error bubbled from [`run`] (writes `error: <detail>` to
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
            // (including the "missing subcommand → print help" path
            // enabled by arg_required_else_help). Render to `err` so
            // tests can assert on the actual stderr emission instead of
            // duplicating clap's format.
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
