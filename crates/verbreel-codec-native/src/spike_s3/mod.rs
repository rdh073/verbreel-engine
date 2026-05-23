//! Spike S3 — zero-copy GPU decode comparison (spec §11 S3).
//!
//! Two parallel H.264 decoders are exposed for side-by-side benchmarking:
//!
//! - [`rsmpeg_decoder::RsmpegDecoder`] — CPU decode via libavcodec
//!   (software baseline; same path as Spike S1 slice A, re-implemented
//!   here standalone because slice A's source lives on a separate spike
//!   branch and the S3 task HARD LOCK forbids touching `src/spike_s1/`).
//!
//! - [`gpu_decoder::GpuDecoder`] — Vulkan Video hardware decode via the
//!   `gpu-video` crate. Decoded frames land directly in `wgpu::Texture`
//!   handles on the GPU; nothing is read back to CPU during the
//!   throughput run.
//!
//! Both decoders expose the same shape: ingest the entire H.264 Annex-B
//! input file, return the total frame count. The `spike_s3_decode_bench`
//! example drives them and computes warm-avg fps + peak RSS per §11 S3
//! pass criteria.
//!
//! Encoding is out of scope for this spike — re-use Spike S1 slice A's
//! encoder if you need an end-to-end transcode demo.

pub mod gpu_decoder;
pub mod harness;
pub mod rsmpeg_decoder;
