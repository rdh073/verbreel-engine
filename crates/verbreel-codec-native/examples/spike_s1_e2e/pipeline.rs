//! End-to-end orchestration: timeline → pipelined GPU → rsmpeg encoder.
//!
//! Drives the submit/collect loop, ensuring the GPU pipeline is kept
//! full but never overflowed: while the in-flight queue has room AND we
//! still have frames to submit, push more; otherwise drain one.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::Result;

use verbreel_codec_native::spike_s1::Encoder;
use verbreel_codec_native::spike_s1::encoder::EncoderPreset;
use verbreel_render::spike_s1::PipelinedGpu;

use super::timeline::{FPS, FrameSource, HEIGHT, TOTAL_FRAMES, Timeline, WIDTH};

pub struct E2EResult {
    pub processed_frames: u32,
    pub e2e_wall: Duration,
    pub gpu_init: Duration,
    pub encoder_init: Duration,
    /// Total time spent submitting frames to the GPU (CPU work only — no
    /// GPU wait). Should be very small; large here would indicate
    /// upload bandwidth saturation.
    pub submit_total: Duration,
    /// Total time spent in `collect_frame_blocking` waiting on GPU
    /// readback. High here = GPU is the bottleneck.
    pub collect_total: Duration,
    /// Total time spent in `encoder.push_frame` (libx264 encode + mux
    /// per frame). High here = codec is the bottleneck.
    pub encode_total: Duration,
    /// Which x264 preset this run used — for downstream reporting.
    pub preset: EncoderPreset,
}

pub fn run_once(
    output_path: &Path,
    pipeline_depth: usize,
    preset: EncoderPreset,
) -> Result<E2EResult> {
    let timeline = Timeline::new();

    let gpu_init_start = Instant::now();
    let mut gpu = PipelinedGpu::new(WIDTH, HEIGHT, pipeline_depth)?;
    let gpu_init = gpu_init_start.elapsed();

    let enc_init_start = Instant::now();
    let mut encoder = Encoder::new(output_path, WIDTH, HEIGHT, FPS, preset)?;
    let encoder_init = enc_init_start.elapsed();

    let e2e_start = Instant::now();
    let mut submitted = 0u32;
    let mut collected = 0u32;
    let mut pts = 0i64;
    let mut submit_total = Duration::ZERO;
    let mut collect_total = Duration::ZERO;
    let mut encode_total = Duration::ZERO;

    while collected < TOTAL_FRAMES {
        // Submit as many frames as the pipeline can hold.
        while submitted < TOTAL_FRAMES && gpu.in_flight() < pipeline_depth {
            let t = Instant::now();
            match timeline.frame_at(submitted) {
                FrameSource::Solo(yuv) => {
                    gpu.submit_solo_frame(yuv)?;
                }
                FrameSource::Cross { a, b, weight } => {
                    gpu.submit_crossfade_frame(a, b, weight)?;
                }
            }
            submit_total += t.elapsed();
            submitted += 1;
        }

        // Drain at least one frame (blocking if necessary).
        let t_c = Instant::now();
        let yuv_out = gpu.collect_frame_blocking()?;
        collect_total += t_c.elapsed();

        let t_e = Instant::now();
        encoder.push_frame(&yuv_out, pts)?;
        encode_total += t_e.elapsed();
        pts += 1;
        collected += 1;
    }

    let t_finish = Instant::now();
    encoder.finish()?;
    encode_total += t_finish.elapsed();
    let e2e_wall = e2e_start.elapsed();

    Ok(E2EResult {
        processed_frames: collected,
        e2e_wall,
        gpu_init,
        encoder_init,
        submit_total,
        collect_total,
        encode_total,
        preset,
    })
}
