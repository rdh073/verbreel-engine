//! `RenderPreset` — pins Research 01 §11's two-preset model at the
//! type level. The deterministic variant trades threading for
//! byte-identical export (§11.1); the performance variant trades
//! byte-identical export for real-time throughput (§11.2).
//!
//! Mirrors [`verbreel_codec_native::CodecPreset`](../../verbreel_codec_native/enum.CodecPreset.html)
//! by design — the same architectural decision is committed at both
//! the encoder and the render frontend.

/// Render preset selecting between export-grade reproducibility and
/// daily-driver throughput.
///
/// Spec §11.1 (deterministic mode) and §11.2 (performance mode) are
/// the only two presets that v1 commits to. Future hardware-acceleration
/// presets land when Research 01 §6 hwaccel design ships — not in v0.
///
/// `serde` derives intentionally omitted: the preset is an internal
/// pipeline-control switch, not a wire-format field. If S1 work
/// surfaces a need for serde round-trip, it lands at S1 alongside the
/// actual render pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RenderPreset {
    /// `§11.1` deterministic mode. wgpu compute pinned to single-thread
    /// scheduling, WGSL operations executed in spec order, encode hand-
    /// off uses libx264 `-x264-params threads=1:...` per Research 01
    /// §5. Guarantees byte-identical MP4 across runs on the same host.
    Deterministic,

    /// `§11.2` performance mode. Default wgpu multi-threading, no
    /// determinism guarantee. Targets `≥60 fps` real-time export on a
    /// 2024-class laptop iGPU.
    Performance,
}

impl RenderPreset {
    /// Lower-case spec id (`"deterministic"` / `"performance"`).
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Deterministic => "deterministic",
            Self::Performance => "performance",
        }
    }
}
