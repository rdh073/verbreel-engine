//! Error type for the composition pipeline.

use thiserror::Error;

/// Errors surfaced from the render pipeline.
///
/// v0 only produces [`Self::NotYetImplemented`]; the other two
/// variants are committed now so downstream callers can pattern-match
/// against the eventual Spike S1 surface without a breaking add at
/// ship.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum RenderError {
    /// Method body deferred to Spike S1 (Research 01 §11).
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
}
