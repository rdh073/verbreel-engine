//! Integration tests for the v0 type surface — [`Codec`],
//! [`CodecPreset`], [`EncodeParams`], [`CodecError`], and the
//! opaque [`Frame`] placeholder.
//!
//! These tests pin the enum variants, the serde wire shape, and the
//! field-level constructor / accessor contracts. They do NOT exercise
//! the encode body — that lives in `tests/encode.rs`.

use verbreel_codec_native::{Codec, CodecError, CodecPreset, EncodeParams, Frame};

// --- Codec enum -----------------------------------------------------------

#[test]
fn codec_h264_variant_exists() {
    let c = Codec::H264;
    assert_eq!(c, Codec::H264);
}

#[test]
fn codec_prores_variant_exists() {
    let c = Codec::ProRes;
    assert_eq!(c, Codec::ProRes);
}

#[test]
fn codec_h264_serializes_as_lowercase_token() {
    let json = serde_json::to_string(&Codec::H264).unwrap();
    assert_eq!(json, "\"h264\"");
}

#[test]
fn codec_prores_serializes_as_lowercase_token() {
    let json = serde_json::to_string(&Codec::ProRes).unwrap();
    assert_eq!(json, "\"prores\"");
}

#[test]
fn codec_round_trips_through_serde() {
    for c in [Codec::H264, Codec::ProRes] {
        let json = serde_json::to_string(&c).unwrap();
        let back: Codec = serde_json::from_str(&json).unwrap();
        assert_eq!(c, back);
    }
}

// --- CodecPreset enum -----------------------------------------------------

#[test]
fn preset_deterministic_variant_exists() {
    let p = CodecPreset::Deterministic;
    assert_eq!(p, CodecPreset::Deterministic);
}

#[test]
fn preset_performance_variant_exists() {
    let p = CodecPreset::Performance;
    assert_eq!(p, CodecPreset::Performance);
}

#[test]
fn preset_deterministic_serializes_as_lowercase_token() {
    let json = serde_json::to_string(&CodecPreset::Deterministic).unwrap();
    assert_eq!(json, "\"deterministic\"");
}

#[test]
fn preset_performance_serializes_as_lowercase_token() {
    let json = serde_json::to_string(&CodecPreset::Performance).unwrap();
    assert_eq!(json, "\"performance\"");
}

#[test]
fn preset_round_trips_through_serde() {
    for p in [CodecPreset::Deterministic, CodecPreset::Performance] {
        let json = serde_json::to_string(&p).unwrap();
        let back: CodecPreset = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }
}

// --- EncodeParams struct --------------------------------------------------

#[test]
fn encode_params_constructs_with_all_fields() {
    let p = EncodeParams {
        codec: Codec::H264,
        preset: CodecPreset::Deterministic,
        width: 1920,
        height: 1080,
        fps_num: 30,
        fps_den: 1,
        bit_rate: Some(8_000_000),
    };
    assert_eq!(p.codec, Codec::H264);
    assert_eq!(p.preset, CodecPreset::Deterministic);
    assert_eq!(p.width, 1920);
    assert_eq!(p.height, 1080);
    assert_eq!(p.fps_num, 30);
    assert_eq!(p.fps_den, 1);
    assert_eq!(p.bit_rate, Some(8_000_000));
}

#[test]
fn encode_params_round_trips_through_serde() {
    let p = EncodeParams {
        codec: Codec::H264,
        preset: CodecPreset::Performance,
        width: 1280,
        height: 720,
        fps_num: 30_000,
        fps_den: 1_001,
        bit_rate: Some(4_000_000),
    };
    let json = serde_json::to_string(&p).unwrap();
    let back: EncodeParams = serde_json::from_str(&json).unwrap();
    assert_eq!(p, back);
}

#[test]
fn encode_params_bit_rate_omitted_when_none() {
    let p = EncodeParams {
        codec: Codec::ProRes,
        preset: CodecPreset::Performance,
        width: 1920,
        height: 1080,
        fps_num: 24,
        fps_den: 1,
        bit_rate: None,
    };
    let json = serde_json::to_string(&p).unwrap();
    // skip_serializing_if = "Option::is_none" must drop the field.
    assert!(
        !json.contains("bit_rate"),
        "bit_rate=None must be omitted from wire shape: {json}"
    );
    let back: EncodeParams = serde_json::from_str(&json).unwrap();
    assert_eq!(p, back);
    assert_eq!(back.bit_rate, None);
}

#[test]
fn encode_params_deserializes_with_missing_bit_rate() {
    // Inbound payload without the optional field must deserialize to None.
    let json = r#"{"codec":"h264","preset":"deterministic","width":640,"height":480,"fps_num":24,"fps_den":1}"#;
    let p: EncodeParams = serde_json::from_str(json).unwrap();
    assert_eq!(p.codec, Codec::H264);
    assert_eq!(p.preset, CodecPreset::Deterministic);
    assert_eq!(p.bit_rate, None);
}

// --- CodecError variants --------------------------------------------------

#[test]
fn error_not_yet_implemented_distinguishable_by_match() {
    let err = CodecError::NotYetImplemented {
        detail: "x".to_string(),
    };
    assert!(matches!(err, CodecError::NotYetImplemented { .. }));
    assert!(!matches!(err, CodecError::InputExhausted));
}

#[test]
fn error_invalid_params_distinguishable_by_match() {
    let err = CodecError::InvalidParams {
        detail: "width must be even".to_string(),
    };
    assert!(matches!(err, CodecError::InvalidParams { .. }));
    assert!(!matches!(err, CodecError::NotYetImplemented { .. }));
}

#[test]
fn error_input_exhausted_distinguishable_by_match() {
    let err = CodecError::InputExhausted;
    assert!(matches!(err, CodecError::InputExhausted));
    assert!(!matches!(err, CodecError::BackendInternal { .. }));
}

#[test]
fn error_backend_internal_distinguishable_by_match() {
    let err = CodecError::BackendInternal {
        detail: "libav -22".to_string(),
    };
    assert!(matches!(err, CodecError::BackendInternal { .. }));
    assert!(!matches!(err, CodecError::InvalidParams { .. }));
}

#[test]
fn error_display_includes_detail_for_string_variants() {
    let e = CodecError::NotYetImplemented {
        detail: "spike_s1".to_string(),
    };
    assert!(e.to_string().contains("spike_s1"));

    let e = CodecError::InvalidParams {
        detail: "odd width".to_string(),
    };
    assert!(e.to_string().contains("odd width"));

    let e = CodecError::BackendInternal {
        detail: "libav -22".to_string(),
    };
    assert!(e.to_string().contains("libav -22"));
}

#[test]
fn error_display_for_input_exhausted_is_non_empty() {
    let e = CodecError::InputExhausted;
    assert!(!e.to_string().is_empty());
}

// --- Frame placeholder ----------------------------------------------------

#[test]
fn frame_constructor_sets_all_fields() {
    let f = Frame::new(640, 480, vec![1, 2, 3, 4]);
    assert_eq!(f.width(), 640);
    assert_eq!(f.height(), 480);
    assert_eq!(f.planes(), &[1, 2, 3, 4]);
}

#[test]
fn frame_accessors_match_constructor_inputs() {
    // Fields are private by design — Frame is an opaque v0 placeholder.
    // Callers construct via Frame::new and read via the borrowed accessors.
    let planes = vec![0u8; 16 * 16];
    let f = Frame::new(16, 16, planes.clone());
    assert_eq!(f.width(), 16);
    assert_eq!(f.height(), 16);
    assert_eq!(f.planes(), planes.as_slice());
}

#[test]
fn frame_clones_to_equal_value() {
    let f = Frame::new(2, 2, vec![9, 8, 7, 6]);
    let g = f.clone();
    assert_eq!(f, g);
}
