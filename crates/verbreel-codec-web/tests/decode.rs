//! Integration tests pinning the v0 [`decode`] stub contract:
//! every call must return [`DecodeError::NotYetImplemented`] with a
//! non-empty `detail` that names Spike S2 / Research 01 §11 so the
//! orchestrator can sanity-check the deferral.

use verbreel_codec_web::{DecodeError, WebDecoder, decode};

#[test]
fn decode_returns_not_yet_implemented_with_empty_input() {
    let res = decode(WebDecoder::H264, &[]);
    assert!(matches!(res, Err(DecodeError::NotYetImplemented { .. })));
}

#[test]
fn decode_returns_not_yet_implemented_with_nonempty_bitstream() {
    // A trivial NAL-looking byte sequence; v0 must not inspect it.
    let bitstream = [0, 0, 0, 1, 0x67, 0x42, 0x00, 0x1f];
    let res = decode(WebDecoder::H264, &bitstream);
    assert!(matches!(res, Err(DecodeError::NotYetImplemented { .. })));
}

#[test]
fn decode_detail_is_non_empty() {
    let err = decode(WebDecoder::H264, &[]).unwrap_err();
    match err {
        DecodeError::NotYetImplemented { detail } => {
            assert!(!detail.is_empty(), "detail must surface deferral context");
        }
        other => panic!("v0 stub must return NotYetImplemented, got {other:?}"),
    }
}

#[test]
fn decode_detail_cites_spike_s2() {
    // The detail string is the contract handshake between this slice
    // and Spike S2. Loop orchestrators grep for the literal token.
    let err = decode(WebDecoder::H264, &[]).unwrap_err();
    let DecodeError::NotYetImplemented { detail } = err else {
        panic!("expected NotYetImplemented variant");
    };
    assert!(
        detail.contains("Spike S2"),
        "detail must name Spike S2: {detail}"
    );
}

#[test]
fn decode_detail_cites_research_01() {
    let err = decode(WebDecoder::H264, &[]).unwrap_err();
    let DecodeError::NotYetImplemented { detail } = err else {
        panic!("expected NotYetImplemented variant");
    };
    assert!(
        detail.contains("Research 01"),
        "detail must cite the Research 01 source of truth: {detail}"
    );
}

#[test]
fn decode_is_not_yet_implemented_for_h265() {
    // The stub must error regardless of which codec is asked for.
    let res = decode(WebDecoder::H265, &[]);
    assert!(matches!(res, Err(DecodeError::NotYetImplemented { .. })));
}
