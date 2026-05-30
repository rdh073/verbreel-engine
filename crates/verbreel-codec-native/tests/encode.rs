//! Integration tests pinning the [`encode`] entry-point contract.
//!
//! The v0 stub returned [`CodecError::NotYetImplemented`] unconditionally;
//! issue #420 replaces that body. The contract is now feature-dependent:
//!
//! - feature OFF (what CI builds): every call returns
//!   [`CodecError::FeatureDisabled`] — no FFmpeg is linked.
//! - feature ON: params are validated before any encoder work, so a
//!   degenerate input surfaces [`CodecError::InvalidParams`].

use verbreel_codec_native::{Codec, CodecError, CodecPreset, EncodeParams, encode};

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

#[cfg(not(feature = "rsmpeg"))]
#[test]
fn encode_feature_off_returns_feature_disabled() {
    let params = sample_params();
    let res = encode(&params, &[]);
    assert!(
        matches!(res, Err(CodecError::FeatureDisabled { .. })),
        "feature-off encode must fail closed with FeatureDisabled"
    );
}

#[cfg(not(feature = "rsmpeg"))]
#[test]
fn encode_feature_off_detail_is_non_empty() {
    let params = sample_params();
    let err = encode(&params, &[]).unwrap_err();
    match err {
        CodecError::FeatureDisabled { detail } => {
            assert!(!detail.is_empty(), "detail must name the missing feature");
        }
        other => panic!("feature-off encode must return FeatureDisabled, got {other:?}"),
    }
}

#[cfg(feature = "rsmpeg")]
#[test]
fn encode_feature_on_rejects_empty_input() {
    let params = sample_params();
    let res = encode(&params, &[]);
    assert!(
        matches!(res, Err(CodecError::InvalidParams { .. })),
        "empty frame slice must surface InvalidParams before encoder init"
    );
}

#[cfg(feature = "rsmpeg")]
#[test]
fn encode_feature_on_rejects_plane_length_mismatch() {
    // A frame whose buffer doesn't match width*height for packed yuv420p.
    use verbreel_codec_native::Frame;
    let params = sample_params();
    let frame = Frame::new(1920, 1080, vec![0u8; 4]);
    let res = encode(&params, std::slice::from_ref(&frame));
    assert!(
        matches!(res, Err(CodecError::InvalidParams { .. })),
        "plane-length mismatch must surface InvalidParams"
    );
}
