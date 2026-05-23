# Spike S1 — Slice A — Results

**Date:** 2026-05-24
**Branch:** spike/01-native-render
**Author:** executor (Claude Code)
**Issue:** rdh073/verbreel-engine#6

## TL;DR

- Goal 1 (FFmpeg 8 vendor-build, reproducible): **PASS** (2m38s cold, 271 MiB artifact)
- Goal 2 (`spike-s1` feature gate keeps default workspace check green-main): **PASS**
- Goal 3 (libx264 §5 preset → byte-identical MP4 across 10 sequential roundtrips): **PASS** (1 unique SHA-256: `432aaab694299a8436e97982fc3fa2f773232c037c810a95f61b1387815e5f6d`)

## Environment

- Host: Ubuntu 24.04 Noble Numbat
- Rust: `rustc 1.95.0 (59807616e 2026-04-14)`
- FFmpeg: 8.0.2 (release/8.0 branch, HEAD `bebde67`), vendor-built via
  `rsmpeg utils/linux_ffmpeg.rs` into
  `vendor/rsmpeg/tmp/ffmpeg_build/` (static `.a` only).
- libx264 (system, via `libx264-dev`): `2:0.164.3108+git31e19f9-1` (Ubuntu noble)
- rsmpeg: `0.18.0+ffmpeg.8.0` with `features = ["ffmpeg8", "link_system_ffmpeg"]`

## Goal 1 — FFmpeg vendor build

- Cold build wall time: **2m38.4s** (24-thread parallel make)
- Artifact dir size: `271M	vendor/rsmpeg/tmp/ffmpeg_build/`
- pkg-config modversions (with `FFMPEG_PKG_CONFIG_PATH` exported):

  | lib | modversion | FFmpeg 8 expected | OK |
  |-----|-----------|-------------------|----|
  | libavcodec    | 62.11.102 | 62.x | ✓ |
  | libavformat   | 62.3.103  | 62.x | ✓ |
  | libavutil     | 60.8.102  | 60.x | ✓ |
  | libswscale    | 9.1.102   | 9.x  | ✓ |
  | libswresample | 6.1.102   | 6.x  | ✓ |

- Built only static archives (`.a`) — no `.so`. Means the spike-s1 binary
  links FFmpeg statically and does NOT need `LD_LIBRARY_PATH` at run
  time (we exported it for belt-and-braces; harness ran fine).

## Goal 2 — Feature gate

- `unset FFMPEG_PKG_CONFIG_PATH && cargo check --workspace --all-targets`:
  **PASS** — `Finished dev profile [...] in 0.08s` (incremental; cold full
  workspace check was 8.88s on the prior run).
- `FFMPEG_PKG_CONFIG_PATH=... cargo check -p verbreel-codec-native --features spike-s1`:
  **PASS** — `Finished dev profile [...] in 2.67s`
- `... --example spike_s1_roundtrip` (feature on): **PASS**
- The example carries `required-features = ["spike-s1"]` so default
  `--all-targets` skips it; that's what keeps green-main intact.

## Goal 3 — Decode↔encode determinism

Harness (`examples/spike_s1_roundtrip.rs`):
1. Generate 240 frames of 1920×1080 YUV420P synthetic source (gradient
   + per-frame counter encoded in the Y plane's top-left 4 bytes).
2. **Pass 1** — encode synth → `run_N_pass1.mp4` (libx264, §5 preset).
3. **Pass 2** — decode `run_N_pass1.mp4` → re-encode → `run_N_pass2.mp4`.
4. SHA-256 `run_N_pass2.mp4`, repeat × 10, compare.

Result:

```
$ cat tmp/spike_s1/hashes.txt
432aaab694299a8436e97982fc3fa2f773232c037c810a95f61b1387815e5f6d
432aaab694299a8436e97982fc3fa2f773232c037c810a95f61b1387815e5f6d
432aaab694299a8436e97982fc3fa2f773232c037c810a95f61b1387815e5f6d
432aaab694299a8436e97982fc3fa2f773232c037c810a95f61b1387815e5f6d
432aaab694299a8436e97982fc3fa2f773232c037c810a95f61b1387815e5f6d
432aaab694299a8436e97982fc3fa2f773232c037c810a95f61b1387815e5f6d
432aaab694299a8436e97982fc3fa2f773232c037c810a95f61b1387815e5f6d
432aaab694299a8436e97982fc3fa2f773232c037c810a95f61b1387815e5f6d
432aaab694299a8436e97982fc3fa2f773232c037c810a95f61b1387815e5f6d
432aaab694299a8436e97982fc3fa2f773232c037c810a95f61b1387815e5f6d
$ sort -u tmp/spike_s1/hashes.txt | wc -l
1
```

- Unique hash count: **1 = PASS**
- Wall time for 10-run loop: **1m3.056s** (release build, single thread
  forced by `x264-params threads=1`)
- Per-iteration average: **~6.3s** (one encode + one decode + one re-encode)
- Pass1 MP4 size: 357,669 bytes (~349 KiB)
- Pass2 MP4 size: 331,871 bytes (~324 KiB)

### Nondeterminism mitigations applied

1. **libx264 params**: `threads=1:sliced-threads=0:sync-lookahead=0:rc-lookahead=0:bframes=0` (§5 canonical).
2. **libx264 preset/tune**: `medium` + `zerolatency` (reinforces single-pass, no look-ahead).
3. **`gop_size = fps * 2`** (2-second GOP — short enough to surface any drift early).
4. **MP4 container `creation_time`**: frozen to `1970-01-01T00:00:00Z`
   via `(*ofmt_ctx.as_mut_ptr()).metadata = ...` (unsafe FFI, `// SAFETY:` comment
   in `encoder.rs:124`). Without this, the mov muxer embeds wall-clock
   time in `mvhd`/`tkhd`/`mdhd` atoms and consecutive runs differ.
5. ffprobe on the output confirms `TAG:encoder=Lavf62.3.103` (the
   FFmpeg lib version, constant per build) and **no `creation_time`
   tag** — so the only "version-leaking" field is the encoder string,
   which doesn't break determinism within a single host.

## Deviations from prompt

- **`libvpx-dev` apt install** — the prompt's apt list omits `libvpx-dev`,
  but rsmpeg's upstream `utils/linux_ffmpeg.rs` enables `--enable-libvpx`
  unconditionally. Cold run of the helper failed with `libvpx enabled but
  no supported decoders found`. Two options: (a) install `libvpx-dev` to
  unblock the upstream helper unmodified, or (b) trim `--enable-libvpx`
  from a local copy of the script. Chose (a) — the prompt explicitly says
  "use rsmpeg's `utils/linux_ffmpeg.rs` helper" (i.e. unmodified) and the
  exclusion list (`no x265, no fdk-aac, no hwaccel libs`) does not name
  libvpx. Suggest the next iteration of the prompt add `libvpx-dev` to
  the apt install line.
- **`LD_LIBRARY_PATH` not actually needed at run time** — the vendor build
  produced only `.a` files, so FFmpeg is statically linked into the
  spike binary. Exported it anyway per the prompt for belt-and-braces;
  removing it doesn't affect the run.
- **AVStream borrow scoping** — `AVFormatContextOutput::new_stream`
  returns an `AVStreamMut<'_>` that holds a mutable borrow of the format
  context. The transcode example in `vendor/rsmpeg/tests/ffmpeg_examples/`
  works around this by structuring the code so the borrow ends before
  next use; we did the same with an explicit `{ }` scope around the
  stream setup (`encoder.rs:101-107`). Minor stylistic note, not a
  spec issue.

## Next-slice readiness (observations for slice B)

These notes are for the assistant authoring **Task 02b — wgpu YUV↔RGB
pipeline**:

- **Color range**: `ffprobe` reports `color_range=unknown` on the output
  MP4 — i.e. neither `pc` (full) nor `tv` (limited) tag was set. libx264
  defaults to limited (TV) range when no `-color_range` flag is passed.
  Slice B's YUV→RGB shader must therefore use the **limited-range
  BT.601** conversion matrix (Y∈[16..235], UV∈[16..240]). If slice B
  wants to standardize on full-range, the encoder needs an explicit
  `colorspace`/`color_range` opt set on the AVCodecContext.
- **Pixel format**: decode side hands you `AV_PIX_FMT_YUV420P` always
  (we assert in `decoder.rs:99`). Three contiguous planes,
  4:2:0 chroma subsampling.
- **Linesize is not width**: rsmpeg/FFmpeg aligns each plane row to a
  SIMD-friendly stride (typically 16 or 32 bytes). The current
  decoder strips stride and emits tight `W*H*3/2` bytes. For slice B's
  zero-copy GPU upload, leaving the strided layout intact and passing
  `linesize[i]` as the texture pitch will avoid a CPU-side copy.
- **No B-frames** (`bframes=0` in §5 preset). Encoder PTS == DTS,
  monotonically increasing by 1. Slice C's crossfade timeline math is
  simpler because of this — no PTS/DTS reordering buffer needed.
- **GOP size 48** (= 2s @ 24fps): seeking accuracy on the input MP4 is
  ≤ 2s. Slice C should either pad clip-in timestamps to GOP boundaries
  or implement a "decode-and-discard" path for sub-GOP seeks.
- **Container metadata override pattern**: the `creation_time` freeze
  is via direct pointer assignment into `AVFormatContext.metadata`.
  rsmpeg does not expose a safe wrapper for the format-level
  metadata setter (only stream-level). Slice C may need to repeat this
  pattern if it adds more containers; consider promoting it to a small
  helper crate.
- **MSRV check**: the workspace rust-version is 1.92; rsmpeg's stated
  MSRV is 1.81. No conflict, but slice B's wgpu 29 wants 1.84+ — already
  satisfied.
