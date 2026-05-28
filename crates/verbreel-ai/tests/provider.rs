//! `run()` contract tests. Pins the v0 NotYetImplemented body and
//! the AI-spike deferral citation.

use verbreel_ai::{AiError, Capability, Provider, run};

#[test]
fn run_returns_not_yet_implemented() {
    let mut p = Provider::new(Capability::Whisper);
    let err = run(&mut p, b"").expect_err("v0 must always Err");
    assert!(matches!(err, AiError::NotYetImplemented { .. }));
}

#[test]
fn run_detail_is_non_empty() {
    let mut p = Provider::new(Capability::Whisper);
    let AiError::NotYetImplemented { detail } = run(&mut p, b"").unwrap_err() else {
        panic!("expected NotYetImplemented");
    };
    assert!(!detail.is_empty());
}

#[test]
fn run_detail_cites_research_04() {
    let mut p = Provider::new(Capability::Whisper);
    let AiError::NotYetImplemented { detail } = run(&mut p, b"").unwrap_err() else {
        panic!("expected NotYetImplemented");
    };
    assert!(
        detail.contains("Research 04"),
        "detail must cite Research 04, got: {detail}"
    );
}

#[test]
fn run_detail_mentions_ai_orchestration_spike() {
    let mut p = Provider::new(Capability::Whisper);
    let AiError::NotYetImplemented { detail } = run(&mut p, b"").unwrap_err() else {
        panic!("expected NotYetImplemented");
    };
    assert!(
        detail.contains("AI orchestration"),
        "detail must mention the AI orchestration spike, got: {detail}"
    );
}

#[test]
fn run_with_each_capability_returns_err() {
    for cap in [
        Capability::Whisper,
        Capability::Onset,
        Capability::Tempo,
        Capability::BeatNet,
    ] {
        let mut p = Provider::new(cap);
        let err = run(&mut p, b"input").expect_err("v0 must Err");
        assert!(matches!(err, AiError::NotYetImplemented { .. }));
    }
}

#[test]
fn run_with_empty_input_does_not_branch() {
    let mut p = Provider::new(Capability::Whisper);
    let err = run(&mut p, &[]).expect_err("v0 must Err");
    assert!(matches!(err, AiError::NotYetImplemented { .. }));
}

#[test]
fn run_with_large_input_does_not_panic() {
    let mut p = Provider::new(Capability::Whisper);
    let huge = vec![0u8; 1024 * 1024];
    let err = run(&mut p, &huge).expect_err("v0 must Err");
    assert!(matches!(err, AiError::NotYetImplemented { .. }));
}

#[test]
fn run_called_twice_consistent() {
    let mut p = Provider::new(Capability::Whisper);
    let first = run(&mut p, b"x").expect_err("v0 must Err");
    let second = run(&mut p, b"x").expect_err("v0 must Err");
    assert_eq!(first, second, "v0 result must be deterministic");
}
