//! `run()` contract tests. Pins the v0 NotYetImplemented body and
//! the Spike-S1 deferral citation.

use verbreel_ir::Tick;
use verbreel_render::{Pipeline, RenderError, RenderPreset, run};

#[test]
fn run_returns_not_yet_implemented() {
    let mut p = Pipeline::new(RenderPreset::Deterministic);
    let err = run(&mut p, Tick::ZERO).expect_err("v0 must always Err");
    assert!(matches!(err, RenderError::NotYetImplemented { .. }));
}

#[test]
fn run_detail_is_non_empty() {
    let mut p = Pipeline::new(RenderPreset::Deterministic);
    let RenderError::NotYetImplemented { detail } = run(&mut p, Tick::ZERO).unwrap_err() else {
        panic!("expected NotYetImplemented");
    };
    assert!(!detail.is_empty());
}

#[test]
fn run_detail_cites_spike_s1() {
    let mut p = Pipeline::new(RenderPreset::Deterministic);
    let RenderError::NotYetImplemented { detail } = run(&mut p, Tick::ZERO).unwrap_err() else {
        panic!("expected NotYetImplemented");
    };
    assert!(
        detail.contains("Spike S1"),
        "detail must cite Spike S1, got: {detail}"
    );
}

#[test]
fn run_detail_cites_research_01() {
    let mut p = Pipeline::new(RenderPreset::Deterministic);
    let RenderError::NotYetImplemented { detail } = run(&mut p, Tick::ZERO).unwrap_err() else {
        panic!("expected NotYetImplemented");
    };
    assert!(
        detail.contains("Research 01"),
        "detail must cite Research 01, got: {detail}"
    );
}

#[test]
fn run_with_performance_preset_also_errs() {
    let mut p = Pipeline::new(RenderPreset::Performance);
    let err = run(&mut p, Tick::ZERO).expect_err("v0 must Err");
    assert!(matches!(err, RenderError::NotYetImplemented { .. }));
}

#[test]
fn run_with_negative_tk_does_not_branch_on_sign() {
    let mut p = Pipeline::new(RenderPreset::Deterministic);
    let err = run(&mut p, Tick(-1)).expect_err("v0 must Err");
    assert!(matches!(err, RenderError::NotYetImplemented { .. }));
}

#[test]
fn run_with_max_tk_does_not_overflow() {
    let mut p = Pipeline::new(RenderPreset::Deterministic);
    let err = run(&mut p, Tick(i64::MAX)).expect_err("v0 must Err");
    assert!(matches!(err, RenderError::NotYetImplemented { .. }));
}

#[test]
fn run_called_twice_consistent() {
    let mut p = Pipeline::new(RenderPreset::Deterministic);
    let first = run(&mut p, Tick::ZERO).expect_err("v0 must Err");
    let second = run(&mut p, Tick::ZERO).expect_err("v0 must Err");
    assert_eq!(first, second, "v0 result must be deterministic");
}
