# Browser preview fallback matrix

How `verbreel-codec-web` selects a preview transport. The decision is
policy #405 option A, implemented as the pure function
`codec_for_preview` and surfaced through `PreviewSessionPlan::resolve`:

> Use `webcodecs` when the client reports `VideoDecoder` decode support;
> otherwise fall back to fMP4/MSE (`mse`), including Safari.

## Decision inputs

The client reports a `PreviewClientCapabilities`:

- `browser_family` — `Safari` or `Other`.
- `has_webcodecs_decode` — whether the global `VideoDecoder`
  constructor exists (probed by `capability::detect` on wasm32).

## Selection matrix

| Browser family | `WebCodecs` decode | Chosen transport | Wire literal | Session metadata |
|---|---|---|---|---|
| Other (Chrome / Firefox / Edge) | present | `WebCodecs` | `webcodecs` | none beyond the literal |
| Other | absent | fMP4/MSE | `mse` | `MseFallbackEnvelope` (fMP4 MIME) |
| Safari | present (17+) | `WebCodecs` | `webcodecs` | none beyond the literal |
| Safari | absent | fMP4/MSE | `mse` | `MseFallbackEnvelope` (fMP4 MIME) |

The browser family never changes the *result* on its own — it is only
carried so that future policy revisions can special-case a family
without a wire change. Under option A both families collapse to the
same `has_webcodecs_decode`-driven choice.

## Codec → MSE MIME

When the `mse` path is chosen, the `MseFallbackEnvelope` carries the
`MediaSource` source-buffer MIME so the client can `addSourceBuffer`
before any media arrives. The MIME mirrors the `WebCodecs` codec
baseline so both paths decode the same bitstream:

| `WebDecoder` | `WebCodecs` codec string | fMP4/MSE MIME |
|---|---|---|
| `H264` | `avc1.640028` | `video/mp4; codecs="avc1.640028"` |
| `H265` | `hvc1.1.6.L93.B0` | `video/mp4; codecs="hvc1.1.6.L93.B0"` |

Segments are always fragmented MP4 (`fragmented: true` in the
envelope) so they append incrementally to the `SourceBuffer`.

## What is rejected

Options B and C from #376 are rejected for v1: B degrades Safari to
unsupported, C introduces a lower-quality parallel frame protocol.
Option A keeps a single fallback (`mse`) for every non-`WebCodecs`
client.

Frame bytes never appear in any serialized session metadata — only the
literal and the envelope MIME travel over the wire (Research 01 §6.2).
