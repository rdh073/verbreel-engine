//! Spike S1 slice A — decode↔encode determinism harness.
//!
//! Run:
//!   FFMPEG_PKG_CONFIG_PATH=$HOME/playground/verbreel/vendor/rsmpeg/tmp/ffmpeg_build/lib/pkgconfig \
//!     cargo run --release -p verbreel-codec-native \
//!       --features spike-s1 --example spike_s1_roundtrip
//!
//! Output: `tmp/spike_s1/{run_N_pass{1,2}.mp4, hashes.txt}`.
//! Exit 0 = "DETERMINISM PASS" (all 10 pass2 SHA-256 identical).
//! Non-zero = mismatch — see stdout for the divergent hashes.

use std::collections::HashSet;
use std::path::PathBuf;

use sha2::{Digest, Sha256};

use verbreel_codec_native::spike_s1::{Decoder, Encoder, generate_raw_yuv420p};

const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;
const FPS: u32 = 24;
const FRAMES: u32 = 240; // 10 seconds
const RUNS: u32 = 10;

fn sha256_file(p: &std::path::Path) -> anyhow::Result<String> {
    let bytes = std::fs::read(p)?;
    Ok(format!("{:x}", Sha256::digest(&bytes)))
}

fn main() -> anyhow::Result<()> {
    std::fs::create_dir_all("tmp/spike_s1")?;
    let mut hashes = Vec::with_capacity(RUNS as usize);

    // Generate synthetic source ONCE — we're testing codec determinism,
    // not input determinism (the latter is trivially deterministic; see
    // synth::tests).
    let synth_yuv = generate_raw_yuv420p(WIDTH, HEIGHT, FRAMES);
    let frame_bytes = (WIDTH * HEIGHT * 3 / 2) as usize;

    for run in 0..RUNS {
        // Pass 1: encode synthetic → run_N_pass1.mp4
        let pass1 = PathBuf::from(format!("tmp/spike_s1/run_{run}_pass1.mp4"));
        let mut enc = Encoder::new(&pass1, WIDTH, HEIGHT, FPS)?;
        for (i, chunk) in synth_yuv.chunks_exact(frame_bytes).enumerate() {
            enc.push_frame(chunk, i as i64)?;
        }
        enc.finish()?;

        // Pass 2: decode run_N_pass1.mp4 → re-encode → run_N_pass2.mp4
        // Proves the decoder+encoder roundtrip is deterministic, not
        // just the encoder against synthetic raw input.
        let pass2 = PathBuf::from(format!("tmp/spike_s1/run_{run}_pass2.mp4"));
        let mut dec = Decoder::new(&pass1)?;
        let mut enc2 = Encoder::new(&pass2, WIDTH, HEIGHT, FPS)?;
        let mut pts = 0i64;
        while let Some(yuv) = dec.next_frame()? {
            enc2.push_frame(&yuv, pts)?;
            pts += 1;
        }
        enc2.finish()?;

        let h = sha256_file(&pass2)?;
        println!("run {run}: pass2 sha256 = {h}");
        hashes.push(h);
    }

    std::fs::write("tmp/spike_s1/hashes.txt", hashes.join("\n") + "\n")?;

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
