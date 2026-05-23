# Spike S1.5 — Intel UHD iGPU sign-off

**Date:** 2026-05-23T22:24:13Z
**Branch:** spike/01-igpu-signoff
**Builds on:** slice D (c4a095c on spike/01-native-render, cherry-picked)
**Author:** executor (Claude Code)
**Issue:** #16
**Host:** 13th Gen Intel Core i5-13420H (12 threads), Intel UHD iGPU + RTX 3050 Mobile dGPU, FFmpeg 8 vendor build + libx264 r3108

## Verdict

**PASS** — both presets remained byte-deterministic on the integrated GPU
(`Intel(R) Graphics (RPL-P)`, Mesa 25.2.8), and the performance preset
cleared the §11.2 60 fps bar by 1.45×. The §11.2 spec note ("Final §11.2
sign-off requires an iGPU run") is now satisfied on this host.

## Adapter used

`name="Intel(R) Graphics (RPL-P)" vendor=0x8086 device=0xa7a8 type=IntegratedGpu backend=Vulkan driver="Intel open-source Mesa driver" driver_info="Mesa 25.2.8-0ubuntu0.24.04.1"`

Selection mechanism: `instance.enumerate_adapters(wgpu::Backends::VULKAN)`
filtered by `device_type == DeviceType::IntegratedGpu`. wgpu's
`PowerPreference::LowPower` was deliberately NOT used (per the new
`AdapterMode` doc-comment, it's unreliable on Linux NVIDIA hybrid hosts —
wgpu#3464).

## Side-by-side table (iGPU only — for dGPU comparison, see SPIKE_S1D_RESULTS.md)

| Metric            | Deterministic (iGPU) | Performance (iGPU) | dGPU reference (slice D) |
|-------------------|----------------------|--------------------|--------------------------|
| Unique SHA-256    | **1**                | **1**              | 1 / 1                    |
| First hash        | `4c9c8713…2086`      | `a063a667…2b18`    | `dc6ecd6d…1749` / `7a1d52c2…ed72` |
| Warm avg wall     | 4.69 s               | 2.76 s             | 4.19 s / 2.11 s          |
| Warm avg fps      | **51.17**            | **86.84**          | 57.24 / 113.82           |
| 60 fps bar (§11.2)| MISS (14.7%)         | **MET (1.45×)**    | MISS / MET (1.90×)       |
| Determinism (§11.1)| **PASS**            | **PASS**           | PASS / PASS              |
| Cold run 0 fps    | 54.74                | 85.25              | 57.01 / 96.73            |
| GPU init (run 0)  | 1.62 s               | 96.12 ms           | 2.01 s / 242 ms          |
| Encoder init      | 1.14 ms              | 1.77 ms            | 3.60 ms / 2.04 ms        |
| Speedup (det → perf) | 1.000×            | 1.697×             | 1.000× / 1.988×          |

## Per-stage warm avg (runs 1–9, 240 frames per run, iGPU)

### Deterministic preset (wall 4.69 s)
| Stage   | Time     | % of wall | Note |
|---------|----------|-----------|------|
| submit  | 291.46 ms |   6.2%   | CPU upload prep |
| collect | 560.60 ms |  12.0%   | GPU sync + readback (iGPU shared memory — comparable to dGPU PCIe in absolute terms) |
| encode  | 3.84 s    |  81.8%   | libx264 dominates (same as dGPU) |

### Performance preset (wall 2.76 s)
| Stage   | Time     | % of wall | Note |
|---------|----------|-----------|------|
| submit  | 395.61 ms |  14.3%   | iGPU upload share rose because total wall halved, not because absolute submit got slower |
| collect | 1.01 s    |  36.6%   | iGPU compute is the bottleneck once encoder is multi-threaded |
| encode  | 1.36 s    |  49.1%   | encoder no longer dominates |

## Cross-GPU SHA delta

- iGPU deterministic hash (`4c9c8713…2086`) **DOES NOT** match dGPU
  deterministic hash (`dc6ecd6d…1749`). This is **expected and
  acceptable** per spec §11.1 — byte-identity is required on the SAME
  host only. Cross-GPU drift comes from:
  - Different SPIR-V codegen between Intel Mesa (RADV-style) and NVIDIA
    proprietary driver
  - Different IEEE-754 fused-multiply-add fusion choices in BT.709
    matrix multiplication on Iris Xe (gen12.2) vs Ampere shading cores
  - Different texture-sampler rounding at chroma upsampling boundaries
- Within each GPU, all 10 runs produced ONE SHA — the on-host
  determinism guarantee holds for both adapters.

## §11.2 iGPU sign-off

Confirmed: spec §11.2 wording stands as-is. The performance preset on
this host's Intel UHD iGPU produces **86.84 fps warm avg**, comfortably
above the 60 fps target. Combined with the slice D dGPU result of
113.82 fps, the §11.2 target is met on both adapter classes — no spec
revision required.

The deterministic preset's 51.17 fps on iGPU misses the 60 fps mark
but that's by design — §11.1 has no perf target; the deterministic
preset is the export-grade reproducibility tool, not the daily-driver
path. Slice D's 57.24 fps on the dGPU was a stronger signal toward
"the deterministic preset is encoder-CPU-bound regardless of GPU
class" — the iGPU run confirms it.

## Deviations from prompt

- **`AdapterMode` not re-exported from `spike_s1/mod.rs`** — the task's
  step 7 hinted at it but the MAY-touch list did NOT include `mod.rs`.
  Used the explicit path `verbreel_render::spike_s1::gpu::AdapterMode`
  everywhere, parallel to slice D's `…::encoder::EncoderPreset`.
- **`enumerate_adapters` wrapped in `pollster::block_on`** — wgpu 29's
  API returns `impl Future<Output = Vec<Adapter>>`, not a sync `Vec`.
  Same pattern as the existing `request_adapter` call sites.
- **`GpuRoundtrip::new` left unchanged** — task explicitly said not to.
  Only `PipelinedGpu::new` got the `AdapterMode` parameter.
- **Issue created with `--label "render"` only** — `spike,phase:1,codec`
  labels did not exist on the repo; only `render` exists. Did not create
  new labels (out of scope).
