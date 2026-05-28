//! Integration tests pinning the v0 [`encode`] stub contract:
//! every call must return [`CodecError::NotYetImplemented`] with a
//! non-empty `detail` that names Spike S1 / Research 01 §11 so the
//! orchestrator can sanity-check the deferral.

use verbreel_codec_native::{Codec, CodecError, CodecPreset, EncodeParams, Frame, encode};

fn sample_params() -> EncodeParams {
    EncodeParams {
        codec: Codec::H264,
        preset: CodecPreset::Deterministic,
        width: 1920,
        height: 1080,
        fps_num: 30,
        fps_den: 1,
        bit_rate: Some(8_000_000),
    }
}

#[test]
fn encode_returns_not_yet_implemented_with_empty_input() {
    let params = sample_params();
    let res = encode(&params, &[]);
    assert!(matches!(res, Err(CodecError::NotYetImplemented { .. })));
}

#[test]
fn encode_returns_not_yet_implemented_with_one_frame() {
    let params = sample_params();
    let frame = Frame::new(1920, 1080, vec![0u8; 4]);
    let res = encode(&params, std::slice::from_ref(&frame));
    assert!(matches!(res, Err(CodecError::NotYetImplemented { .. })));
}

#[test]
fn encode_detail_is_non_empty() {
    let params = sample_params();
    let err = encode(&params, &[]).unwrap_err();
    match err {
        CodecError::NotYetImplemented { detail } => {
            assert!(!detail.is_empty(), "detail must surface deferral context");
        }
        other => panic!("v0 stub must return NotYetImplemented, got {other:?}"),
    }
}

#[test]
fn encode_detail_cites_spike_s1() {
    // The detail string is the contract handshake between this slice
    // and Spike S1. Loop orchestrators grep for the literal token.
    let params = sample_params();
    let err = encode(&params, &[]).unwrap_err();
    let CodecError::NotYetImplemented { detail } = err else {
        panic!("expected NotYetImplemented variant");
    };
    assert!(
        detail.contains("Spike S1"),
        "detail must name Spike S1: {detail}"
    );
}

#[test]
fn encode_detail_cites_research_01() {
    let params = sample_params();
    let err = encode(&params, &[]).unwrap_err();
    let CodecError::NotYetImplemented { detail } = err else {
        panic!("expected NotYetImplemented variant");
    };
    assert!(
        detail.contains("Research 01") || detail.contains("§11"),
        "detail must cite the Research 01 §11 source of truth: {detail}"
    );
}

#[test]
fn encode_is_not_yet_implemented_for_performance_preset() {
    // The stub must error regardless of which preset / codec is asked.
    let params = EncodeParams {
        preset: CodecPreset::Performance,
        ..sample_params()
    };
    let res = encode(&params, &[]);
    assert!(matches!(res, Err(CodecError::NotYetImplemented { .. })));
}

#[test]
fn encode_is_not_yet_implemented_for_prores_codec() {
    let params = EncodeParams {
        codec: Codec::ProRes,
        ..sample_params()
    };
    let res = encode(&params, &[]);
    assert!(matches!(res, Err(CodecError::NotYetImplemented { .. })));
}
