//! `verbreel project list` end-to-end via the lib's [`run`] entrypoint.
//!
//! Tests capture stdout into a `Vec<u8>` and assert on the JSON shape
//! the verb returns — no subprocess plumbing.

use clap::Parser;
use serde_json::Value;
use verbreel_cli::{Cli, run};

/// Helper — parse argv and dispatch, returning `(exit_code, stdout)`.
fn dispatch(argv: &[&str]) -> (i32, String) {
    let cli = Cli::try_parse_from(argv).expect("argv must parse");
    let mut out: Vec<u8> = Vec::new();
    let code = run(cli, &mut out).expect("run must not fail at v1 floor");
    let body = String::from_utf8(out).expect("stdout must be UTF-8");
    (code, body)
}

#[test]
fn project_list_exits_zero() {
    let (code, _) = dispatch(&["verbreel", "project", "list"]);
    assert_eq!(code, 0);
}

#[test]
fn project_list_emits_well_formed_json() {
    let (_, body) = dispatch(&["verbreel", "project", "list"]);
    let _: Value = serde_json::from_str(&body)
        .unwrap_or_else(|e| panic!("stdout was not valid JSON: {e}\nbody:\n{body}"));
}

#[test]
fn project_list_envelope_has_projects_key() {
    let (_, body) = dispatch(&["verbreel", "project", "list"]);
    let v: Value = serde_json::from_str(&body).unwrap();
    assert!(
        v.get("projects").is_some(),
        "envelope must expose `projects` key, got: {body}"
    );
}

#[test]
fn project_list_projects_is_array() {
    let (_, body) = dispatch(&["verbreel", "project", "list"]);
    let v: Value = serde_json::from_str(&body).unwrap();
    assert!(
        v.get("projects").and_then(Value::as_array).is_some(),
        "`projects` must be a JSON array, got: {body}"
    );
}

#[test]
fn project_list_array_is_empty_at_v1_floor() {
    let (_, body) = dispatch(&["verbreel", "project", "list"]);
    let v: Value = serde_json::from_str(&body).unwrap();
    let arr = v.get("projects").and_then(Value::as_array).unwrap();
    assert!(
        arr.is_empty(),
        "v1 floor must report empty projects list, got: {body}"
    );
}

#[test]
fn project_list_envelope_has_only_projects_key() {
    let (_, body) = dispatch(&["verbreel", "project", "list"]);
    let v: Value = serde_json::from_str(&body).unwrap();
    let obj = v.as_object().expect("top-level JSON must be an object");
    let keys: Vec<&String> = obj.keys().collect();
    assert_eq!(
        keys,
        vec![&String::from("projects")],
        "envelope must expose exactly one key at v1 floor"
    );
}

#[test]
fn project_list_output_is_pretty_printed() {
    let (_, body) = dispatch(&["verbreel", "project", "list"]);
    assert!(
        body.contains('\n'),
        "output should be multi-line pretty JSON, got: {body:?}"
    );
}

#[test]
fn project_list_output_ends_with_newline() {
    let (_, body) = dispatch(&["verbreel", "project", "list"]);
    assert!(
        body.ends_with('\n'),
        "writeln! must terminate output with a newline, got: {body:?}"
    );
}

#[test]
fn project_list_is_deterministic_across_invocations() {
    let (_, body_a) = dispatch(&["verbreel", "project", "list"]);
    let (_, body_b) = dispatch(&["verbreel", "project", "list"]);
    // The project_id minted internally must NOT leak into the data
    // envelope — otherwise back-to-back runs would diverge.
    assert_eq!(body_a, body_b, "envelope must be stable across runs");
}
