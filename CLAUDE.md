# CLAUDE.md — verbreel-engine

## Project
Verbreel engine — Rust implementation of the Verbreel spec.
Spec lives at: ~/playground/verbreel-spec/spec/ (or https://github.com/rdh073/verbreel-spec)
This repo: ~/playground/verbreel/

## Quick reference
- Spec conventions: spec/commands/conventions.md (§0.1–§0.18)
- Research stack: spec/research/01–07

## Architecture rules
- Every mutation goes through `verbreel_state::engine::apply()` — never mutate state directly
- Event log (`events.jsonl`) written BEFORE in-memory patch applied (§0.8 write ordering)
- Content-addressed assets: `assets/<aa>/<sha256>.<ext>` — never copy without hashing first
- Canonical JSON for hashing: `verbreel_canon::jcs::canonicalize()` — never `serde_json::to_string()`

## Crate dependency rule
```
verbreel-types → (no internal deps)
verbreel-args  → verbreel-types
verbreel-canon → verbreel-types
verbreel-events → verbreel-types, verbreel-canon
verbreel-state  → verbreel-types, verbreel-args, verbreel-events
verbreel-ir     → verbreel-types
verbreel-render → verbreel-ir, verbreel-codec-native
verbreel-codec-native → verbreel-ir
verbreel-codec-web    → verbreel-ir  (wasm32-only)
verbreel-storage → verbreel-types, verbreel-events
verbreel-ai      → verbreel-types, verbreel-state
verbreel-runtime → verbreel-state, verbreel-storage, verbreel-canon, verbreel-ir, verbreel-render
verbreel-cli     → verbreel-state, verbreel-storage (+ verbreel-runtime behind native-render)
verbreel-mcp     → verbreel-state, verbreel-storage (+ verbreel-runtime behind native-render)
verbreel-http    → verbreel-state, verbreel-storage (+ verbreel-runtime behind native-render)
verbreel-wasm    → verbreel-state, verbreel-ir, verbreel-codec-web  (wasm32-only)
```

No cycles. Never add a dep that would create a cycle.

## Commit conventions
`<crate>: <imperative summary>` e.g. `state: implement §0.13 asset-path invariant`
Areas: types / args / canon / events / state / ir / render / codec / storage / ai / cli / mcp / http / wasm / ci / spec

## Do NOT
- Break the write ordering (§0.8): apply() must write event BEFORE patching in-memory state
- Skip the JCS canonicalize step when computing project_hash
- Use `serde_json::to_string_pretty()` for project.json serialization (use vr-jcs)
- Add `unsafe` to verbreel-state, verbreel-events, verbreel-canon without a written justification
- Use `std::collections::HashMap` for ordered-key-required structures (use IndexMap)

## Spike goals (Phase 1, branch spike/S1–S3)
- S1: rsmpeg decode → wgpu YUV→RGB → composite → libx264 encode, 10s 1080p, ≥60fps, deterministic
- S2: shared WGSL native vs wasm32, pixel diff ≤1/255 per channel
- S3: gpu-video (Vulkan Video) zero-copy decode, ≥30% throughput improvement

## Project board protocol
- Board Status is auto-projected from PR/issue lifecycle by `board-sync.yml` — do NOT manually move the board. Mapping: issue assigned / draft PR open → In progress; PR ready-for-review → In review; PR merged → Done; PR closed unmerged → In progress.
- `.github/scripts/board-move.sh N "<Status>"` is a manual override for exceptions only. board-sync is not a gate and never blocks a PR.
- Always land changes via PR, not close-via-commit-message.
- Estimate must be set on the Backlog entry; no Estimate = 0 is allowed except for sub-issues that explicitly link to a parent estimate.
