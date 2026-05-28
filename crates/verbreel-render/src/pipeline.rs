//! Composition pipeline opaque handle + per-tick render entry point.
//!
//! v0 commits the public surface; real wgpu `RenderPipeline`,
//! `BindGroup`s, and WGSL shader sources land with Spike S1.

use verbreel_ir::Tick;

use crate::error::RenderError;
use crate::preset::RenderPreset;

/// Canonical Spike-S1 deferral string. Pinned so refactors cannot
/// silently drift the citation a future reader needs to find the
/// design context.
pub(crate) const SPIKE_S1_DEFERRAL_DETAIL: &str =
    "wgpu render pipeline lands with Spike S1 (Research 01 §11)";

/// Composition pipeline handle.
///
/// Opaque by design — Spike S1 will grow non-clone wgpu state inside
/// (`wgpu::Device`, `wgpu::Queue`, render pipelines, shader cache)
/// and these traits do not survive the upgrade:
///
/// - No `Clone` (wgpu handles are not `Clone` in general).
/// - No `Default` (initialisation requires a `RenderPreset`).
/// - No `Copy`.
///
/// Mirrors the same trait-discipline as `verbreel_codec_native::Frame`
/// / `verbreel_wasm::EngineHandle`.
#[derive(Debug)]
pub struct Pipeline {
    preset: RenderPreset,
}

impl Pipeline {
    /// Construct a pipeline bound to a [`RenderPreset`]. v0 stores the
    /// preset only; S1 wires up the wgpu resources here.
    #[must_use]
    pub fn new(preset: RenderPreset) -> Self {
        Self { preset }
    }

    /// The [`RenderPreset`] this pipeline was constructed against.
    #[must_use]
    pub fn preset(&self) -> RenderPreset {
        self.preset
    }
}

/// Render a single tick through the pipeline. v0 returns
/// [`RenderError::NotYetImplemented`] unconditionally with
/// [`SPIKE_S1_DEFERRAL_DETAIL`] as the body. S1 replaces the body
/// with real wgpu submission.
///
/// `at_tk` is the engine-tick timestamp ([`verbreel_types::Tick`],
/// 240,000 Hz base per §0.2). The typed newtype prevents callers
/// from accidentally passing a frame count or millisecond timestamp.
///
/// # Errors
///
/// Returns [`RenderError::NotYetImplemented`] unconditionally at v0.
/// Future variants ([`RenderError::SurfaceLost`],
/// [`RenderError::ShaderCompile`]) arrive with Spike S1.
pub fn run(_pipeline: &mut Pipeline, _at_tk: Tick) -> Result<(), RenderError> {
    Err(RenderError::NotYetImplemented {
        detail: SPIKE_S1_DEFERRAL_DETAIL.to_string(),
    })
}
