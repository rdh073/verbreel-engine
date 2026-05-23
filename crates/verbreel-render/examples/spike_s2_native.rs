//! Spike S2 native runner — multiply-blend on Vulkan via wgpu.
//!
//! Usage:
//!   cargo run --release -p verbreel-render --features spike-s2 --example spike_s2_native
//!
//! Generates the deterministic input PNGs on first run and writes
//! `tmp/spike_s2/native_frame.png`. The same input PNGs (input_a.png +
//! input_b.png) get copied into `crates/verbreel-render/web/` for the
//! wasm runner.

use std::path::PathBuf;

use verbreel_render::spike_s2;

fn main() -> anyhow::Result<()> {
    let out_dir = PathBuf::from("tmp/spike_s2");
    std::fs::create_dir_all(&out_dir)?;

    let input_a = out_dir.join("input_a.png");
    let input_b = out_dir.join("input_b.png");
    let output = out_dir.join("native_frame.png");

    if !input_a.exists() {
        spike_s2::synth::write_input_a(&input_a)?;
        println!("wrote {}", input_a.display());
    }
    if !input_b.exists() {
        spike_s2::synth::write_input_b(&input_b)?;
        println!("wrote {}", input_b.display());
    }

    let adapter = spike_s2::native::run_native(&input_a, &input_b, &output)?;
    println!(
        "Native frame written: {} (adapter: {:?} backend={:?} type={:?})",
        output.display(),
        adapter.name,
        adapter.backend,
        adapter.device_type
    );
    Ok(())
}
