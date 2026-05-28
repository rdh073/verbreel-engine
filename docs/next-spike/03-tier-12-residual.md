# Tier 1+2 Residual Cleanup

| Item | Concrete gap | LOC | FILES | CALL_SITES | Points | Class |
|---|---|---:|---:|---:|---:|---|
| B | Lifecycle migration to `verbreel-storage` primitives: move `lifecycle.rs` call paths, including `ProjectStore::create_with_registry`, onto the CAS/flock/tempfile helpers without bypassing `verbreel_state::engine::apply()`. | 420 | 7 | 12 | 5 | MECHANICAL |
| C | Args integration via `Schema::from_value` for the next 5-10 minimal-args verbs after the existing well-known registry slice. | 280 | 8 | 10 | 3 | MECHANICAL |
| D | Asset CAS wiring: connect `asset.import` sha256/path behavior to `verbreel-storage/src/cas.rs` and canonical `assets/<aa>/<sha256>.<ext>` layout. | 360 | 6 | 4 | 5 | MECHANICAL |
| E | Timestamp newtype migration for 50+ call-sites currently using `timestamp_rfc3339_now` or raw RFC3339 strings. | 520 | 14 | 55 | 5 | MECHANICAL |
| E' | Color newtype close-loop: finish remaining `Canvas.background`, text color, shadow color, and fixture normalization after the partial typed-Color slice. | 160 | 5 | 12 | 2 | MECHANICAL |
| F | `Track.effects` -> `Vec<Effect>` conversion at `audio_denoise` JSON-Patch sites, including reconstructor and fixture canonical data checks. | 190 | 4 | 6 | 3 | MECHANICAL |
| G | `font.list` + `text.style` `E_FONT_UNKNOWN` enforcement needs a concrete font registry policy before implementation. | 520 | 8 | 6 | 5 | DESIGN |

## G Design Question

Which font registry strategy should v1 enforce for `font.list` and `E_FONT_UNKNOWN`?

- A. System `fontconfig`/`fontdb` discovery: accurate for host exports, but conformance fixtures must pin a synthetic registry.
- B. Bundled fonts only: deterministic and portable, but rejects valid system fonts unless explicitly imported later.
- C. Static lookup table: simplest validation path, but it can advertise fonts that are not actually installed.

## Notes

- The invariant for B/D is write ordering and CAS identity: mutation must flow through state apply and storage primitives before events are committed, not through ad hoc filesystem writes.
- The invariant for E/E' is typed canonical data: timestamps and colors must normalize before they enter event data, not during pretty-printing or fixture comparison.

## Project Board Sync

| Item | Issue | Project status | Estimate |
|---|---:|---|---:|
| B | #379 | Backlog | 5 |
| C | #380 | Backlog | 3 |
| D | #381 | Backlog | 5 |
| E | #382 | Backlog | 5 |
| E' | #383 | Backlog | 2 |
| F | #384 | Backlog | 3 |
| G | #385 | Backlog | 5 |
