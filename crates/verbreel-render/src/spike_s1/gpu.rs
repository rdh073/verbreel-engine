//! Spike S1 slice B — wgpu YUV↔RGB roundtrip pipeline.
//!
//! Picks a Vulkan adapter (HighPerformance), builds two compute pipelines
//! (`yuv_to_rgb` + `rgb_to_yuv`), and exposes [`GpuRoundtrip::process_frame`]
//! that runs upload → forward → inverse → readback for one frame and
//! returns the round-tripped YUV bytes. No persistent state between
//! frames beyond the pipelines + textures + staging buffers themselves
//! (which are reused).
//!
//! No `unsafe`. wgpu is safe-Rust through-and-through.

use std::sync::mpsc;

use anyhow::{Context, Result, anyhow};
use wgpu::util::DeviceExt;

use super::shaders::{RGB_TO_YUV, YUV_TO_RGB};

const COPY_ALIGN: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;

/// Spike-only knob: which adapter to pick for the GPU pipeline.
///
/// `HighPerformance` is slice D's behavior (picks dGPU on dual-GPU laptops).
/// `Integrated` enumerates adapters and selects the first `DeviceType::IntegratedGpu`.
/// `LowPower` is `wgpu`'s built-in hint (UNRELIABLE on Linux NVIDIA hybrid per
/// wgpu#3464 — provided only for completeness; do NOT use for sign-off).
///
/// PRODUCTION CODE MUST PASS `HighPerformance` to preserve slice D's invariants.
/// This enum exists only so the iGPU sign-off harness (spike S1.5) can collect
/// comparison data without forking the GPU module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdapterMode {
    HighPerformance,
    Integrated,
    LowPower,
}

/// All GPU state for the spike-B roundtrip pipeline.
pub struct GpuRoundtrip {
    device: wgpu::Device,
    queue: wgpu::Queue,
    adapter_info: wgpu::AdapterInfo,

    width: u32,
    height: u32,

    // Persistent textures (sample-input + storage-output for both passes)
    y_in_tex: wgpu::Texture,
    u_in_tex: wgpu::Texture,
    v_in_tex: wgpu::Texture,
    // Intermediate texture for forward (yuv->rgb) output / inverse input.
    // Held only to keep the GPU resource alive; the bind groups reference
    // its TextureView, not this field directly — hence dead_code.
    #[allow(dead_code)]
    rgb_tex: wgpu::Texture,
    y_out_tex: wgpu::Texture,
    u_out_tex: wgpu::Texture,
    v_out_tex: wgpu::Texture,

    // Readback staging buffers (one per output plane), with padded rows.
    y_read_buf: wgpu::Buffer,
    u_read_buf: wgpu::Buffer,
    v_read_buf: wgpu::Buffer,

    // Padded row strides for readback (in bytes).
    y_row_bytes: u32,
    uv_row_bytes: u32,

    // Pipelines + bind groups (rebuilt nothing per-frame).
    fwd_pipeline: wgpu::ComputePipeline,
    inv_pipeline: wgpu::ComputePipeline,
    fwd_bg: wgpu::BindGroup,
    inv_bg: wgpu::BindGroup,
}

impl GpuRoundtrip {
    /// Initialize the pipeline for the given output resolution. Synchronous
    /// (uses `pollster` to drive the async wgpu init).
    pub fn new(width: u32, height: u32) -> Result<Self> {
        assert!(width.is_multiple_of(2), "width must be even");
        assert!(height.is_multiple_of(2), "height must be even");
        assert!(
            width.is_multiple_of(8) && height.is_multiple_of(8),
            "spike-B dispatches in 8×8 workgroups; width/height must be multiples of 8"
        );

        // wgpu 29: InstanceDescriptor is non-Default and is passed by value.
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            flags: wgpu::InstanceFlags::default(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            backend_options: wgpu::BackendOptions::default(),
            display: None,
        });

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .context("request_adapter (Vulkan HighPerformance)")?;

        let adapter_info = adapter.get_info();

        // WebGPU spec gates: only Rgba8Unorm supports STORAGE_BINDING out of
        // the box. R8Unorm cannot be a storage texture in the baseline tier
        // (the WGSL format list excludes single-channel 8-bit formats), so
        // the output Y/U/V planes are written as Rgba8Unorm storage and the
        // R channel is extracted on CPU readback. Input Y/U/V planes are
        // sampled-only R8Unorm, which is fine.
        let features = adapter.get_texture_format_features(wgpu::TextureFormat::Rgba8Unorm);
        if !features
            .allowed_usages
            .contains(wgpu::TextureUsages::STORAGE_BINDING)
        {
            return Err(anyhow!(
                "adapter {:?} does not support STORAGE_BINDING for Rgba8Unorm — \
                 slice B requires it (vendor={:#x} device={:#x})",
                adapter_info.name,
                adapter_info.vendor,
                adapter_info.device,
            ));
        }

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("spike-s1-b device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::default(),
            trace: wgpu::Trace::Off,
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
        }))
        .context("request_device")?;

        // ---------- textures ----------
        // Helper for the 3 sample-only INPUT planes (R8Unorm).
        let make_input_plane = |label: &str, w: u32, h: u32| -> wgpu::Texture {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        // Helper for the 3 storage-write OUTPUT planes (Rgba8Unorm; see comment
        // above on why R8Unorm storage isn't available).
        let make_output_plane = |label: &str, w: u32, h: u32| -> wgpu::Texture {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };

        let y_in_tex = make_input_plane("y_in", width, height);
        let u_in_tex = make_input_plane("u_in", width / 2, height / 2);
        let v_in_tex = make_input_plane("v_in", width / 2, height / 2);

        let rgb_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("rgb_intermediate"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });

        let y_out_tex = make_output_plane("y_out", width, height);
        let u_out_tex = make_output_plane("u_out", width / 2, height / 2);
        let v_out_tex = make_output_plane("v_out", width / 2, height / 2);

        // ---------- staging buffers ----------
        // Output textures are Rgba8Unorm (4 bytes/pixel); per-row stride in
        // bytes is `4 * width`, padded up to COPY_ALIGN for copy_texture_to_buffer.
        let y_row_bytes = padded_row_bytes(4 * width);
        let uv_row_bytes = padded_row_bytes(4 * (width / 2));

        let y_read_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("y_readback"),
            size: (y_row_bytes as u64) * (height as u64),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let u_read_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("u_readback"),
            size: (uv_row_bytes as u64) * (height as u64 / 2),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let v_read_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("v_readback"),
            size: (uv_row_bytes as u64) * (height as u64 / 2),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        // ---------- pipelines + bind groups ----------
        let fwd_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("yuv_to_rgb"),
            source: wgpu::ShaderSource::Wgsl(YUV_TO_RGB.into()),
        });
        let inv_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rgb_to_yuv"),
            source: wgpu::ShaderSource::Wgsl(RGB_TO_YUV.into()),
        });

        let fwd_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fwd_bgl"),
            entries: &[
                sampled_entry(0),
                sampled_entry(1),
                sampled_entry(2),
                storage_write_entry(3, wgpu::TextureFormat::Rgba8Unorm),
            ],
        });
        let inv_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("inv_bgl"),
            entries: &[
                sampled_entry(0),
                storage_write_entry(1, wgpu::TextureFormat::Rgba8Unorm),
                storage_write_entry(2, wgpu::TextureFormat::Rgba8Unorm),
                storage_write_entry(3, wgpu::TextureFormat::Rgba8Unorm),
            ],
        });

        // wgpu 29: bind_group_layouts entries are Option<&BindGroupLayout>;
        // `push_constant_ranges` was replaced by `immediate_size` (bytes).
        let fwd_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fwd_pl"),
            bind_group_layouts: &[Some(&fwd_bgl)],
            immediate_size: 0,
        });
        let inv_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("inv_pl"),
            bind_group_layouts: &[Some(&inv_bgl)],
            immediate_size: 0,
        });

        let fwd_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("fwd_pipeline"),
            layout: Some(&fwd_pl),
            module: &fwd_module,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        let inv_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("inv_pipeline"),
            layout: Some(&inv_pl),
            module: &inv_module,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        let y_in_view = y_in_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let u_in_view = u_in_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let v_in_view = v_in_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let rgb_view = rgb_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let y_out_view = y_out_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let u_out_view = u_out_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let v_out_view = v_out_tex.create_view(&wgpu::TextureViewDescriptor::default());

        let fwd_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fwd_bg"),
            layout: &fwd_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&y_in_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&u_in_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&v_in_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&rgb_view),
                },
            ],
        });
        let inv_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("inv_bg"),
            layout: &inv_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&rgb_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&y_out_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&u_out_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&v_out_view),
                },
            ],
        });

        // Sanity-touch util::DeviceExt so the import isn't dead — wgpu-util
        // is already in the dep graph via wgpu and we may want it in slice C.
        let _ = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("warmup"),
            contents: &[0u8; 4],
            usage: wgpu::BufferUsages::COPY_SRC,
        });

        Ok(Self {
            device,
            queue,
            adapter_info,
            width,
            height,
            y_in_tex,
            u_in_tex,
            v_in_tex,
            rgb_tex,
            y_out_tex,
            u_out_tex,
            v_out_tex,
            y_read_buf,
            u_read_buf,
            v_read_buf,
            y_row_bytes,
            uv_row_bytes,
            fwd_pipeline,
            inv_pipeline,
            fwd_bg,
            inv_bg,
        })
    }

    /// Roundtrip one frame: upload YUV → forward shader → inverse shader →
    /// readback YUV. Output length always equals input length
    /// (`W*H*3/2` bytes, contiguous planar YUV420P).
    pub fn process_frame(&self, yuv_in: &[u8]) -> Result<Vec<u8>> {
        let expected = (self.width as usize) * (self.height as usize) * 3 / 2;
        if yuv_in.len() != expected {
            return Err(anyhow!(
                "process_frame: expected {expected} bytes for {}×{} YUV420P, got {}",
                self.width,
                self.height,
                yuv_in.len()
            ));
        }

        // -------- upload --------
        let y_plane_len = (self.width * self.height) as usize;
        let chroma_plane_len = ((self.width / 2) * (self.height / 2)) as usize;
        let y_bytes = &yuv_in[..y_plane_len];
        let u_bytes = &yuv_in[y_plane_len..y_plane_len + chroma_plane_len];
        let v_bytes = &yuv_in[y_plane_len + chroma_plane_len..];

        write_plane(
            &self.queue,
            &self.y_in_tex,
            y_bytes,
            self.width,
            self.height,
        );
        write_plane(
            &self.queue,
            &self.u_in_tex,
            u_bytes,
            self.width / 2,
            self.height / 2,
        );
        write_plane(
            &self.queue,
            &self.v_in_tex,
            v_bytes,
            self.width / 2,
            self.height / 2,
        );

        // -------- dispatch + readback --------
        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("spike_b_enc"),
            });

        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fwd_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.fwd_pipeline);
            pass.set_bind_group(0, &self.fwd_bg, &[]);
            pass.dispatch_workgroups(self.width / 8, self.height / 8, 1);
        }
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("inv_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.inv_pipeline);
            pass.set_bind_group(0, &self.inv_bg, &[]);
            pass.dispatch_workgroups(self.width / 8, self.height / 8, 1);
        }

        copy_tex_to_buf(
            &mut enc,
            &self.y_out_tex,
            &self.y_read_buf,
            self.width,
            self.height,
            self.y_row_bytes,
        );
        copy_tex_to_buf(
            &mut enc,
            &self.u_out_tex,
            &self.u_read_buf,
            self.width / 2,
            self.height / 2,
            self.uv_row_bytes,
        );
        copy_tex_to_buf(
            &mut enc,
            &self.v_out_tex,
            &self.v_read_buf,
            self.width / 2,
            self.height / 2,
            self.uv_row_bytes,
        );

        self.queue.submit(Some(enc.finish()));

        let mut out = Vec::with_capacity(expected);
        read_plane(
            &self.device,
            &self.y_read_buf,
            self.width,
            self.height,
            self.y_row_bytes,
            &mut out,
        )?;
        read_plane(
            &self.device,
            &self.u_read_buf,
            self.width / 2,
            self.height / 2,
            self.uv_row_bytes,
            &mut out,
        )?;
        read_plane(
            &self.device,
            &self.v_read_buf,
            self.width / 2,
            self.height / 2,
            self.uv_row_bytes,
            &mut out,
        )?;

        Ok(out)
    }

    /// Human-readable adapter summary for the results doc.
    pub fn adapter_info(&self) -> String {
        let i = &self.adapter_info;
        format!(
            "name={:?} vendor={:#x} device={:#x} type={:?} backend={:?} driver={:?} driver_info={:?}",
            i.name, i.vendor, i.device, i.device_type, i.backend, i.driver, i.driver_info,
        )
    }
}

fn padded_row_bytes(width_in_bytes: u32) -> u32 {
    width_in_bytes.div_ceil(COPY_ALIGN) * COPY_ALIGN
}

fn sampled_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn storage_write_entry(binding: u32, format: wgpu::TextureFormat) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
        count: None,
    }
}

fn write_plane(queue: &wgpu::Queue, tex: &wgpu::Texture, bytes: &[u8], w: u32, h: u32) {
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(w),
            rows_per_image: Some(h),
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
}

fn copy_tex_to_buf(
    enc: &mut wgpu::CommandEncoder,
    tex: &wgpu::Texture,
    buf: &wgpu::Buffer,
    w: u32,
    h: u32,
    row_bytes: u32,
) {
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: buf,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row_bytes),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
}

fn read_plane(
    device: &wgpu::Device,
    buf: &wgpu::Buffer,
    w: u32,
    h: u32,
    row_bytes: u32,
    out: &mut Vec<u8>,
) -> Result<()> {
    let slice = buf.slice(..);
    let (tx, rx) = mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |res| {
        let _ = tx.send(res);
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .context("poll(Wait) for readback")?;
    rx.recv()
        .context("readback channel closed")?
        .context("map_async failed")?;

    {
        let data = slice.get_mapped_range();
        let rb = row_bytes as usize;
        let w_us = w as usize;
        // Output textures are Rgba8Unorm storage — each pixel is 4 bytes,
        // the R channel carries the Y/U/V sample, GBA are ignored. Extract
        // every 4th byte to recover the planar single-channel layout.
        for y in 0..h as usize {
            let row_start = y * rb;
            for x in 0..w_us {
                out.push(data[row_start + x * 4]);
            }
        }
    }
    buf.unmap();
    Ok(())
}

// ============================================================================
// Spike S1 slice C — pipelined end-to-end frame processor
// ============================================================================
//
// `PipelinedGpu` is the slice-C extension that keeps N frames in flight
// concurrently. Slice B's `GpuRoundtrip` above stays as-is (still the
// single-buffered reference for the GPU determinism check).
//
// Design notes:
//   * Per-frame resources live in a ring of `pipeline_depth` slots. Each
//     slot owns its own input/output textures, blend uniform, and staging
//     buffer. The orchestrator submits a frame to a free slot, then
//     collects the front slot in FIFO order.
//   * Crossfade frames upload BOTH source YUVs to the slot, run
//     `yuv_to_rgb` twice, run `crossfade` (with `weight` written to the
//     slot's uniform buffer), then `rgb_to_yuv` reading the blend output.
//   * Solo frames upload one source, run `yuv_to_rgb`, then `rgb_to_yuv`
//     directly off `rgb_a` — no crossfade dispatch.
//   * Readback uses one packed staging buffer per slot (Y rows padded,
//     then U rows padded, then V rows padded) plus an opaque
//     `SubmissionIndex` per slot so `collect_frame_blocking` can wait
//     for exactly that submission.
//
// No `unsafe`. The wgpu-29 quirks from slice B (Instance::new by value,
// `immediate_size`, `Option<&BindGroupLayout>`, `Trace::Off`,
// `PollType<SubmissionIndex>`) are reapplied silently.

use std::collections::VecDeque;

use crate::spike_s1::shaders::CROSSFADE;

/// What the timeline wants for a given frame.
#[derive(Clone, Copy, Debug)]
pub enum FrameOp {
    /// Single source — pass through (YUV→RGB→YUV).
    Solo,
    /// Crossfade between two sources. `weight` is clip B's contribution:
    /// 0 = pure A, 1 = pure B. The orchestrator should NOT submit this
    /// variant with weight ∈ {0, 1} — use `Solo` for the in-the-clear
    /// regions to skip the crossfade dispatch entirely.
    Crossfade { weight: f32 },
}

/// One ring slot — all GPU resources for one in-flight frame.
struct Slot {
    // Input planes (A path; B path used only for crossfade)
    y_a_in: wgpu::Texture,
    u_a_in: wgpu::Texture,
    v_a_in: wgpu::Texture,
    y_b_in: wgpu::Texture,
    u_b_in: wgpu::Texture,
    v_b_in: wgpu::Texture,
    // Intermediate RGB targets
    #[allow(dead_code)]
    rgb_a: wgpu::Texture,
    #[allow(dead_code)]
    rgb_b: wgpu::Texture,
    #[allow(dead_code)]
    rgb_blend: wgpu::Texture,
    // Final YUV outputs (Rgba8Unorm; R channel carries the sample)
    #[allow(dead_code)]
    y_out: wgpu::Texture,
    #[allow(dead_code)]
    u_out: wgpu::Texture,
    #[allow(dead_code)]
    v_out: wgpu::Texture,

    // Blend weight uniform (16 bytes, padded)
    blend_uniform: wgpu::Buffer,

    // Packed staging buffer: [Y padded rows][U padded rows][V padded rows]
    staging: wgpu::Buffer,
    y_offset: u64,
    u_offset: u64,
    v_offset: u64,

    // Bind groups
    fwd_bg_a: wgpu::BindGroup,
    fwd_bg_b: wgpu::BindGroup,
    crossfade_bg: wgpu::BindGroup,
    inv_bg_from_a: wgpu::BindGroup,
    inv_bg_from_blend: wgpu::BindGroup,
}

/// Tracks an in-flight frame waiting to be drained.
struct InFlight {
    slot_idx: usize,
    submission_index: wgpu::SubmissionIndex,
    op: FrameOp,
    /// Opaque counter returned to the caller (matches the order in which
    /// `submit_*_frame` was called). Not the wgpu submission index.
    serial: u64,
}

pub struct PipelinedGpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    adapter_info: wgpu::AdapterInfo,

    width: u32,
    height: u32,
    pipeline_depth: usize,
    y_row_bytes: u32,
    uv_row_bytes: u32,

    fwd_pipeline: wgpu::ComputePipeline,
    crossfade_pipeline: wgpu::ComputePipeline,
    inv_pipeline: wgpu::ComputePipeline,

    slots: Vec<Slot>,
    free_slots: VecDeque<usize>,
    in_flight: VecDeque<InFlight>,
    next_serial: u64,
}

impl PipelinedGpu {
    pub fn new(
        width: u32,
        height: u32,
        pipeline_depth: usize,
        adapter_mode: AdapterMode,
    ) -> Result<Self> {
        assert!(
            width.is_multiple_of(8) && height.is_multiple_of(8),
            "spike-C dispatches in 8×8 workgroups; width/height must be multiples of 8"
        );
        assert!(pipeline_depth >= 1, "pipeline_depth must be ≥ 1");

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            flags: wgpu::InstanceFlags::default(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            backend_options: wgpu::BackendOptions::default(),
            display: None,
        });
        let adapter = match adapter_mode {
            AdapterMode::HighPerformance => {
                pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    compatible_surface: None,
                    force_fallback_adapter: false,
                }))
                .context("request_adapter (Vulkan HighPerformance)")?
            }
            AdapterMode::LowPower => {
                pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::LowPower,
                    compatible_surface: None,
                    force_fallback_adapter: false,
                }))
                .context("request_adapter (Vulkan LowPower)")?
            }
            AdapterMode::Integrated => {
                let adapters =
                    pollster::block_on(instance.enumerate_adapters(wgpu::Backends::VULKAN));
                adapters
                    .into_iter()
                    .find(|a| a.get_info().device_type == wgpu::DeviceType::IntegratedGpu)
                    .context("no integrated GPU adapter found (check vulkaninfo --summary)")?
            }
        };
        let adapter_info = adapter.get_info();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("spike-s1-c device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::default(),
            trace: wgpu::Trace::Off,
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
        }))
        .context("request_device")?;

        // ---------- pipelines + layouts (shared across all slots) ----------
        let fwd_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("yuv_to_rgb"),
            source: wgpu::ShaderSource::Wgsl(YUV_TO_RGB.into()),
        });
        let inv_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rgb_to_yuv"),
            source: wgpu::ShaderSource::Wgsl(RGB_TO_YUV.into()),
        });
        let cf_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("crossfade"),
            source: wgpu::ShaderSource::Wgsl(CROSSFADE.into()),
        });

        let fwd_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fwd_bgl"),
            entries: &[
                sampled_entry(0),
                sampled_entry(1),
                sampled_entry(2),
                storage_write_entry(3, wgpu::TextureFormat::Rgba8Unorm),
            ],
        });
        let inv_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("inv_bgl"),
            entries: &[
                sampled_entry(0),
                storage_write_entry(1, wgpu::TextureFormat::Rgba8Unorm),
                storage_write_entry(2, wgpu::TextureFormat::Rgba8Unorm),
                storage_write_entry(3, wgpu::TextureFormat::Rgba8Unorm),
            ],
        });
        let cf_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("crossfade_bgl"),
            entries: &[
                sampled_entry(0),
                sampled_entry(1),
                storage_write_entry(2, wgpu::TextureFormat::Rgba8Unorm),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let fwd_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fwd_pl"),
            bind_group_layouts: &[Some(&fwd_bgl)],
            immediate_size: 0,
        });
        let inv_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("inv_pl"),
            bind_group_layouts: &[Some(&inv_bgl)],
            immediate_size: 0,
        });
        let cf_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cf_pl"),
            bind_group_layouts: &[Some(&cf_bgl)],
            immediate_size: 0,
        });

        let fwd_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("fwd_pipeline"),
            layout: Some(&fwd_pl),
            module: &fwd_module,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        let inv_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("inv_pipeline"),
            layout: Some(&inv_pl),
            module: &inv_module,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        let crossfade_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("crossfade_pipeline"),
            layout: Some(&cf_pl),
            module: &cf_module,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        // Staging buffer layout (per slot):
        //   [0 .. y_size)               Y plane padded rows
        //   [y_size .. y_size+uv_size)  U plane padded rows
        //   [y_size+uv_size .. total)   V plane padded rows
        // Each plane's offset is 256-byte-aligned because `y_row_bytes` and
        // `uv_row_bytes` are themselves multiples of COPY_ALIGN, so any
        // (row_bytes * height) product is also a multiple. The per-slot
        // offsets are computed inside `build_slot`.
        let y_row_bytes = padded_row_bytes(4 * width);
        let uv_row_bytes = padded_row_bytes(4 * (width / 2));
        let y_size = (y_row_bytes as u64) * (height as u64);
        let uv_size = (uv_row_bytes as u64) * (height as u64 / 2);
        let staging_size: u64 = y_size + 2 * uv_size;

        // ---------- build N slots ----------
        let mut slots = Vec::with_capacity(pipeline_depth);
        for i in 0..pipeline_depth {
            slots.push(build_slot(
                &device,
                width,
                height,
                staging_size,
                &fwd_bgl,
                &inv_bgl,
                &cf_bgl,
                i,
            )?);
        }

        let free_slots: VecDeque<usize> = (0..pipeline_depth).collect();

        Ok(Self {
            device,
            queue,
            adapter_info,
            width,
            height,
            pipeline_depth,
            y_row_bytes,
            uv_row_bytes,
            fwd_pipeline,
            crossfade_pipeline,
            inv_pipeline,
            slots,
            free_slots,
            in_flight: VecDeque::with_capacity(pipeline_depth),
            next_serial: 0,
        })
    }

    pub fn submit_solo_frame(&mut self, yuv: &[u8]) -> Result<u64> {
        let expected = (self.width as usize) * (self.height as usize) * 3 / 2;
        if yuv.len() != expected {
            return Err(anyhow!(
                "submit_solo_frame: expected {expected} bytes for {}×{} YUV420P, got {}",
                self.width,
                self.height,
                yuv.len()
            ));
        }
        let slot_idx = self
            .free_slots
            .pop_front()
            .ok_or_else(|| anyhow!("no free slot — caller violated in_flight() < depth"))?;

        // Upload A planes only.
        upload_yuv_to_slot(
            &self.queue,
            &self.slots[slot_idx],
            yuv,
            self.width,
            self.height,
            true,
        );

        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("solo_enc"),
            });
        // fwd_a → rgb_a
        {
            let mut p = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("solo_fwd_a"),
                timestamp_writes: None,
            });
            p.set_pipeline(&self.fwd_pipeline);
            p.set_bind_group(0, &self.slots[slot_idx].fwd_bg_a, &[]);
            p.dispatch_workgroups(self.width / 8, self.height / 8, 1);
        }
        // inv ← rgb_a
        {
            let mut p = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("solo_inv"),
                timestamp_writes: None,
            });
            p.set_pipeline(&self.inv_pipeline);
            p.set_bind_group(0, &self.slots[slot_idx].inv_bg_from_a, &[]);
            p.dispatch_workgroups(self.width / 8, self.height / 8, 1);
        }
        self.encode_readback(&mut enc, slot_idx);

        let submission_index = self.queue.submit(Some(enc.finish()));
        let serial = self.next_serial;
        self.next_serial += 1;
        self.in_flight.push_back(InFlight {
            slot_idx,
            submission_index,
            op: FrameOp::Solo,
            serial,
        });
        Ok(serial)
    }

    pub fn submit_crossfade_frame(
        &mut self,
        yuv_a: &[u8],
        yuv_b: &[u8],
        weight: f32,
    ) -> Result<u64> {
        let expected = (self.width as usize) * (self.height as usize) * 3 / 2;
        if yuv_a.len() != expected || yuv_b.len() != expected {
            return Err(anyhow!(
                "submit_crossfade_frame: each input must be {expected} bytes (got A={}, B={})",
                yuv_a.len(),
                yuv_b.len()
            ));
        }
        if !(weight.is_finite() && (0.0..=1.0).contains(&weight)) {
            return Err(anyhow!(
                "submit_crossfade_frame: weight must be finite in [0,1], got {weight}"
            ));
        }
        let slot_idx = self
            .free_slots
            .pop_front()
            .ok_or_else(|| anyhow!("no free slot — caller violated in_flight() < depth"))?;

        // Upload both A and B planes.
        upload_yuv_to_slot(
            &self.queue,
            &self.slots[slot_idx],
            yuv_a,
            self.width,
            self.height,
            true,
        );
        upload_yuv_to_slot(
            &self.queue,
            &self.slots[slot_idx],
            yuv_b,
            self.width,
            self.height,
            false,
        );

        // Write blend weight uniform (16 bytes, weight in slot 0).
        let pad = [weight, 0.0, 0.0, 0.0];
        self.queue.write_buffer(
            &self.slots[slot_idx].blend_uniform,
            0,
            bytemuck::cast_slice(&pad),
        );

        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("crossfade_enc"),
            });
        // fwd_a → rgb_a
        {
            let mut p = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("cf_fwd_a"),
                timestamp_writes: None,
            });
            p.set_pipeline(&self.fwd_pipeline);
            p.set_bind_group(0, &self.slots[slot_idx].fwd_bg_a, &[]);
            p.dispatch_workgroups(self.width / 8, self.height / 8, 1);
        }
        // fwd_b → rgb_b
        {
            let mut p = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("cf_fwd_b"),
                timestamp_writes: None,
            });
            p.set_pipeline(&self.fwd_pipeline);
            p.set_bind_group(0, &self.slots[slot_idx].fwd_bg_b, &[]);
            p.dispatch_workgroups(self.width / 8, self.height / 8, 1);
        }
        // crossfade(rgb_a, rgb_b, weight) → rgb_blend
        {
            let mut p = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("cf_blend"),
                timestamp_writes: None,
            });
            p.set_pipeline(&self.crossfade_pipeline);
            p.set_bind_group(0, &self.slots[slot_idx].crossfade_bg, &[]);
            p.dispatch_workgroups(self.width / 8, self.height / 8, 1);
        }
        // inv ← rgb_blend
        {
            let mut p = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("cf_inv"),
                timestamp_writes: None,
            });
            p.set_pipeline(&self.inv_pipeline);
            p.set_bind_group(0, &self.slots[slot_idx].inv_bg_from_blend, &[]);
            p.dispatch_workgroups(self.width / 8, self.height / 8, 1);
        }
        self.encode_readback(&mut enc, slot_idx);

        let submission_index = self.queue.submit(Some(enc.finish()));
        let serial = self.next_serial;
        self.next_serial += 1;
        self.in_flight.push_back(InFlight {
            slot_idx,
            submission_index,
            op: FrameOp::Crossfade { weight },
            serial,
        });
        Ok(serial)
    }

    pub fn try_collect_frame(&mut self) -> Result<Option<Vec<u8>>> {
        if self.in_flight.is_empty() {
            return Ok(None);
        }
        // Non-blocking poll. If the front frame is still in flight, return None.
        // wgpu's Poll variant returns immediately; we then check whether the
        // front slot's map_async callback has had a chance to fire by trying a
        // blocking wait with a zero-ish timeout. Cheaper / simpler: just call
        // the blocking collector — it polls then maps. For slice C the
        // orchestrator only calls collect_frame_blocking anyway, so this path
        // is unexercised by the harness but kept for the spec API.
        self.collect_frame_blocking().map(Some)
    }

    pub fn collect_frame_blocking(&mut self) -> Result<Vec<u8>> {
        let inflight = self
            .in_flight
            .pop_front()
            .ok_or_else(|| anyhow!("collect_frame_blocking: queue empty"))?;
        let slot_idx = inflight.slot_idx;

        // Queue the map request; the callback fires once the GPU finishes
        // writing the staging buffer for THIS submission.
        let (tx, rx) = mpsc::channel();
        self.slots[slot_idx]
            .staging
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |r| {
                let _ = tx.send(r);
            });

        // Block until our specific submission completes and callbacks fire.
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(inflight.submission_index.clone()),
                timeout: None,
            })
            .context("poll wait_for(submission_index)")?;
        rx.recv()
            .context("map_async channel closed")?
            .context("map_async failed")?;

        let frame_bytes = (self.width as usize) * (self.height as usize) * 3 / 2;
        let mut out = Vec::with_capacity(frame_bytes);
        {
            let slot = &self.slots[slot_idx];
            let data = slot.staging.slice(..).get_mapped_range();
            extract_plane_from_padded_rgba(
                &data[slot.y_offset as usize
                    ..(slot.y_offset + plane_size(self.y_row_bytes, self.height)) as usize],
                self.width,
                self.height,
                self.y_row_bytes,
                &mut out,
            );
            extract_plane_from_padded_rgba(
                &data[slot.u_offset as usize
                    ..(slot.u_offset + plane_size(self.uv_row_bytes, self.height / 2)) as usize],
                self.width / 2,
                self.height / 2,
                self.uv_row_bytes,
                &mut out,
            );
            extract_plane_from_padded_rgba(
                &data[slot.v_offset as usize
                    ..(slot.v_offset + plane_size(self.uv_row_bytes, self.height / 2)) as usize],
                self.width / 2,
                self.height / 2,
                self.uv_row_bytes,
                &mut out,
            );
        }
        self.slots[slot_idx].staging.unmap();
        self.free_slots.push_back(slot_idx);
        debug_assert_eq!(out.len(), frame_bytes);
        debug_assert!(matches!(
            inflight.op,
            FrameOp::Solo | FrameOp::Crossfade { .. }
        ));
        let _ = inflight.serial; // serial returned by submit_* is for caller bookkeeping; we don't reuse it here
        Ok(out)
    }

    pub fn in_flight(&self) -> usize {
        self.in_flight.len()
    }

    pub fn pipeline_depth(&self) -> usize {
        self.pipeline_depth
    }

    pub fn adapter_info(&self) -> String {
        let i = &self.adapter_info;
        format!(
            "name={:?} vendor={:#x} device={:#x} type={:?} backend={:?} driver={:?} driver_info={:?}",
            i.name, i.vendor, i.device, i.device_type, i.backend, i.driver, i.driver_info,
        )
    }

    fn encode_readback(&self, enc: &mut wgpu::CommandEncoder, slot_idx: usize) {
        let slot = &self.slots[slot_idx];
        copy_tex_to_buf_at(
            enc,
            &slot.y_out,
            &slot.staging,
            slot.y_offset,
            self.width,
            self.height,
            self.y_row_bytes,
        );
        copy_tex_to_buf_at(
            enc,
            &slot.u_out,
            &slot.staging,
            slot.u_offset,
            self.width / 2,
            self.height / 2,
            self.uv_row_bytes,
        );
        copy_tex_to_buf_at(
            enc,
            &slot.v_out,
            &slot.staging,
            slot.v_offset,
            self.width / 2,
            self.height / 2,
            self.uv_row_bytes,
        );
    }
}

fn plane_size(row_bytes: u32, rows: u32) -> u64 {
    (row_bytes as u64) * (rows as u64)
}

#[allow(clippy::too_many_arguments)]
fn build_slot(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    staging_size: u64,
    fwd_bgl: &wgpu::BindGroupLayout,
    inv_bgl: &wgpu::BindGroupLayout,
    cf_bgl: &wgpu::BindGroupLayout,
    idx: usize,
) -> Result<Slot> {
    let make_input = |label: &str, w: u32, h: u32| -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        })
    };
    let make_rgb_intermediate = |label: &str| -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
    };
    let make_yuv_output = |label: &str, w: u32, h: u32| -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    };

    let y_a_in = make_input(&format!("slot{idx}_y_a_in"), width, height);
    let u_a_in = make_input(&format!("slot{idx}_u_a_in"), width / 2, height / 2);
    let v_a_in = make_input(&format!("slot{idx}_v_a_in"), width / 2, height / 2);
    let y_b_in = make_input(&format!("slot{idx}_y_b_in"), width, height);
    let u_b_in = make_input(&format!("slot{idx}_u_b_in"), width / 2, height / 2);
    let v_b_in = make_input(&format!("slot{idx}_v_b_in"), width / 2, height / 2);
    let rgb_a = make_rgb_intermediate(&format!("slot{idx}_rgb_a"));
    let rgb_b = make_rgb_intermediate(&format!("slot{idx}_rgb_b"));
    let rgb_blend = make_rgb_intermediate(&format!("slot{idx}_rgb_blend"));
    let y_out = make_yuv_output(&format!("slot{idx}_y_out"), width, height);
    let u_out = make_yuv_output(&format!("slot{idx}_u_out"), width / 2, height / 2);
    let v_out = make_yuv_output(&format!("slot{idx}_v_out"), width / 2, height / 2);

    let blend_uniform = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(&format!("slot{idx}_blend_u")),
        size: 16,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(&format!("slot{idx}_staging")),
        size: staging_size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    // Views
    let y_a_v = y_a_in.create_view(&wgpu::TextureViewDescriptor::default());
    let u_a_v = u_a_in.create_view(&wgpu::TextureViewDescriptor::default());
    let v_a_v = v_a_in.create_view(&wgpu::TextureViewDescriptor::default());
    let y_b_v = y_b_in.create_view(&wgpu::TextureViewDescriptor::default());
    let u_b_v = u_b_in.create_view(&wgpu::TextureViewDescriptor::default());
    let v_b_v = v_b_in.create_view(&wgpu::TextureViewDescriptor::default());
    let rgb_a_v = rgb_a.create_view(&wgpu::TextureViewDescriptor::default());
    let rgb_b_v = rgb_b.create_view(&wgpu::TextureViewDescriptor::default());
    let rgb_blend_v = rgb_blend.create_view(&wgpu::TextureViewDescriptor::default());
    let y_out_v = y_out.create_view(&wgpu::TextureViewDescriptor::default());
    let u_out_v = u_out.create_view(&wgpu::TextureViewDescriptor::default());
    let v_out_v = v_out.create_view(&wgpu::TextureViewDescriptor::default());

    let fwd_bg_a = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(&format!("slot{idx}_fwd_bg_a")),
        layout: fwd_bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&y_a_v),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&u_a_v),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&v_a_v),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(&rgb_a_v),
            },
        ],
    });
    let fwd_bg_b = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(&format!("slot{idx}_fwd_bg_b")),
        layout: fwd_bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&y_b_v),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&u_b_v),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&v_b_v),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(&rgb_b_v),
            },
        ],
    });
    let crossfade_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(&format!("slot{idx}_cf_bg")),
        layout: cf_bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&rgb_a_v),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&rgb_b_v),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&rgb_blend_v),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: blend_uniform.as_entire_binding(),
            },
        ],
    });
    let inv_bg_from_a = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(&format!("slot{idx}_inv_bg_a")),
        layout: inv_bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&rgb_a_v),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&y_out_v),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&u_out_v),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(&v_out_v),
            },
        ],
    });
    let inv_bg_from_blend = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(&format!("slot{idx}_inv_bg_blend")),
        layout: inv_bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&rgb_blend_v),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&y_out_v),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&u_out_v),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(&v_out_v),
            },
        ],
    });

    let y_row_bytes = padded_row_bytes(4 * width);
    let uv_row_bytes = padded_row_bytes(4 * (width / 2));
    let y_size = (y_row_bytes as u64) * (height as u64);
    let uv_size = (uv_row_bytes as u64) * (height as u64 / 2);

    Ok(Slot {
        y_a_in,
        u_a_in,
        v_a_in,
        y_b_in,
        u_b_in,
        v_b_in,
        rgb_a,
        rgb_b,
        rgb_blend,
        y_out,
        u_out,
        v_out,
        blend_uniform,
        staging,
        y_offset: 0,
        u_offset: y_size,
        v_offset: y_size + uv_size,
        fwd_bg_a,
        fwd_bg_b,
        crossfade_bg,
        inv_bg_from_a,
        inv_bg_from_blend,
    })
}

fn upload_yuv_to_slot(
    queue: &wgpu::Queue,
    slot: &Slot,
    yuv: &[u8],
    width: u32,
    height: u32,
    is_a: bool,
) {
    let y_plane_len = (width * height) as usize;
    let chroma_plane_len = ((width / 2) * (height / 2)) as usize;
    let y_bytes = &yuv[..y_plane_len];
    let u_bytes = &yuv[y_plane_len..y_plane_len + chroma_plane_len];
    let v_bytes = &yuv[y_plane_len + chroma_plane_len..];
    let (y_tex, u_tex, v_tex) = if is_a {
        (&slot.y_a_in, &slot.u_a_in, &slot.v_a_in)
    } else {
        (&slot.y_b_in, &slot.u_b_in, &slot.v_b_in)
    };
    write_plane(queue, y_tex, y_bytes, width, height);
    write_plane(queue, u_tex, u_bytes, width / 2, height / 2);
    write_plane(queue, v_tex, v_bytes, width / 2, height / 2);
}

fn copy_tex_to_buf_at(
    enc: &mut wgpu::CommandEncoder,
    tex: &wgpu::Texture,
    buf: &wgpu::Buffer,
    offset: u64,
    w: u32,
    h: u32,
    row_bytes: u32,
) {
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: buf,
            layout: wgpu::TexelCopyBufferLayout {
                offset,
                bytes_per_row: Some(row_bytes),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
}

/// `padded` is a tightly-padded-row Rgba8Unorm region with `row_bytes` stride.
/// Extract the R channel and append `w * h` single-channel bytes to `out`.
fn extract_plane_from_padded_rgba(
    padded: &[u8],
    w: u32,
    h: u32,
    row_bytes: u32,
    out: &mut Vec<u8>,
) {
    let rb = row_bytes as usize;
    let w_us = w as usize;
    for y in 0..h as usize {
        let row_start = y * rb;
        for x in 0..w_us {
            out.push(padded[row_start + x * 4]);
        }
    }
}
