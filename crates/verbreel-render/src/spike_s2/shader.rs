//! Shared WGSL source — same string fed to native (Vulkan via Naga) and
//! Chrome WebGPU. The whole point of S2 is that this `&str` is identical
//! on both targets.

pub const MULTIPLY_BLEND: &str = include_str!("../../shaders/spike_s2/multiply_blend.wgsl");
