# AI Crate Decision Packet

## Research Recap

Research 04 §9.1 picks `ort` as the engine inference runtime. Research 04 §3, §4, §6, and §7 map v1 AI-adjacent features to MixFormerV2-S/YuNet/OpenCV LK, faster-whisper, and model-free DSP where applicable. Python sidecars remain JSON-over-stdin/stdout for workloads that are not cleanly native yet.

## Concrete v1 Floor

Minimum public API for tracker/caption/audio-analysis verbs to pass conformance:

- `Capability`, `Provider`, `AiError`, `ModelId`, `ModelVersion`, and `ExecutionProvider` types.
- Provider registry that reports tracker algorithms, caption models, and audio-analysis algorithms into `list_capabilities` without importing UI or transport crates.
- `run_tracker`, `run_stt`, and `run_audio_analysis` async entrypoints returning canonical data structs owned by state verbs.
- Sidecar process facade with deterministic request/response schema; no broad catch-and-ignore fallback.
- Model cache key helper that includes model id/version, source asset hash, algorithm params, and schema version.

Sizing: LOC=980, FILES=12, CALL_SITES=7, CLASS=DESIGN.

## Cargo.toml Diff Projection

```toml
[dependencies]
verbreel-types = { path = "../verbreel-types" }
verbreel-state = { path = "../verbreel-state" }
tokio.workspace = true
tracing.workspace = true
thiserror.workspace = true
serde.workspace = true
serde_json.workspace = true
sha2.workspace = true
ort = { version = "2", optional = true, features = ["load-dynamic"] }
```

Project issue: #377.

## Three-Week Skeleton Plan

| Week | Slice | Points | LOC | FILES | CALL_SITES | Class |
|---|---|---:|---:|---:|---:|---|
| 1 | Provider registry + capability snapshots | 2 | 220 | 4 | 2 | MECHANICAL |
| 1 | Model cache-key and version structs | 2 | 160 | 3 | 2 | MECHANICAL |
| 2 | ort session facade with feature-gated runtime loading | 5 | 360 | 5 | 2 | MECHANICAL |
| 2 | Python sidecar protocol wrapper | 5 | 320 | 4 | 3 | MECHANICAL |
| 3 | Tracker/caption/audio-analysis adapter smoke tests | 8 | 520 | 7 | 7 | MECHANICAL |

## Open Question

What `ort` execution-provider auto-promote order should v1 use when the caller does not pin one?

- A. CUDA, TensorRT, DirectML, CoreML, CPU: fastest high-end default, more platform variance.
- B. CoreML, DirectML, CUDA, TensorRT, CPU: laptop-friendly default, less ideal for Linux GPU hosts.
- C. CPU only unless configured: deterministic and easy to support, slower for tracker/STT workloads.
