# Changelog

All notable changes to this project are documented here.

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versioning: [SemVer](https://semver.org/). MAJOR for breaking CLI/API/output
changes, MINOR for new features, PATCH for bug fixes.

## [Unreleased]

### Added

### Changed

### Deprecated

### Removed

### Fixed

### Security

## [1.0.0] - 2026-05-31

### Added

- Production state kernel for the v1 verb surface, including project, asset,
  clip, track, effect, text, caption, audio, timeline, preview, template,
  stock, tracker, compound, render queue, font, help/schema/describe, and
  capability verbs. The v1 conformance binary now validates 121/121 fixtures.
- Event-log persistence, idempotency replay, startup reconstructor validation,
  canonical project hashing, typed IDs/colors/timestamps, and spec-backed
  project invariants for write ordering, track/clip shape, asset references,
  keyframes, effects, fades, and speed curves.
- Storage primitives for filesystem projects and content-addressed assets,
  including CAS-backed `asset.import` and lifecycle integration.
- CLI, MCP, and HTTP surfaces for `project.list`; with the `native-render`
  feature they also route `render.start` through the native runtime.
- Native render spine: composition graph executor, rsmpeg decode/encode,
  wgpu compositor, deterministic preset, byte-stable S1 render path, and the
  runtime adapter that writes encoded MP4 outputs.
- Browser preview spine: WebCodecs decode, fMP4/MSE fallback, and the
  `verbreel-wasm` preview-session bridge.
- AI provider spine: ort facade, Python sidecar protocol, provider registry,
  model cache keys, and tracker/STT/audio-analysis adapters.
- Runnable native render example plus README quickstart covering install,
  CLI/MCP/HTTP surfaces, and the render smoke artifact.
- CI gates for cargo check/test/fmt/clippy, wasm32 check, conformance, project
  board sync, and Claude auto-review.

### Changed

- Workspace version policy is now a single inherited `1.0.0` version across
  all packages.
- v1.0.0 distribution is GitHub Release CLI binary assets only. Workspace
  crates remain path-internal and are marked non-publishable for crates.io.
- PR review automation moved from PR-Agent to the Claude Code Action with a
  read-only auto-review gate.

### Fixed

- Render determinism now uses stable graph ordering and content-addressed cache
  inputs instead of hash-map iteration order or JSON identity shortcuts.
- Geometry mismatch, async WebCodecs drain/flush ordering, timestamp range
  handling, and ort runtime pre-flight failures are surfaced as explicit errors
  instead of being masked.
