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

// --- dispatch() — full parse + run + error-formatting loop ----------------

use verbreel_cli::dispatch;

#[test]
fn dispatch_bare_verbreel_prints_help_to_stderr_via_arg_required_else_help() {
    // `arg_required_else_help = true` on the top-level Cli must turn a
    // bare `verbreel` invocation into a help print (exit code 2 per
    // clap's DisplayHelpOnMissingArgumentOrSubcommand convention).
    // Pinning this via dispatch() exercises the REAL stderr emission
    // path, not just `clap::Error::render`.
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = dispatch(["verbreel"], &mut out, &mut err);
    assert_ne!(code, 0, "bare invocation must not exit 0");
    let err_text = String::from_utf8(err).expect("stderr must be utf-8");
    assert!(
        err_text.to_lowercase().contains("usage") || err_text.contains("verbreel"),
        "stderr must render the help/usage block, got:\n{err_text}"
    );
}

#[test]
fn dispatch_bare_project_prints_help_to_stderr_via_arg_required_else_help() {
    // Same contract for the noun-level: `verbreel project` without a
    // verb must show help, not crash with an unknown-subcommand-like
    // error.
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = dispatch(["verbreel", "project"], &mut out, &mut err);
    assert_ne!(code, 0, "bare 'project' must not exit 0");
    let err_text = String::from_utf8(err).expect("stderr must be utf-8");
    assert!(
        err_text.to_lowercase().contains("usage") || err_text.contains("project"),
        "stderr must render the project-help/usage block, got:\n{err_text}"
    );
}

#[test]
fn dispatch_unknown_subcommand_writes_clap_error_to_stderr() {
    // Unknown subcommand at the top level routes to clap's error
    // formatter, which dispatch() must render into `err` — never to
    // stdout, never silently.
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = dispatch(["verbreel", "totally-fake-noun"], &mut out, &mut err);
    assert_ne!(code, 0, "unknown subcommand must not exit 0");
    assert!(out.is_empty(), "stdout must remain empty on error path");
    let err_text = String::from_utf8(err).expect("stderr must be utf-8");
    assert!(
        err_text.contains("totally-fake-noun"),
        "stderr must echo the offending subcommand, got:\n{err_text}"
    );
}

#[test]
fn dispatch_happy_path_writes_to_stdout_only() {
    // The success path must NOT touch stderr — that would surprise
    // operators piping `verbreel project list | jq` and seeing
    // unexpected diagnostic noise on terminal 2.
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = dispatch(["verbreel", "project", "list"], &mut out, &mut err);
    assert_eq!(code, 0);
    assert!(!out.is_empty(), "stdout must carry the verb output");
    assert!(
        err.is_empty(),
        "stderr must remain empty on the happy path, got: {}",
        String::from_utf8_lossy(&err)
    );
}
