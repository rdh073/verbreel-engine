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

use verbreel_ir::{AssetHash, IrNodeId};

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

/// One decoded source's frames, keyed in [`RenderJobSpec::decoded`] by the
/// content-addressed [`AssetHash`] the plan layer references. The state crate
/// resolves an [`AssetHash`] to a concrete path and hands the decoded frames
/// in; render never opens the asset store itself (dep-rule).
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
    /// Decoded source frames keyed by the layer's content-addressed
    /// [`AssetHash`], supplied by the state crate (which owns asset
    /// resolution). A plan layer that carries `source_asset = Some(h)` reads
    /// `decoded[h].frames[f]` for output frame `f`; a layer with no asset (or
    /// an asset absent from this map) composites as opaque black.
    ///
    /// Keyed by `AssetHash` rather than source-node id so each layer resolves
    /// its *own* source: the layer holds the asset it reads, the lookup is a
    /// direct content-addressed hit, and the result does not depend on map
    /// iteration order — two renders of the same spec are byte-identical.
    pub decoded: HashMap<AssetHash, DecodedSource>,
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
    /// The job runs the full pipeline before returning. On success the terminal
    /// [`RenderStatus::Done`] is registered under the returned id; on failure
    /// the error is returned directly and *nothing* is inserted — the id is
    /// never handed to the caller, so a `Failed` entry under it would be an
    /// unreachable orphan that leaks in the process-wide default registry.
    /// The [`RenderStatus::Failed`] variant exists for the async executor,
    /// which will register a job *before* running it under a caller-known id.
    ///
    /// # Errors
    ///
    /// Propagates the first [`RenderError`] from compositor init, compositing,
    /// or encode. On `Err` no registry entry is created.
    ///
    /// # Panics
    ///
    /// Panics if the registry mutex is poisoned (see [`Self::status`]).
    pub fn start_render(&self, spec: &RenderJobSpec) -> Result<RenderJobId, RenderError> {
        let status = run_job(spec)?;
        let id = RenderJobId::now();
        self.insert(id, status);
        Ok(id)
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
            Some(asset) => decoded_frame_planes(spec, asset, frame_index, frame_len)?,
            None => black_yuv420p(frame_len),
        };
        layers.push(CompositeLayer {
            planes,
            alpha_q16: layer.alpha_q16,
        });
    }
    Ok(layers)
}

/// Pick the decoded planes a layer reads for one output frame.
///
/// Resolves the layer's *own* source by its content-addressed [`AssetHash`]:
/// a direct `decoded[asset]` lookup, so each layer in a multi-track
/// composition reads its correct video and the result never depends on map
/// iteration order. The frame index is clamped to the source's last frame so
/// a short source holds on its final picture. A missing asset or an empty
/// source falls back to opaque black so the pipeline never panics on a gap.
///
/// A decoded frame whose plane length disagrees with the output geometry is a
/// state↔render contract violation, not a gap: the state crate handed us a
/// source whose geometry contradicts the job spec. Returning black there would
/// hash deterministically while masking corrupt visible output, so we surface
/// it as [`RenderError::InvalidInput`] instead.
fn decoded_frame_planes(
    spec: &RenderJobSpec,
    asset: &AssetHash,
    frame_index: usize,
    frame_len: usize,
) -> Result<Vec<u8>, RenderError> {
    let Some(source) = spec.decoded.get(asset) else {
        return Ok(black_yuv420p(frame_len));
    };
    if source.frames.is_empty() {
        return Ok(black_yuv420p(frame_len));
    }
    let idx = frame_index.min(source.frames.len() - 1);
    let frame = &source.frames[idx];
    let actual = frame.planes().len();
    if actual == frame_len {
        Ok(frame.planes().to_vec())
    } else {
        Err(RenderError::InvalidInput {
            detail: format!(
                "decoded source {asset} frame {idx} plane length {actual} != output geometry {frame_len}"
            ),
        })
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::RenderLayer;
    use verbreel_codec_native::Frame;

    /// A valid 64-char lowercase-hex `AssetHash` from a single repeated nibble.
    fn asset(nibble: char) -> AssetHash {
        AssetHash::new(std::iter::repeat_n(nibble, 64).collect::<String>()).unwrap()
    }

    /// A flat yuv420p buffer of 4x4 filled with one Y value (chroma at 128).
    fn frame_4x4(y: u8) -> Frame {
        let len = yuv420p_len(4, 4);
        let chroma = len / 6;
        let luma = len - 2 * chroma;
        let mut buf = vec![y; luma];
        buf.extend(std::iter::repeat_n(128u8, 2 * chroma));
        Frame::new(4, 4, buf)
    }

    fn spec_with_two_sources() -> RenderJobSpec {
        let mut decoded = HashMap::new();
        decoded.insert(
            asset('a'),
            DecodedSource {
                frames: vec![frame_4x4(200)],
            },
        );
        decoded.insert(
            asset('b'),
            DecodedSource {
                frames: vec![frame_4x4(40)],
            },
        );
        RenderJobSpec {
            preset: RenderPreset::Deterministic,
            width: 4,
            height: 4,
            fps_num: 30,
            fps_den: 1,
            frames: vec![RenderPlan {
                layers: vec![
                    RenderLayer {
                        source_asset: Some(asset('a')),
                        alpha_q16: u16::MAX,
                        cache_hash: [0u8; 32],
                    },
                    RenderLayer {
                        source_asset: Some(asset('b')),
                        alpha_q16: u16::MAX,
                        cache_hash: [0u8; 32],
                    },
                ],
                tick: 0,
            }],
            decoded,
        }
    }

    /// The P1 regression test: each layer resolves its OWN source by asset
    /// hash. Layer A (asset 'a') must read source A's pixels (Y=200) and layer
    /// B (asset 'b') must read source B's (Y=40) — not "the first via
    /// `.values()`", which fed every layer the same source. GPU-free: asserts
    /// on `decoded_frame_planes` directly so it runs feature-off in CI.
    #[test]
    fn each_layer_resolves_its_own_source_by_asset_hash() {
        let spec = spec_with_two_sources();
        let frame_len = yuv420p_len(spec.width, spec.height);

        let planes_a = decoded_frame_planes(&spec, &asset('a'), 0, frame_len).unwrap();
        let planes_b = decoded_frame_planes(&spec, &asset('b'), 0, frame_len).unwrap();

        assert_eq!(planes_a[0], 200, "layer A must read source A's luma");
        assert_eq!(planes_b[0], 40, "layer B must read source B's luma");
        assert_ne!(
            planes_a, planes_b,
            "two distinct sources must resolve to distinct planes"
        );
    }

    /// Resolution is deterministic: repeated calls over a multi-source map
    /// return identical bytes (no dependence on `HashMap` iteration order).
    #[test]
    fn source_resolution_is_order_independent() {
        let spec = spec_with_two_sources();
        let frame_len = yuv420p_len(spec.width, spec.height);
        let first = decoded_frame_planes(&spec, &asset('a'), 0, frame_len).unwrap();
        for _ in 0..16 {
            assert_eq!(
                first,
                decoded_frame_planes(&spec, &asset('a'), 0, frame_len).unwrap(),
                "per-layer source resolution must be byte-stable across calls"
            );
        }
    }

    /// `build_layers` maps the two-layer plan to two composite layers, each
    /// carrying its own source's planes.
    #[test]
    fn build_layers_maps_each_layer_to_its_source() {
        let spec = spec_with_two_sources();
        let frame_len = yuv420p_len(spec.width, spec.height);
        let layers = build_layers(&spec, &spec.frames[0], 0, frame_len).unwrap();
        assert_eq!(layers.len(), 2);
        assert_eq!(layers[0].planes[0], 200);
        assert_eq!(layers[1].planes[0], 40);
    }

    /// A missing asset (or `None`) falls back to opaque black, never panics.
    #[test]
    fn missing_asset_falls_back_to_black() {
        let spec = spec_with_two_sources();
        let frame_len = yuv420p_len(spec.width, spec.height);
        let planes = decoded_frame_planes(&spec, &asset('c'), 0, frame_len).unwrap();
        assert_eq!(planes, black_yuv420p(frame_len));
    }

    /// A decoded frame whose plane length disagrees with the output geometry is
    /// a state↔render contract violation. It must surface as
    /// [`RenderError::InvalidInput`] — NOT be masked as black, which would hash
    /// deterministically while hiding visible-output corruption. GPU-free.
    #[test]
    fn decoded_plane_size_mismatch_is_invalid_input_not_black() {
        let mut spec = spec_with_two_sources();
        // Source 'a' decoded at 4x4 (its planes are sized for 4x4)...
        let frame_len_a = yuv420p_len(spec.width, spec.height);
        assert_eq!(
            spec.decoded[&asset('a')].frames[0].planes().len(),
            frame_len_a
        );
        // ...but the job spec now demands 8x8 output, so the decoded planes no
        // longer match the requested geometry.
        spec.width = 8;
        spec.height = 8;
        let frame_len = yuv420p_len(spec.width, spec.height);
        assert_ne!(frame_len, frame_len_a);

        let err = decoded_frame_planes(&spec, &asset('a'), 0, frame_len).unwrap_err();
        assert!(
            matches!(err, RenderError::InvalidInput { .. }),
            "size mismatch must be InvalidInput, got {err:?}"
        );
    }

    /// `status` on an unregistered id returns [`RenderError::UnknownJob`].
    #[test]
    fn status_unknown_job_errors() {
        let reg = JobRegistry::new();
        let err = reg.status(RenderJobId::now()).unwrap_err();
        assert!(matches!(err, RenderError::UnknownJob { .. }));
    }

    /// `cancel` on an unregistered id returns [`RenderError::UnknownJob`].
    #[test]
    fn cancel_unknown_job_errors() {
        let reg = JobRegistry::new();
        let err = reg.cancel(RenderJobId::now()).unwrap_err();
        assert!(matches!(err, RenderError::UnknownJob { .. }));
    }

    /// Feature-off, the codec facade fails closed with [`RenderError::Codec`]
    /// rather than linking `FFmpeg`. Feature-on the stubs are replaced by the
    /// real rsmpeg path, so this only asserts the fail-closed contract.
    #[cfg(not(feature = "rsmpeg"))]
    #[test]
    fn codec_stub_fails_closed_feature_off() {
        let decode_err =
            crate::codec::decode_source(std::path::Path::new("/nonexistent.mp4")).unwrap_err();
        assert!(matches!(decode_err, RenderError::Codec { .. }));

        let encode_err =
            crate::codec::encode_frames(RenderPreset::Deterministic, 4, 4, 30, 1, &[]).unwrap_err();
        assert!(matches!(encode_err, RenderError::Codec { .. }));
    }
}
