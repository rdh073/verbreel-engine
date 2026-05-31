//! rsmpeg decode + probe facade. Feature-gated behind `rsmpeg`.
//!
//! Establishes the S1 decode-side invariant: a container path resolves to
//! its primary video stream, every decoded picture is normalized to packed
//! `yuv420p`, and the planes land in a [`Frame`] whose layout matches what
//! [`encode`](crate::encode()) consumes — so decode→encode is a closed
//! round-trip with no pixel-format renegotiation between the halves.
//!
//! Plane contract (shared with [`encode`](mod@crate::encode)): [`Frame::planes`] holds
//! `yuv420p` packed with no inter-row padding — the full `width * height` Y
//! plane, then the `(width / 2) * (height / 2)` U plane, then the equally
//! sized V plane. Width and height must both be even.
//!
//! Caveat: the round-trip assumes a constant pixel-format / resolution
//! stream (the S1 target). The `SwsContext` is built once from the first
//! decoded frame's source format and dimensions and cached; a mid-stream
//! format or resolution change would scale subsequent frames against a
//! stale context.

use std::ffi::CString;
use std::path::Path;

use rsmpeg::avcodec::{AVCodec, AVCodecContext};
use rsmpeg::avformat::AVFormatContextInput;
use rsmpeg::avutil::AVFrame;
use rsmpeg::error::RsmpegError;
use rsmpeg::ffi;
use rsmpeg::swscale::SwsContext;

use crate::codec::Codec;
use crate::error::CodecError;
use crate::frame::Frame;
use crate::probe::ProbeMetadata;

/// Map an `rsmpeg` error into the crate error surface.
fn map_rsmpeg(context: &str, err: &RsmpegError) -> CodecError {
    CodecError::BackendInternal {
        detail: format!("{context}: {err}"),
    }
}

/// Resolve a filesystem path into a NUL-terminated C string for `FFmpeg`.
fn path_cstr(path: &Path) -> Result<CString, CodecError> {
    let s = path.to_str().ok_or_else(|| CodecError::InvalidParams {
        detail: "path is not valid UTF-8".to_string(),
    })?;
    CString::new(s).map_err(|_| CodecError::InvalidParams {
        detail: "path contains an interior NUL byte".to_string(),
    })
}

/// Map an `FFmpeg` codec id onto the v0 [`Codec`] surface.
fn codec_from_id(id: ffi::AVCodecID) -> Result<Codec, CodecError> {
    match id {
        ffi::AVCodecID_AV_CODEC_ID_H264 => Ok(Codec::H264),
        ffi::AVCodecID_AV_CODEC_ID_PRORES => Ok(Codec::ProRes),
        other => Err(CodecError::InvalidParams {
            detail: format!("unsupported source codec id {other}"),
        }),
    }
}

/// Probe the primary video stream of `path` for `asset.probe` / `asset.import`.
///
/// Opens the container, locates the best video stream, and reads its codec,
/// coded dimensions, rational frame rate, and duration without decoding a
/// single frame.
///
/// # Errors
///
/// - [`CodecError::InvalidParams`] — path is not representable for `FFmpeg`,
///   or the stream uses a codec outside the v0 [`Codec`] surface.
/// - [`CodecError::BackendInternal`] — the container has no video stream,
///   the stream reports a negative/garbage/zero coded dimension, or libav
///   surfaced an open/probe error.
pub fn probe(path: &Path) -> Result<ProbeMetadata, CodecError> {
    let cpath = path_cstr(path)?;
    let input = AVFormatContextInput::open(&cpath, None, &mut None)
        .map_err(|e| map_rsmpeg("open input", &e))?;

    let (stream_index, _decoder) = input
        .find_best_stream(ffi::AVMediaType_AVMEDIA_TYPE_VIDEO)
        .map_err(|e| map_rsmpeg("find best stream", &e))?
        .ok_or_else(|| CodecError::BackendInternal {
            detail: "no video stream in container".to_string(),
        })?;

    let stream = input
        .streams()
        .get(stream_index)
        .ok_or_else(|| CodecError::BackendInternal {
            detail: "best video stream index out of range".to_string(),
        })?;
    let codecpar = stream.codecpar();

    // Prefer `r_frame_rate` (the stream's real/base frame rate) over
    // `avg_frame_rate`: the average is derived from packet timestamps and is
    // noisy for short clips (a 12-frame 30 fps clip can report 360/11), while
    // the base rate round-trips the encoder's declared rate exactly.
    let r = stream.r_frame_rate;
    let (fps_num, fps_den) = if r.num > 0 && r.den > 0 {
        (r.num, r.den)
    } else {
        let avg = stream.avg_frame_rate;
        (avg.num, avg.den)
    };
    if fps_num <= 0 || fps_den <= 0 {
        return Err(CodecError::BackendInternal {
            detail: "stream reports no usable frame rate".to_string(),
        });
    }

    let duration_us = {
        let d = stream.duration;
        let tb = stream.time_base;
        if d > 0 && tb.den > 0 {
            let us = i128::from(d) * i128::from(tb.num) * 1_000_000 / i128::from(tb.den);
            u64::try_from(us).ok()
        } else {
            None
        }
    };

    // A negative or garbage coded dimension is an untrusted-container
    // failure, not a 0-pixel stream: reject it rather than storing 0 in
    // `ProbeMetadata` (the same silent-coercion class the encode path
    // already rejects). `fps_num`/`fps_den` are guaranteed positive `i32`
    // by the `> 0` guard above, so their `u32` conversion cannot fail.
    let width = u32::try_from(codecpar.width).map_err(|_| CodecError::BackendInternal {
        detail: format!("stream reports invalid coded width {}", codecpar.width),
    })?;
    let height = u32::try_from(codecpar.height).map_err(|_| CodecError::BackendInternal {
        detail: format!("stream reports invalid coded height {}", codecpar.height),
    })?;
    if width == 0 || height == 0 {
        return Err(CodecError::BackendInternal {
            detail: format!("stream reports zero coded dimension {width}x{height}"),
        });
    }

    Ok(ProbeMetadata {
        codec: codec_from_id(codecpar.codec_id)?,
        width,
        height,
        fps_num: u32::try_from(fps_num).unwrap_or(0),
        fps_den: u32::try_from(fps_den).unwrap_or(0),
        duration_us,
    })
}

/// Decode every video frame of `path` into packed-`yuv420p` [`Frame`]s.
///
/// Each decoded picture is scaled (identity scale when already `yuv420p`)
/// into the packed plane layout documented at the module level. The decode
/// loop drains the decoder at EOF so trailing buffered frames are emitted.
///
/// # Errors
///
/// - [`CodecError::InvalidParams`] — unrepresentable path, or the stream
///   uses a codec outside the v0 surface.
/// - [`CodecError::BackendInternal`] — no video stream, or any libav
///   open/decode/scale error.
pub fn decode_frames(path: &Path) -> Result<Vec<Frame>, CodecError> {
    let cpath = path_cstr(path)?;
    let mut input = AVFormatContextInput::open(&cpath, None, &mut None)
        .map_err(|e| map_rsmpeg("open input", &e))?;

    let (stream_index, decoder) = input
        .find_best_stream(ffi::AVMediaType_AVMEDIA_TYPE_VIDEO)
        .map_err(|e| map_rsmpeg("find best stream", &e))?
        .ok_or_else(|| CodecError::BackendInternal {
            detail: "no video stream in container".to_string(),
        })?;

    let mut dec_ctx = decode_context(&input, stream_index, &decoder)?;

    let mut frames = Vec::new();
    let mut sws: Option<SwsContext> = None;

    loop {
        match input.read_packet() {
            Ok(Some(packet)) => {
                if usize::try_from(packet.stream_index).ok() != Some(stream_index) {
                    continue;
                }
                dec_ctx
                    .send_packet(Some(&packet))
                    .map_err(|e| map_rsmpeg("send packet", &e))?;
                drain_decoder(&mut dec_ctx, &mut sws, &mut frames)?;
            }
            Ok(None) => break,
            Err(e) => return Err(map_rsmpeg("read packet", &e)),
        }
    }

    // Flush: a null packet pushes the decoder to emit any buffered frames.
    dec_ctx
        .send_packet(None)
        .map_err(|e| map_rsmpeg("flush decoder", &e))?;
    drain_decoder(&mut dec_ctx, &mut sws, &mut frames)?;

    Ok(frames)
}

/// Build and open the decoder context for the chosen video stream.
fn decode_context(
    input: &AVFormatContextInput,
    stream_index: usize,
    decoder: &AVCodec,
) -> Result<AVCodecContext, CodecError> {
    let stream = input
        .streams()
        .get(stream_index)
        .ok_or_else(|| CodecError::BackendInternal {
            detail: "video stream index out of range".to_string(),
        })?;
    let mut dec_ctx = AVCodecContext::new(decoder);
    dec_ctx
        .apply_codecpar(&stream.codecpar())
        .map_err(|e| map_rsmpeg("apply codecpar", &e))?;
    dec_ctx
        .open(None)
        .map_err(|e| map_rsmpeg("open decoder", &e))?;
    Ok(dec_ctx)
}

/// Pull every ready frame out of the decoder, converting each to a packed
/// `yuv420p` [`Frame`].
fn drain_decoder(
    dec_ctx: &mut AVCodecContext,
    sws: &mut Option<SwsContext>,
    frames: &mut Vec<Frame>,
) -> Result<(), CodecError> {
    loop {
        match dec_ctx.receive_frame() {
            Ok(frame) => frames.push(to_yuv420p_frame(&frame, sws)?),
            Err(RsmpegError::DecoderDrainError | RsmpegError::DecoderFlushedError) => break,
            Err(e) => return Err(map_rsmpeg("receive frame", &e)),
        }
    }
    Ok(())
}

/// Convert one decoded [`AVFrame`] into the packed-`yuv420p` [`Frame`].
fn to_yuv420p_frame(src: &AVFrame, sws: &mut Option<SwsContext>) -> Result<Frame, CodecError> {
    let width = src.width;
    let height = src.height;
    let (uw, uh) = match (u32::try_from(width), u32::try_from(height)) {
        (Ok(w), Ok(h)) if w > 0 && h > 0 => (w, h),
        _ => {
            return Err(CodecError::BackendInternal {
                detail: "decoded frame has non-positive dimensions".to_string(),
            });
        }
    };
    if !uw.is_multiple_of(2) || !uh.is_multiple_of(2) {
        return Err(CodecError::BackendInternal {
            detail: format!("decoded frame has odd dimensions {uw}x{uh} (yuv420p requires even)"),
        });
    }

    let mut dst = AVFrame::new();
    dst.set_width(width);
    dst.set_height(height);
    dst.set_format(ffi::AVPixelFormat_AV_PIX_FMT_YUV420P);
    dst.alloc_buffer()
        .map_err(|e| map_rsmpeg("alloc yuv420p frame", &e))?;

    // `sws_getContext` returns null for an unsupported SOURCE pixel format,
    // which is reachable from an untrusted container — map null to an error
    // and propagate rather than panicking inside `get_or_insert_with`.
    let ctx = if let Some(ctx) = sws {
        ctx
    } else {
        let built = SwsContext::get_context(
            width,
            height,
            src.format,
            width,
            height,
            ffi::AVPixelFormat_AV_PIX_FMT_YUV420P,
            ffi::SWS_BILINEAR,
        )
        .ok_or_else(|| CodecError::BackendInternal {
            detail: format!(
                "sws_getContext returned null for source pixel format {}",
                src.format
            ),
        })?;
        sws.insert(built)
    };
    ctx.scale_frame(src, 0, height, &mut dst)
        .map_err(|e| map_rsmpeg("scale to yuv420p", &e))?;

    Ok(pack_yuv420p(&dst, uw, uh))
}

/// Copy a `yuv420p` [`AVFrame`]'s three planes into a single padding-free
/// buffer following the module-level plane contract.
fn pack_yuv420p(frame: &AVFrame, width: u32, height: u32) -> Frame {
    let w = width as usize;
    let h = height as usize;
    let cw = w / 2;
    let ch = h / 2;
    let mut planes = Vec::with_capacity(w * h + 2 * cw * ch);

    // SAFETY: `frame` is an allocated yuv420p AVFrame of `width`x`height`
    // (alloc_buffer succeeded above), so `data[0..3]` are non-null and each
    // plane has at least `linesize[i]` bytes per row for the row counts read
    // here (`h` luma rows, `ch` chroma rows). We never read past the valid
    // width of a row, so inter-row stride padding is skipped, not consumed.
    // `linesize` is always non-negative for a successfully allocated frame.
    unsafe {
        copy_plane(&mut planes, frame.data[0], frame.linesize[0], w, h);
        copy_plane(&mut planes, frame.data[1], frame.linesize[1], cw, ch);
        copy_plane(&mut planes, frame.data[2], frame.linesize[2], cw, ch);
    }

    Frame::new(width, height, planes)
}

/// Append `rows` rows of `row_bytes` bytes from a strided plane into `out`.
///
/// # Safety
///
/// `base` must point to a readable plane of at least `rows` rows, each row
/// `stride` bytes apart, with at least `row_bytes` valid bytes per row.
unsafe fn copy_plane(
    out: &mut Vec<u8>,
    base: *const u8,
    stride: i32,
    row_bytes: usize,
    rows: usize,
) {
    let stride = stride.unsigned_abs() as usize;
    for row in 0..rows {
        // SAFETY: caller guarantees `base + row * stride` starts a row with
        // at least `row_bytes` readable bytes (see callers' invariant).
        let row_ptr = unsafe { base.add(row * stride) };
        let slice = unsafe { std::slice::from_raw_parts(row_ptr, row_bytes) };
        out.extend_from_slice(slice);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An odd-dimension decoded picture is unreachable through a real libav
    /// container (coded dims are macroblock-aligned, hence even), so the
    /// even-dim guard is exercised by feeding the conversion an
    /// odd-dimension `AVFrame` directly.
    fn odd_dim_frame() -> AVFrame {
        let mut frame = AVFrame::new();
        frame.set_width(63);
        frame.set_height(48);
        frame.set_format(ffi::AVPixelFormat_AV_PIX_FMT_YUV420P);
        frame
            .alloc_buffer()
            .expect("alloc odd-dim yuv420p test frame");
        frame
    }

    #[test]
    fn odd_dimension_frame_rejected_as_backend_internal() {
        let src = odd_dim_frame();
        let mut sws = None;
        let err = to_yuv420p_frame(&src, &mut sws).unwrap_err();
        assert!(
            matches!(err, CodecError::BackendInternal { .. }),
            "odd-dimension decoded frame should be BackendInternal, got {err:?}"
        );
    }
}
