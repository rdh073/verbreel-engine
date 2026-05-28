//! verbreel-cli — `clap`-derived command-line surface.
//!
//! Per the crate dependency rule in `CLAUDE.md`:
//!
//! ```text
//! verbreel-cli → verbreel-state, verbreel-storage
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

use clap::{Parser, Subcommand};

pub mod project;

/// Top-level `clap` parser for the `verbreel` binary.
#[derive(Debug, Parser)]
#[command(
    name = "verbreel",
    version,
    about = "Verbreel engine CLI",
    propagate_version = true
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
}

/// `verbreel project ...` argument group.
#[derive(Debug, clap::Args)]
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
pub fn run(cli: Cli, out: &mut dyn Write) -> anyhow::Result<i32> {
    match cli.command {
        Command::Project(p) => match p.action {
            ProjectAction::List => project::list(out),
        },
    }
}
