# WASM Crate Decision Packet

## Research Recap

Research 02 §7 requires the same Rust source to support native and WASM builds. Research 05 §7 defines the browser persistence story around OPFS-like semantics, while Research 03 §4 keeps browser preview on WebCodecs/fMP4 transports. The WASM crate is the adapter boundary, not a second engine implementation.

## Concrete v1 Floor

Minimum public API for browser surfaces to pass conformance:

- `EngineHandle`, `WasmError`, `ProjectHandle`, and `PreviewSessionHandle` exported through wasm-bindgen.
- Thin `apply_json(command_json) -> Result<JsValue, WasmError>` bridge that routes into `verbreel-state` with native features disabled.
- Preview-session bridge to `verbreel-codec-web`; frame bytes are streamed outside event logs.
- Panic hook and tracing bridge for developer diagnostics; no telemetry by default.
- Storage adapter trait placeholder for OPFS without implementing native fs4 locks in wasm32.

Sizing: LOC=680, FILES=8, CALL_SITES=5, CLASS=DESIGN.

## Cargo.toml Diff Projection

```toml
[dependencies]
verbreel-state = { path = "../verbreel-state", default-features = false }
verbreel-ir = { path = "../verbreel-ir" }
verbreel-codec-web = { path = "../verbreel-codec-web" }
tracing.workspace = true
thiserror.workspace = true
serde.workspace = true
serde_json.workspace = true
wasm-bindgen = "0.2"
js-sys = "0.3"
console_error_panic_hook = "0.1"
getrandom = { version = "0.3", features = ["wasm_js"] }
```

Project issue: #378.

## Three-Week Skeleton Plan

| Week | Slice | Points | LOC | FILES | CALL_SITES | Class |
|---|---|---:|---:|---:|---:|---|
| 1 | wasm-bindgen error/result surface | 2 | 160 | 3 | 1 | MECHANICAL |
| 1 | `apply_json` state bridge with native features disabled | 3 | 220 | 3 | 3 | MECHANICAL |
| 2 | Preview session handle + codec-web handoff | 5 | 280 | 4 | 4 | MECHANICAL |
| 2 | OPFS adapter trait and in-memory test backend | 5 | 320 | 5 | 2 | MECHANICAL |
| 3 | wasm-pack smoke harness + browser fixture | 5 | 260 | 5 | 5 | MECHANICAL |

## Open Question

Should v1 WASM embed only preview or the full editor command surface?

- A. Preview-only embedding: smallest browser payload, no full editor offline flow.
- B. Editor-also embedding: matches native command surface, more OPFS and lock-emulation work.
- C. Preview now with explicit `editor-preview` feature gate: incremental payload, one extra feature matrix to maintain.
