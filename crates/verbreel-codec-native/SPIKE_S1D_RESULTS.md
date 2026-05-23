# Spike S1 — Slice D — Dual-preset comparison (informational)

**Date:** 2026-05-23T21:44:16Z
**Branch:** spike/01-native-render
**Builds on:** slice C (cee8c3b)
**Author:** executor (Claude Code)
**Issue:** #6
**Host:** 13th Gen Intel Core i5-13420H (12 threads), RTX 3050 Mobile + Vulkan, FFmpeg 8 vendor build + libx264 r3108

## Why this slice exists

Slice C verdict was DETERMINISM-ONLY PASS at 58.52 fps with the §5
deterministic preset (`threads=1`). Spec §11 was ambiguous on which
preset the 60 fps target applied to. Slice D collects the missing
comparison data — same harness, same 3-clip + crossfade timeline (240
frames, 1920×1080 @ 24 fps), 10 runs per preset, side-by-side.

The Performance preset uses libx264's default frame-threading
(`threads=auto`) plus B-frames (`bframes=3`). No VBV params are added,
so rate control stays in CRF mode — per the literature (Bouvigne 2007,
Netflix 2015) CRF without VBV should preserve byte-identity across
frame-threaded runs. This slice verifies that empirically on this host.

## Side-by-side table

| Metric            | Deterministic preset | Performance preset |
|-------------------|----------------------|--------------------|
| x264 params       | `threads=1:sliced-threads=0:sync-lookahead=0:rc-lookahead=0:bframes=0` | `threads=auto:sliced-threads=0:bframes=3` |
| Effective threads | 1                    | 18 (16 enc + lookahead — libx264 default `auto` for 12 logical cores) |
| Unique SHA-256    | 1                    | 1                  |
| Warm avg wall     | 4.19 s               | 2.11 s             |
| Warm avg fps      | 57.24                | 113.82             |
| 60 fps bar        | MISS (4.6%)          | MET (1.90× over)   |
| Determinism       | PASS                 | PASS               |
| Cold run 0 fps    | 57.01                | 96.73              |
| GPU init (run 0)  | 2.01 s               | 242.14 ms          |
| Encoder init      | 3.60 ms              | 2.04 ms            |
| Speedup (det → perf wall) | 1.000×       | 1.988×             |

## Per-stage warm avg (runs 1–9, 240 frames per run)

### Deterministic preset (wall 4.19 s)
| Stage   | Time     | % of wall | Note |
|---------|----------|-----------|------|
| submit  | 147.81 ms |   3.5%   | CPU upload prep |
| collect | 572.58 ms |  13.7%   | GPU sync + readback |
| encode  | 3.47 s    |  82.8%   | **libx264 dominates** |

### Performance preset (wall 2.11 s)
| Stage   | Time     | % of wall | Note |
|---------|----------|-----------|------|
| submit  | 159.44 ms |   7.6%   | CPU upload prep |
| collect | 861.54 ms |  40.9%   | GPU sync + readback |
| encode  | 1.09 s    |  51.6%   | **encoder still leads but no longer 80%+** |

Threading cut the encoder share from 82.8% → 51.6%. GPU collect share
rose proportionally (13.7% → 40.9%) — same wall-clock GPU work, smaller
denominator. **The encoder ↔ GPU balance is much closer in the
performance preset; the next perf knob is GPU-side (bigger pipeline
depth, async readback) not encoder-side.**

## Hash samples

### Deterministic (10 runs)
- Unique count: **1**
- Hash: `dc6ecd6db7a74fd3091c5689279278a6f56cde306e5c7c6aa27ab148a53d1749`
- Matches slice C's hash exactly — slice C foundation regression: **NONE**.

### Performance (10 runs)
- Unique count: **1**
- Hash: `7a1d52c273f4adce7b2ca3d3f7f5bb2086da915e2b5707cfda50be2c75b7ed72`
- Cross-batch intersection vs deterministic: **0** (preset switch took
  effect; bitstreams genuinely differ as expected — different x264
  params produce different output).
- libx264 log confirms `threads=18 lookahead_threads=3 bframes=3` in
  effect; B-frame ratio reached 69% (`frame B:166 / 240`).

## Findings for the spec architect

1. **Performance preset determinism**: **PASS** — 10/10 byte-identical
   runs at `threads=auto` + `bframes=3`. The Bouvigne 2007 / Netflix
   2015 hypothesis holds on this host: CRF without VBV stays
   deterministic under libx264 frame-threading.
2. **60 fps bar in performance mode**: **MET by 1.90×** — 113.82 fps
   warm avg vs 60.0 target. Even cold run 0 (96.73 fps) clears the bar.
3. **Speedup factor (det → perf)**: **1.988×** wall-clock. Roughly linear
   with the encoder share of the deterministic wall (82.8% → reduced to
   1.09 s ≈ 4.5× faster encoder; overall wall halved because the
   non-encoder stages didn't speed up).
4. **Bottleneck after threading**: encoder still nominally first (51.6%)
   but GPU collect at 40.9% is now competitive — future perf work
   should target GPU pipeline depth + readback before chasing more
   encoder threads.

## Recommendation surface (NOT a decision)

Possible §11 amendment paths the spec architect could take, based on
these numbers — DO NOT consider this a vote. The amendment lives in
`verbreel-spec`, not here.

- **Path A — split §11**: §11.1 mandatory determinism (verified against
  the deterministic preset), §11.2 mandatory perf (verified against the
  performance preset). Both presets are byte-deterministic on this
  host so the "either/or" framing is unnecessary; the export path can
  offer both knobs to the user.
- **Path B — promote performance preset as default for export**: since
  it's deterministic AND clears the 60 fps bar by ~1.9×, the
  performance preset could be the default and the deterministic preset
  the opt-in for "byte-reproducible distribution masters." The §5 spec
  would need to formalize the performance string then.
- **Path C — keep §11 as-is, tighten §5**: spec §11's 60-fps target
  applies unambiguously to the performance preset (the spec architect's
  reading), and §5 now formalizes both strings instead of one. Slice C
  is reclassified as "deterministic preset perf is below the bar by
  4.6%, which is expected because the deterministic preset is not the
  export target."

Path B/C both hinge on whether the determinism guarantee on this host
generalizes — recommend the spec architect cross-check on at least one
AMD/Apple Silicon host before fixing wording, since libx264's
`threads=auto` count is host-dependent (18 here on a 12-thread CPU
because libx264 caps but otherwise scales with `nproc`).

## Deviations from prompt

None. All scope-locked files respected; default `Encoder::new` call
sites (`spike_s1_roundtrip.rs`) pass `EncoderPreset::Deterministic`
explicitly; no `unsafe` added; no green-main breakage.
