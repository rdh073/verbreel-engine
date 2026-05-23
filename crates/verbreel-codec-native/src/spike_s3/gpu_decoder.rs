//! gpu-video Vulkan Video hardware decode (zero-copy GPU path).
//!
//! Wraps `gpu_video::{VulkanInstance, VulkanAdapter, VulkanDevice}`
//! and the `WgpuTexturesDecoder` so the harness can drive the same
//! input end-to-end and count emitted frames.
//!
//! Decoded frames materialise as `wgpu::Texture` handles in NV12 on
//! the GPU. The decode-bench drops them immediately (no readback) —
//! that is the whole point of the "zero-copy" claim: the bytes never
//! cross the PCIe bus into host memory during the throughput run.
//!
//! `gpu-video 0.4.0` requires `wgpu 29` (matches workspace). The
//! `compatible_surface: None` field on `VulkanAdapterDescriptor` is
//! the default — no winit window is required for decode-only use.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use gpu_video::{
    EncodedInputChunk, VulkanInstance,
    parameters::{DecoderParameters, VulkanAdapterDescriptor, VulkanDeviceDescriptor},
};

pub struct GpuDecoder {
    adapter_info: String,
}

impl GpuDecoder {
    /// One-time probe of the host Vulkan stack to capture the
    /// adapter description string for the bench report. Builds and
    /// drops a transient instance + adapter; no long-lived state
    /// kept on the struct because gpu-video's adapter borrows from
    /// the instance and would require self-referential trickery.
    pub fn new() -> Result<Self> {
        let instance = VulkanInstance::new().context("VulkanInstance::new")?;
        let adapter = instance
            .create_adapter(&VulkanAdapterDescriptor::default())
            .context("VulkanInstance::create_adapter (headless, no surface)")?;
        let info = format_adapter_info(adapter.info());
        Ok(Self { adapter_info: info })
    }

    /// Decode every NAL unit in `input` and return the total frame
    /// count. Frames stay on the GPU as `wgpu::Texture` handles and
    /// are dropped without readback — measures the pure
    /// Vulkan-Video-to-wgpu-texture throughput.
    ///
    /// Builds a fresh Vulkan stack each invocation so successive
    /// bench runs start from a clean device + decoder + reference
    /// picture cache.
    pub fn decode_all_to_gpu(&self, input: &Path) -> Result<u32> {
        let bytestream =
            fs::read(input).with_context(|| format!("read H.264 input {}", input.display()))?;

        let instance = VulkanInstance::new().context("VulkanInstance::new")?;
        let adapter = instance
            .create_adapter(&VulkanAdapterDescriptor::default())
            .context("VulkanInstance::create_adapter (headless, no surface)")?;
        let device = adapter
            .create_device(&VulkanDeviceDescriptor {
                wgpu_limits: wgpu::Limits {
                    max_binding_array_elements_per_shader_stage: 128,
                    max_immediate_size: 128,
                    ..Default::default()
                },
                ..Default::default()
            })
            .context("VulkanAdapter::create_device")?;

        let mut decoder = device
            .create_wgpu_textures_decoder_h264(DecoderParameters::default())
            .context("create_wgpu_textures_decoder_h264")?;

        let mut count: u32 = 0;

        // 4 KiB chunks match the gpu-video upstream examples; the H.264
        // parser is chunk-size agnostic, so this is just an I/O knob.
        for chunk in bytestream.chunks(4096) {
            let frames = decoder
                .decode(EncodedInputChunk {
                    data: chunk,
                    pts: None,
                })
                .context("WgpuTexturesDecoder::decode")?;
            count += frames.len() as u32;
            // OutputFrame<wgpu::Texture> drops here — frame backing
            // memory stays GPU-side until wgpu garbage-collects it.
        }

        let remaining = decoder.flush().context("WgpuTexturesDecoder::flush")?;
        count += remaining.len() as u32;

        Ok(count)
    }

    /// Adapter info string captured at construction.
    pub fn adapter_info(&self) -> &str {
        &self.adapter_info
    }
}

fn format_adapter_info(info: &gpu_video::capabilities::AdapterInfo) -> String {
    format!(
        "name=\"{}\" driver=\"{}\" device_type={:?} decode={} encode={}",
        info.name,
        info.driver_name,
        info.device_type,
        info.supports_decoding,
        info.supports_encoding,
    )
}
