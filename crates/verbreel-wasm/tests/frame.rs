//! `run_frame()` contract tests. Pins the v0 NotYetImplemented body
//! and the Spike-S2 deferral citation.

use verbreel_wasm::{EngineHandle, WasmError, run_frame};

#[test]
fn run_frame_returns_not_yet_implemented() {
    let mut handle = EngineHandle::new();
    let err = run_frame(&mut handle, 0).expect_err("v0 must always Err");
    assert!(matches!(err, WasmError::NotYetImplemented { .. }));
}

#[test]
fn run_frame_detail_is_non_empty() {
    let mut handle = EngineHandle::new();
    let err = run_frame(&mut handle, 0).expect_err("v0 must always Err");
    let WasmError::NotYetImplemented { detail } = err else {
        panic!("expected NotYetImplemented, got different variant");
    };
    assert!(
        !detail.is_empty(),
        "NotYetImplemented detail must be non-empty"
    );
}

#[test]
fn run_frame_detail_cites_spike_s2() {
    let mut handle = EngineHandle::new();
    let WasmError::NotYetImplemented { detail } =
        run_frame(&mut handle, 0).expect_err("v0 must Err")
    else {
        panic!("expected NotYetImplemented");
    };
    assert!(
        detail.contains("Spike S2"),
        "detail must cite Spike S2, got: {detail}"
    );
}

#[test]
fn run_frame_detail_cites_research_01() {
    let mut handle = EngineHandle::new();
    let WasmError::NotYetImplemented { detail } =
        run_frame(&mut handle, 0).expect_err("v0 must Err")
    else {
        panic!("expected NotYetImplemented");
    };
    assert!(
        detail.contains("Research 01"),
        "detail must cite Research 01, got: {detail}"
    );
}

#[test]
fn run_frame_works_with_negative_at_tk() {
    // The signature accepts `i64`; at v0 the body returns
    // NotYetImplemented regardless of tick value, so negative ticks
    // must not panic or branch on sign.
    let mut handle = EngineHandle::new();
    let err = run_frame(&mut handle, -1).expect_err("v0 must Err");
    assert!(matches!(err, WasmError::NotYetImplemented { .. }));
}

#[test]
fn run_frame_works_with_large_at_tk() {
    let mut handle = EngineHandle::new();
    let err = run_frame(&mut handle, i64::MAX).expect_err("v0 must Err");
    assert!(matches!(err, WasmError::NotYetImplemented { .. }));
}

#[test]
fn run_frame_called_twice_consistent() {
    let mut handle = EngineHandle::new();
    let first = run_frame(&mut handle, 0).expect_err("v0 must Err");
    let second = run_frame(&mut handle, 0).expect_err("v0 must Err");
    assert_eq!(first, second, "v0 result must be deterministic");
}
