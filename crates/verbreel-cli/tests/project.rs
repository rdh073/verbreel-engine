//! `verbreel project list` and generic-dispatch read coverage via the
//! lib's [`run`] entrypoint.
//!
//! After PR4 every command emits the full §0.1 envelope, so the
//! `projects` array lives at `data.projects` (was top-level). Tests
//! capture stdout into a `Vec<u8>` and assert on the envelope shape.

mod common;

use clap::Parser;
use serde_json::Value;
use verbreel_cli::{Cli, run};

/// Parse argv and dispatch under a temp home, returning `(exit, stdout)`.
fn dispatch(argv: &[&str]) -> (i32, String) {
    common::with_home(|_home| {
        let cli = Cli::try_parse_from(argv).expect("argv must parse");
        let mut out: Vec<u8> = Vec::new();
        let code = run(cli, &mut out).expect("run must not fail to write");
        let body = String::from_utf8(out).expect("stdout must be UTF-8");
        (code, body)
    })
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
        v.pointer("/data/projects").is_some(),
        "envelope must expose `data.projects`, got: {body}"
    );
}

#[test]
fn project_list_projects_is_array() {
    let (_, body) = dispatch(&["verbreel", "project", "list"]);
    let v: Value = serde_json::from_str(&body).unwrap();
    assert!(
        v.pointer("/data/projects")
            .and_then(Value::as_array)
            .is_some(),
        "`data.projects` must be a JSON array, got: {body}"
    );
}

#[test]
fn project_list_array_is_empty_at_v1_floor() {
    let (_, body) = dispatch(&["verbreel", "project", "list"]);
    let v: Value = serde_json::from_str(&body).unwrap();
    let arr = v
        .pointer("/data/projects")
        .and_then(Value::as_array)
        .unwrap();
    assert!(
        arr.is_empty(),
        "v1 floor must report empty projects list, got: {body}"
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
    // `project.list` is a read with `event_id: ""`, so the envelope is
    // byte-stable across runs — no minted id leaks into the body.
    assert_eq!(body_a, body_b, "envelope must be stable across runs");
}

// --- generic-dispatch coverage (new) --------------------------------------

#[test]
fn top_level_envelope_has_ok_true_for_read() {
    let (_, body) = dispatch(&["verbreel", "project", "list"]);
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        v.get("ok").and_then(Value::as_bool),
        Some(true),
        "read must report ok:true, got: {body}"
    );
}

#[test]
fn envelope_has_patch_warnings_event_id_keys() {
    let (_, body) = dispatch(&["verbreel", "project", "list"]);
    let v: Value = serde_json::from_str(&body).unwrap();
    for key in ["ok", "data", "patch", "warnings", "event_id"] {
        assert!(
            v.get(key).is_some(),
            "ok-envelope must carry `{key}`, got: {body}"
        );
    }
}

#[test]
fn flat_meta_verb_dispatches() {
    // `list_capabilities` is a flat (dotless) meta verb — the
    // reconstructor must keep it single-token, not coerce it to a
    // `list_capabilities.<verb>` dotted id.
    let (code, body) = dispatch(&["verbreel", "list_capabilities"]);
    assert_eq!(code, 0, "flat meta verb must exit 0, got: {body}");
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v.get("ok").and_then(Value::as_bool), Some(true));
}

#[test]
fn dotted_verb_id_reconstructed() {
    // `project list` must reconstruct to the dotted id `project.list`
    // and dispatch the registry verb (ok:true with a `projects` array).
    let (code, body) = dispatch(&["verbreel", "project", "list"]);
    assert_eq!(code, 0);
    let v: Value = serde_json::from_str(&body).unwrap();
    assert!(
        v.pointer("/data/projects").is_some(),
        "project.list data must carry `projects`, got: {body}"
    );
}
