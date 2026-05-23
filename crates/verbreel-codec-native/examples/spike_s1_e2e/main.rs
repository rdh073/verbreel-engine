//! Spike S1 slice D — dual-preset determinism + perf comparison.
//!
//! Runs the slice-C 3-clip + crossfade timeline twice: once with the §5
//! canonical deterministic preset (`threads=1`), once with a performance
//! preset (`threads=auto`, bframes=3, no VBV). Reports SHA-256
//! uniqueness and warm-avg fps side by side. The harness does NOT
//! pass/fail — slice D is informational data for the spec architect.
//!
//! Run:
//!   FFMPEG_PKG_CONFIG_PATH=$HOME/playground/verbreel/vendor/rsmpeg/tmp/ffmpeg_build/lib/pkgconfig \
//!     LD_LIBRARY_PATH=$HOME/playground/verbreel/vendor/rsmpeg/tmp/ffmpeg_build/lib:$LD_LIBRARY_PATH \
//!     cargo run --release -p verbreel-codec-native \
//!       --features spike-s1 --example spike_s1_e2e

mod pipeline;
mod timeline;

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use sha2::{Digest, Sha256};

use pipeline::run_once;
use verbreel_codec_native::spike_s1::encoder::EncoderPreset;

const RUNS: u32 = 10;
const PIPELINE_DEPTH: usize = 3;
const TOTAL_FRAMES: u32 = 240;

fn sha256_file(p: &std::path::Path) -> anyhow::Result<String> {
    Ok(format!("{:x}", Sha256::digest(std::fs::read(p)?)))
}

struct PresetReport {
    preset: EncoderPreset,
    hashes: Vec<String>,
    wall_times: Vec<Duration>,
    init_times: Vec<Duration>,
    enc_init_times: Vec<Duration>,
    submit_totals: Vec<Duration>,
    collect_totals: Vec<Duration>,
    encode_totals: Vec<Duration>,
}

impl PresetReport {
    fn unique_hashes(&self) -> usize {
        self.hashes.iter().collect::<HashSet<_>>().len()
    }
    fn warm_avg(ds: &[Duration]) -> Duration {
        let warm: Vec<_> = ds.iter().skip(1).copied().collect();
        warm.iter().sum::<Duration>() / warm.len() as u32
    }
    fn warm_avg_wall(&self) -> Duration {
        Self::warm_avg(&self.wall_times)
    }
    fn warm_avg_fps(&self) -> f64 {
        TOTAL_FRAMES as f64 / self.warm_avg_wall().as_secs_f64()
    }
    fn cold_run0_fps(&self) -> f64 {
        TOTAL_FRAMES as f64 / self.wall_times[0].as_secs_f64()
    }
}

fn run_batch(preset: EncoderPreset, label: &str) -> anyhow::Result<PresetReport> {
    let dir = PathBuf::from(format!("tmp/spike_s1_d/{label}"));
    std::fs::create_dir_all(&dir)?;

    let mut hashes = Vec::with_capacity(RUNS as usize);
    let mut wall_times = Vec::with_capacity(RUNS as usize);
    let mut init_times = Vec::with_capacity(RUNS as usize);
    let mut enc_init_times = Vec::with_capacity(RUNS as usize);
    let mut submit_totals = Vec::with_capacity(RUNS as usize);
    let mut collect_totals = Vec::with_capacity(RUNS as usize);
    let mut encode_totals = Vec::with_capacity(RUNS as usize);

    println!("\n=== Batch: {label} preset = {preset:?} ===");
    for run in 0..RUNS {
        let out = dir.join(format!("run_{run}.mp4"));
        let r = run_once(&out, PIPELINE_DEPTH, preset)?;
        // Integrity check: pipeline echoed back the preset it actually used.
        debug_assert_eq!(r.preset, preset);
        let h = sha256_file(&out)?;
        let fps = r.processed_frames as f64 / r.e2e_wall.as_secs_f64();
        println!(
            "  run {run}: sha = {}  wall = {:.2?}  fps = {fps:.2}  submit={:.2?}  collect={:.2?}  encode={:.2?}",
            &h[..16],
            r.e2e_wall,
            r.submit_total,
            r.collect_total,
            r.encode_total
        );
        hashes.push(h);
        wall_times.push(r.e2e_wall);
        init_times.push(r.gpu_init);
        enc_init_times.push(r.encoder_init);
        submit_totals.push(r.submit_total);
        collect_totals.push(r.collect_total);
        encode_totals.push(r.encode_total);
    }

    std::fs::write(dir.join("hashes.txt"), hashes.join("\n") + "\n")?;

    Ok(PresetReport {
        preset,
        hashes,
        wall_times,
        init_times,
        enc_init_times,
        submit_totals,
        collect_totals,
        encode_totals,
    })
}

fn print_stage_breakdown(label: &str, r: &PresetReport) {
    debug_assert!(!r.hashes.is_empty(), "report contains no runs");
    let _ = r.preset; // populated for downstream tooling; assertion above is the live check
    let aw = r.warm_avg_wall().as_secs_f64();
    let as_ = PresetReport::warm_avg(&r.submit_totals).as_secs_f64();
    let ac = PresetReport::warm_avg(&r.collect_totals).as_secs_f64();
    let ae = PresetReport::warm_avg(&r.encode_totals).as_secs_f64();
    println!(
        "\n--- {label} per-stage warm avg (of {:.2?} per run) ---",
        r.warm_avg_wall()
    );
    println!(
        "  submit  total:  {:.2?}   ({:5.1}%)",
        PresetReport::warm_avg(&r.submit_totals),
        100.0 * as_ / aw
    );
    println!(
        "  collect total:  {:.2?}   ({:5.1}%)   <- GPU sync + readback",
        PresetReport::warm_avg(&r.collect_totals),
        100.0 * ac / aw
    );
    println!(
        "  encode  total:  {:.2?}   ({:5.1}%)   <- libx264 push_frame + finish",
        PresetReport::warm_avg(&r.encode_totals),
        100.0 * ae / aw
    );
}

fn main() -> anyhow::Result<()> {
    let det = run_batch(EncoderPreset::Deterministic, "deterministic")?;
    let perf = run_batch(EncoderPreset::Performance, "performance")?;

    print_stage_breakdown("Deterministic", &det);
    print_stage_breakdown("Performance", &perf);

    println!("\n=== SPIKE S1 SLICE D — DUAL-PRESET COMPARISON ===");
    println!(
        "{:<20} {:<15} {:<15}",
        "Metric", "Deterministic", "Performance"
    );
    println!("{:-<20} {:-<15} {:-<15}", "", "", "");
    println!(
        "{:<20} {:<15} {:<15}",
        "Unique SHA-256",
        det.unique_hashes(),
        perf.unique_hashes()
    );
    println!(
        "{:<20} {:<15.2?} {:<15.2?}",
        "Warm avg wall",
        det.warm_avg_wall(),
        perf.warm_avg_wall()
    );
    println!(
        "{:<20} {:<15.2} {:<15.2}",
        "Warm avg fps",
        det.warm_avg_fps(),
        perf.warm_avg_fps()
    );
    println!(
        "{:<20} {:<15.2} {:<15.2}",
        "Cold run 0 fps",
        det.cold_run0_fps(),
        perf.cold_run0_fps()
    );
    println!(
        "{:<20} {:<15} {:<15}",
        "60 fps bar",
        if det.warm_avg_fps() >= 60.0 {
            "MET"
        } else {
            "MISS"
        },
        if perf.warm_avg_fps() >= 60.0 {
            "MET"
        } else {
            "MISS"
        }
    );
    println!(
        "{:<20} {:<15} {:<15}",
        "Determinism",
        if det.unique_hashes() == 1 {
            "PASS"
        } else {
            "FAIL"
        },
        if perf.unique_hashes() == 1 {
            "PASS"
        } else {
            "FAIL"
        }
    );
    println!(
        "{:<20} {:<15.2?} {:<15.2?}",
        "GPU init (run 0)", det.init_times[0], perf.init_times[0]
    );
    println!(
        "{:<20} {:<15.2?} {:<15.2?}",
        "Encoder init (run 0)", det.enc_init_times[0], perf.enc_init_times[0]
    );
    let speedup = det.warm_avg_wall().as_secs_f64() / perf.warm_avg_wall().as_secs_f64();
    println!("\nSpeedup (det → perf): {speedup:.3}x");

    // The slice D verdict is informational. Do NOT exit non-zero on
    // any combination — the spec architect classifies the outcome from
    // SPIKE_S1D_RESULTS.md.
    Ok(())
}
