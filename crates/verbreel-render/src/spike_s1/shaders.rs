//! Spike S1 — embedded WGSL shader sources.

pub const YUV_TO_RGB: &str = include_str!("../../shaders/spike_s1/yuv_to_rgb.wgsl");
pub const RGB_TO_YUV: &str = include_str!("../../shaders/spike_s1/rgb_to_yuv.wgsl");
pub const CROSSFADE: &str = include_str!("../../shaders/spike_s1/crossfade.wgsl");
