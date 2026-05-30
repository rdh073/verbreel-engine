//! Render job interface: the `start_render` / `status` / `cancel` surface the
//! state crate calls.
//!
//! ## Dep-rule boundary
//!
//! `verbreel-render` MUST NOT import `verbreel-state` and MUST NOT write the
//! event log. So this module exposes a *pure* render service: `verbreel-state`
//! builds a [`RenderJobSpec`] (from a `Project` it lowered to a neutral
//! [`verbreel_ir::Graph`]), calls [`start_render`], and gets back a
//! [`RenderJobId`] plus terminal [`RenderStatus`] it can persist through
//! `verbreel_state::engine::apply()`. Render never touches the project, the
//! events file, or the asset store — it composites frames and encodes bytes.
//!
//! ## Job lifecycle (v1 floor)
//!
//! The v1 floor runs the pipeline synchronously inside [`start_render`]:
//! acquire the wgpu compositor, decode the source(s), composite each frame,
//! encode to MP4 bytes, and register the terminal status in an in-process
//! [`JobRegistry`]. [`status`] reads the registry; [`cancel`] drops the entry.
//! A future async executor can replace the synchronous body without changing
//! this signature (it is already `id`-keyed and registry-backed).

use std::collections::HashMap;
use std::sync::Mutex;

use verbreel_ir::IrNodeId;

use crate::adapter::RenderPlan;
use crate::error::RenderError;
use crate::gpu::{CompositeLayer, Compositor};
use crate::preset::RenderPreset;

/// Identifier for a render job. A strict-`UUIDv7` reusing [`IrNodeId`]'s
/// newtype so the state crate can mint ids the same way it mints node ids
/// (no new id type to wire through `verbreel-types`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RenderJobId(IrNodeId);

impl RenderJobId {
    /// Mint a fresh job id.
    #[must_use]
    pub fn now() -> Self {
        Self(IrNodeId::now())
    }

    /// Wrap an existing node id as a job id (the state crate already holds
    /// minted `UUIDv7`s and can reuse one as the job key).
    #[must_use]
    pub fn from_node_id(id: IrNodeId) -> Self {
        Self(id)
    }

    /// The underlying id, for logging / persistence.
    #[must_use]
    pub fn as_node_id(&self) -> IrNodeId {
        self.0
    }
}

impl std::fmt::Display for RenderJobId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// One decoded source's frames, keyed by the content-addressed asset the plan
/// referenced. The state crate resolves an [`verbreel_types::AssetHash`] to a
/// concrete path and hands the decoded frames in; render never opens the asset
/// store itself (dep-rule).
#[derive(Debug, Clone)]
pub struct DecodedSource {
    /// Packed-yuv420p frames, in presentation order.
    pub frames: Vec<verbreel_codec_native::Frame>,
}

/// Everything render needs to produce an output, with no reference to
/// `verbreel-state`.
#[derive(Debug)]
pub struct RenderJobSpec {
    /// The render preset (deterministic vs performance).
    pub preset: RenderPreset,
    /// Output width in pixels (even).
    pub width: u32,
    /// Output height in pixels (even).
    pub height: u32,
    /// Output frame-rate numerator.
    pub fps_num: u32,
    /// Output frame-rate denominator.
    pub fps_den: u32,
    /// The per-frame render plans, in output-frame order. Each plan names the
    /// layers (and their source assets) to composite for that frame.
    pub frames: Vec<RenderPlan>,
    /// Decoded source frames keyed by source-node id, supplied by the state
    /// crate (which owns asset resolution). The layer at frame `f`, layer `l`
    /// reads `decoded[source_node].frames[f]` when the plan layer carries an
    /// asset; layers with no asset composite as opaque black.
    pub decoded: HashMap<IrNodeId, DecodedSource>,
}

/// Terminal (or in-flight) state of a render job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderStatus {
    /// Job is running. The v1 floor is synchronous so callers only observe
    /// this transiently across threads; reserved for the async executor.
    Running,
    /// Job finished: the encoded MP4 container bytes.
    Done {
        /// Number of frames encoded.
        frame_count: usize,
        /// Encoded MP4 container bytes.
        output: Vec<u8>,
    },
    /// Job failed. The render error is stringified so the registry entry is
    /// `Clone` even though [`RenderError`] is not.
    Failed {
        /// The failure detail.
        detail: String,
    },
}

/// In-process registry of job statuses.
///
/// Not a persistent store — render owns no disk state. `verbreel-state`
/// persists the terminal status through `apply()`; this registry only lets a
/// caller poll a job between `start_render` and reading its result in the same
/// process.
#[derive(Debug, Default)]
pub struct JobRegistry {
    jobs: Mutex<HashMap<RenderJobId, RenderStatus>>,
}

impl JobRegistry {
    /// Construct an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Run a render job to completion and register its terminal status.
    ///
    /// Synchronous at the v1 floor: builds the compositor, composites every
    /// frame plan, encodes the result, and stores [`RenderStatus::Done`] (or
    /// [`RenderStatus::Failed`]) under a freshly minted id. Returns the id so
    /// the caller can read the status back via [`Self::status`].
    ///
    /// The job runs the full pipeline before returning; a failure mid-pipeline
    /// is captured as [`RenderStatus::Failed`] *and* returned as the `Err`, so
    /// the caller sees the error directly without a second registry read.
    ///
    /// # Errors
    ///
    /// Propagates the first [`RenderError`] from compositor init, compositing,
    /// or encode. The same error is also recorded in the registry under the
    /// returned-by-`Ok`-path-only id; on `Err` no terminal `Done` is stored.
    ///
    /// # Panics
    ///
    /// Panics if the registry mutex is poisoned (see [`Self::status`]).
    pub fn start_render(&self, spec: &RenderJobSpec) -> Result<RenderJobId, RenderError> {
        let id = RenderJobId::now();
        match run_job(spec) {
            Ok(status) => {
                self.insert(id, status);
                Ok(id)
            }
            Err(e) => {
                self.insert(
                    id,
                    RenderStatus::Failed {
                        detail: e.to_string(),
                    },
                );
                Err(e)
            }
        }
    }

    /// Read a job's status.
    ///
    /// # Errors
    ///
    /// [`RenderError::UnknownJob`] if the id is not registered (never started,
    /// or already cancelled).
    ///
    /// # Panics
    ///
    /// Panics if the registry mutex is poisoned — i.e. a prior caller panicked
    /// while holding the lock. Surfacing it loudly is correct: a poisoned lock
    /// means the registry's invariants may be broken.
    pub fn status(&self, id: RenderJobId) -> Result<RenderStatus, RenderError> {
        self.jobs
            .lock()
            .expect("job registry mutex poisoned")
            .get(&id)
            .cloned()
            .ok_or_else(|| RenderError::UnknownJob {
                detail: id.to_string(),
            })
    }

    /// Cancel a job: drop its registry entry.
    ///
    /// At the v1 floor the pipeline is synchronous, so by the time a caller
    /// holds an id the work is already done; cancel therefore just forgets the
    /// result. The async executor will grow real interruption here.
    ///
    /// # Errors
    ///
    /// [`RenderError::UnknownJob`] if the id is not registered.
    ///
    /// # Panics
    ///
    /// Panics if the registry mutex is poisoned (see [`Self::status`]).
    pub fn cancel(&self, id: RenderJobId) -> Result<(), RenderError> {
        self.jobs
            .lock()
            .expect("job registry mutex poisoned")
            .remove(&id)
            .map(|_| ())
            .ok_or_else(|| RenderError::UnknownJob {
                detail: id.to_string(),
            })
    }

    fn insert(&self, id: RenderJobId, status: RenderStatus) {
        self.jobs
            .lock()
            .expect("job registry mutex poisoned")
            .insert(id, status);
    }
}

/// Run the full decode-composited-encode pipeline for a spec.
///
/// Pulled out of [`JobRegistry::start_render`] so the body is testable without
/// the registry and so the registry method stays a thin status-recording shell.
fn run_job(spec: &RenderJobSpec) -> Result<RenderStatus, RenderError> {
    if spec.frames.is_empty() {
        return Err(RenderError::InvalidInput {
            detail: "render spec has no frames".to_string(),
        });
    }

    let compositor = Compositor::new(spec.preset)?;
    let frame_len = yuv420p_len(spec.width, spec.height);

    let mut composited: Vec<verbreel_codec_native::Frame> = Vec::with_capacity(spec.frames.len());
    for (frame_index, plan) in spec.frames.iter().enumerate() {
        let layers = build_layers(spec, plan, frame_index, frame_len)?;
        let bytes = compositor.composite(spec.width, spec.height, &layers)?;
        composited.push(verbreel_codec_native::Frame::new(
            spec.width,
            spec.height,
            bytes,
        ));
    }

    let frame_count = composited.len();
    let output = crate::codec::encode_frames(
        spec.preset,
        spec.width,
        spec.height,
        spec.fps_num,
        spec.fps_den,
        &composited,
    )?;

    Ok(RenderStatus::Done {
        frame_count,
        output,
    })
}

/// Assemble the GPU compositor layers for one output frame from its plan.
///
/// A plan layer with an asset reads the matching decoded source's frame at
/// `frame_index` (clamped to the last available frame so a short source holds
/// on its final picture). A plan layer with no asset, or whose decoded source
/// is missing, composites as opaque black so the pipeline never panics on a
/// gap.
fn build_layers(
    spec: &RenderJobSpec,
    plan: &RenderPlan,
    frame_index: usize,
    frame_len: usize,
) -> Result<Vec<CompositeLayer>, RenderError> {
    if plan.layers.is_empty() {
        return Err(RenderError::InvalidInput {
            detail: format!("frame {frame_index} plan has no layers"),
        });
    }
    let mut layers = Vec::with_capacity(plan.layers.len());
    for layer in &plan.layers {
        let planes = match &layer.source_asset {
            Some(_asset) => decoded_frame_planes(spec, frame_index, frame_len),
            None => black_yuv420p(frame_len),
        };
        layers.push(CompositeLayer {
            planes,
            alpha_q16: layer.alpha_q16,
        });
    }
    Ok(layers)
}

/// Pick the decoded planes for a frame. The v1 floor keys decoded sources by
/// insertion order: it walks `spec.decoded` and uses the first source's frame
/// at `frame_index` (clamped). A richer per-layer source mapping lands when the
/// state crate wires asset->source-node resolution; until then a single source
/// covers the smoke path.
fn decoded_frame_planes(spec: &RenderJobSpec, frame_index: usize, frame_len: usize) -> Vec<u8> {
    for source in spec.decoded.values() {
        if source.frames.is_empty() {
            continue;
        }
        let idx = frame_index.min(source.frames.len() - 1);
        let frame = &source.frames[idx];
        if frame.planes().len() == frame_len {
            return frame.planes().to_vec();
        }
    }
    black_yuv420p(frame_len)
}

/// Packed-yuv420p length in bytes for `width`x`height`.
fn yuv420p_len(width: u32, height: u32) -> usize {
    let w = width as usize;
    let h = height as usize;
    w * h + 2 * (w / 2) * (h / 2)
}

/// An opaque black yuv420p frame buffer: Y=16, U=V=128 (BT.601 black).
fn black_yuv420p(frame_len: usize) -> Vec<u8> {
    // The Y plane occupies the first 2/3 of a yuv420p buffer; chroma the rest.
    // Computing the split from frame_len keeps this independent of dimensions.
    let chroma = frame_len / 6; // (w/2)*(h/2) == frame_len/6 for yuv420p
    let luma = frame_len - 2 * chroma;
    let mut buf = vec![16u8; luma];
    buf.extend(std::iter::repeat_n(128u8, 2 * chroma));
    buf
}
