//! `--help` / `--version` parser surface — pure clap, no `run`.
//!
//! The generic dispatcher has no enumerated subcommands, so help no
//! longer auto-advertises `project` / `list`. It advertises the
//! `<noun> <verb>` usage shape instead. These tests pin that surface so
//! a future change does not silently drop the usage line or rename the
//! binary.

use clap::{CommandFactory, Parser};
use verbreel_cli::Cli;

#[test]
fn help_flag_returns_displayed_help_error() {
    let err = Cli::try_parse_from(["verbreel", "--help"]).unwrap_err();
    assert_eq!(err.kind(), clap::error::ErrorKind::DisplayHelp);
}

#[test]
fn help_text_advertises_noun_verb_usage() {
    // The generic surface documents `<noun> <verb>` in its about line
    // (clap has no enum to derive subcommand names from).
    let help = Cli::command().render_long_help().to_string();
    assert!(
        help.contains("<noun> <verb>"),
        "expected the `<noun> <verb>` usage shape in --help, got:\n{help}"
    );
}

#[test]
fn help_text_advertises_binary_name() {
    let help = Cli::command().render_long_help().to_string();
    assert!(
        help.contains("verbreel"),
        "expected binary name `verbreel` in --help, got:\n{help}"
    );
}

#[test]
fn help_text_omits_internal_rustdoc_details() {
    let help = Cli::command().render_long_help().to_string();
    assert!(
        !help.contains("arg_required_else_help"),
        "help should show user-facing text, not parser internals:\n{help}"
    );
}

#[test]
fn version_flag_returns_displayed_version_error() {
    let err = Cli::try_parse_from(["verbreel", "--version"]).unwrap_err();
    assert_eq!(err.kind(), clap::error::ErrorKind::DisplayVersion);
}

#[test]
fn missing_subcommand_prints_help() {
    // Bare `verbreel` (empty tail) must still print help via
    // arg_required_else_help, not dispatch an empty command. Under the
    // generic `trailing_var_arg` parser this surfaces as the
    // MissingRequiredArgument / DisplayHelpOnMissingArgumentOrSubcommand
    // family — either way it is a non-success parse outcome with exit 2.
    let err = Cli::try_parse_from(["verbreel"]).unwrap_err();
    assert_eq!(
        err.exit_code(),
        2,
        "bare `verbreel` must exit 2, got: {err}"
    );
}
