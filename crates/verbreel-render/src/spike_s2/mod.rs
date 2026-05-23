//! Spike S2 — cross-target WGSL pixel-diff harness.
//!
//! Spec §11 Spike S2: same WGSL multiply-blend shader, native Vulkan vs
//! Chrome WebGPU, per-pixel ΔRGB ≤ 1/255 for ≥99.9% of pixels, max ≤2/255.
//!
//! Not production code; behind `--features spike-s2`. The native side
//! builds via `examples/spike_s2_native.rs`; the wasm32 side via
//! `wasm-pack build --target web` + the `web/` host page.

pub mod shader;
pub mod synth;

#[cfg(not(target_arch = "wasm32"))]
pub mod native;

#[cfg(target_arch = "wasm32")]
pub mod wasm;
