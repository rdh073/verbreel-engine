//! H.264/MP4 decoder wrapper for the Spike S1 determinism harness.
//!
//! Single-stream (best-video), pulls one decoded frame at a time and
//! re-emits it as tightly-packed YUV420P bytes (linesize handled
//! internally so callers see a clean `W*H*3/2` blob per frame). Audio
//! and metadata streams are ignored.

use std::path::Path;

use anyhow::{Context, Result, anyhow};
use rsmpeg::{
    avcodec::{AVCodecContext, AVPacket},
    avformat::AVFormatContextInput,
    avutil::AVFrame,
    error::RsmpegError,
    ffi,
};

pub struct Decoder {
    ifmt_ctx: AVFormatContextInput,
    dec_ctx: AVCodecContext,
    video_stream_index: i32,
    /// Frames already pulled out of the decoder but not yet returned to
    /// the caller. Lets us drain multiple frames from a single packet
    /// without losing them across `next_frame` calls.
    pending: std::collections::VecDeque<Vec<u8>>,
    /// True once we've sent the flush packet (None) to the decoder.
    /// Prevents repeated flush sends, which `send_packet` errors on.
    flushed: bool,
}

impl Decoder {
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
            pending: std::collections::VecDeque::new(),
            flushed: false,
        })
    }

    /// Pull one decoded YUV420P frame. Returns `Ok(None)` once the
    /// stream is exhausted.
    pub fn next_frame(&mut self) -> Result<Option<Vec<u8>>> {
        loop {
            if let Some(buf) = self.pending.pop_front() {
                return Ok(Some(buf));
            }

            // No pending frames — feed one more packet (or flush) and
            // collect everything the decoder emits.
            if !self.flushed {
                match self.ifmt_ctx.read_packet().context("read_packet")? {
                    Some(pkt) => {
                        if pkt.stream_index == self.video_stream_index {
                            self.send_and_collect(Some(&pkt))?;
                        }
                        // Non-video packets: just keep reading.
                    }
                    None => {
                        // EOF on demuxer — flush the decoder.
                        self.flushed = true;
                        self.send_and_collect(None)?;
                    }
                }
            } else {
                // Already flushed and no more pending frames.
                return Ok(None);
            }
        }
    }

    fn send_and_collect(&mut self, pkt: Option<&AVPacket>) -> Result<()> {
        self.dec_ctx.send_packet(pkt).context("send_packet")?;
        loop {
            match self.dec_ctx.receive_frame() {
                Ok(frame) => {
                    let yuv = frame_to_tight_yuv420p(&frame)?;
                    self.pending.push_back(yuv);
                }
                Err(RsmpegError::DecoderDrainError) | Err(RsmpegError::DecoderFlushedError) => {
                    return Ok(());
                }
                Err(e) => return Err(e).context("receive_frame"),
            }
        }
    }
}

/// Copy an AVFrame's three planes into a contiguous YUV420P byte vec
/// with row-stride stripped (align=1, tight packing).
fn frame_to_tight_yuv420p(frame: &AVFrame) -> Result<Vec<u8>> {
    if frame.format != ffi::AV_PIX_FMT_YUV420P {
        return Err(anyhow!(
            "decoded frame is not YUV420P (format={}); spike S1 slice A \
             assumes the input was encoded by our own libx264 encoder",
            frame.format
        ));
    }
    let w = frame.width as usize;
    let h = frame.height as usize;
    let chroma_w = w / 2;
    let chroma_h = h / 2;

    let mut out = Vec::with_capacity(w * h * 3 / 2);

    // SAFETY: AVFrame guarantees data[0..2] are non-null and point to
    // buffers of size linesize[i] * height (or height/2 for chroma).
    // We read exactly `w` (or `chroma_w`) bytes per row, never past the
    // natural width, so out-of-bounds is impossible regardless of the
    // alignment padding linesize encodes.
    unsafe {
        let ly = frame.linesize[0] as usize;
        for y in 0..h {
            out.extend_from_slice(std::slice::from_raw_parts(frame.data[0].add(y * ly), w));
        }
        let lu = frame.linesize[1] as usize;
        for y in 0..chroma_h {
            out.extend_from_slice(std::slice::from_raw_parts(
                frame.data[1].add(y * lu),
                chroma_w,
            ));
        }
        let lv = frame.linesize[2] as usize;
        for y in 0..chroma_h {
            out.extend_from_slice(std::slice::from_raw_parts(
                frame.data[2].add(y * lv),
                chroma_w,
            ));
        }
    }

    Ok(out)
}
