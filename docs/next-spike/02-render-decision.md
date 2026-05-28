# Render Crate Decision Packet

## Research Recap

Research 01 §7 picks `wgpu + rsmpeg` for the render engine and shares WGSL shader semantics between native and browser builds. Research 02 §6 keeps this inside the Rust workspace dependency graph. The accepted path is native FFmpeg via rsmpeg for decode/encode and wgpu for compositing/effects; ffmpeg.wasm, headless Chrome, GStreamer, and MLT stay rejected as core engine paths.

## Concrete v1 Floor

Minimum public API for render-dependent verbs to pass conformance:

- `RenderPreset`, `RenderJobSpec`, `RenderJobId`, `RenderStatus`, and `RenderError` type surface.
- `list_presets() -> &[RenderPreset]` with the eight preset ids currently surfaced by `render.list_presets` and `list_capabilities`.
- `start_render(spec) -> Result<RenderJobId, RenderError>` and `status/cancel(job_id)` interfaces that the state crate can call without bypassing `verbreel_state::engine::apply()`.
- Deterministic-mode knob that pins shader path, frame rounding, color transform, and encoder params for conformance tests.
- No state mutation inside `verbreel-render`; event-log mutation remains owned by `verbreel-state`.

Sizing: LOC=1150, FILES=14, CALL_SITES=6, CLASS=DESIGN.

## Cargo.toml Diff Projection

```toml
[dependencies]
verbreel-ir = { path = "../verbreel-ir" }
verbreel-types = { path = "../verbreel-types" }
verbreel-codec-native = { path = "../verbreel-codec-native" }
wgpu.workspace = true
tracing.workspace = true
thiserror.workspace = true
serde.workspace = true
serde_json.workspace = true
bytemuck = { version = "1", features = ["derive"] }
rsmpeg = { workspace = true, optional = true }
```

Project issue: #375.

## Three-Week Skeleton Plan

| Week | Slice | Points | LOC | FILES | CALL_SITES | Class |
|---|---|---:|---:|---:|---:|---|
| 1 | Preset registry + public status/job structs | 2 | 220 | 4 | 1 | MECHANICAL |
| 1 | IR-to-render input adapter with deterministic frame windowing | 3 | 260 | 4 | 2 | MECHANICAL |
| 2 | wgpu pipeline bootstrap + WGSL shader module loading | 5 | 360 | 5 | 1 | MECHANICAL |
| 2 | rsmpeg encode/decode facade behind feature gate | 5 | 420 | 5 | 2 | MECHANICAL |
| 3 | End-to-end render smoke harness + state crate integration hook | 8 | 540 | 7 | 6 | MECHANICAL |

## Open Question

Which hardware acceleration priority should v1 use for encode/decode selection?

- A. NVENC first, then VAAPI, then VideoToolbox: best for Linux/NVIDIA CI hosts, less native on macOS.
- B. VideoToolbox first, then VAAPI, then NVENC: best for Apple laptops, weaker Linux default.
- C. VAAPI first, then VideoToolbox, then NVENC: best open-driver default, weaker discrete-GPU throughput.
