# Next Spike Summary

## Verb Coverage

- v1.1 conformance coverage: 121 / 121 = 100.0%.
- Raw command-header audit: 128 headers, 127 unique verb ids.
- Registry/conformance delta: six native project lifecycle modules exist with tests but are outside `default_registry()` and conformance; they are folded into #379.
- Conformance gate result: `cargo run -p verbreel-conformance` => `conformance: PASS — 121 verbs, 121 fixtures`.

## Crate Completion

| Crate | LOC | Status | Completion |
|---|---:|---|---:|
| verbreel-state | 60,338 | DONE (121 of 121 verbs apply; lifecycle registry cleanup queued in #379) | 100% |
| verbreel-events | 682 | DONE | 100% |
| verbreel-types | 605 | DONE | 100% |
| verbreel-canon | 498 | DONE | 100% |
| verbreel-storage | 433 | DONE (CAS + flock + layout; asset.import wiring queued in #381) | 100% |
| verbreel-args | 353 | DONE (next schema integration queued in #380) | 100% |
| verbreel-codec-native | 324 | SKELETON | 20% |
| verbreel-mcp | 311 | DONE (#343 deferred duplex harness slice) | 95% |
| verbreel-http | 309 | DONE | 100% |
| verbreel-cli | 207 | DONE | 100% |
| verbreel-ir | 200 | PARTIAL (node_id + cache_key only, no graph executor) | 40% |
| verbreel-conformance | 168 | DONE (CI gate live) | 100% |
| verbreel-ai | 1 | STUB; decision issue #377 | 5% |
| verbreel-codec-web | 1 | STUB; decision issue #376 | 5% |
| verbreel-render | 1 | STUB; decision issue #375 | 5% |
| verbreel-wasm | 1 | STUB; decision issue #378 | 5% |

## Weighted Progress

| Bucket | Weight | Completion | Weighted points |
|---|---:|---:|---:|
| spec & schema | 10 | 100% | 10.00 |
| verb contracts | 30 | 100% | 30.00 |
| engine state machine | 20 | 92% | 18.40 |
| surfaces (CLI/MCP/HTTP) | 10 | 90% | 9.00 |
| storage+events+canon | 10 | 88% | 8.80 |
| render | 8 | 15% | 1.20 |
| codec | 6 | 30% | 1.80 |
| IR/cache | 3 | 40% | 1.20 |
| AI/tracker | 2 | 20% | 0.40 |
| wasm | 1 | 15% | 0.15 |
| Total | 100 | | 80.95% |

## Issue Sync

- Issues created in this spike: 12, range #375-#386.
- Project #3 default page count after sync: 30; full `--limit 1000` item count: 174.
- New Backlog issues: #375, #376, #377, #378, #379, #380, #381, #382, #383, #384, #385. PR tracking issue #386 is In Review.
- `Status=Backlog` and numeric `Estimate` were set through ProjectV2 `item-edit` for each new item.

## Class Counts

- MECHANICAL actionable items: 28.
- DESIGN items: 5.
- BLOCKED items: 1.

## DESIGN Items

- #375 render hwaccel matrix: A. NVENC -> VAAPI -> VideoToolbox; B. VideoToolbox -> VAAPI -> NVENC; C. VAAPI -> VideoToolbox -> NVENC.
- #376 codec-web Safari fallback: A. fMP4/MSE fallback; B. unsupported without WebCodecs; C. NDJSON image frames for Safari only.
- #377 ai ort auto-promote: A. CUDA -> TensorRT -> DirectML -> CoreML -> CPU; B. CoreML -> DirectML -> CUDA -> TensorRT -> CPU; C. CPU only unless configured.
- #378 wasm embedding scope: A. preview-only; B. editor-also; C. preview now with `editor-preview` feature gate.
- #385 font registry: A. system `fontconfig`/`fontdb`; B. bundled fonts only; C. static lookup table.

## BLOCKED Items

- #343 mcp duplex harness slice remains upstream for end-to-end `ServerHandler` trait integration coverage; it is not blocking the Backlog population done in this spike.

## Files Emitted

- `docs/next-spike/01-coverage.md`
- `docs/next-spike/02-render-decision.md`
- `docs/next-spike/02-codec-web-decision.md`
- `docs/next-spike/02-ai-decision.md`
- `docs/next-spike/02-wasm-decision.md`
- `docs/next-spike/03-tier-12-residual.md`
- `docs/next-spike/00-summary.md`
- `.github/project-meta.json`
- `.github/scripts/board-add.sh`
- `.github/scripts/board-move.sh`
- `.github/workflows/board-gate.yml`
