//! The public type surface stays present and compiles regardless of feature
//! state. This guards against a refactor accidentally gating a pure-data
//! type (e.g. [`ProbeMetadata`]) behind the `rsmpeg` feature.

use verbreel_codec_native::{Codec, CodecPreset, EncodeParams, Frame, ProbeMetadata};

#[test]
fn type_surface_is_present() {
    let meta = ProbeMetadata {
        codec: Codec::H264,
        width: 16,
        height: 16,
        fps_num: 30,
        fps_den: 1,
        duration_us: Some(1_000_000),
    };
    assert_eq!(meta.width, 16);

    let frame = Frame::new(2, 2, vec![0u8; 4 + 2]);
    assert_eq!(frame.width(), 2);

    let params = EncodeParams {
        codec: Codec::ProRes,
        preset: CodecPreset::Performance,
        width: 16,
        height: 16,
        fps_num: 24,
        fps_den: 1,
        bit_rate: None,
    };
    assert_eq!(params.width, 16);
}
