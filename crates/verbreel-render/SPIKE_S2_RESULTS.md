# Spike S2 — cross-target pixel diff

**Date:** 2026-05-23T22:53:39Z
**Branch:** spike/02-shared-shader
**Author:** executor (Claude Code) + human (manual Chrome step)
**Issue:** #7

## Verdict

**PASS** — 100% of 2,073,600 pixels are byte-identical between the
native Vulkan/Naga path (NVIDIA RTX 3050 Mobile) and Chrome WebGPU.
Max per-channel delta = 0. The two output PNGs are also byte-identical
files (same SHA-256), which proves both the GPU output AND the PNG
encoding round-trip cleanly across targets.

Spec §11 S2 pass criteria are met with no drift to spare — this is
the strongest possible outcome on a multiply-blend op.

## Setup

| Item              | Native side                                | Wasm side                                                          |
|-------------------|--------------------------------------------|--------------------------------------------------------------------|
| Adapter           | NVIDIA GeForce RTX 3050 Laptop GPU         | Chrome 148.0.7778.167 (Official Build) (64-bit), WebGPU            |
| Backend           | Vulkan                                     | BrowserWebGpu (Chrome's Tint→SPIR-V→Vulkan/Mesa pipeline)          |
| wgpu version      | 29.0.3                                     | 29.0.3 (same crate, target_arch = wasm32)                          |
| Shader            | `crates/verbreel-render/shaders/spike_s2/multiply_blend.wgsl` (shared `&str`) | identical bytes |
| WGSL compiler     | Naga                                       | Tint (in Chrome) → SPIR-V → host driver                            |
| Input A           | 1920×1080 horizontal gradient (R=x/W, G=0.5, B=1−x/W) | same bytes (`include_bytes!` from `web/input_a.png`) |
| Input B           | 1920×1080 vertical gradient (R=0.5, G=y/H, B=1−y/H) | same bytes (`include_bytes!` from `web/input_b.png`) |
| Output format     | rgba8unorm storage texture                  | rgba8unorm storage texture                                         |
| PNG encoder       | `png` crate 0.18 with `Compression::Balanced` | same `png` crate compiled to wasm                                |

Input PNGs (used by both sides):
- `tmp/spike_s2/input_a.png` → `c11d…` (SHA-256 captured during sync)
- `tmp/spike_s2/input_b.png` → `34e7…`

Output PNGs:
- `tmp/spike_s2/native_frame.png` → SHA-256 `25af2ca17df346cc9aee52818108548c85844f9686f4c57e0c5f414ab507b624`
- `tmp/spike_s2/wasm_frame.png`   → SHA-256 `25af2ca17df346cc9aee52818108548c85844f9686f4c57e0c5f414ab507b624`
- **Byte-identical** (`cmp` returned 0).

## Diff numbers (verbatim from `tmp/spike_s2/diff_report.txt`)

```
SPIKE S2 — Cross-target pixel diff
Native:  tmp/spike_s2/native_frame.png (1920×1080)
Wasm:    tmp/spike_s2/wasm_frame.png

Pixels total:               2073600
Pixels with any |Δ| > 0:    0 (0.0000%)
Pixels with any |Δ| > 1:    0 (0.0000%)
Pixels with any |Δ| > 2:    0 (0.0000%)
Pixels clean (no drift):    100.0000%
Pixels within ≤1 tolerance: 100.0000%
Pixels within ≤2 tolerance: 100.0000%

Per-channel |Δ| > 1 counts:
  |ΔR| > 1: 0
  |ΔG| > 1: 0
  |ΔB| > 1: 0

Max channel deltas:
  |ΔR| max: 0
  |ΔG| max: 0
  |ΔB| max: 0
  per-pixel max: 0

§11 S2 Pass criteria:
  ≥99.9% pixels within ≤1:  100.0000%  → PASS
  max per-pixel ≤2:         0     → PASS

Verdict: PASS
```

## §11 S2 Pass criteria

| Criterion               | Threshold | Actual         | Verdict |
|-------------------------|-----------|----------------|---------|
| Pixels with any \|Δ\| ≤ 1 | ≥ 99.9%  | **100.0000%**  | **PASS** |
| Max per-pixel \|Δ\|       | ≤ 2      | **0**          | **PASS** |

## Why is the agreement total?

Three reasons stack:

1. **Multiply-blend in 8-bit linear has no FMA freedom.** Once both
   inputs are quantized to 8 bits (sampled from `rgba8unorm` textures,
   so they reach the shader as exact `f32` representations of `i/255`),
   the per-channel multiply is a single fp32 op with no fusion choice
   for the compiler. The result is then quantized to 8 bits again on
   `textureStore` to `rgba8unorm`, which the WebGPU spec defines as
   round-to-nearest-even.
2. **Both sides compile to the same Vulkan driver class.** Native goes
   Naga → SPIR-V → NVIDIA driver. Chrome on this Linux host goes
   Tint → SPIR-V → NVIDIA driver (Chrome's WebGPU backend on Linux
   picks Vulkan, not OpenGL/ANGLE). Different WGSL frontends but the
   same SPIR-V execution path for this opcode.
3. **PNG encoding is fully deterministic with the `png` crate's
   `Balanced` compression mode.** Same input bytes → same output bytes
   on both targets. (This is why the wrapper PNGs round-trip too, not
   just the raw RGBA.)

The S1.5 finding ("cross-GPU drift expected from FMA fusion") does
NOT apply here because the multiply-blend op has no FMA opportunity
— there's no add-after-multiply that the compiler could fuse.
Spec §11 S2 chose the op well.

## For Phase 2 design

This result supports the strongest interpretation of §12 EXIT CRITERIA
#2: **wasm32 builds can deliver preview-grade output, not just
best-effort screenshots**, at least for shaders in this class
(no-FMA-op + rgba8unorm I/O). Phase 2 architecture should NOT default
to "force native preview" — it should default to the shared-WGSL
path and only fall back to native rendering when a specific op
exercises FMA or higher-precision intermediate storage (rgba16float
storage targets, for example, will need re-verification).

The §11 S2 spike question — "does Chrome WebGPU produce byte-comparable
pixels to native Vulkan with the same WGSL?" — answered: yes,
unconditionally, for this shader. Recommend extending the harness
in Phase 2 with at least one op that DOES have FMA freedom
(`textureSampleLevel` + linear filter + linear blend) before the spec
generalizes the result.

## Deviations from prompt

- **Port 8000 → 8001** — port 8000 was already bound on this host by
  an SSH tunnel (`ss -tlnp` showed `ssh,pid=2205032,fd=4` listening).
  Updated `serve.sh` to take a `PORT` env var defaulting to 8001.
- **`png 0.18` API differences** — the task example used
  `Compression::Default` (renamed to `Balanced` in 0.18) and
  `Decoder::new(&[u8])` which needs `BufRead + Seek` since 0.18.
  Wrapped input in `std::io::Cursor` and switched the constant to
  `Balanced`. Same change in `synth.rs`, `native.rs`, `wasm.rs`.
- **`wgpu 29 PollType::Wait`** is a struct variant — used
  `PollType::wait_indefinitely()` like spike S1 does.
- **`web-sys` `console` feature was missing** — first wasm-pack build
  failed with "cannot find `console` in `web_sys`". Added it to the
  feature list.
- **`wasm-pack` not preinstalled** — installed via
  `cargo install wasm-pack --locked` (v0.15.0). Took ~50 s.
- **Native runner uses `PowerPreference::HighPerformance`** per the
  task literal — picks NVIDIA RTX 3050 on this dual-GPU host. The
  comparison is effectively "NVIDIA Vulkan via Naga" vs "Chrome
  WebGPU via Tint → NVIDIA Vulkan", so both sides hit the same
  driver. A future S2 follow-up could force iGPU on the native side
  to widen the cross-stack coverage.
- **Crate root `pkg/` left out of commit** — it's a wasm-pack output
  artifact; the served copy in `web/pkg/` is the committed one.
