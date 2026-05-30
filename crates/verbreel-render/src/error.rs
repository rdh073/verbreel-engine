//! Error type for the composition pipeline.

use thiserror::Error;

/// Errors surfaced from the render pipeline.
///
/// The first three variants were committed at v0 so downstream callers could
/// pattern-match against the eventual Spike S1 surface without a breaking add
/// at ship; the remaining variants land with the S1 wgpu + rsmpeg pipeline.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum RenderError {
    /// Method body deferred to Spike S1 (Research 01 §11). Retained for enum
    /// API stability; the live pipeline no longer emits it.
    #[error("verbreel-render method not yet implemented: {detail}")]
    NotYetImplemented {
        /// Pointer to where the real implementation lands.
        detail: String,
    },

    /// `wgpu::Surface` was lost (window minimised, GPU reset, …) and
    /// the pipeline cannot proceed until the caller re-creates it.
    /// Reserved at v0; produced by S1's surface-bound paths.
    #[error("render surface lost: {detail}")]
    SurfaceLost {
        /// Underlying `wgpu::SurfaceError` message.
        detail: String,
    },

    /// WGSL shader compilation failed at pipeline build time. The
    /// `naga` translator's diagnostics get bubbled up via `detail`
    /// so callers can surface a meaningful error.
    #[error("render shader compile failed: {detail}")]
    ShaderCompile {
        /// `naga` diagnostic chain.
        detail: String,
    },

    /// No wgpu adapter could be acquired. On CI runners with no GPU and no
    /// software Vulkan this is the expected outcome; the determinism tests
    /// skip when they see it rather than failing.
    #[error("no wgpu adapter available: {detail}")]
    NoAdapter {
        /// Why adapter acquisition failed (e.g. `request_adapter` returned
        /// `None`, or device request failed).
        detail: String,
    },

    /// The compositor was handed invalid frame inputs (zero layers, odd
    /// dimensions, a layer whose plane buffer length disagrees with the
    /// declared size). Caught before any GPU work is submitted.
    #[error("invalid render input: {detail}")]
    InvalidInput {
        /// Which input constraint was violated.
        detail: String,
    },

    /// A GPU operation failed at submit/map time (buffer map error, device
    /// poll error). Distinct from [`Self::NoAdapter`]: the device exists but
    /// an operation on it failed.
    #[error("gpu operation failed: {detail}")]
    GpuOperation {
        /// The failing operation and its underlying wgpu error.
        detail: String,
    },

    /// The rsmpeg-backed decode/encode facade was reached without the
    /// `rsmpeg` feature, or the underlying [`verbreel_codec_native`] call
    /// failed. The body carries the codec-side detail.
    #[error("codec facade error: {detail}")]
    Codec {
        /// The codec-side failure detail.
        detail: String,
    },

    /// `status`/`cancel` were called with a job id no in-process registry
    /// knows about.
    #[error("unknown render job: {detail}")]
    UnknownJob {
        /// The job id that was not found.
        detail: String,
    },
}
