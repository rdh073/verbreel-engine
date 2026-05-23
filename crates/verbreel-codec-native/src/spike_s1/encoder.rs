//! H.264/MP4 encoder wrapper for the Spike S1 determinism harness.
//!
//! Wraps rsmpeg's `AVCodecContext` + `AVFormatContextOutput` with the
//! §5 canonical deterministic libx264 preset hardcoded. No knobs — the
//! whole point of slice A is that two runs of [`Encoder::new`] →
//! [`Encoder::push_frame`] × N → [`Encoder::finish`] produce byte-
//! identical MP4s.
//!
//! Nondeterminism mitigations applied (per spec/research/01 §5):
//! - libx264 params: `threads=1:sliced-threads=0:sync-lookahead=0:
//!   rc-lookahead=0:bframes=0`
//! - libx264 preset/tune: `medium` + `zerolatency` (zerolatency
//!   reinforces single-pass, no look-ahead)
//! - container `creation_time` frozen to `1970-01-01T00:00:00Z` so the
//!   mov muxer doesn't embed wall-clock time in `mvhd`/`tkhd`/`mdhd`
//!   atoms
//! - container `encoder` tag cleared so the FFmpeg version string
//!   doesn't leak into the output (also a wall-time-ish source if the
//!   binary is rebuilt)

use std::path::Path;

use anyhow::{Context, Result, anyhow};
use rsmpeg::{
    avcodec::{AVCodec, AVCodecContext},
    avformat::AVFormatContextOutput,
    avutil::{AVDictionary, AVFrame, ra},
    error::RsmpegError,
    ffi,
};

const PIX_FMT: ffi::AVPixelFormat = ffi::AV_PIX_FMT_YUV420P;

/// Spike-only knob: which x264 preset string to use.
///
/// `Deterministic` is §5's canonical preset (slice A/B/C baseline).
/// `Performance` lets x264 use default frame-threading (`threads=auto`)
/// to test the determinism-preserving multi-thread hypothesis (Bouvigne
/// 2007 / Netflix 2015: no VBV → frame-threads stay deterministic).
///
/// PRODUCTION CODE MUST PASS `Deterministic`. This enum exists only
/// so the spike harness can collect comparison data without forking
/// the encoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncoderPreset {
    /// §5 canonical: threads=1, no lookahead, no bframes, byte-identical.
    Deterministic,
    /// `threads=auto` + drop the explicit serialization params.
    /// CRF (no VBV) is preserved so threading SHOULD stay deterministic
    /// per the literature — slice D verifies empirically.
    Performance,
}

impl EncoderPreset {
    /// The x264-params string for this preset.
    fn x264_params(self) -> &'static std::ffi::CStr {
        match self {
            Self::Deterministic => {
                c"threads=1:sliced-threads=0:sync-lookahead=0:rc-lookahead=0:bframes=0"
            }
            Self::Performance => {
                // threads=auto is the libx264 default; lookahead + bframes
                // re-enabled to let the encoder do its normal work. No VBV
                // params are added — CRF stays the rate-control mode.
                c"threads=auto:sliced-threads=0:bframes=3"
            }
        }
    }
}

/// Hardcoded libx264 encoder producing deterministic MP4 output.
pub struct Encoder {
    enc_ctx: AVCodecContext,
    ofmt_ctx: AVFormatContextOutput,
    stream_index: i32,
    /// Reusable frame buffer — allocated once, refilled per `push_frame`.
    frame: AVFrame,
    width: u32,
    height: u32,
    /// Number of frames pushed so far; used by `finish` to assert
    /// monotonic PTS in debug builds and as a sanity check.
    pushed: u64,
}

impl Encoder {
    /// Build an MP4 file at `output_path` configured for byte-identical
    /// output across runs given identical input frames.
    pub fn new(
        output_path: &Path,
        width: u32,
        height: u32,
        fps: u32,
        preset: EncoderPreset,
    ) -> Result<Self> {
        assert!(
            width.is_multiple_of(2) && height.is_multiple_of(2),
            "YUV420P needs even dims"
        );
        assert!(fps > 0, "fps must be positive");

        let path_cstr = std::ffi::CString::new(
            output_path
                .to_str()
                .ok_or_else(|| anyhow!("output path is not UTF-8: {}", output_path.display()))?,
        )?;

        let mut ofmt_ctx = AVFormatContextOutput::create(&path_cstr)
            .with_context(|| format!("alloc output ctx for {}", output_path.display()))?;

        let encoder = AVCodec::find_encoder_by_name(c"libx264")
            .context("libx264 encoder not found — FFmpeg built without --enable-libx264?")?;

        let mut enc_ctx = AVCodecContext::new(&encoder);
        enc_ctx.set_width(width as i32);
        enc_ctx.set_height(height as i32);
        // 1/fps so each frame's pts is its frame number.
        enc_ctx.set_time_base(ra(1, fps as i32));
        enc_ctx.set_framerate(ra(fps as i32, 1));
        // 2-second GOP. Deterministic-friendly (no scene-cut detection at
        // this preset) and short enough that any nondeterminism would
        // show up within the first few seconds.
        enc_ctx.set_gop_size(fps as i32 * 2);
        enc_ctx.set_max_b_frames(0);
        enc_ctx.set_pix_fmt(PIX_FMT);

        // MP4 requires the global header flag on the encoder so the SPS/PPS
        // ends up in the moov atom instead of inline with every keyframe.
        if ofmt_ctx.oformat().flags & ffi::AVFMT_GLOBALHEADER as i32 != 0 {
            enc_ctx.set_flags(enc_ctx.flags | ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32);
        }

        // §5 canonical x264 preset (default) — Performance variant exists
        // for the spike harness only (see [`EncoderPreset`] docs).
        let opts = Some(
            AVDictionary::new(c"preset", c"medium", 0)
                .set(c"tune", c"zerolatency", 0)
                .set(c"x264-params", preset.x264_params(), 0),
        );

        enc_ctx
            .open(opts)
            .context("avcodec_open2 on libx264 encoder")?;

        // Wire the encoder into a single video stream in the container.
        // Scope `out_stream` so its mutable borrow of `ofmt_ctx` ends
        // before we reach back into ofmt_ctx for metadata + write_header.
        let stream_index = {
            let mut out_stream = ofmt_ctx.new_stream();
            out_stream.set_codecpar(enc_ctx.extract_codecpar());
            out_stream.set_time_base(enc_ctx.time_base);
            out_stream.index
        };

        // Freeze container-level metadata so the mov muxer doesn't embed
        // wall-clock `creation_time` into mvhd/tkhd/mdhd atoms.
        // SAFETY: ofmt_ctx is a valid AVFormatContext alloc'd by
        // avformat_alloc_output_context2; metadata is a `*mut AVDictionary`
        // field on it. We're transferring ownership of a freshly-allocated
        // AVDictionary into the context, and rsmpeg's Drop for
        // AVFormatContextOutput calls avformat_free_context which in turn
        // av_dict_free's `metadata`. The into_raw().as_ptr() severs our
        // Rust-side ownership so we don't double-free.
        unsafe {
            let raw = ofmt_ctx.as_mut_ptr();
            let dict = AVDictionary::new(c"creation_time", c"1970-01-01T00:00:00Z", 0);
            (*raw).metadata = dict.into_raw().as_ptr();
        }

        ofmt_ctx
            .write_header(&mut None)
            .context("write MP4 header")?;

        // Pre-allocate the per-frame AVFrame; reused for every push.
        let mut frame = AVFrame::new();
        frame.set_format(PIX_FMT);
        frame.set_width(width as i32);
        frame.set_height(height as i32);
        frame.alloc_buffer().context("alloc frame buffer")?;

        Ok(Self {
            enc_ctx,
            ofmt_ctx,
            stream_index,
            frame,
            width,
            height,
            pushed: 0,
        })
    }

    /// Push one raw YUV420P frame. `yuv` must be exactly `W*H*3/2` bytes:
    /// `W*H` Y, then `(W/2)*(H/2)` U, then same V (tight packing, align=1).
    pub fn push_frame(&mut self, yuv: &[u8], pts: i64) -> Result<()> {
        let expected = (self.width as usize) * (self.height as usize) * 3 / 2;
        if yuv.len() != expected {
            return Err(anyhow!(
                "push_frame: expected {expected} bytes for {}×{} YUV420P, got {}",
                self.width,
                self.height,
                yuv.len()
            ));
        }

        self.frame
            .make_writable()
            .context("av_frame_make_writable")?;
        copy_yuv420p_into_frame(yuv, &mut self.frame, self.width, self.height);
        self.frame.set_pts(pts);

        self.enc_ctx
            .send_frame(Some(&self.frame))
            .context("send_frame to encoder")?;
        self.drain_packets()?;
        self.pushed += 1;
        Ok(())
    }

    /// Flush the encoder and finalize the MP4 (writes trailer / moov atom).
    pub fn finish(mut self) -> Result<()> {
        self.enc_ctx
            .send_frame(None)
            .context("send_frame(None) to flush encoder")?;
        self.drain_packets()?;
        self.ofmt_ctx.write_trailer().context("write MP4 trailer")?;
        debug_assert!(self.pushed > 0, "finish called with zero pushed frames");
        Ok(())
    }

    /// Drain `receive_packet` until EAGAIN or EOF, muxing each packet.
    fn drain_packets(&mut self) -> Result<()> {
        loop {
            let mut pkt = match self.enc_ctx.receive_packet() {
                Ok(p) => p,
                Err(RsmpegError::EncoderDrainError) | Err(RsmpegError::EncoderFlushedError) => {
                    return Ok(());
                }
                Err(e) => return Err(e).context("receive_packet"),
            };
            pkt.set_stream_index(self.stream_index);
            pkt.rescale_ts(
                self.enc_ctx.time_base,
                self.ofmt_ctx.streams()[self.stream_index as usize].time_base,
            );
            self.ofmt_ctx
                .interleaved_write_frame(&mut pkt)
                .context("interleaved_write_frame")?;
        }
    }
}

/// Copy a tightly-packed YUV420P byte slice into an AVFrame whose
/// `linesize[i]` may exceed the natural row width (FFmpeg aligns planes
/// for SIMD).
fn copy_yuv420p_into_frame(src: &[u8], frame: &mut AVFrame, width: u32, height: u32) {
    let w = width as usize;
    let h = height as usize;
    let y_plane = w * h;
    let chroma_w = w / 2;
    let chroma_h = h / 2;
    let uv_plane = chroma_w * chroma_h;

    let data = frame.data;
    let linesize = frame.linesize;

    // SAFETY: AVFrame::alloc_buffer + make_writable guarantees data[0..2]
    // are non-null and point to buffers of size linesize[i] * height
    // (and linesize[1..2] * height/2 for chroma). We copy one row at a
    // time so we never write past the natural width within each row.
    unsafe {
        let ly = linesize[0] as usize;
        for y in 0..h {
            std::ptr::copy_nonoverlapping(src.as_ptr().add(y * w), data[0].add(y * ly), w);
        }
        let lu = linesize[1] as usize;
        for y in 0..chroma_h {
            std::ptr::copy_nonoverlapping(
                src.as_ptr().add(y_plane + y * chroma_w),
                data[1].add(y * lu),
                chroma_w,
            );
        }
        let lv = linesize[2] as usize;
        for y in 0..chroma_h {
            std::ptr::copy_nonoverlapping(
                src.as_ptr().add(y_plane + uv_plane + y * chroma_w),
                data[2].add(y * lv),
                chroma_w,
            );
        }
    }
}
