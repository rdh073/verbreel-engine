# Verbreel Engine

Verb-driven, agent-native video editor engine.

## What this is

The reference implementation of the [Verbreel spec](https://github.com/rdh073/verbreel-spec).
Exposes three interfaces — CLI, MCP server, HTTP server — all backed by one deterministic
Rust engine.

## Architecture

```
verbreel/
├── crates/
│   ├── verbreel-types/          # shared serde types, no logic
│   ├── verbreel-args/           # per-verb args schemas + canonical JSON validator
│   ├── verbreel-canon/          # RFC 8785 JCS + project_hash (spec §0.5.2)
│   ├── verbreel-events/         # events.jsonl writer/reader, idempotency (spec §0.8)
│   ├── verbreel-state/          # project graph, §0.13 invariants, apply()/reconstructor
│   ├── verbreel-ir/             # composition IR, tick-rate math, cache_hash
│   ├── verbreel-render/         # wgpu pipelines, WGSL shaders (spec §13)
│   ├── verbreel-codec-native/   # rsmpeg decode/encode + hwaccel
│   ├── verbreel-codec-web/      # WebCodecs shim (wasm32-only)
│   ├── verbreel-storage/        # filesystem CAS, OPFS shim (spec §3, App. D)
│   ├── verbreel-ai/             # ort runtime, Python sidecar dispatch (spec §18, §19)
│   ├── verbreel-cli/            # `verbreel` binary, clap derive
│   ├── verbreel-mcp/            # `verbreel-mcp` MCP stdio server (rmcp)
│   ├── verbreel-http/           # `verbreel-http` HTTP server (axum 0.8)
│   └── verbreel-wasm/           # wasm-bindgen crate for browser preview
└── shaders/                     # WGSL shaders — one source for native + web
```

## Getting started

```bash
# Build the CLI (native)
cargo build --release -p verbreel-cli

# Build the MCP server
cargo build --release -p verbreel-mcp

# Build the HTTP server
cargo build --release -p verbreel-http

# Build the WASM preview module
cargo build --release -p verbreel-wasm --target wasm32-unknown-unknown
```

## Spec

Full specification: https://github.com/rdh073/verbreel-spec  
Conventions (§0): `spec/commands/conventions.md`  
Research / stack decisions: `spec/research/`

## Stack

| Layer | Choice | Rationale |
|---|---|---|
| Language | Rust 1.85, edition 2024 | Dual native+WASM target, MCP SDK (rmcp), determinism |
| Render | wgpu 29 + WGSL | One shader source for Vulkan/Metal/DX12/WebGPU |
| Decode/encode | rsmpeg 0.18 (FFmpeg 8) | HW accel (NVENC/VAAPI/VideoToolbox), hwaccel-swappable |
| Async | Tokio 1 | Work-stealing, Tokio-native axum + rmcp |
| HTTP | axum 0.8 | Tower middleware, hyper underneath |
| MCP | rmcp 0.16 | Official Anthropic-blessed Rust SDK |
| CLI | clap 4 derive | Tab-completion via clap_complete |
| Canonical JSON | vr-jcs 0.4 | RFC 8785 conformant, ships §3.5 test vectors |

## License

MIT OR Apache-2.0
