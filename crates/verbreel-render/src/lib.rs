//! verbreel-render — wgpu compositor + rsmpeg encode pipeline (v1 floor).
//!
//! The render half of Spike S1: a neutral [`verbreel_ir`] composition graph is
//! lowered to per-frame [`RenderPlan`](adapter::RenderPlan)s, each frame is
//! composited on a headless wgpu compute pipeline (`YUV420p` -> `RGB` ->
//! z-ordered source-over -> `YUV420p` via the shared WGSL kernel), and the
//! composited frames are encoded to MP4 bytes through [`verbreel_codec_native`]
//! behind the `rsmpeg` feature.
//!
//! ## Crate dependency rule
//!
//! ```text
//! verbreel-render → verbreel-ir + verbreel-codec-native + wgpu
//! ```
//!
//! `verbreel-render` MUST NOT import `verbreel-state`. The IR-to-render adapter
//! consumes the neutral graph types ([`verbreel_ir::CompositionInput`] /
//! [`verbreel_ir::Graph`] / [`verbreel_ir::Evaluator`]), never state's
//! `Project`. Render exposes [`start_render`] / [`status`] / [`cancel`] for the
//! state crate to call; it performs no state mutation and writes no event log
//! (§0.8 write ordering stays owned by `verbreel_state::engine::apply()`).
//!
//! ## Determinism contract
//!
//! [`RenderPreset::Deterministic`] requests the software fallback wgpu adapter
//! (`force_fallback_adapter: true` — lavapipe), pins the WGSL color-transform
//! matrices and frame quantization, and selects the codec crate's
//! deterministic libx264 params. Two renders of the same inputs on the same
//! host produce a byte-identical MP4 (§0.1 reproducibility).
//!
//! ## Feature gating
//!
//! `wgpu` is a normal dependency: it compiles in CI without a GPU, and only its
//! runtime device init needs an adapter (the GPU paths return
//! [`RenderError::NoAdapter`] and tests skip when no adapter is present). The
//! rsmpeg decode/encode path is behind the `rsmpeg` feature (forwarding to
//! `verbreel-codec-native/rsmpeg`); feature-off the encode/decode entry points
//! fail closed with [`RenderError::Codec`] instead of linking `FFmpeg`.

#![deny(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]

pub mod adapter;
pub mod codec;
pub mod error;
pub mod gpu;
pub mod hwaccel;
pub mod job;
pub mod pipeline;
pub mod preset;

pub use adapter::{FrameWindow, RenderLayer, RenderPlan, frame_window, tick_to_frame_index};
pub use codec::{decode_source, encode_frames};
pub use error::RenderError;
pub use gpu::{CompositeLayer, Compositor};
pub use hwaccel::{HwAccelKind, V1_HWACCEL_PRIORITY, hwaccel_priority_for_preset};
pub use job::{DecodedSource, JobRegistry, RenderJobId, RenderJobSpec, RenderStatus};
pub use pipeline::{Pipeline, run};
pub use preset::RenderPreset;

use std::sync::OnceLock;

/// Process-wide default job registry backing the free [`start_render`] /
/// [`status`] / [`cancel`] functions.
fn default_registry() -> &'static JobRegistry {
    static REGISTRY: OnceLock<JobRegistry> = OnceLock::new();
    REGISTRY.get_or_init(JobRegistry::new)
}

/// Run a render job to completion and return its id.
///
/// The state-crate entry point named in the decision packet. Delegates to a
/// process-wide [`JobRegistry`]; callers that want an isolated registry (tests,
/// concurrent engines) construct their own [`JobRegistry`] and call
/// [`JobRegistry::start_render`] directly.
///
/// # Errors
///
/// Propagates the first [`RenderError`] from compositor init
/// ([`RenderError::NoAdapter`] when no GPU/software adapter is present),
/// compositing, or encode.
///
/// # Panics
///
/// Panics if the process-wide registry mutex is poisoned (see
/// [`JobRegistry::status`]).
pub fn start_render(spec: &RenderJobSpec) -> Result<RenderJobId, RenderError> {
    default_registry().start_render(spec)
}

/// Read a job's [`RenderStatus`] from the process-wide registry.
///
/// # Errors
///
/// [`RenderError::UnknownJob`] if the id is not registered.
///
/// # Panics
///
/// Panics if the process-wide registry mutex is poisoned.
pub fn status(id: RenderJobId) -> Result<RenderStatus, RenderError> {
    default_registry().status(id)
}

/// Cancel a job (drop its result) from the process-wide registry.
///
/// # Errors
///
/// [`RenderError::UnknownJob`] if the id is not registered.
///
/// # Panics
///
/// Panics if the process-wide registry mutex is poisoned.
pub fn cancel(id: RenderJobId) -> Result<(), RenderError> {
    default_registry().cancel(id)
}
