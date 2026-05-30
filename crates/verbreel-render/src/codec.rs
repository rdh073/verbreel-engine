//! rsmpeg decode/encode facade — delegates to [`verbreel_codec_native`].
//!
//! `verbreel-render` owns the wgpu compositing stage; the actual container
//! decode and MP4 encode live in `verbreel-codec-native` behind its own
//! `rsmpeg` feature (system `FFmpeg` 6.x). This module is the thin adapter that
//! the GPU stage hands frames to: it maps a [`RenderPreset`] onto the codec
//! crate's [`CodecPreset`] and normalises the codec-side error into a
//! [`RenderError`].
//!
//! Feature gating: with the render crate's `rsmpeg` feature off, the encode
//! and decode entry points compile but fail closed with
//! [`RenderError::Codec`] rather than linking `FFmpeg` — so feature-off
//! `cargo check --workspace` stays toolchain-free and CI (no `FFmpeg`) never
//! links a codec backend. The feature-on path forwards to the real rsmpeg
//! calls.

use crate::error::RenderError;
use crate::preset::RenderPreset;

/// Map a render preset onto the codec crate's preset. Deterministic ->
/// deterministic (byte-stable libx264 params), performance -> performance.
#[cfg(feature = "rsmpeg")]
fn codec_preset(preset: RenderPreset) -> verbreel_codec_native::CodecPreset {
    match preset {
        RenderPreset::Deterministic => verbreel_codec_native::CodecPreset::Deterministic,
        RenderPreset::Performance => verbreel_codec_native::CodecPreset::Performance,
    }
}

/// Decode a source container into packed-yuv420p frames.
///
/// Feature-off this returns [`RenderError::Codec`] immediately. Feature-on it
/// forwards to [`verbreel_codec_native::decode::decode_frames`].
///
/// # Errors
///
/// - [`RenderError::Codec`] — the `rsmpeg` feature is off, or the codec-side
///   decode failed (the body carries the codec error).
#[cfg(feature = "rsmpeg")]
pub fn decode_source(
    path: &std::path::Path,
) -> Result<Vec<verbreel_codec_native::Frame>, RenderError> {
    verbreel_codec_native::decode::decode_frames(path).map_err(|e| RenderError::Codec {
        detail: format!("decode `{}`: {e}", path.display()),
    })
}

/// Feature-off decode stub. Fails closed without linking `FFmpeg`.
///
/// # Errors
///
/// Always returns [`RenderError::Codec`] — the `rsmpeg` feature is off.
#[cfg(not(feature = "rsmpeg"))]
pub fn decode_source(
    path: &std::path::Path,
) -> Result<Vec<verbreel_codec_native::Frame>, RenderError> {
    let _ = path;
    Err(RenderError::Codec {
        detail: "decode requires the render crate's `rsmpeg` feature (system `FFmpeg` 6.x)"
            .to_string(),
    })
}

/// Encode composited yuv420p frames into MP4 container bytes using the codec
/// preset that matches `preset`.
///
/// `frames` carry packed yuv420p planes at `width`x`height`; the render
/// preset selects the byte-stable deterministic encode params or the
/// performance ones. Forwards to [`verbreel_codec_native::encode`].
///
/// # Errors
///
/// - [`RenderError::Codec`] — the `rsmpeg` feature is off, or the codec-side
///   encode failed (invalid params, libav error).
#[cfg(feature = "rsmpeg")]
pub fn encode_frames(
    preset: RenderPreset,
    width: u32,
    height: u32,
    fps_num: u32,
    fps_den: u32,
    frames: &[verbreel_codec_native::Frame],
) -> Result<Vec<u8>, RenderError> {
    let params = verbreel_codec_native::EncodeParams {
        codec: verbreel_codec_native::Codec::H264,
        preset: codec_preset(preset),
        width,
        height,
        fps_num,
        fps_den,
        bit_rate: None,
    };
    verbreel_codec_native::encode(&params, frames).map_err(|e| RenderError::Codec {
        detail: format!("encode {width}x{height}@{fps_num}/{fps_den}: {e}"),
    })
}

/// Feature-off encode stub. Fails closed without linking `FFmpeg`.
///
/// # Errors
///
/// Always returns [`RenderError::Codec`] — the `rsmpeg` feature is off.
#[cfg(not(feature = "rsmpeg"))]
pub fn encode_frames(
    preset: RenderPreset,
    width: u32,
    height: u32,
    fps_num: u32,
    fps_den: u32,
    frames: &[verbreel_codec_native::Frame],
) -> Result<Vec<u8>, RenderError> {
    let _ = (preset, width, height, fps_num, fps_den, frames);
    Err(RenderError::Codec {
        detail: "encode requires the render crate's `rsmpeg` feature (system `FFmpeg` 6.x)"
            .to_string(),
    })
}
