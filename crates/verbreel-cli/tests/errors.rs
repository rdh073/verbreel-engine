//! Parse-error paths — pin clap's `ErrorKind` for each bad argv shape
//! so future surface additions don't silently regress the user-facing
//! exit codes.

use clap::Parser;
use verbreel_cli::Cli;

#[test]
fn unknown_top_level_subcommand_is_a_parse_error() {
    let err = Cli::try_parse_from(["verbreel", "bogus"]).unwrap_err();
    assert_eq!(err.kind(), clap::error::ErrorKind::InvalidSubcommand);
}

#[test]
fn unknown_top_level_subcommand_uses_exit_code_two() {
    // clap defaults parse-error exits to code 2; this is the contract
    // shell scripts depend on.
    let err = Cli::try_parse_from(["verbreel", "bogus"]).unwrap_err();
    assert_eq!(err.exit_code(), 2);
}

#[test]
fn unknown_project_action_is_a_parse_error() {
    let err = Cli::try_parse_from(["verbreel", "project", "frobnicate"]).unwrap_err();
    assert_eq!(err.kind(), clap::error::ErrorKind::InvalidSubcommand);
}

#[test]
fn project_with_no_action_reports_help_on_missing_subcommand() {
    // clap distinguishes "bare top-level with no sub" (MissingSubcommand)
    // from "intermediate noun with no sub"
    // (DisplayHelpOnMissingArgumentOrSubcommand). Pin the latter so the
    // user-facing behaviour (`verbreel project` prints help, doesn't
    // silently exit 0) is locked in.
    let err = Cli::try_parse_from(["verbreel", "project"]).unwrap_err();
    assert_eq!(
        err.kind(),
        clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
    );
}

#[test]
fn unknown_flag_on_list_is_a_parse_error() {
    let err = Cli::try_parse_from(["verbreel", "project", "list", "--nope"]).unwrap_err();
    assert_eq!(err.kind(), clap::error::ErrorKind::UnknownArgument);
}

#[test]
fn unknown_top_level_subcommand_mentions_offending_token() {
    let err = Cli::try_parse_from(["verbreel", "bogus"]).unwrap_err();
    let rendered = err.to_string();
    assert!(
        rendered.contains("bogus"),
        "error text should mention the offending subcommand, got:\n{rendered}"
    );
}
