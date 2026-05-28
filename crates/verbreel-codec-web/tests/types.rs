//! Integration tests for the v0 type surface — [`WebDecoder`],
//! [`DecodeError`], and the opaque [`DecodedFrame`] placeholder.
//!
//! These tests pin the enum variants, the serde wire shape, and the
//! field-level constructor / accessor contracts. They do NOT exercise
//! the decode body — that lives in `tests/decode.rs`.

use verbreel_codec_web::{DecodeError, DecodedFrame, WebDecoder};

// --- WebDecoder enum ------------------------------------------------------

#[test]
fn decoder_h264_variant_exists() {
    let d = WebDecoder::H264;
    assert_eq!(d, WebDecoder::H264);
}

#[test]
fn decoder_h265_variant_exists() {
    let d = WebDecoder::H265;
    assert_eq!(d, WebDecoder::H265);
}

#[test]
fn decoder_h264_serializes_as_lowercase_token() {
    let json = serde_json::to_string(&WebDecoder::H264).unwrap();
    assert_eq!(json, "\"h264\"");
}

#[test]
fn decoder_h265_serializes_as_lowercase_token() {
    let json = serde_json::to_string(&WebDecoder::H265).unwrap();
    assert_eq!(json, "\"h265\"");
}

#[test]
fn decoder_round_trips_through_serde() {
    for d in [WebDecoder::H264, WebDecoder::H265] {
        let json = serde_json::to_string(&d).unwrap();
        let back: WebDecoder = serde_json::from_str(&json).unwrap();
        assert_eq!(d, back);
    }
}

// --- DecodeError variants -------------------------------------------------

#[test]
fn error_not_yet_implemented_distinguishable_by_match() {
    let err = DecodeError::NotYetImplemented {
        detail: "x".to_string(),
    };
    assert!(matches!(err, DecodeError::NotYetImplemented { .. }));
    assert!(!matches!(err, DecodeError::UnsupportedCodec { .. }));
}

#[test]
fn error_unsupported_codec_distinguishable_by_match() {
    let err = DecodeError::UnsupportedCodec {
        requested: "av1".to_string(),
    };
    assert!(matches!(err, DecodeError::UnsupportedCodec { .. }));
    assert!(!matches!(err, DecodeError::NotYetImplemented { .. }));
}

#[test]
fn error_malformed_bitstream_distinguishable_by_match() {
    let err = DecodeError::MalformedBitstream {
        detail: "missing NAL start code".to_string(),
    };
    assert!(matches!(err, DecodeError::MalformedBitstream { .. }));
    assert!(!matches!(err, DecodeError::DecoderInternal { .. }));
}

#[test]
fn error_decoder_internal_distinguishable_by_match() {
    let err = DecodeError::DecoderInternal {
        detail: "VideoDecoderError: codec-error".to_string(),
    };
    assert!(matches!(err, DecodeError::DecoderInternal { .. }));
    assert!(!matches!(err, DecodeError::MalformedBitstream { .. }));
}

#[test]
fn error_display_includes_detail_for_string_variants() {
    let e = DecodeError::NotYetImplemented {
        detail: "spike_s2".to_string(),
    };
    assert!(e.to_string().contains("spike_s2"));

    let e = DecodeError::MalformedBitstream {
        detail: "bad NAL framing".to_string(),
    };
    assert!(e.to_string().contains("bad NAL framing"));

    let e = DecodeError::DecoderInternal {
        detail: "browser error 42".to_string(),
    };
    assert!(e.to_string().contains("browser error 42"));
}

#[test]
fn error_unsupported_codec_display_includes_requested() {
    let e = DecodeError::UnsupportedCodec {
        requested: "vp9".to_string(),
    };
    assert!(e.to_string().contains("vp9"));
}

// --- DecodedFrame placeholder ---------------------------------------------

#[test]
fn frame_constructor_sets_all_fields() {
    let f = DecodedFrame::new(640, 480, vec![1, 2, 3, 4], 16_667);
    assert_eq!(f.width(), 640);
    assert_eq!(f.height(), 480);
    assert_eq!(f.planes(), &[1, 2, 3, 4]);
    assert_eq!(f.pts_micros(), 16_667);
}

#[test]
fn frame_accessors_match_constructor_inputs() {
    // Fields are private by design — DecodedFrame is an opaque v0 placeholder.
    // Callers construct via DecodedFrame::new and read via the borrowed accessors.
    let planes = vec![0u8; 16 * 16];
    let f = DecodedFrame::new(16, 16, planes.clone(), 0);
    assert_eq!(f.width(), 16);
    assert_eq!(f.height(), 16);
    assert_eq!(f.planes(), planes.as_slice());
    assert_eq!(f.pts_micros(), 0);
}

#[test]
fn frame_pts_micros_round_trips_full_u64_range() {
    // pts_micros mirrors WebCodecs VideoFrame.timestamp (microseconds);
    // pin u64 range so Spike S2 cannot accidentally narrow the field.
    let f = DecodedFrame::new(2, 2, vec![], u64::MAX);
    assert_eq!(f.pts_micros(), u64::MAX);
}

#[test]
fn frame_clones_to_equal_value() {
    let f = DecodedFrame::new(2, 2, vec![9, 8, 7, 6], 1_000);
    let g = f.clone();
    assert_eq!(f, g);
}
