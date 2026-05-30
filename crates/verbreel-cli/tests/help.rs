//! `--help` / `--version` / `-h` parser surface — pure clap, no
//! `run` invocation. These tests pin the documented surface so that
//! adding a new subcommand later doesn't accidentally remove or rename
//! an existing one.

use clap::{CommandFactory, Parser};
use verbreel_cli::Cli;

#[test]
fn help_flag_returns_displayed_help_error() {
    let err = Cli::try_parse_from(["verbreel", "--help"]).unwrap_err();
    assert_eq!(err.kind(), clap::error::ErrorKind::DisplayHelp);
}

#[test]
fn help_text_advertises_project_subcommand() {
    let help = Cli::command().render_long_help().to_string();
    assert!(
        help.contains("project"),
        "expected `project` noun in --help, got:\n{help}"
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
        "help should show user-facing command text, not parser implementation details:\n{help}"
    );
}

#[test]
fn version_flag_returns_displayed_version_error() {
    let err = Cli::try_parse_from(["verbreel", "--version"]).unwrap_err();
    assert_eq!(err.kind(), clap::error::ErrorKind::DisplayVersion);
}

#[test]
fn project_help_advertises_list_action() {
    let err = Cli::try_parse_from(["verbreel", "project", "--help"]).unwrap_err();
    assert_eq!(err.kind(), clap::error::ErrorKind::DisplayHelp);
    let rendered = err.to_string();
    assert!(
        rendered.contains("list"),
        "expected `list` action in `project --help`, got:\n{rendered}"
    );
}

#[test]
fn missing_subcommand_prints_help() {
    // Bare `verbreel` (no sub) should print help, not silently succeed.
    // clap conflates this with the "show help" path under
    // DisplayHelpOnMissingArgumentOrSubcommand at the derive-API
    // default settings used by this crate.
    let err = Cli::try_parse_from(["verbreel"]).unwrap_err();
    assert_eq!(
        err.kind(),
        clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand,
        "bare `verbreel` invocation should display help, got: {err}"
    );
}
