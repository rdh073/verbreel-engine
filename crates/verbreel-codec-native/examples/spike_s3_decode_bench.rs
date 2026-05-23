//! Spike S3 — decode-bench: rsmpeg CPU decode vs gpu-video GPU decode.
//!
//! Measures per-path warm-avg fps + peak RSS during sustained decode of
//! the same 240-frame 1080p H.264 Annex-B stream produced by Spike S1
//! slice D's deterministic encoder. The harness alternates rsmpeg and
//! gpu-video runs (rsmpeg first each iteration) so any thermal or
//! cache-warmup bias affects both equally.
//!
//! Pass criteria per spec §11 S3:
//!   - ≥30% fps improvement (gpu-video / rsmpeg ≥ 1.30), OR
//!   - ≥50% peak-RSS reduction (gpu-video keeps frames GPU-side).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use verbreel_codec_native::spike_s3::gpu_decoder::GpuDecoder;
use verbreel_codec_native::spike_s3::harness::peak_rss_kb;
use verbreel_codec_native::spike_s3::rsmpeg_decoder::RsmpegDecoder;

const RUNS: u32 = 5;
const EXPECTED_FRAMES: u32 = 240;

fn bench_rsmpeg(input: &Path) -> Result<(Duration, u32, u64)> {
    let t0 = Instant::now();
    let mut decoder = RsmpegDecoder::new(input)?;
    let n = decoder.decode_all()?;
    let peak = peak_rss_kb();
    Ok((t0.elapsed(), n, peak))
}

fn bench_gpu_video(decoder: &GpuDecoder, input: &Path) -> Result<(Duration, u32, u64)> {
    let t0 = Instant::now();
    let n = decoder.decode_all_to_gpu(input)?;
    let peak = peak_rss_kb();
    Ok((t0.elapsed(), n, peak))
}

fn warm_avg(samples: &[Duration]) -> Duration {
    // Drop the first sample (cold-cache). The remaining `RUNS - 1`
    // samples are averaged. With RUNS = 5 that's 4 warm samples per
    // path, enough to swamp single-run jitter without overspending
    // wall-clock on the bench itself.
    let warm: Vec<Duration> = samples.iter().skip(1).copied().collect();
    let total: Duration = warm.iter().sum();
    total / warm.len() as u32
}

fn main() -> Result<()> {
    let input = PathBuf::from("tmp/spike_s3/input.h264");
    ensure!(
        input.exists(),
        "missing input: {} — run the Annex-B extract step from the task prompt first",
        input.display()
    );

    // One-shot adapter probe, before the bench loop. Holds onto the
    // GpuDecoder across runs so we don't re-probe each iteration; the
    // per-run Vulkan stack is rebuilt inside `decode_all_to_gpu`.
    let gpu = GpuDecoder::new().context("GpuDecoder::new (Vulkan probe)")?;
    println!("gpu-video adapter: {}", gpu.adapter_info());
    println!(
        "input: {} ({} expected frames)",
        input.display(),
        EXPECTED_FRAMES
    );

    let mut rsmpeg_walls = Vec::with_capacity(RUNS as usize);
    let mut gpu_walls = Vec::with_capacity(RUNS as usize);
    let mut rsmpeg_peak_kb: u64 = 0;
    let mut gpu_peak_kb: u64 = 0;

    for run in 0..RUNS {
        let (w, n, p) = bench_rsmpeg(&input)?;
        ensure!(
            n == EXPECTED_FRAMES,
            "rsmpeg run {run} got {n} frames, expected {EXPECTED_FRAMES}"
        );
        rsmpeg_walls.push(w);
        rsmpeg_peak_kb = rsmpeg_peak_kb.max(p);
        println!(
            "rsmpeg    run {run}: {n} frames in {w:.2?}  (peak RSS so far: {rsmpeg_peak_kb} kB)"
        );

        let (w, n, p) = bench_gpu_video(&gpu, &input)?;
        ensure!(
            n == EXPECTED_FRAMES,
            "gpu-video run {run} got {n} frames, expected {EXPECTED_FRAMES}"
        );
        gpu_walls.push(w);
        gpu_peak_kb = gpu_peak_kb.max(p);
        println!("gpu-video run {run}: {n} frames in {w:.2?}  (peak RSS so far: {gpu_peak_kb} kB)");
    }

    let rsmpeg_avg = warm_avg(&rsmpeg_walls);
    let gpu_avg = warm_avg(&gpu_walls);
    let rsmpeg_fps = EXPECTED_FRAMES as f64 / rsmpeg_avg.as_secs_f64();
    let gpu_fps = EXPECTED_FRAMES as f64 / gpu_avg.as_secs_f64();

    let fps_speedup = gpu_fps / rsmpeg_fps;
    let rss_reduction_pct = if rsmpeg_peak_kb > 0 {
        100.0 * (rsmpeg_peak_kb as f64 - gpu_peak_kb as f64) / rsmpeg_peak_kb as f64
    } else {
        0.0
    };

    println!();
    println!("=== Spike S3 — decode-bench ===");
    println!("rsmpeg    warm avg wall : {rsmpeg_avg:.2?}  fps: {rsmpeg_fps:.2}");
    println!("gpu-video warm avg wall : {gpu_avg:.2?}  fps: {gpu_fps:.2}");
    println!("FPS speedup (gpu-video / rsmpeg): {fps_speedup:.2}x");
    println!("Peak RSS rsmpeg    : {rsmpeg_peak_kb} kB");
    println!("Peak RSS gpu-video : {gpu_peak_kb} kB");
    println!("RSS reduction      : {rss_reduction_pct:.2}%");

    // §11 S3 pass-criteria check.
    let pass_fps = fps_speedup >= 1.30;
    let pass_rss = rss_reduction_pct >= 50.0;
    let verdict = if pass_fps || pass_rss { "PASS" } else { "FAIL" };

    println!();
    println!("§11 S3 pass criteria:");
    println!(
        "  fps speedup ≥1.30x : {} ({fps_speedup:.2}x)",
        if pass_fps { "PASS" } else { "FAIL" }
    );
    println!(
        "  RSS reduction ≥50% : {} ({rss_reduction_pct:.2}%)",
        if pass_rss { "PASS" } else { "FAIL" }
    );
    println!("Verdict: {verdict}");

    Ok(())
}
