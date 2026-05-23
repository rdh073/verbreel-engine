//! Spike S1 slice C — end-to-end determinism + perf harness.
//!
//! Wires slice A's rsmpeg encoder/decoder (encoder side only — decode
//! is bit-exact within slice A and re-tested implicitly via final
//! decode-back checks if desired) and slice B's wgpu YUV↔RGB pipeline
//! into a single 3-clip+1-crossfade timeline. Runs 10 sequential
//! end-to-end passes and reports the §11 S1 pass/fail signal.
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

const RUNS: u32 = 10;
const PIPELINE_DEPTH: usize = 3;
const TOTAL_FRAMES_F64: f64 = 240.0;

fn sha256_file(p: &std::path::Path) -> anyhow::Result<String> {
    Ok(format!("{:x}", Sha256::digest(std::fs::read(p)?)))
}

fn main() -> anyhow::Result<()> {
    std::fs::create_dir_all("tmp/spike_s1_c")?;
    let mut hashes: Vec<String> = Vec::with_capacity(RUNS as usize);
    let mut wall_times: Vec<Duration> = Vec::with_capacity(RUNS as usize);
    let mut init_times: Vec<Duration> = Vec::with_capacity(RUNS as usize);
    let mut enc_init_times: Vec<Duration> = Vec::with_capacity(RUNS as usize);
    let mut submit_totals: Vec<Duration> = Vec::with_capacity(RUNS as usize);
    let mut collect_totals: Vec<Duration> = Vec::with_capacity(RUNS as usize);
    let mut encode_totals: Vec<Duration> = Vec::with_capacity(RUNS as usize);

    for run in 0..RUNS {
        let out = PathBuf::from(format!("tmp/spike_s1_c/run_{run}.mp4"));
        let r = run_once(&out, PIPELINE_DEPTH)?;
        let h = sha256_file(&out)?;
        let fps = r.processed_frames as f64 / r.e2e_wall.as_secs_f64();
        println!(
            "run {run}: sha = {}  wall = {:.2?}  fps = {fps:.2}  submit={:.2?}  collect={:.2?}  encode={:.2?}",
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

    std::fs::write("tmp/spike_s1_c/hashes.txt", hashes.join("\n") + "\n")?;

    // Skip run 0 for fps stats (cold GPU + libx264 first-frame setup).
    fn avg(ds: &[Duration]) -> Duration {
        let sum: Duration = ds.iter().copied().sum();
        sum / ds.len() as u32
    }
    let warm_wall: Vec<Duration> = wall_times.iter().skip(1).copied().collect();
    let warm_submit: Vec<Duration> = submit_totals.iter().skip(1).copied().collect();
    let warm_collect: Vec<Duration> = collect_totals.iter().skip(1).copied().collect();
    let warm_encode: Vec<Duration> = encode_totals.iter().skip(1).copied().collect();
    let avg_wall = avg(&warm_wall);
    let avg_fps = TOTAL_FRAMES_F64 / avg_wall.as_secs_f64();
    let cold_run0 = wall_times[0];

    let unique: HashSet<&String> = hashes.iter().collect();
    println!();
    println!("=== Spike S1 — END-TO-END RESULTS ===");
    println!("Unique SHA-256 across {RUNS} runs: {}", unique.len());
    println!("Cold run 0 wall:                   {cold_run0:.2?}");
    println!(
        "Warm avg wall (runs 1-{}):          {avg_wall:.2?}",
        RUNS - 1
    );
    println!("Warm avg end-to-end fps:           {avg_fps:.2}");
    println!("GPU init cost (run 0):             {:.2?}", init_times[0]);
    println!(
        "Encoder init cost (run 0):         {:.2?}",
        enc_init_times[0]
    );
    println!();
    println!("Per-stage warm avg (out of {:.2?} per run):", avg_wall);
    let aw = avg_wall.as_secs_f64();
    let as_ = avg(&warm_submit).as_secs_f64();
    let ac = avg(&warm_collect).as_secs_f64();
    let ae = avg(&warm_encode).as_secs_f64();
    println!(
        "  submit  total:  {:.2?}   ({:5.1}%)",
        avg(&warm_submit),
        100.0 * as_ / aw
    );
    println!(
        "  collect total:  {:.2?}   ({:5.1}%)   <- GPU sync + readback",
        avg(&warm_collect),
        100.0 * ac / aw
    );
    println!(
        "  encode  total:  {:.2?}   ({:5.1}%)   <- libx264 push_frame + finish",
        avg(&warm_encode),
        100.0 * ae / aw
    );
    println!(
        "Per-frame avg: submit={:.2?}  collect={:.2?}  encode={:.2?}",
        avg(&warm_submit) / 240,
        avg(&warm_collect) / 240,
        avg(&warm_encode) / 240
    );

    let det_pass = unique.len() == 1;
    let perf_pass = avg_fps >= 60.0;

    if det_pass && perf_pass {
        println!("\n\u{2713}\u{2713} SPIKE S1 PASS: determinism + perf \u{2265} 60 fps");
        Ok(())
    } else if det_pass {
        anyhow::bail!(
            "\u{2713} determinism PASS, \u{2717} perf FAIL: {avg_fps:.2} fps < 60.00 fps"
        );
    } else if perf_pass {
        anyhow::bail!(
            "\u{2717} determinism FAIL: {} unique hashes, \u{2713} perf PASS",
            unique.len()
        );
    } else {
        anyhow::bail!(
            "\u{2717}\u{2717} both FAIL: det={} unique, perf={avg_fps:.2} fps",
            unique.len()
        );
    }
}
