//! §2 (flag → args) and §4 (CLI-side coercion) coverage.
//!
//! Scalar coercion, the `--args` escape hatch, and duration→ticks are
//! pure functions exposed by the `args` / `duration` modules, unit-tested
//! here directly. Project-context injection (§0.12) and the §0.3 guard
//! are exercised through the full dispatch path with a temp home.

mod common;

use serde_json::{Value, json};
use verbreel_cli::args::{coerce_scalar, parse_flags};
use verbreel_cli::duration::duration_str_to_ticks;

fn s(v: &str) -> String {
    v.to_string()
}

#[test]
fn scalar_coercion_int_float_bool_string() {
    assert_eq!(coerce_scalar("5"), json!(5));
    assert_eq!(coerce_scalar("5").as_i64(), Some(5));
    assert_eq!(coerce_scalar("1.5"), json!(1.5));
    assert_eq!(coerce_scalar("true"), json!(true));
    assert_eq!(coerce_scalar("false"), json!(false));
    assert_eq!(coerce_scalar("foo"), json!("foo"));
    // A negative integer stays an integer (hyphen-value, not a flag).
    assert_eq!(coerce_scalar("-10"), json!(-10));
}

#[test]
fn bare_flag_is_boolean_true() {
    let parsed = parse_flags(&[s("--overwrite")]).expect("parse");
    assert_eq!(parsed.args.get("overwrite"), Some(&json!(true)));
}

#[test]
fn flag_value_pair_coerces() {
    let parsed = parse_flags(&[s("--n"), s("5"), s("--name"), s("foo")]).expect("parse");
    assert_eq!(parsed.args.get("n"), Some(&json!(5)));
    assert_eq!(parsed.args.get("name"), Some(&json!("foo")));
}

#[test]
fn args_json_escape_hatch_merges() {
    // `--args '{"a":1}' --b 2` produces {"a":1,"b":2}.
    let parsed = parse_flags(&[s("--args"), s(r#"{"a":1}"#), s("--b"), s("2")]).expect("parse");
    assert_eq!(parsed.args.get("a"), Some(&json!(1)));
    assert_eq!(parsed.args.get("b"), Some(&json!(2)));
}

#[test]
fn repeated_flag_wins_over_args_blob_on_collision() {
    // Documented precedence: explicit `--flag` overrides the `--args`
    // blob on a key collision.
    let parsed =
        parse_flags(&[s("--args"), s(r#"{"k":"blob"}"#), s("--k"), s("flag")]).expect("parse");
    assert_eq!(
        parsed.args.get("k"),
        Some(&json!("flag")),
        "explicit flag must win over the --args blob"
    );
}

#[test]
fn non_object_args_json_is_rejected() {
    let err = parse_flags(&[s("--args"), s("[1,2,3]")]).expect_err("array must reject");
    assert_eq!(err.code, "E_BAD_ARGS");
}

#[test]
fn project_flag_is_captured_not_an_arg() {
    let parsed = parse_flags(&[s("--project"), s("abc")]).expect("parse");
    assert_eq!(parsed.project.as_deref(), Some("abc"));
    assert!(
        parsed.args.get("project").is_none(),
        "--project is routing, not a verb arg"
    );
}

// --- §0.2 duration → ticks (240000 Hz) ------------------------------------

#[test]
fn duration_string_coerces_to_ticks() {
    // 5s × 240000 = 1_200_000.
    assert_eq!(coerce_scalar("5s"), json!(1_200_000));
    assert_eq!(duration_str_to_ticks("5s"), Some(1_200_000));
}

#[test]
fn duration_forms_all_resolve() {
    assert_eq!(duration_str_to_ticks("5s"), Some(1_200_000));
    assert_eq!(duration_str_to_ticks("5000ms"), Some(1_200_000));
    assert_eq!(duration_str_to_ticks("1200000tk"), Some(1_200_000));
    assert_eq!(duration_str_to_ticks("00:00:05.000"), Some(1_200_000));
    assert_eq!(duration_str_to_ticks("00:01:00"), Some(60 * 240_000));
}

#[test]
fn bare_integer_is_ticks_not_duration() {
    // A bare integer is already ticks: not matched by the duration
    // grammar, so it stays an i64 via numeric coercion (no double-mult).
    assert_eq!(duration_str_to_ticks("1200000"), None);
    assert_eq!(coerce_scalar("1200000"), json!(1_200_000));
}

#[test]
fn non_duration_string_stays_string() {
    assert_eq!(duration_str_to_ticks("preset-name"), None);
    assert_eq!(coerce_scalar("h264"), json!("h264"));
}

// --- §0.12 project context + §0.3 guard (full dispatch) -------------------

use clap::Parser;
use verbreel_cli::{Cli, run};

fn dispatch_under(home: &std::path::Path, argv: &[&str]) -> (i32, Value) {
    let cli = Cli::try_parse_from(argv).expect("argv must parse");
    let mut out: Vec<u8> = Vec::new();
    let code = run(cli, &mut out).expect("run must not fail to write");
    let body = String::from_utf8(out).expect("utf-8");
    let v: Value = serde_json::from_str(&body).unwrap_or_else(|e| panic!("not JSON: {e}\n{body}"));
    let _ = home;
    (code, v)
}

#[test]
fn project_flag_opens_and_dispatches_scoped_verb() {
    common::with_home(|home| {
        let id = common::create_registered_project(home);
        // `clip list` is a project-scoped read; with --project it opens
        // the registered project and dispatches, returning ok:true.
        let (code, v) = dispatch_under(home, &["verbreel", "clip", "list", "--project", &id]);
        assert_eq!(code, 0, "scoped read with --project must succeed: {v}");
        assert_eq!(v.get("ok").and_then(Value::as_bool), Some(true));
    });
}

#[test]
fn active_project_file_supplies_project_id() {
    common::with_home(|home| {
        let id = common::create_registered_project(home);
        // Write the active-project file directly, then omit --project.
        verbreel_cli::home::write_active_project(home, &id).expect("write active");
        let (code, v) = dispatch_under(home, &["verbreel", "clip", "list"]);
        assert_eq!(code, 0, "scoped read via active-project must succeed: {v}");
        assert_eq!(v.get("ok").and_then(Value::as_bool), Some(true));
    });
}

#[test]
fn prefix_without_project_context_errors() {
    // §0.3 guard: a project-scoped verb with no --project and no
    // active-project → E_PROJECT_NOT_FOUND on stdout, exit 1, *before*
    // any prefix resolution.
    common::with_home(|home| {
        let (code, v) = dispatch_under(home, &["verbreel", "clip", "list", "--clip_id", "c9f3a8"]);
        assert_eq!(code, 1);
        assert_eq!(
            v.get("code").and_then(Value::as_str),
            Some("E_PROJECT_NOT_FOUND"),
            "got: {v}"
        );
    });
}

#[test]
fn create_with_activate_writes_active_project() {
    // The §0.12 writer: `project create --activate` persists the returned
    // id to the active-project file for the next invocation.
    common::with_home(|home| {
        let dest = home.join("created");
        let (code, v) = dispatch_under(
            home,
            &[
                "verbreel",
                "project",
                "create",
                "--name",
                "activated",
                "--canvas",
                "64x64",
                "--at",
                dest.to_str().unwrap(),
                "--activate",
            ],
        );
        assert_eq!(code, 0, "create must succeed: {v}");
        let written = verbreel_cli::home::read_active_project(home)
            .expect("active-project file must be written");
        let id = v
            .pointer("/data/project_id")
            .and_then(Value::as_str)
            .expect("data.project_id");
        assert_eq!(written, id, "active-project must hold the created id");
    });
}
