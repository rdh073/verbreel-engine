//! Wasm runner — runs the same multiply-blend WGSL on Chrome WebGPU.
//!
//! Hosted by `crates/verbreel-render/web/index.html` after
//! `wasm-pack build --target web --release -- --features spike-s2`.
//! The function exposed below is async (WebGPU init + readback are
//! Promise-based on the browser side) and triggers a download of the
//! resulting PNG so the human can move it next to `native_frame.png`
//! for the diff step.
//!
//! Input PNGs are embedded at compile time via `include_bytes!` — the
//! `web/input_a.png` and `web/input_b.png` files MUST be the exact bytes
//! produced by `synth::write_input_*`. The native runner copies them
//! there as part of the build flow.

use anyhow::{Context, Result, anyhow};
use std::io::Cursor;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

use super::shader::MULTIPLY_BLEND;
use super::synth::decode_rgba8_png;

// Input PNG bytes baked at compile time.
const INPUT_A_PNG: &[u8] = include_bytes!("../../web/input_a.png");
const INPUT_B_PNG: &[u8] = include_bytes!("../../web/input_b.png");

/// JS-callable entry. Awaitable from the host page.
#[wasm_bindgen]
pub async fn run_wasm() -> Result<(), JsValue> {
    console_error_panic_hook::set_once();
    match run_wasm_inner().await {
        Ok(()) => Ok(()),
        Err(e) => {
            let msg = format!("spike-s2 wasm error: {e:#}");
            web_sys::console::error_1(&JsValue::from_str(&msg));
            Err(JsValue::from_str(&msg))
        }
    }
}

async fn run_wasm_inner() -> Result<()> {
    let (wa, ha, rgba_a) = decode_rgba8_png(INPUT_A_PNG)?;
    let (wb, hb, rgba_b) = decode_rgba8_png(INPUT_B_PNG)?;
    anyhow::ensure!(
        wa == wb && ha == hb,
        "input dim mismatch: A {wa}×{ha} vs B {wb}×{hb}"
    );
    let (w, h) = (wa, ha);

    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::BROWSER_WEBGPU,
        flags: wgpu::InstanceFlags::default(),
        memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        backend_options: wgpu::BackendOptions::default(),
        display: None,
    });
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        })
        .await
        .context("request_adapter (Chrome WebGPU)")?;
    let adapter_info = adapter.get_info();
    log_info(&format!(
        "Adapter: name={:?} backend={:?} type={:?}",
        adapter_info.name, adapter_info.backend, adapter_info.device_type
    ));

    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("spike-s2 wasm device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_webgl2_defaults()
                .using_resolution(adapter.limits()),
            memory_hints: wgpu::MemoryHints::default(),
            trace: wgpu::Trace::Off,
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
        })
        .await
        .context("request_device")?;

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
        &rgba_a,
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
        &rgba_b,
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

    let row_bytes = w * 4;
    let padded_row_bytes = align_up(row_bytes, wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let total_bytes = (padded_row_bytes as u64) * (h as u64);
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("staging"),
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
        pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
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

    // Browser readback: map_async returns a channel; await via wasm-bindgen.
    let slice = staging.slice(..);
    let (tx, rx) = futures_oneshot();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    // Yield to the browser event loop so the GPU has a chance to finish.
    // wgpu's wasm backend polls automatically once the queue is submitted.
    rx.await
        .map_err(|_| anyhow!("map_async sender dropped"))?
        .map_err(|e| anyhow!("buffer map failed: {e:?}"))?;
    let mapped = slice.get_mapped_range();
    let padded_bytes: Vec<u8> = mapped.to_vec();
    drop(mapped);
    drop(staging);

    // Strip row padding.
    let row_bytes_usize = row_bytes as usize;
    let mut tight = Vec::with_capacity(row_bytes_usize * h as usize);
    for row in 0..h as usize {
        let start = row * padded_row_bytes as usize;
        tight.extend_from_slice(&padded_bytes[start..start + row_bytes_usize]);
    }

    // Encode PNG with the same settings as native.
    let mut png_bytes = Vec::with_capacity(tight.len());
    {
        let mut encoder = png::Encoder::new(Cursor::new(&mut png_bytes), w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Balanced);
        let mut writer = encoder.write_header().context("write PNG header")?;
        writer
            .write_image_data(&tight)
            .context("write PNG image data")?;
    }
    log_info(&format!(
        "Encoded PNG: {} bytes (w={w} h={h})",
        png_bytes.len()
    ));

    trigger_download(&png_bytes, "wasm_frame.png")?;
    Ok(())
}

fn align_up(value: u32, align: u32) -> u32 {
    value.div_ceil(align) * align
}

fn log_info(msg: &str) {
    web_sys::console::log_1(&JsValue::from_str(msg));
}

fn trigger_download(bytes: &[u8], filename: &str) -> Result<()> {
    let window = web_sys::window().context("no window")?;
    let document = window.document().context("no document")?;

    // Build a Blob from the bytes.
    let array = js_sys::Uint8Array::new_with_length(bytes.len() as u32);
    array.copy_from(bytes);
    let parts = js_sys::Array::new();
    parts.push(&array.buffer());
    let blob_opts = web_sys::BlobPropertyBag::new();
    blob_opts.set_type("image/png");
    let blob = web_sys::Blob::new_with_buffer_source_sequence_and_options(&parts, &blob_opts)
        .map_err(|e| anyhow!("Blob::new: {e:?}"))?;
    let url = web_sys::Url::create_object_url_with_blob(&blob)
        .map_err(|e| anyhow!("createObjectURL: {e:?}"))?;

    let a: web_sys::HtmlAnchorElement = document
        .create_element("a")
        .map_err(|e| anyhow!("createElement: {e:?}"))?
        .dyn_into()
        .map_err(|e| anyhow!("dyn_into HtmlAnchorElement: {e:?}"))?;
    a.set_href(&url);
    a.set_download(filename);
    a.click();
    web_sys::Url::revoke_object_url(&url).map_err(|e| anyhow!("revokeObjectURL: {e:?}"))?;
    Ok(())
}

// ----------------------------------------------------------------------
// Tiny oneshot for `map_async`. The wgpu wasm backend resolves the
// `map_async` callback once the GPU has finished the submitted work, so
// awaiting this future is equivalent to `device.poll(Wait)` on native.
// We don't pull in the `futures` crate just for one oneshot.
// ----------------------------------------------------------------------

use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context as TaskContext, Poll, Waker};

struct OneshotInner<T> {
    value: Option<T>,
    waker: Option<Waker>,
    dropped: bool,
}

struct OneshotSender<T>(Rc<RefCell<OneshotInner<T>>>);
struct OneshotReceiver<T>(Rc<RefCell<OneshotInner<T>>>);

impl<T> OneshotSender<T> {
    fn send(self, value: T) -> Result<(), T> {
        let mut inner = self.0.borrow_mut();
        if inner.dropped {
            return Err(value);
        }
        inner.value = Some(value);
        if let Some(w) = inner.waker.take() {
            drop(inner);
            w.wake();
        }
        Ok(())
    }
}

impl<T> Drop for OneshotSender<T> {
    fn drop(&mut self) {
        let mut inner = self.0.borrow_mut();
        inner.dropped = true;
        if let Some(w) = inner.waker.take() {
            drop(inner);
            w.wake();
        }
    }
}

impl<T> Future for OneshotReceiver<T> {
    type Output = Result<T, ()>;
    fn poll(self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Self::Output> {
        let mut inner = self.0.borrow_mut();
        if let Some(v) = inner.value.take() {
            Poll::Ready(Ok(v))
        } else if inner.dropped {
            Poll::Ready(Err(()))
        } else {
            inner.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

fn futures_oneshot<T>() -> (OneshotSender<T>, OneshotReceiver<T>) {
    let inner = Rc::new(RefCell::new(OneshotInner {
        value: None,
        waker: None,
        dropped: false,
    }));
    (OneshotSender(inner.clone()), OneshotReceiver(inner))
}
