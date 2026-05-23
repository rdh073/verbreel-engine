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
