//! Type-surface tests for verbreel-wasm v0. Pins the `EngineHandle`
//! constructor + accessor contract, the `WasmError` variant set, and
//! the `Send + Sync` bounds JS-bridge callers will rely on.

use verbreel_wasm::{EMBEDDING_SCOPE_WIRE, EmbeddingScope, EngineHandle, WasmError};

// --- EngineHandle ---------------------------------------------------------

#[test]
fn engine_handle_new_returns_non_empty_schema_version() {
    let handle = EngineHandle::new();
    assert!(
        !handle.schema_version().is_empty(),
        "schema_version() must echo the state crate's SCHEMA_VERSION, got empty"
    );
}

#[test]
fn engine_handle_schema_version_mirrors_verbreel_state() {
    let handle = EngineHandle::new();
    assert_eq!(handle.schema_version(), verbreel_state::SCHEMA_VERSION);
}

#[test]
fn engine_handle_two_instances_carry_same_schema_version() {
    // Two independent constructions must agree on the schema version —
    // a regression that pinned the version per-instance would surface
    // here. (No Clone derive on EngineHandle by design — see
    // engine.rs doc comment.)
    let a = EngineHandle::new();
    let b = EngineHandle::new();
    assert_eq!(a.schema_version(), b.schema_version());
}

#[test]
fn engine_handle_is_debug() {
    let handle = EngineHandle::new();
    let rendered = format!("{handle:?}");
    assert!(
        rendered.contains("EngineHandle"),
        "Debug impl must name the type, got: {rendered}"
    );
}

#[test]
fn engine_handle_embedding_scope_is_preview_only() {
    let handle = EngineHandle::new();
    assert_eq!(handle.embedding_scope(), EmbeddingScope::PreviewOnly);
    assert_eq!(handle.embedding_scope_wire(), EMBEDDING_SCOPE_WIRE);
}

#[test]
fn engine_handle_supports_preview_but_not_editor_embedding() {
    let handle = EngineHandle::new();
    assert!(
        handle.supports_preview_embedding(),
        "v1 wasm embedding must expose browser preview"
    );
    assert!(
        !handle.supports_editor_embedding(),
        "v1 wasm embedding must not expose full editor commands"
    );
}

// --- Send + Sync compile-time check --------------------------------------

#[test]
fn engine_handle_is_send_and_sync() {
    // Compile-time assertion via static functions that bind the
    // trait bounds. If `EngineHandle` is ever made `!Send` or
    // `!Sync` by accident (e.g. a future `Rc` field) this test
    // refuses to compile.
    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}
    assert_send::<EngineHandle>();
    assert_sync::<EngineHandle>();
}

// --- EmbeddingScope -------------------------------------------------------

#[test]
fn embedding_scope_preview_only_wire_literal_is_stable() {
    assert_eq!(EMBEDDING_SCOPE_WIRE, "preview-only");
    assert_eq!(EmbeddingScope::PreviewOnly.as_str(), "preview-only");
}

#[test]
fn embedding_scope_preview_only_support_matrix() {
    let scope = EmbeddingScope::PreviewOnly;
    assert!(scope.supports_preview());
    assert!(!scope.supports_editor());
}

// --- WasmError variants ---------------------------------------------------

#[test]
fn wasm_error_not_yet_implemented_variant_constructs() {
    let err = WasmError::NotYetImplemented {
        detail: "anything".to_string(),
    };
    assert!(matches!(err, WasmError::NotYetImplemented { .. }));
}

#[test]
fn wasm_error_invalid_project_json_variant_constructs() {
    let err = WasmError::InvalidProjectJson {
        detail: "trailing comma at offset 42".to_string(),
    };
    assert!(matches!(err, WasmError::InvalidProjectJson { .. }));
}

#[test]
fn wasm_error_engine_internal_variant_constructs() {
    let err = WasmError::EngineInternal {
        detail: "wgpu surface lost".to_string(),
    };
    assert!(matches!(err, WasmError::EngineInternal { .. }));
}

#[test]
fn wasm_error_display_includes_detail() {
    let err = WasmError::NotYetImplemented {
        detail: "hello detail".to_string(),
    };
    let rendered = err.to_string();
    assert!(
        rendered.contains("hello detail"),
        "Display must surface detail, got: {rendered}"
    );
}

#[test]
fn wasm_error_distinct_variants_compare_unequal() {
    let a = WasmError::NotYetImplemented {
        detail: "x".to_string(),
    };
    let b = WasmError::InvalidProjectJson {
        detail: "x".to_string(),
    };
    let c = WasmError::EngineInternal {
        detail: "x".to_string(),
    };
    assert_ne!(a, b);
    assert_ne!(a, c);
    assert_ne!(b, c);
}

#[test]
fn wasm_error_same_variant_with_same_detail_compares_equal() {
    let a = WasmError::NotYetImplemented {
        detail: "same".to_string(),
    };
    let b = WasmError::NotYetImplemented {
        detail: "same".to_string(),
    };
    assert_eq!(a, b);
}

#[test]
fn wasm_error_same_variant_with_different_detail_compares_unequal() {
    let a = WasmError::NotYetImplemented {
        detail: "alpha".to_string(),
    };
    let b = WasmError::NotYetImplemented {
        detail: "beta".to_string(),
    };
    assert_ne!(a, b);
}
