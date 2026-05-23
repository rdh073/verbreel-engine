//! Native multiply-blend runner — Vulkan via wgpu 29 + Naga.
//!
//! Inputs: two RGBA8 PNGs at the same resolution.
//! Output: one RGBA8 PNG of the same resolution, encoded with the same
//! deterministic settings as `synth::write_input_*` so byte-identity is
//! a function of the GPU pipeline alone.
//!
//! No `unsafe`. Same wgpu 29 quirks as S1 slice B (Instance::new by value,
//! `request_adapter` future returns `Option<Adapter>`, etc.).

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

use anyhow::{Context, Result, anyhow};

use super::shader::MULTIPLY_BLEND;
use super::synth::decode_rgba8_png;

/// One-shot: load two PNGs, multiply-blend them on Vulkan, write the
/// result PNG to `output_path`. Returns the adapter info so the caller
/// can record which GPU was used.
pub fn run_native(
    input_a_path: &Path,
    input_b_path: &Path,
    output_path: &Path,
) -> Result<wgpu::AdapterInfo> {
    let bytes_a =
        std::fs::read(input_a_path).with_context(|| format!("read {}", input_a_path.display()))?;
    let bytes_b =
        std::fs::read(input_b_path).with_context(|| format!("read {}", input_b_path.display()))?;
    let (wa, ha, rgba_a) = decode_rgba8_png(&bytes_a)?;
    let (wb, hb, rgba_b) = decode_rgba8_png(&bytes_b)?;
    anyhow::ensure!(
        wa == wb && ha == hb,
        "input dim mismatch: A {wa}×{ha} vs B {wb}×{hb}"
    );
    let (w, h) = (wa, ha);

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

    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("spike-s2 native device"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        memory_hints: wgpu::MemoryHints::default(),
        trace: wgpu::Trace::Off,
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
    }))
    .context("request_device")?;

    let (output_buf, padded_row_bytes) =
        execute_multiply_blend(&device, &queue, w, h, &rgba_a, &rgba_b)?;

    // Strip the 256-byte row padding before writing the PNG.
    let row_bytes = (w * 4) as usize;
    let mut tight = Vec::with_capacity(row_bytes * h as usize);
    for row in 0..h as usize {
        let start = row * padded_row_bytes as usize;
        tight.extend_from_slice(&output_buf[start..start + row_bytes]);
    }

    write_rgba8_png(output_path, w, h, &tight)?;
    Ok(adapter_info)
}

/// Run the compute pass and return (padded readback bytes, padded row stride).
fn execute_multiply_blend(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    w: u32,
    h: u32,
    rgba_a: &[u8],
    rgba_b: &[u8],
) -> Result<(Vec<u8>, u32)> {
    let make_input = |label: &str| -> wgpu::Texture {
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
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        })
    };
    let src_a = make_input("src_a");
    let src_b = make_input("src_b");

    let dst = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("dst"),
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
    });

    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &src_a,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        rgba_a,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(w * 4),
            rows_per_image: Some(h),
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &src_b,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        rgba_b,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(w * 4),
            rows_per_image: Some(h),
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );

    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("multiply_blend"),
        source: wgpu::ShaderSource::Wgsl(MULTIPLY_BLEND.into()),
    });

    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("multiply_blend pipeline"),
        layout: None,
        module: &module,
        entry_point: Some("main"),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    });

    let bgl = pipeline.get_bind_group_layout(0);
    let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("multiply_blend bg"),
        layout: &bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(
                    &src_a.create_view(&wgpu::TextureViewDescriptor::default()),
                ),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(
                    &src_b.create_view(&wgpu::TextureViewDescriptor::default()),
                ),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(
                    &dst.create_view(&wgpu::TextureViewDescriptor::default()),
                ),
            },
        ],
    });

    // 256-byte row alignment required by copy_texture_to_buffer.
    let row_bytes = w * 4;
    let padded_row_bytes = align_up(row_bytes, wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let total_bytes = (padded_row_bytes as u64) * (h as u64);

    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("multiply_blend staging"),
        size: total_bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("multiply_blend encoder"),
    });
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("multiply_blend pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, Some(&bg), &[]);
        let groups_x = w.div_ceil(8);
        let groups_y = h.div_ceil(8);
        pass.dispatch_workgroups(groups_x, groups_y, 1);
    }
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &dst,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_row_bytes),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(std::iter::once(encoder.finish()));

    let slice = staging.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        tx.send(r).expect("map_async result channel closed");
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|e| anyhow!("device.poll: {e:?}"))?;
    rx.recv()
        .context("map_async channel recv")?
        .map_err(|e| anyhow!("buffer map failed: {e:?}"))?;
    let bytes = slice.get_mapped_range().to_vec();
    drop(staging);
    Ok((bytes, padded_row_bytes))
}

fn align_up(value: u32, align: u32) -> u32 {
    value.div_ceil(align) * align
}

fn write_rgba8_png(path: &Path, w: u32, h: u32, rgba: &[u8]) -> Result<()> {
    let file =
        File::create(path).with_context(|| format!("create PNG output {}", path.display()))?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Balanced);
    let mut writer = encoder.write_header().context("write PNG header")?;
    writer
        .write_image_data(rgba)
        .context("write PNG image data")?;
    Ok(())
}
