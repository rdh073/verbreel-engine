//! Spike S1 slice B — GPU YUV↔RGB roundtrip determinism harness.
//!
//! Run:
//!   cargo run --release -p verbreel-render --features spike-s1 \
//!     --example spike_s1_gpu_roundtrip
//!
//! Output: `tmp/spike_s1_b/{run_N.yuv, hashes.txt}`.
//! Exit 0 = "DETERMINISM PASS" (all 10 SHA-256 identical).

use std::collections::HashSet;
use std::path::PathBuf;

use sha2::{Digest, Sha256};
use verbreel_render::spike_s1::{GpuRoundtrip, generate_raw_yuv420p};

const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;
const FRAMES: u32 = 240; // 10 s @ 24 fps (matches slice A's harness)
const RUNS: u32 = 10;
const FRAME_BYTES: usize = (WIDTH * HEIGHT * 3 / 2) as usize;

fn sha256_bytes(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))
}

fn main() -> anyhow::Result<()> {
    std::fs::create_dir_all("tmp/spike_s1_b")?;

    let synth = generate_raw_yuv420p(WIDTH, HEIGHT, FRAMES);
    assert_eq!(synth.len(), FRAME_BYTES * FRAMES as usize);

    // Init GPU once, reuse across runs (matches what slice C would do).
    let gpu = GpuRoundtrip::new(WIDTH, HEIGHT)?;
    println!("adapter: {}", gpu.adapter_info());

    let mut hashes = Vec::with_capacity(RUNS as usize);

    for run in 0..RUNS {
        let mut out = Vec::with_capacity(FRAME_BYTES * FRAMES as usize);
        for (i, frame_in) in synth.chunks_exact(FRAME_BYTES).enumerate() {
            let frame_out = gpu.process_frame(frame_in)?;
            assert_eq!(
                frame_out.len(),
                FRAME_BYTES,
                "frame {i} of run {run} produced wrong byte count"
            );
            out.extend_from_slice(&frame_out);
        }

        let path = PathBuf::from(format!("tmp/spike_s1_b/run_{run}.yuv"));
        std::fs::write(&path, &out)?;
        let h = sha256_bytes(&out);
        println!("run {run}: sha256 = {h}");
        hashes.push(h);
    }

    std::fs::write("tmp/spike_s1_b/hashes.txt", hashes.join("\n") + "\n")?;

    let unique: HashSet<_> = hashes.iter().collect();
    if unique.len() == 1 {
        println!("\n\u{2713} DETERMINISM PASS: all {RUNS} runs produced identical SHA-256");
        Ok(())
    } else {
        anyhow::bail!(
            "\u{2717} DETERMINISM FAIL: {} unique hashes across {} runs",
            unique.len(),
            RUNS
        );
    }
}
