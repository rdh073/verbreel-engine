# Spike S3 — zero-copy GPU decode via gpu-video

**Date:** 2026-05-24T00:00:00+07:00
**Branch:** spike/03-zero-copy-decode
**Author:** executor (Claude Code)
**Issue:** #8

## Verdict

**FAIL** per spec §11 S3.

Both pass criteria miss:
- FPS speedup gpu-video / rsmpeg = **0.30×** (bar: ≥1.30×) — gpu-video is
  ~3.3× **slower** than rsmpeg CPU decode on this 240-frame 1080p stream.
- Peak RSS reduction = **0.00%** (bar: ≥50%) — same VmPeak on both paths.

This is a clean §11 S3 fail-criterion match: "No measurable improvement
→ defer gpu-video to future optimization." It is NOT a §11 S3 instability
fail — Vulkan Video extensions worked, the decoder produced 240/240 frames
matching the rsmpeg baseline, no validation-layer crashes. The hardware
decode path runs; it just runs slower than the host CPU on this workload.

## Environment

| Item              | Value                                                          |
|-------------------|----------------------------------------------------------------|
| Host              | Ubuntu 24.04.4 LTS — kernel 6.17.0-23-generic                  |
| Rust              | rustc 1.95.0 (59807616e 2026-04-14), cargo 1.95.0              |
| Vulkan            | 1.3.275 (Instance), `VK_KHR_video_decode_h264` rev 9, `VK_KHR_video_decode_queue` rev 8 |
| GPU adapter       | NVIDIA GeForce RTX 3050 Laptop GPU (`DISCRETE_GPU`)            |
| Driver            | NVIDIA (decode=true, encode=true per gpu-video probe)          |
| Queue family      | `QUEUE_TRANSFER_BIT | QUEUE_SPARSE_BINDING_BIT | QUEUE_VIDEO_DECODE_BIT_KHR` (count=1) |
| rsmpeg path       | rsmpeg 0.18.0+ffmpeg.8.0 → libavcodec H.264 software decode    |
| gpu-video path    | gpu-video 0.4.0 → Vulkan Video H.264 → `wgpu::Texture` (NV12)  |
| wgpu              | 29 (shared workspace pin)                                      |
| Input             | `tmp/spike_s3/input.h264` (240 frames, 1080p H.264 High, no B-frames, 595,819 B Annex-B) |
| Input origin      | Spike S1 slice D deterministic preset MP4 → `ffmpeg -bsf:v h264_mp4toannexb` |
| Bench config      | 5 runs per path, alternating rsmpeg → gpu-video, warm-avg drops run 0 |

## Side-by-side

| Metric                    | rsmpeg (CPU)     | gpu-video (GPU)  | Ratio / Δ                  |
|---------------------------|------------------|------------------|----------------------------|
| Run 0 wall                | 232.67 ms        | 891.84 ms        | -                          |
| Run 1 wall                | 229.71 ms        | 765.14 ms        | -                          |
| Run 2 wall                | 228.36 ms        | 751.31 ms        | -                          |
| Run 3 wall                | 227.54 ms        | 760.96 ms        | -                          |
| Run 4 wall                | 227.21 ms        | 760.65 ms        | -                          |
| **Warm-avg wall (4 runs)**| **228.21 ms**    | **759.51 ms**    | **3.33× slower**           |
| Decoded frames per run    | 240              | 240              | sanity check passes        |
| **Warm-avg fps**          | **1051.68**      | **315.99**       | **0.30× speedup**          |
| Peak RSS (VmPeak)         | 1,199,652 kB     | 1,199,652 kB     | 0.00% reduction            |

## §11 S3 pass-criteria check

| Criterion              | Threshold | Actual     | Verdict |
|------------------------|-----------|------------|---------|
| FPS speedup            | ≥ 1.30×   | 0.30×      | **FAIL** |
| RSS reduction          | ≥ 50%     | 0.00%      | **FAIL** |
| At least one of above  | OR        | both fail  | **FAIL** |
| Determinism regression | none      | 240/240 both paths | PASS |
| No `unsafe` in spike_s3| required  | confirmed (grep audit clean) | PASS |
| Green-main preserved   | required  | `cargo check --workspace --all-targets` passes without spike feature | PASS |

## Why is gpu-video slower here?

Three hypotheses, in order of plausibility:

1. **Per-session setup amortizes poorly on a 240-frame stream.** Each
   `decode_all_to_gpu` call rebuilds a Vulkan instance + adapter + device
   + Vulkan Video session + reference picture cache. On a ~10-second
   1080p input that fixed cost is ~0.5 s; on a 10× longer stream it
   would be the same ~0.5 s amortized over 2400 frames instead of 240.
   The CPU rsmpeg path also rebuilds its codec context per run, but
   libavcodec's setup is sub-millisecond.

2. **NVIDIA driver Vulkan Video is newer than NVDEC (NVENC's decode
   counterpart) and not equally optimized.** NVIDIA exposes both
   `nvdec` (proprietary, used by FFmpeg's `h264_cuvid`) and Vulkan Video
   on the same hardware decoder block. Anecdotal reports from the
   Vulkan + Mesa community suggest NVDEC's CUDA path is 2-3× faster
   than the Vulkan Video path on the same silicon, because Vulkan
   Video adds spec-mandated synchronization that NVDEC skips.

3. **CPU decode is genuinely fast on this workload.** Coffee Lake-class
   CPUs with libavcodec's SSE4/AVX2 H.264 paths sustain >1000 fps on
   1080p H.264 — well above 24 fps real-time. The "GPU decode wins"
   intuition assumes either (a) the CPU is busy with composite/encode
   work in parallel, or (b) the stream is 4K HEVC where CPU decode
   chokes. Neither applies here.

The "zero-copy" claim (frames stay GPU-side as NV12 textures) is
TECHNICALLY confirmed — `decode_all_to_gpu` returns `wgpu::Texture`
handles and never reads back. But VmPeak doesn't reflect VRAM, so the
host-memory savings are invisible in this proxy and decoded-NV12 staging
plus driver mapping inflate both paths' VmPeak identically (~1.2 GB
each, dominated by Vulkan instance + decoder + libavcodec context).

## For Phase 2

**Recommendation: stay on rsmpeg for the production decode path.**

`gpu-video` did not clear the §11 S3 bar. Phase 2 should treat hardware
decode as an **optimization gated behind a feature flag**, not the
default. The hybrid model the spec hinted at (§12 EXIT CRITERIA #1:
rsmpeg/FFmpeg fallback path) becomes: rsmpeg is the path, gpu-video is
the (currently slower) optional accelerator.

Three follow-up directions, ranked:

1. **Re-bench on a longer stream (5 minutes at 1080p, ~7500 frames).**
   If the per-session setup is the bottleneck, this should flip the
   verdict. Cheap to run on the same harness — just swap the input.
   A 1-hour evergreen would be even better but ~7500 frames is enough
   to amortize Vulkan Video session creation.

2. **Compare against `h264_cuvid` (NVDEC) via rsmpeg.** rsmpeg already
   supports hwaccel; passing `hw_device_ctx` with CUDA backend would
   give the apples-to-apples NVIDIA hardware decode number. If NVDEC
   is 2-3× faster than gpu-video on the same GPU, that confirms
   hypothesis #2 and tells us "use NVDEC, not Vulkan Video, on NVIDIA."

3. **Concurrency test: decode + composite + encode in parallel.**
   This spike measured serial decode-only. If the production pipeline
   has the CPU doing render+encode while a GPU decoder runs alongside,
   gpu-video's wall-time may matter less than the CPU offload. Worth
   re-running with a co-running CPU-bound workload to see if the GPU
   path frees up the CPU for the other stages.

The current FAIL is empirical, not principled — Vulkan Video CAN
outperform CPU decode on the right workload (4K HEVC, long-form
content, concurrent CPU pressure). This spike's input does not exercise
those scenarios. Issue #8 should remain OPEN pending one of the three
follow-up paths above.

## Deviations from prompt

- **System ffmpeg used for Annex-B extraction.** Task pre-flight assumed
  the vendor FFmpeg build at
  `vendor/rsmpeg/tmp/ffmpeg_build/bin/ffmpeg` exists. On this host the
  vendor build is libraries-only (no `bin/`); the system FFmpeg 6.1.1
  Ubuntu package — which has `--enable-gpl --enable-libx264` and the
  `h264_mp4toannexb` BSF compiled in — was used instead. The extracted
  Annex-B is a pure bitstream copy (`-c:v copy`), so the BSF tool
  identity doesn't affect the bytes the decoder sees.

- **Both decoders return frame counts only; no YUV bytes extracted.**
  The task-prompt's `RsmpegDecoder` API spec was
  `fn next_frame(&mut self) -> Result<Option<Vec<u8>>>`. Returning
  YUV420P bytes via rsmpeg's `AVFrame::data` requires `unsafe`
  `slice::from_raw_parts` per plane (see Spike S1 slice A's decoder
  for the canonical pattern). The §11 S3 task constraint **"No
  `unsafe` in either decoder"** forbids that. The decoders both expose
  `decode_all(_to_gpu)` returning a `u32` frame count instead; the
  bench loop doesn't need YUV bytes, only the count + wall-clock. This
  also makes the comparison fair — neither path does a post-decode
  per-pixel copy. Surfacing as a deviation because the API shape changed
  vs. the task spec.

- **No `wgpu` dev-dep added.** Task step 5 hinted at adding wgpu as a
  workspace-level dev-dep "IF the decoder needs it." gpu-video already
  re-exports the wgpu types it needs from inside its public API
  (`vulkan_device.wgpu_device()` etc.); the spike harness consumes those
  re-exports without needing a direct wgpu dep on
  `verbreel-codec-native`. wgpu IS listed as an optional dep under the
  spike-s3 feature for `VulkanDeviceDescriptor::wgpu_limits`, which is
  the only direct wgpu type touched.

- **Per-run Vulkan stack rebuilt inside `decode_all_to_gpu`.** Task
  spec hinted at building the stack once in `GpuDecoder::new()` and
  reusing across runs. gpu-video's lifetime parameters make that
  awkward in safe Rust without `Box::leak`-style trickery (the adapter
  borrows from the instance, the device borrows from the adapter). The
  benchmark fairness argument also slightly favors per-run setup:
  rsmpeg rebuilds its codec context per run (because each run
  re-opens the input), so gpu-video doing the same is symmetric.
  The per-run Vulkan setup IS part of the measured wall-clock — that
  is exactly the "per-session overhead" cost the FAIL surfaces.

- **System rustc/cargo via rustup.** PATH on this host didn't include
  `~/.cargo/bin` by default; the cron-launched shell inherited a
  stripped PATH. Manually exported `PATH="$HOME/.cargo/bin:$PATH"`
  before every cargo invocation. No project-side change needed.
