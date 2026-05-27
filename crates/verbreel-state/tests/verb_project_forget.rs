//! Tests for `project.forget` (§2.8) — eighty-third production verb.
//!
//! The verb is a free function with no `Verb` trait impl (engine-
//! scoped — see the module doc comment in
//! `src/verbs/project_forget.rs`), so these tests exercise the
//! function surface directly. No project fixture is needed because
//! the verb does not read or mutate any project state.

use serde_json::{Value, json};
use verbreel_state::{ProjectForgetArgs, ProjectForgetData, ProjectForgetError, project_forget};

// --- Args deserialization -------------------------------------------------

#[test]
fn args_deser_path_only() {
    let raw = json!({ "path": "/tmp/my-project" });
    let typed: ProjectForgetArgs = serde_json::from_value(raw).expect("path-only args deserialize");
    assert_eq!(typed.path.as_deref(), Some("/tmp/my-project"));
    assert!(typed.project_id.is_none());
}

#[test]
fn args_deser_project_id_only() {
    let raw = json!({ "project_id": "0190b8d3-15e3-7000-bd00-000000000001" });
    let typed: ProjectForgetArgs = serde_json::from_value(raw).expect("id-only args deserialize");
    assert!(typed.path.is_none());
    assert_eq!(
        typed.project_id.as_deref(),
        Some("0190b8d3-15e3-7000-bd00-000000000001")
    );
}

#[test]
fn args_deser_rejects_unknown_field() {
    let raw = json!({ "path": "/tmp/p", "stray": true });
    let err = serde_json::from_value::<ProjectForgetArgs>(raw)
        .expect_err("deny_unknown_fields rejects stray keys");
    assert!(err.to_string().contains("unknown field"));
}

#[test]
fn args_deser_both_fields_present() {
    // Deserialization itself succeeds — the mutual-exclusion check is
    // a `forget()` runtime validation, not a serde-level constraint.
    let raw = json!({ "path": "/tmp/p", "project_id": "abc" });
    let typed: ProjectForgetArgs =
        serde_json::from_value(raw).expect("both-fields args deserialize");
    assert!(typed.path.is_some());
    assert!(typed.project_id.is_some());
}

#[test]
fn args_deser_empty_object() {
    // Empty object is valid at the serde layer because both fields
    // are optional. `forget()` surfaces this as `ArgsIncompatible`.
    let raw = json!({});
    let typed: ProjectForgetArgs = serde_json::from_value(raw).expect("empty object deserializes");
    assert!(typed.path.is_none());
    assert!(typed.project_id.is_none());
}

// --- Mutual exclusion -----------------------------------------------------

#[test]
fn both_fields_set_returns_args_incompatible() {
    let args = ProjectForgetArgs {
        path: Some("/tmp/p".to_string()),
        project_id: Some("0190b8d3-15e3-7000-bd00-000000000001".to_string()),
    };
    let err = project_forget(&args).expect_err("both-set must fail");
    match err {
        ProjectForgetError::ArgsIncompatible { detail } => {
            assert!(detail.contains("mutually exclusive"));
        }
        other => panic!("expected ArgsIncompatible, got {other:?}"),
    }
}

#[test]
fn neither_field_set_returns_args_incompatible() {
    let args = ProjectForgetArgs::default();
    let err = project_forget(&args).expect_err("neither-set must fail");
    match err {
        ProjectForgetError::ArgsIncompatible { detail } => {
            assert!(detail.contains("exactly one"));
        }
        other => panic!("expected ArgsIncompatible, got {other:?}"),
    }
}

// --- Path-form validation -------------------------------------------------

#[test]
fn empty_path_returns_bad_range() {
    let args = ProjectForgetArgs {
        path: Some(String::new()),
        project_id: None,
    };
    let err = project_forget(&args).expect_err("empty path must fail");
    match err {
        ProjectForgetError::BadRange { detail } => {
            assert!(detail.contains("empty"));
        }
        other => panic!("expected BadRange, got {other:?}"),
    }
}

#[test]
fn leading_nul_byte_path_returns_bad_range() {
    let args = ProjectForgetArgs {
        path: Some("\0/tmp/p".to_string()),
        project_id: None,
    };
    let err = project_forget(&args).expect_err("NUL-byte path must fail");
    match err {
        ProjectForgetError::BadRange { detail } => {
            assert!(detail.contains("NUL"));
        }
        other => panic!("expected BadRange, got {other:?}"),
    }
}

#[test]
fn embedded_nul_byte_path_returns_bad_range() {
    let args = ProjectForgetArgs {
        path: Some("/tmp/\0proj".to_string()),
        project_id: None,
    };
    let err = project_forget(&args).expect_err("embedded NUL must fail");
    assert!(matches!(err, ProjectForgetError::BadRange { .. }));
}

// --- Id form (v1 floor: always not found) --------------------------------

#[test]
fn id_form_returns_project_not_found() {
    let args = ProjectForgetArgs {
        path: None,
        project_id: Some("0190b8d3-15e3-7000-bd00-000000000001".to_string()),
    };
    let err = project_forget(&args).expect_err("id form v1 floor never resolves");
    assert!(matches!(err, ProjectForgetError::ProjectNotFound { .. }));
}

#[test]
fn id_form_echoes_lookup_in_error() {
    let id = "01900000-0000-7000-8000-000000000abc".to_string();
    let args = ProjectForgetArgs {
        path: None,
        project_id: Some(id.clone()),
    };
    let err = project_forget(&args).expect_err("id form v1 floor never resolves");
    match err {
        ProjectForgetError::ProjectNotFound { lookup } => assert_eq!(lookup, id),
        other => panic!("expected ProjectNotFound, got {other:?}"),
    }
}

// --- Path-form happy path -------------------------------------------------

#[test]
fn absolute_path_returns_was_in_index_false() {
    let args = ProjectForgetArgs {
        path: Some("/home/user/projects/demo".to_string()),
        project_id: None,
    };
    let data = project_forget(&args).expect("path form happy path");
    assert_eq!(
        data,
        ProjectForgetData {
            removed_path: "/home/user/projects/demo".to_string(),
            was_in_index: false,
        }
    );
}

#[test]
fn removed_path_echoes_input_verbatim() {
    let raw = "/very/specific/path-with-dashes-and_underscores";
    let args = ProjectForgetArgs {
        path: Some(raw.to_string()),
        project_id: None,
    };
    let data = project_forget(&args).expect("path form happy path");
    assert_eq!(data.removed_path, raw);
}

#[test]
fn relative_path_returns_was_in_index_false() {
    // Spec doesn't require absolute paths; the index would store
    // whatever the user opened. v1 floor accepts and echoes it.
    let args = ProjectForgetArgs {
        path: Some("./local-project".to_string()),
        project_id: None,
    };
    let data = project_forget(&args).expect("relative path is well-formed at v1");
    assert_eq!(data.removed_path, "./local-project");
    assert!(!data.was_in_index);
}

// --- Data shape lock ------------------------------------------------------

#[test]
fn envelope_has_exactly_two_keys() {
    let args = ProjectForgetArgs {
        path: Some("/tmp/p".to_string()),
        project_id: None,
    };
    let data = project_forget(&args).expect("happy path");
    let value: Value = serde_json::to_value(&data).expect("envelope serializes");
    let obj = value.as_object().expect("envelope is a JSON object");

    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();

    let mut expected = vec!["removed_path", "was_in_index"];
    expected.sort_unstable();

    assert_eq!(keys, expected);
}

#[test]
fn was_in_index_is_json_boolean() {
    let args = ProjectForgetArgs {
        path: Some("/tmp/p".to_string()),
        project_id: None,
    };
    let data = project_forget(&args).expect("happy path");
    let value: Value = serde_json::to_value(&data).expect("envelope serializes");
    assert_eq!(value["was_in_index"], json!(false));
    assert!(value["was_in_index"].is_boolean());
}

// --- Error message stability ---------------------------------------------

#[test]
fn args_incompatible_display_surfaces_detail() {
    let err = ProjectForgetError::ArgsIncompatible {
        detail: "stable message text",
    };
    let s = err.to_string();
    assert!(s.contains("args incompatible"));
    assert!(s.contains("stable message text"));
}

#[test]
fn bad_range_display_surfaces_detail() {
    let err = ProjectForgetError::BadRange {
        detail: "specific complaint".to_string(),
    };
    let s = err.to_string();
    assert!(s.contains("bad range"));
    assert!(s.contains("specific complaint"));
}
