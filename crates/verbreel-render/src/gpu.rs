//! wgpu compute compositor: `YUV420p` layers -> `RGB` -> z-ordered source-over
//! composite -> `YUV420p`, via the shared WGSL kernel (`shaders/composite.wgsl`).
//!
//! This is the headless half of Spike S1. There is no surface and no
//! rasterizer: the compositor is a single compute dispatch over storage
//! buffers, so the only nondeterminism source is the GPU's float evaluation,
//! which is reproducible run-to-run on a fixed adapter. The deterministic
//! preset requests the software fallback adapter (`force_fallback_adapter:
//! true`, lavapipe on Linux CI hosts that have it) so the output is also
//! vendor-independent.
//!
//! ## Adapter policy
//!
//! [`RenderPreset::Deterministic`] -> `force_fallback_adapter: true` (software
//! rasterizer, byte-stable). [`RenderPreset::Performance`] -> the host's
//! preferred adapter. This mirrors the hwaccel policy in [`crate::hwaccel`]:
//! deterministic mode is software-only.
//!
//! ## Graceful skip
//!
//! [`Compositor::new`] returns [`RenderError::NoAdapter`] when no adapter is
//! available. CI runners have no Vulkan adapter, so the determinism tests
//! treat `NoAdapter` as "skip this test" rather than a failure — keeping
//! feature-off CI green without a GPU.

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::error::RenderError;
use crate::preset::RenderPreset;

/// WGSL compositor kernel, embedded at build time so the shader path is pinned
/// (no runtime file lookup that could drift the deterministic output).
const COMPOSITE_WGSL: &str = include_str!("../shaders/composite.wgsl");

/// Compute workgroup size in X/Y, mirrored from `@workgroup_size(8, 8, 1)` in
/// the WGSL. Used to compute dispatch counts.
const WORKGROUP_DIM: u32 = 8;

/// Per-dispatch uniform mirroring the WGSL `Params` struct. `repr(C)` + `Pod`
/// so the host layout matches the shader's std140-compatible layout exactly.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct Params {
    width: u32,
    height: u32,
    layer_count: u32,
    layer_stride_bytes: u32,
}

/// Per-layer metadata mirroring the WGSL `LayerMeta` struct (16-byte aligned).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct LayerMeta {
    alpha_q16: u32,
    _pad: [u32; 3],
}

/// A single yuv420p layer fed to the compositor, plus its source-over alpha.
#[derive(Debug, Clone)]
pub struct CompositeLayer {
    /// Packed yuv420p bytes for this layer (same plane contract as
    /// [`verbreel_codec_native::Frame`]).
    pub planes: Vec<u8>,
    /// Source-over alpha, fixed-point 0..=65535 (1.0 == 65535).
    pub alpha_q16: u16,
}

/// Headless wgpu compositor bound to a [`RenderPreset`].
///
/// Holds the device/queue/pipeline; reusable across frames so the adapter and
/// shader are initialised once per render job, not once per frame.
#[derive(Debug)]
pub struct Compositor {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    preset: RenderPreset,
}

impl Compositor {
    /// Initialise the compositor: acquire an adapter per the preset's policy,
    /// request a device, and build the compute pipeline from the embedded WGSL.
    ///
    /// # Errors
    ///
    /// - [`RenderError::NoAdapter`] — no adapter satisfied the request (the CI
    ///   skip path).
    /// - [`RenderError::ShaderCompile`] — the WGSL failed to compile.
    /// - [`RenderError::GpuOperation`] — device request failed.
    pub fn new(preset: RenderPreset) -> Result<Self, RenderError> {
        pollster::block_on(Self::new_async(preset))
    }

    async fn new_async(preset: RenderPreset) -> Result<Self, RenderError> {
        let instance = wgpu::Instance::default();

        // Deterministic mode pins the software fallback adapter for vendor
        // independence; performance mode takes the host's preferred GPU.
        let force_fallback = matches!(preset, RenderPreset::Deterministic);
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::None,
                force_fallback_adapter: force_fallback,
                compatible_surface: None,
            })
            .await
            .map_err(|e| RenderError::NoAdapter {
                detail: format!("request_adapter failed: {e}"),
            })?;

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("verbreel-render-compositor"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_defaults(),
                ..Default::default()
            })
            .await
            .map_err(|e| RenderError::GpuOperation {
                detail: format!("request_device failed: {e}"),
            })?;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("composite.wgsl"),
            source: wgpu::ShaderSource::Wgsl(COMPOSITE_WGSL.into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("compositor-bgl"),
            entries: &[
                uniform_entry(0),
                storage_entry(1, true),
                storage_entry(2, true),
                storage_entry(3, false),
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("compositor-pll"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("compositor-pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        Ok(Self {
            device,
            queue,
            pipeline,
            bind_group_layout,
            preset,
        })
    }

    /// The preset this compositor was built against.
    #[must_use]
    pub fn preset(&self) -> RenderPreset {
        self.preset
    }

    /// Composite the given layers into a single packed-yuv420p frame buffer.
    ///
    /// Layers are composited bottom-to-top (index 0 first) with source-over
    /// alpha over an opaque black background. All layers and the output share
    /// `width`x`height`; both must be even (the yuv420p chroma constraint).
    ///
    /// # Errors
    ///
    /// - [`RenderError::InvalidInput`] — odd dimensions, no layers, or a layer
    ///   whose plane buffer length disagrees with `width`x`height`.
    /// - [`RenderError::GpuOperation`] — buffer map / device poll failure.
    pub fn composite(
        &self,
        width: u32,
        height: u32,
        layers: &[CompositeLayer],
    ) -> Result<Vec<u8>, RenderError> {
        let frame_len = validate_layers(width, height, layers)?;

        // Pack every layer back-to-back into one storage buffer; the shader
        // indexes a layer by `layer_index * layer_stride_bytes` then divides by
        // 4 to address the word holding that byte.
        let mut packed: Vec<u8> = Vec::with_capacity(frame_len * layers.len());
        for layer in layers {
            packed.extend_from_slice(&layer.planes);
        }
        // Storage buffers are word-addressed; pad the byte buffer up to a
        // 4-byte multiple so the final partial word is fully backed.
        pad_to_word(&mut packed);

        let layer_stride_bytes =
            u32::try_from(frame_len).map_err(|_| RenderError::InvalidInput {
                detail: "frame size exceeds u32 byte count".to_string(),
            })?;

        let params = Params {
            width,
            height,
            layer_count: u32::try_from(layers.len()).unwrap_or(u32::MAX),
            layer_stride_bytes,
        };
        let metas: Vec<LayerMeta> = layers
            .iter()
            .map(|l| LayerMeta {
                alpha_q16: u32::from(l.alpha_q16),
                _pad: [0; 3],
            })
            .collect();

        // The output buffer holds one u32 per output byte (see `store_byte` in
        // the WGSL: one word per byte avoids a read-modify-write race). Its size
        // in bytes is therefore `frame_len * 4`.
        let out_words_bytes = (frame_len * 4) as u64;

        let params_buf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("params"),
                contents: bytemuck::bytes_of(&params),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let layers_buf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("layers"),
                contents: &packed,
                usage: wgpu::BufferUsages::STORAGE,
            });
        let meta_buf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("layer-meta"),
                contents: bytemuck::cast_slice(&metas),
                usage: wgpu::BufferUsages::STORAGE,
            });
        let out_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("out"),
            size: out_words_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: out_words_bytes,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("compositor-bg"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: layers_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: meta_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: out_buf.as_entire_binding(),
                },
            ],
        });

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("compositor-encoder"),
            });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("compositor-pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            let groups_x = width.div_ceil(WORKGROUP_DIM);
            let groups_y = height.div_ceil(WORKGROUP_DIM);
            pass.dispatch_workgroups(groups_x, groups_y, 1);
        }
        encoder.copy_buffer_to_buffer(&out_buf, 0, &readback, 0, out_words_bytes);
        self.queue.submit(std::iter::once(encoder.finish()));

        self.read_back(&readback, frame_len)
    }

    /// Map the readback buffer and repack the one-u32-per-byte output words
    /// back into a packed `frame_len`-byte yuv420p buffer.
    fn read_back(&self, readback: &wgpu::Buffer, frame_len: usize) -> Result<Vec<u8>, RenderError> {
        let slice = readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |res| {
            // The receiver is alive for the duration of the poll below, so the
            // send cannot fail; ignore the result rather than unwrapping in a
            // GPU callback that must not panic.
            let _ = tx.send(res);
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| RenderError::GpuOperation {
                detail: format!("device poll failed: {e}"),
            })?;
        match rx.recv() {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                return Err(RenderError::GpuOperation {
                    detail: format!("buffer map failed: {e}"),
                });
            }
            Err(e) => {
                return Err(RenderError::GpuOperation {
                    detail: format!("map callback dropped: {e}"),
                });
            }
        }
        let data = slice.get_mapped_range();
        // Each output byte was stored as the low byte of its own u32 word.
        let words: &[u32] = bytemuck::cast_slice(&data);
        let out: Vec<u8> = words[..frame_len]
            .iter()
            .map(|w| u8::try_from(w & 0xff).expect("masked to 0..=255, always fits u8"))
            .collect();
        drop(data);
        readback.unmap();
        Ok(out)
    }
}

/// Validate dimensions and per-layer plane lengths; return the packed
/// yuv420p frame length in bytes.
fn validate_layers(
    width: u32,
    height: u32,
    layers: &[CompositeLayer],
) -> Result<usize, RenderError> {
    if layers.is_empty() {
        return Err(RenderError::InvalidInput {
            detail: "composite requires at least one layer".to_string(),
        });
    }
    if !width.is_multiple_of(2) || !height.is_multiple_of(2) {
        return Err(RenderError::InvalidInput {
            detail: format!("dimensions {width}x{height} must be even (yuv420p)"),
        });
    }
    if width == 0 || height == 0 {
        return Err(RenderError::InvalidInput {
            detail: "dimensions must be non-zero".to_string(),
        });
    }
    let w = width as usize;
    let h = height as usize;
    let frame_len = w * h + 2 * (w / 2) * (h / 2);
    for (i, layer) in layers.iter().enumerate() {
        if layer.planes.len() != frame_len {
            return Err(RenderError::InvalidInput {
                detail: format!(
                    "layer {i} has {} plane bytes, expected {frame_len} for {width}x{height} yuv420p",
                    layer.planes.len()
                ),
            });
        }
    }
    Ok(frame_len)
}

/// Pad a byte buffer up to the next 4-byte boundary (storage buffers are
/// word-addressed in WGSL; a partial trailing word must be fully backed).
fn pad_to_word(buf: &mut Vec<u8>) {
    while !buf.len().is_multiple_of(4) {
        buf.push(0);
    }
}

fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn storage_entry(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}
