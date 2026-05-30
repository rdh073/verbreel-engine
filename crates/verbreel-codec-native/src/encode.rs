//! [`encode`] — libx264 / encoder facade.
//!
//! With the `rsmpeg` feature on, this lands the encode half of Spike S1:
//! packed-`yuv420p` [`Frame`]s → encoder init honoring `CodecPreset` →
//! packet sink → MP4 container bytes. With the feature off, every call
//! returns [`CodecError::FeatureDisabled`] so CI (no `FFmpeg`) links nothing.
//!
//! The S1 byte-stability invariant lives in `CodecPreset::Deterministic`:
//! the canonical `-x264-params` string and `creation_time=0` pin every
//! documented source of libx264 nondeterminism plus the container's mtime,
//! so two encodes of the same frames on the same host hash identically.

#[cfg(feature = "rsmpeg")]
use crate::codec::Codec;
use crate::error::CodecError;
use crate::frame::Frame;
use crate::params::EncodeParams;
#[cfg(feature = "rsmpeg")]
use crate::preset::CodecPreset;

/// Encode a sequence of packed-`yuv420p` frames into MP4 container bytes.
///
/// Frame plane layout MUST follow the `decode` module contract:
/// `width * height` Y bytes, then `(width/2)*(height/2)` U bytes, then the
/// equally sized V plane, with no inter-row padding. Width and height MUST
/// be even (the `yuv420p` chroma-subsampling constraint).
///
/// With `CodecPreset::Deterministic` the output is byte-identical across
/// runs on the same host; with `CodecPreset::Performance` it is not.
///
/// # Errors
///
/// - [`CodecError::FeatureDisabled`] — the crate was built without the
///   `rsmpeg` feature.
/// - [`CodecError::InvalidParams`] — empty frame slice, odd dimensions,
///   zero `fps_den`, or a frame whose plane buffer length disagrees with
///   `width`/`height`.
/// - [`CodecError::BackendInternal`] — libav surfaced an encoder, muxer,
///   or temp-file error.
#[cfg(not(feature = "rsmpeg"))]
pub fn encode(_params: &EncodeParams, _frames: &[Frame]) -> Result<Vec<u8>, CodecError> {
    Err(CodecError::FeatureDisabled {
        detail: "encode requires the `rsmpeg` feature (system `FFmpeg` 6.x)".to_string(),
    })
}

/// See the feature-off sibling for the full contract.
///
/// # Errors
///
/// See the feature-off sibling's `# Errors` section; the feature-on path
/// adds the libav-surfaced failures.
#[cfg(feature = "rsmpeg")]
pub fn encode(params: &EncodeParams, frames: &[Frame]) -> Result<Vec<u8>, CodecError> {
    imp::encode(params, frames)
}

/// Validate the params + frame buffers against the `yuv420p` plane contract.
///
/// Pure and `FFmpeg`-free, but only exercised on the feature-on encode path,
/// so it is gated to avoid a dead-code warning on the feature-off stub.
#[cfg(feature = "rsmpeg")]
pub(crate) fn validate(params: &EncodeParams, frames: &[Frame]) -> Result<(), CodecError> {
    if frames.is_empty() {
        return Err(CodecError::InvalidParams {
            detail: "frame sequence is empty".to_string(),
        });
    }
    if !params.width.is_multiple_of(2) || !params.height.is_multiple_of(2) {
        return Err(CodecError::InvalidParams {
            detail: "width and height must be even (yuv420p)".to_string(),
        });
    }
    if params.width == 0 || params.height == 0 {
        return Err(CodecError::InvalidParams {
            detail: "width and height must be non-zero".to_string(),
        });
    }
    if params.fps_den == 0 || params.fps_num == 0 {
        return Err(CodecError::InvalidParams {
            detail: "fps_num and fps_den must be non-zero".to_string(),
        });
    }
    let w = params.width as usize;
    let h = params.height as usize;
    let expected = w * h + 2 * (w / 2) * (h / 2);
    for (i, frame) in frames.iter().enumerate() {
        if frame.width() != params.width || frame.height() != params.height {
            return Err(CodecError::InvalidParams {
                detail: format!(
                    "frame {i} is {}x{}, params declare {}x{}",
                    frame.width(),
                    frame.height(),
                    params.width,
                    params.height
                ),
            });
        }
        if frame.planes().len() != expected {
            return Err(CodecError::InvalidParams {
                detail: format!(
                    "frame {i} has {} plane bytes, expected {expected} for packed yuv420p",
                    frame.planes().len()
                ),
            });
        }
    }
    Ok(())
}

#[cfg(feature = "rsmpeg")]
mod imp {
    use std::ffi::CString;

    use rsmpeg::avcodec::{AVCodec, AVCodecContext};
    use rsmpeg::avformat::AVFormatContextOutput;
    use rsmpeg::avutil::{AVDictionary, AVFrame, AVRational};
    use rsmpeg::error::RsmpegError;
    use rsmpeg::ffi;

    use super::{Codec, CodecError, CodecPreset, EncodeParams, Frame, validate};

    /// Canonical §11.1 x264 parameter string. Removes the four documented
    /// libx264 nondeterminism sources (frame threads, slice threads, both
    /// lookahead pipelines, B-frame reorder) so deterministic-mode output
    /// is byte-stable across runs.
    const DETERMINISTIC_X264_PARAMS: &str =
        "threads=1:sliced-threads=0:sync-lookahead=0:rc-lookahead=0:bframes=0";

    fn map_rsmpeg(context: &str, err: &RsmpegError) -> CodecError {
        CodecError::BackendInternal {
            detail: format!("{context}: {err}"),
        }
    }

    fn cstr(s: &str) -> Result<CString, CodecError> {
        CString::new(s).map_err(|_| CodecError::InvalidParams {
            detail: format!("interior NUL byte in `{s}`"),
        })
    }

    /// `FFmpeg` encoder short-name for a v0 codec.
    fn encoder_name(codec: Codec) -> &'static str {
        match codec {
            Codec::H264 => "libx264",
            Codec::ProRes => "prores_ks",
        }
    }

    pub(super) fn encode(params: &EncodeParams, frames: &[Frame]) -> Result<Vec<u8>, CodecError> {
        validate(params, frames)?;

        // Mux to a temp file, then read the bytes back. `FFmpeg`'s MP4 muxer
        // needs a seekable sink to backpatch the moov atom; a temp file is
        // the simplest seekable AVIO and avoids a fragile custom-IO buffer.
        let tmp = tempfile::Builder::new()
            .suffix(".mp4")
            .tempfile()
            .map_err(|e| CodecError::BackendInternal {
                detail: format!("create temp file: {e}"),
            })?;
        let tmp_path = tmp.path().to_path_buf();
        let cpath = cstr(
            tmp_path
                .to_str()
                .ok_or_else(|| CodecError::BackendInternal {
                    detail: "temp path not valid UTF-8".to_string(),
                })?,
        )?;

        let enc_name = cstr(encoder_name(params.codec))?;
        let encoder = AVCodec::find_encoder_by_name(&enc_name).ok_or_else(|| {
            CodecError::BackendInternal {
                detail: format!(
                    "encoder `{}` not found in this build",
                    encoder_name(params.codec)
                ),
            }
        })?;

        let fps_num = i32::try_from(params.fps_num).map_err(|_| CodecError::InvalidParams {
            detail: format!("fps_num {} exceeds i32 range", params.fps_num),
        })?;
        let fps_den = i32::try_from(params.fps_den).map_err(|_| CodecError::InvalidParams {
            detail: format!("fps_den {} exceeds i32 range", params.fps_den),
        })?;
        let width = i32::try_from(params.width).map_err(|_| CodecError::InvalidParams {
            detail: format!("width {} exceeds i32 range", params.width),
        })?;
        let height = i32::try_from(params.height).map_err(|_| CodecError::InvalidParams {
            detail: format!("height {} exceeds i32 range", params.height),
        })?;

        let time_base = AVRational {
            num: fps_den,
            den: fps_num,
        };
        let frame_rate = AVRational {
            num: fps_num,
            den: fps_den,
        };

        let mut enc_ctx = AVCodecContext::new(&encoder);
        enc_ctx.set_width(width);
        enc_ctx.set_height(height);
        enc_ctx.set_pix_fmt(ffi::AVPixelFormat_AV_PIX_FMT_YUV420P);
        enc_ctx.set_time_base(time_base);
        enc_ctx.set_framerate(frame_rate);
        if let Some(bit_rate) = params.bit_rate {
            enc_ctx.set_bit_rate(i64::from(bit_rate));
        }

        // The MP4 muxer requires the codec's extradata in the container
        // header (global header) rather than in-band.
        enc_ctx.set_flags(
            enc_ctx.flags | i32::try_from(ffi::AV_CODEC_FLAG_GLOBAL_HEADER).unwrap_or(0),
        );

        let open_opts = encoder_options(params)?;
        enc_ctx
            .open(open_opts)
            .map_err(|e| map_rsmpeg("open encoder", &e))?;

        let mut out_ctx = AVFormatContextOutput::create(&cpath, None)
            .map_err(|e| map_rsmpeg("create output", &e))?;
        {
            let mut stream = out_ctx.new_stream();
            stream.set_codecpar(enc_ctx.extract_codecpar());
            stream.set_time_base(time_base);
        }

        let mut header_opts = container_options(params)?;
        out_ctx
            .write_header(&mut header_opts)
            .map_err(|e| map_rsmpeg("write header", &e))?;

        for (i, frame) in frames.iter().enumerate() {
            let av_frame = build_av_frame(params, frame, i64::try_from(i).unwrap_or(0))?;
            enc_ctx
                .send_frame(Some(&av_frame))
                .map_err(|e| map_rsmpeg("send frame", &e))?;
            drain_encoder(&mut enc_ctx, &mut out_ctx, time_base)?;
        }

        enc_ctx
            .send_frame(None)
            .map_err(|e| map_rsmpeg("flush encoder", &e))?;
        drain_encoder(&mut enc_ctx, &mut out_ctx, time_base)?;

        out_ctx
            .write_trailer()
            .map_err(|e| map_rsmpeg("write trailer", &e))?;
        // Drop the output context (flushes AVIO) before reading the file.
        drop(out_ctx);

        std::fs::read(&tmp_path).map_err(|e| CodecError::BackendInternal {
            detail: format!("read encoded temp file: {e}"),
        })
    }

    /// Encoder private options. Deterministic mode pins the canonical x264
    /// parameter string; performance mode leaves encoder defaults.
    fn encoder_options(params: &EncodeParams) -> Result<Option<AVDictionary>, CodecError> {
        match (params.codec, params.preset) {
            (Codec::H264, CodecPreset::Deterministic) => {
                let key = cstr("x264-params")?;
                let val = cstr(DETERMINISTIC_X264_PARAMS)?;
                Ok(Some(AVDictionary::new(&key, &val, 0)))
            }
            _ => Ok(None),
        }
    }

    /// Container muxer options. Deterministic mode pins `creation_time=0`
    /// so the MP4 mvhd/tkhd timestamps don't leak wall-clock into the bytes.
    fn container_options(params: &EncodeParams) -> Result<Option<AVDictionary>, CodecError> {
        match params.preset {
            CodecPreset::Deterministic => {
                let key = cstr("creation_time")?;
                let val = cstr("0")?;
                Ok(Some(AVDictionary::new(&key, &val, 0)))
            }
            CodecPreset::Performance => Ok(None),
        }
    }

    /// Materialize a packed-`yuv420p` [`Frame`] into an [`AVFrame`].
    fn build_av_frame(
        params: &EncodeParams,
        frame: &Frame,
        pts: i64,
    ) -> Result<AVFrame, CodecError> {
        let width = i32::try_from(params.width).map_err(|_| CodecError::InvalidParams {
            detail: format!("width {} exceeds i32 range", params.width),
        })?;
        let height = i32::try_from(params.height).map_err(|_| CodecError::InvalidParams {
            detail: format!("height {} exceeds i32 range", params.height),
        })?;

        let mut av_frame = AVFrame::new();
        av_frame.set_width(width);
        av_frame.set_height(height);
        av_frame.set_format(ffi::AVPixelFormat_AV_PIX_FMT_YUV420P);
        av_frame.set_pts(pts);
        av_frame
            .alloc_buffer()
            .map_err(|e| map_rsmpeg("alloc encode frame", &e))?;

        let luma_w = params.width as usize;
        let luma_h = params.height as usize;
        let chroma_w = luma_w / 2;
        let chroma_h = luma_h / 2;
        let planes = frame.planes();
        let (y_plane, rest) = planes.split_at(luma_w * luma_h);
        let (u_plane, v_plane) = rest.split_at(chroma_w * chroma_h);

        // SAFETY: `av_frame` is a freshly allocated yuv420p frame of
        // `width`x`height`; `data[0..3]` are non-null with `linesize[i]`
        // bytes per row. We write exactly luma_w/chroma_w bytes per row for
        // luma_h/chroma_h rows, never exceeding a plane's row width, so
        // strided destination writes stay in bounds.
        unsafe {
            write_plane(
                av_frame.data[0],
                av_frame.linesize[0],
                y_plane,
                luma_w,
                luma_h,
            );
            write_plane(
                av_frame.data[1],
                av_frame.linesize[1],
                u_plane,
                chroma_w,
                chroma_h,
            );
            write_plane(
                av_frame.data[2],
                av_frame.linesize[2],
                v_plane,
                chroma_w,
                chroma_h,
            );
        }

        Ok(av_frame)
    }

    /// Copy a padding-free plane into a strided destination plane.
    ///
    /// # Safety
    ///
    /// `dst` must point to a writable plane of at least `rows` rows, each
    /// `stride` bytes apart, with at least `row_bytes` writable bytes per
    /// row. `src` must hold exactly `rows * row_bytes` bytes.
    unsafe fn write_plane(dst: *mut u8, stride: i32, src: &[u8], row_bytes: usize, rows: usize) {
        let stride = stride.unsigned_abs() as usize;
        for row in 0..rows {
            // SAFETY: caller guarantees `dst + row * stride` starts a row
            // with at least `row_bytes` writable bytes, and `src` is laid out
            // row-major with `row_bytes` per row.
            let dst_row = unsafe { dst.add(row * stride) };
            let src_row = &src[row * row_bytes..(row + 1) * row_bytes];
            unsafe {
                std::ptr::copy_nonoverlapping(src_row.as_ptr(), dst_row, row_bytes);
            }
        }
    }

    /// Pull every ready packet out of the encoder and mux it.
    fn drain_encoder(
        enc_ctx: &mut AVCodecContext,
        out_ctx: &mut AVFormatContextOutput,
        time_base: AVRational,
    ) -> Result<(), CodecError> {
        loop {
            match enc_ctx.receive_packet() {
                Ok(mut packet) => {
                    packet.set_stream_index(0);
                    let stream_tb = out_ctx.streams().get(0).map_or(time_base, |s| s.time_base);
                    packet.rescale_ts(time_base, stream_tb);
                    out_ctx
                        .interleaved_write_frame(&mut packet)
                        .map_err(|e| map_rsmpeg("write frame", &e))?;
                }
                Err(RsmpegError::EncoderDrainError | RsmpegError::EncoderFlushedError) => break,
                Err(e) => return Err(map_rsmpeg("receive packet", &e)),
            }
        }
        Ok(())
    }
}
