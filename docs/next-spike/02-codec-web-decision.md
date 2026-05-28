# Codec Web Crate Decision Packet

## Research Recap

Research 03 §4 picks WebCodecs over binary WebSocket frames as the primary browser preview transport and fMP4/MSE as the fallback. Research 01 §6.2 keeps browser preview distinct from native render: WebCodecs is the interactive preview path, not the canonical export renderer.

## Concrete v1 Floor

Minimum public API for web preview verbs to pass conformance:

- `WebPreviewCodec`, `DecoderConfig`, `EncodedChunk`, `FrameHandle`, and `CodecWebError` types.
- `WebPreviewCodec::webcodecs()` constructor that emits the canonical wire literals for capability reporting.
- `WebPreviewCodec::mse_fmp4()` fallback constructor for browsers without WebCodecs decode support.
- Serialization surfaces for preview-session metadata only; frame bytes stay out of event logs.
- A no-op native build surface so workspace checks do not require wasm32 during normal CI.

Sizing: LOC=740, FILES=9, CALL_SITES=4, CLASS=DESIGN.

## Cargo.toml Diff Projection

```toml
[dependencies]
verbreel-ir = { path = "../verbreel-ir" }
serde.workspace = true
thiserror.workspace = true
tracing.workspace = true

[target.'cfg(target_arch = "wasm32")'.dependencies]
wasm-bindgen = "0.2"
js-sys = "0.3"
web-sys = { version = "0.3", features = ["VideoDecoder", "EncodedVideoChunk", "VideoFrame", "ReadableStream"] }
```

Project issue: #376.

## Three-Week Skeleton Plan

| Week | Slice | Points | LOC | FILES | CALL_SITES | Class |
|---|---|---:|---:|---:|---:|---|
| 1 | Stable Rust type surface + serde snapshots | 2 | 180 | 4 | 1 | MECHANICAL |
| 1 | wasm32 WebCodecs bindings behind cfg gates | 3 | 220 | 3 | 1 | MECHANICAL |
| 2 | fMP4/MSE fallback envelope and capability detector | 5 | 300 | 4 | 2 | MECHANICAL |
| 2 | Preview-session handshake integration | 5 | 260 | 4 | 4 | MECHANICAL |
| 3 | Browser smoke tests and fallback matrix docs | 3 | 180 | 4 | 2 | MECHANICAL |

## Open Question

What Safari fallback policy should v1 enforce?

- A. Require fMP4/MSE fallback for Safari: wider browser support, more codec packaging work.
- B. Preview.session returns unsupported on Safari without WebCodecs: smaller v1, worse product surface.
- C. Keep NDJSON image frames for Safari only: simplest fallback, lower playback quality and higher bandwidth.
