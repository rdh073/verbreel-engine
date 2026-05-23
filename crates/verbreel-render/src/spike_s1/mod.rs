//! Spike S1 slice B — wgpu YUV↔RGB roundtrip determinism harness.
//!
//! Not production code; not in default build. Behind `--features spike-s1`.
//! See `SPIKE_S1B_RESULTS.md` for results and slice C handoff notes.

pub mod gpu;
pub mod shaders;
pub mod synth;

pub use gpu::GpuRoundtrip;
pub use synth::generate_raw_yuv420p;
