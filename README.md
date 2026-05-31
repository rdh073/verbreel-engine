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

## Quickstart

```bash
# Native toolchain
rustup toolchain install 1.92
rustup default 1.92

# Build every default workspace target. This path is feature-off and does not
# require FFmpeg headers.
cargo build --workspace

# Build the CLI. Add `--features native-render` when you want render.start.
cargo build --release -p verbreel-cli

# Build the MCP server. Add `--features native-render` when you want render.start.
cargo build --release -p verbreel-mcp

# Build the HTTP server. Add `--features native-render` when you want render.start.
cargo build --release -p verbreel-http

# Build the WASM preview module
cargo build --release -p verbreel-wasm --target wasm32-unknown-unknown
```

The release binaries are:

| Surface | Binary | Current v1 floor |
|---|---|---|
| CLI | `target/release/verbreel` | `project list`; `render start` in native-render builds |
| MCP | `target/release/verbreel-mcp` | MCP `tools/list`, `project.list`; `render.start` in native-render builds |
| HTTP | `target/release/verbreel-http` | `GET /healthz`, `GET /tools`, `POST /tools/project.list`; `POST /tools/render.start` in native-render builds |

Run the current surfaces directly from the workspace:

```bash
cargo run -p verbreel-cli -- project list
cargo run -p verbreel-mcp
VERBREEL_HTTP_ADDR=127.0.0.1:8080 cargo run -p verbreel-http
curl -s http://127.0.0.1:8080/tools
curl -s -X POST http://127.0.0.1:8080/tools/project.list \
  -H 'content-type: application/json' \
  -d '{}'
```

## Native Render Example

The native render spine example is the first end-to-end render walkthrough. It
creates an on-disk project, imports a generated PPM image through `asset.import`,
renames the seeded video track, adds and edits a clip, then drives the
composition-root `render.start` path into the wgpu compositor and rsmpeg MP4
encoder. The default workspace remains FFmpeg-free; `cargo run -p examples`
re-enters itself with the optional `native-render` feature when you run it.

Linux prerequisites:

```bash
sudo apt-get update
sudo apt-get install -y \
  pkg-config libclang-dev \
  libavcodec-dev libavformat-dev libavutil-dev libswscale-dev \
  mesa-vulkan-drivers
```

macOS prerequisites:

```bash
brew install ffmpeg llvm
```

Run the example:

```bash
cargo run -p examples
```

It writes:

```text
${CARGO_TARGET_DIR:-target}/verbreel-examples/native-render-spine-s1-1080p.mp4
${CARGO_TARGET_DIR:-target}/verbreel-examples/native-render-spine-s1-1080p.json
```

The JSON manifest records the verb sequence, output bytes, SHA-256, 1080p frame
count, and render fps smoke baseline. Set `VERBREEL_EXAMPLES_MIN_RENDER_FPS` to
raise the local regression floor. CLI/MCP/HTTP expose `render.start` when built
with `--features native-render`; those surfaces use the same native runtime path
as the release binary.

## Release

The v1.0.0 release uses one workspace version and GitHub Release assets for the
CLI binary. Workspace crates are intentionally not published to crates.io for
v1.0.0. See [RELEASE.md](RELEASE.md) for the distribution policy, verification
commands, and annotated tag flow.

## Spec

Full specification: https://github.com/rdh073/verbreel-spec  
Conventions (§0): `spec/commands/conventions.md`  
Research / stack decisions: `spec/research/`

## Stack

| Layer | Choice | Rationale |
|---|---|---|
| Language | Rust 1.92, edition 2024 | Dual native+WASM target, MCP SDK (rmcp), determinism |
| Render | wgpu 29 + WGSL | One shader source for Vulkan/Metal/DX12/WebGPU |
| Decode/encode | rsmpeg 0.14.2 (system FFmpeg 6.x) | HW accel (NVENC/VAAPI/VideoToolbox), hwaccel-swappable |
| Async | Tokio 1 | Work-stealing, Tokio-native axum + rmcp |
| HTTP | axum 0.8 | Tower middleware, hyper underneath |
| MCP | rmcp 0.16 | Official Anthropic-blessed Rust SDK |
| CLI | clap 4 derive | Tab-completion via clap_complete |
| Canonical JSON | vr-jcs 0.4 | RFC 8785 conformant, ships §3.5 test vectors |

## License

MIT OR Apache-2.0
