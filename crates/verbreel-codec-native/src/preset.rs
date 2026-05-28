//! [`CodecPreset`] — Research 01 §5's two-preset model.
//!
//! Pins the §11.1 / §11.2 split that Spike S1 will satisfy with real
//! rsmpeg+libx264 calls. The wire tokens are stable v0 contract — verb
//! args that pass through `render.queue.add` carry the same lowercase
//! string.

use serde::{Deserialize, Serialize};

/// Encoder preset — selects the libx264 parameter set Spike S1 will
/// hand to the encoder.
///
/// Two values are committed in v0. They mirror the §5 / §11 split
/// exactly: deterministic trades threading for byte-identical output,
/// performance trades byte-identical output for threading speedup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CodecPreset {
    /// §11.1 deterministic mode. Wire token: `"deterministic"`.
    ///
    /// Spike S1 will configure libx264 with the canonical parameter
    /// string and pin the container creation timestamp:
    ///
    /// ```text
    /// -x264-params threads=1:sliced-threads=0:sync-lookahead=0:rc-lookahead=0:bframes=0
    /// creation_time=0
    /// ```
    ///
    /// These params remove the four documented sources of libx264
    /// nondeterminism (frame threads, slice threads, both lookahead
    /// pipelines, and B-frame reordering). With them set, 10 sequential
    /// runs on the same host produce a single SHA-256 across the
    /// output MP4 — the §0.1 reproducibility contract.
    ///
    /// Cost: single-threaded encode. Use this preset for CI,
    /// archival masters, and any path whose downstream consumer
    /// hashes the bytes.
    Deterministic,

    /// §11.2 performance mode. Wire token: `"performance"`.
    ///
    /// Spike S1 will leave libx264 on its default threading model
    /// (frame-threading enabled, both lookahead pipelines on, B-frames
    /// allowed) with no VBV constraint, targeting real-time export
    /// throughput (≥60 fps end-to-end on a 2024-class laptop iGPU).
    ///
    /// Trade-off: byte-identical output is NOT guaranteed across
    /// runs. Use this preset for preview, proxy, and daily-review
    /// paths where wall-clock matters more than reproducibility.
    Performance,
}
