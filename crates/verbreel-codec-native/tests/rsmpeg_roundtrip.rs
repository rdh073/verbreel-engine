//! Feature-on regression: exercise both facades against synthetic frames.
//!
//! Rather than checking in a binary fixture, we synthesize a deterministic
//! frame sequence, encode it through the deterministic preset, then decode
//! it back and assert the round-trip dims / fps / frame-count match a pinned
//! manifest. A second test encodes the same sequence twice and asserts the
//! two MP4 blobs hash identically — the §11.1 byte-stability contract.

#![cfg(feature = "rsmpeg")]

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::Write;

use verbreel_codec_native::decode::{decode_frames, probe};
use verbreel_codec_native::{Codec, CodecPreset, EncodeParams, Frame, encode};

const WIDTH: u32 = 64;
const HEIGHT: u32 = 48;
const FPS_NUM: u32 = 30;
const FPS_DEN: u32 = 1;
const FRAME_COUNT: usize = 12;

fn params() -> EncodeParams {
    EncodeParams {
        codec: Codec::H264,
        preset: CodecPreset::Deterministic,
        width: WIDTH,
        height: HEIGHT,
        fps_num: FPS_NUM,
        fps_den: FPS_DEN,
        bit_rate: Some(200_000),
    }
}

/// A deterministic packed-`yuv420p` frame: a moving gradient so successive
/// frames differ (a constant frame would let the encoder collapse to a
/// single coded picture and the frame-count assertion would be vacuous).
fn synth_frame(index: usize) -> Frame {
    let w = WIDTH as usize;
    let h = HEIGHT as usize;
    let cw = w / 2;
    let ch = h / 2;
    let mut planes = Vec::with_capacity(w * h + 2 * cw * ch);
    for y in 0..h {
        for x in 0..w {
            planes.push(((x + y + index * 7) % 256) as u8);
        }
    }
    for _ in 0..(cw * ch) {
        planes.push(((128 + index) % 256) as u8);
    }
    for _ in 0..(cw * ch) {
        planes.push(((64 + index * 2) % 256) as u8);
    }
    Frame::new(WIDTH, HEIGHT, planes)
}

fn synth_sequence() -> Vec<Frame> {
    (0..FRAME_COUNT).map(synth_frame).collect()
}

fn write_temp_mp4(bytes: &[u8]) -> tempfile::NamedTempFile {
    let mut tmp = tempfile::Builder::new()
        .suffix(".mp4")
        .tempfile()
        .expect("create temp mp4");
    tmp.write_all(bytes).expect("write mp4 bytes");
    tmp.flush().expect("flush mp4");
    tmp
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

#[test]
fn roundtrip_dims_fps_frame_count_match_manifest() {
    let frames = synth_sequence();
    let mp4 = encode(&params(), &frames).expect("encode synthetic sequence");
    assert!(!mp4.is_empty(), "encode produced no bytes");

    let tmp = write_temp_mp4(&mp4);

    let meta = probe(tmp.path()).expect("probe encoded clip");
    assert_eq!(meta.codec, Codec::H264, "probed codec");
    assert_eq!(meta.width, WIDTH, "probed width");
    assert_eq!(meta.height, HEIGHT, "probed height");
    assert_eq!(meta.fps_num, FPS_NUM, "probed fps_num");
    assert_eq!(meta.fps_den, FPS_DEN, "probed fps_den");

    let decoded = decode_frames(tmp.path()).expect("decode encoded clip");
    assert_eq!(decoded.len(), FRAME_COUNT, "round-trip frame count");
    for (i, frame) in decoded.iter().enumerate() {
        assert_eq!(frame.width(), WIDTH, "decoded frame {i} width");
        assert_eq!(frame.height(), HEIGHT, "decoded frame {i} height");
        let w = WIDTH as usize;
        let h = HEIGHT as usize;
        let expected = w * h + 2 * (w / 2) * (h / 2);
        assert_eq!(
            frame.planes().len(),
            expected,
            "decoded frame {i} packed plane length"
        );
    }
}

#[test]
fn deterministic_preset_is_byte_stable() {
    let frames = synth_sequence();
    let first = encode(&params(), &frames).expect("first encode");
    let second = encode(&params(), &frames).expect("second encode");
    assert_eq!(
        sha256(&first),
        sha256(&second),
        "deterministic preset must produce byte-identical MP4 across runs"
    );
    // Guard against the assertion passing on two empty blobs.
    assert!(first.len() > 64, "encoded blob suspiciously small");
}

#[test]
fn invalid_params_rejected_before_encode() {
    use verbreel_codec_native::CodecError;
    let frames = synth_sequence();
    let mut bad = params();
    bad.width = 63; // odd — violates yuv420p
    let err = encode(&bad, &frames).unwrap_err();
    assert!(
        matches!(err, CodecError::InvalidParams { .. }),
        "odd width should be InvalidParams, got {err:?}"
    );

    let err = encode(&params(), &[]).unwrap_err();
    assert!(
        matches!(err, CodecError::InvalidParams { .. }),
        "empty frame slice should be InvalidParams, got {err:?}"
    );
}

#[test]
fn over_range_params_rejected_as_invalid_params() {
    use verbreel_codec_native::CodecError;
    let frames = synth_sequence();

    // `fps_num` past `i32::MAX` must surface InvalidParams at the i32
    // coercion, not silently zero the rate. The synthetic frames match
    // `params.width`/`height`, so `validate` clears its dimension-mismatch
    // and plane-length branches (fps is only checked non-zero there) and the
    // call reaches the `i32::try_from(fps_num)` guard.
    let mut bad_fps = params();
    bad_fps.fps_num = 3_000_000_000; // > i32::MAX
    let err = encode(&bad_fps, &frames).unwrap_err();
    assert!(
        matches!(err, CodecError::InvalidParams { .. }),
        "over-range fps_num should be InvalidParams, got {err:?}"
    );
}

#[test]
fn non_video_file_probes_as_backend_internal() {
    use verbreel_codec_native::CodecError;

    // A few bytes of text is not a media container: libav's open/probe
    // fails and the catch-all must name the failing decode-side operation
    // (BackendInternal), not the encode half.
    let mut tmp = tempfile::Builder::new()
        .suffix(".txt")
        .tempfile()
        .expect("create temp text file");
    tmp.write_all(b"not a media container\n")
        .expect("write text bytes");
    tmp.flush().expect("flush text");

    let err = probe(tmp.path()).unwrap_err();
    assert!(
        matches!(err, CodecError::BackendInternal { .. }),
        "probing a non-video file should be BackendInternal, got {err:?}"
    );

    let err = decode_frames(tmp.path()).unwrap_err();
    assert!(
        matches!(err, CodecError::BackendInternal { .. }),
        "decoding a non-video file should be BackendInternal, got {err:?}"
    );
}

#[test]
fn frame_hashing_is_stable_for_identical_planes() {
    // Frame derives Hash; equal planes hash equal. Cheap guard that the
    // synthetic generator is deterministic across calls.
    let mut a = DefaultHasher::new();
    let mut b = DefaultHasher::new();
    synth_frame(3).hash(&mut a);
    synth_frame(3).hash(&mut b);
    assert_eq!(a.finish(), b.finish());
}
