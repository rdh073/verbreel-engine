//! Failure-path coverage for the generic dispatcher.
//!
//! The generic `trailing_var_arg` parser accepts any `<noun> <verb>
//! [--flags]` tail, so unknown nouns/verbs are no longer clap
//! `InvalidSubcommand` parse errors (exit 2). They become dispatch-time
//! §0.1 `ok:false` envelopes on **stdout** (exit 1). The only surviving
//! clap exit-2 paths are `--help` / `--version` and a genuinely
//! malformed argv.

mod common;

use clap::Parser;
use serde_json::Value;
use verbreel_cli::{Cli, dispatch};

/// `dispatch` under a temp home, returning `(exit, stdout, stderr)`.
fn run_argv(argv: &[&str]) -> (i32, String, String) {
    common::with_home(|_home| {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = dispatch(argv, &mut out, &mut err);
        (
            code,
            String::from_utf8(out).expect("stdout utf-8"),
            String::from_utf8(err).expect("stderr utf-8"),
        )
    })
}

#[test]
fn unknown_top_level_subcommand_dispatches_to_unknown_verb() {
    // clap accepts `bogus frob` now; it dispatches to id `bogus.frob`,
    // the engine returns `E_UNKNOWN_VERB`, exit 1 (was clap exit 2).
    let (code, out, _) = run_argv(&["verbreel", "bogus", "frob"]);
    assert_eq!(code, 1, "unknown verb is ok:false → exit 1");
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v.get("ok").and_then(Value::as_bool), Some(false));
    assert_eq!(
        v.get("code").and_then(Value::as_str),
        Some("E_UNKNOWN_VERB"),
        "got: {out}"
    );
}

#[test]
fn unknown_verb_uses_exit_code_one_not_two() {
    let (code, _, _) = run_argv(&["verbreel", "bogus", "frob"]);
    assert_eq!(code, 1, "unknown verb is now exit 1 (ok:false), not 2");
}

#[test]
fn unknown_project_action_is_unknown_verb() {
    // `project frobnicate` → id `project.frobnicate`, engine ok:false.
    let (code, out, _) = run_argv(&["verbreel", "project", "frobnicate"]);
    assert_eq!(code, 1);
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(
        v.get("code").and_then(Value::as_str),
        Some("E_UNKNOWN_VERB")
    );
}

#[test]
fn bare_noun_is_unknown_verb() {
    // `verbreel project` (noun, no verb) reconstructs to id `project.`
    // (a dotted id with an empty verb), which the engine rejects with
    // `E_UNKNOWN_VERB`, exit 1 — pinned over the old clap help path.
    let (code, out, _) = run_argv(&["verbreel", "project"]);
    assert_eq!(code, 1);
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(
        v.get("code").and_then(Value::as_str),
        Some("E_UNKNOWN_VERB")
    );
}

#[test]
fn unknown_flag_folds_into_args_and_is_dispatched() {
    // The generic parser folds arbitrary `--flags` into the args map;
    // `--nope value` is no longer a clap parse error. It lands in the
    // args and the engine response is asserted (here: `project.list`
    // ignores the extra flag and still returns ok:true).
    let (code, out, _) = run_argv(&["verbreel", "project", "list", "--nope", "x"]);
    assert_eq!(code, 0, "extra flag must not break a read, got: {out}");
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v.get("ok").and_then(Value::as_bool), Some(true));
}

#[test]
fn unknown_verb_message_names_offending_token_on_stdout() {
    // The offending token now appears in the §0.1 envelope's `message`
    // on stdout, not in a clap stderr error.
    let (_, out, err) = run_argv(&["verbreel", "totally_fake", "frob"]);
    assert!(
        out.contains("totally_fake.frob"),
        "stdout envelope message must name the verb, got:\n{out}"
    );
    assert!(err.is_empty(), "stderr must stay empty, got:\n{err}");
}

#[test]
fn dispatch_bare_verbreel_prints_help_to_stderr_via_arg_required_else_help() {
    // Bare `verbreel` still exits non-zero with help via
    // arg_required_else_help (clap renders to stderr).
    let (code, _out, err) = run_argv(&["verbreel"]);
    assert_ne!(code, 0, "bare invocation must not exit 0");
    assert!(
        err.to_lowercase().contains("usage") || err.contains("verbreel"),
        "stderr must render the help/usage block, got:\n{err}"
    );
}

#[test]
fn dispatch_unknown_subcommand_writes_envelope_to_stdout() {
    // Inverted from the old contract: an unknown subcommand no longer
    // writes a clap error to stderr. It writes a §0.1 ok:false envelope
    // to stdout; stderr stays empty.
    let (code, out, err) = run_argv(&["verbreel", "totally-fake-noun", "frob"]);
    assert_eq!(code, 1, "unknown subcommand is ok:false → exit 1");
    assert!(err.is_empty(), "stderr must stay empty, got:\n{err}");
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v.get("ok").and_then(Value::as_bool), Some(false));
}

#[test]
fn dispatch_happy_path_writes_to_stdout_only() {
    let (code, out, err) = run_argv(&["verbreel", "project", "list"]);
    assert_eq!(code, 0);
    assert!(!out.is_empty(), "stdout must carry the verb output");
    assert!(
        err.is_empty(),
        "stderr must stay empty on the happy path, got: {err}"
    );
}

// --- exit-code contract (new) ---------------------------------------------

#[test]
fn parse_failure_exits_two() {
    // Bare `verbreel` (empty required tail) is the live clap-level parse
    // failure: arg_required_else_help turns it into a non-success parse
    // outcome with exit code 2, proving the §0.1 "exit 2 = parse-level,
    // before the engine" contract still has a live path. `--help` does
    // NOT qualify — it is an intentional help display with exit 0.
    let err = Cli::try_parse_from(["verbreel"]).unwrap_err();
    assert_eq!(err.exit_code(), 2);
    let (code, _, _) = run_argv(&["verbreel"]);
    assert_eq!(code, 2, "dispatch must surface the clap exit-2 path");
}

#[test]
fn ok_false_exits_one() {
    // A project-scoped verb with no open project → `E_PROJECT_NOT_FOUND`,
    // ok:false on stdout, exit 1.
    let (code, out, _) = run_argv(&["verbreel", "clip", "add"]);
    assert_eq!(code, 1);
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v.get("ok").and_then(Value::as_bool), Some(false));
    assert_eq!(
        v.get("code").and_then(Value::as_str),
        Some("E_PROJECT_NOT_FOUND")
    );
}
