//! rsmpeg CPU H.264 decode baseline (software, libavcodec).
//!
//! Single-stream (best-video), drains every frame to completion. The
//! decoded `AVFrame` is dropped immediately — we do NOT extract YUV
//! bytes to the caller. Two reasons:
//!
//! 1. The S3 spike measures decode throughput, not post-decode
//!    pixel access. Both the rsmpeg path AND the gpu-video path
//!    have to do equivalent post-decode work or the comparison is
//!    unfair; for the gpu-video path that work is "hand a wgpu
//!    texture to the user", with no copy. The rsmpeg-side mirror
//!    is "hand an AVFrame to the user, drop it". Anything beyond
//!    that (tightly-packed YUV byte copy, NV12 conversion, etc.)
//!    would penalize whichever path we instrument it on.
//!
//! 2. The §11 S3 task scope is "No `unsafe` in either decoder."
//!    rsmpeg's `AVFrame::data` exposes raw `[*mut u8; 8]` plane
//!    pointers — reading those into a `Vec<u8>` requires `unsafe`
//!    `slice::from_raw_parts` per plane (see Spike S1 slice A's
//!    decoder for the pattern). By dropping the frame after the
//!    library has finished decoding it, we never touch raw
//!    pointers and the entire module stays in safe Rust.

use std::path::Path;

use anyhow::{Context, Result, anyhow};
use rsmpeg::{avcodec::AVCodecContext, avformat::AVFormatContextInput, error::RsmpegError, ffi};

pub struct RsmpegDecoder {
    ifmt_ctx: AVFormatContextInput,
    dec_ctx: AVCodecContext,
    video_stream_index: i32,
}

impl RsmpegDecoder {
    /// Open an H.264 Annex-B file (or any container FFmpeg's demuxer
    /// can sniff) and prepare a decoder context for its best video
    /// stream. Returns an error if no video stream is present or the
    /// decoder cannot be opened.
    pub fn new(input_path: &Path) -> Result<Self> {
        let path_cstr = std::ffi::CString::new(
            input_path
                .to_str()
                .ok_or_else(|| anyhow!("input path is not UTF-8: {}", input_path.display()))?,
        )?;

        let ifmt_ctx = AVFormatContextInput::open(&path_cstr)
            .with_context(|| format!("open input {}", input_path.display()))?;

        let (idx, decoder) = ifmt_ctx
            .find_best_stream(ffi::AVMEDIA_TYPE_VIDEO)
            .context("find_best_stream(video)")?
            .ok_or_else(|| anyhow!("no video stream in {}", input_path.display()))?;

        let mut dec_ctx = AVCodecContext::new(&decoder);
        dec_ctx
            .apply_codecpar(&ifmt_ctx.streams()[idx].codecpar())
            .context("apply_codecpar")?;
        dec_ctx.open(None).context("avcodec_open2 on decoder")?;

        Ok(Self {
            ifmt_ctx,
            dec_ctx,
            video_stream_index: idx as i32,
        })
    }

    /// Drive the demuxer + decoder to completion and return the
    /// total decoded frame count. Frames are decoded synchronously
    /// via `send_packet` + `receive_frame`, then dropped immediately
    /// after the library returns them — no YUV extraction, no
    /// `unsafe` reads of plane pointers.
    pub fn decode_all(&mut self) -> Result<u32> {
        let mut count: u32 = 0;

        // Feed packets until EOF.
        while let Some(pkt) = self.ifmt_ctx.read_packet().context("read_packet")? {
            if pkt.stream_index == self.video_stream_index {
                self.dec_ctx
                    .send_packet(Some(&pkt))
                    .context("send_packet")?;
                count += self.drain_decoder()?;
            }
            // Non-video packets are ignored.
        }

        // Drain phase: send a single flush packet and pull the rest.
        self.dec_ctx
            .send_packet(None)
            .context("send_packet(flush)")?;
        count += self.drain_decoder()?;

        Ok(count)
    }

    /// Pull every frame currently waiting in the decoder. Returns the
    /// number drained on this call. Returns 0 when the decoder is
    /// hungry or flushed (no more frames available this round).
    fn drain_decoder(&mut self) -> Result<u32> {
        let mut n: u32 = 0;
        loop {
            match self.dec_ctx.receive_frame() {
                Ok(_frame) => {
                    // The frame is decoded. Drop it immediately —
                    // refcount goes to zero, libav releases the
                    // backing buffers. Throughput is captured by the
                    // wall-clock surrounding the caller's loop.
                    n += 1;
                }
                Err(RsmpegError::DecoderDrainError) | Err(RsmpegError::DecoderFlushedError) => {
                    return Ok(n);
                }
                Err(e) => return Err(e).context("receive_frame"),
            }
        }
    }
}
