//! Hardware acceleration policy surface for `RenderPreset`.
//!
//! v1 floor only: this module defines a stable accelerator priority
//! order and the preset-level eligibility rule. It does not probe
//! host capabilities and does not select codecs.

use crate::preset::RenderPreset;

/// Hardware acceleration kinds that the v1 floor policy may consider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HwAccelKind {
    /// NVIDIA NVENC acceleration path.
    Nvenc,
    /// Linux VAAPI acceleration path.
    Vaapi,
    /// Apple `VideoToolbox` acceleration path.
    VideoToolbox,
}

impl HwAccelKind {
    /// Stable wire identifier for logging and selection traces.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Nvenc => "NVENC",
            Self::Vaapi => "VAAPI",
            Self::VideoToolbox => "VideoToolbox",
        }
    }
}

/// v1 floor priority order: `NVENC -> VAAPI -> VideoToolbox`.
pub const V1_HWACCEL_PRIORITY: [HwAccelKind; 3] = [
    HwAccelKind::Nvenc,
    HwAccelKind::Vaapi,
    HwAccelKind::VideoToolbox,
];

const NO_HWACCEL: [HwAccelKind; 0] = [];

/// Returns the hwaccel priority list allowed for the given preset.
///
/// Deterministic mode stays software-only by contract; only
/// performance mode may consider hardware acceleration.
#[must_use]
pub fn hwaccel_priority_for_preset(preset: RenderPreset) -> &'static [HwAccelKind] {
    match preset {
        RenderPreset::Deterministic => &NO_HWACCEL,
        RenderPreset::Performance => &V1_HWACCEL_PRIORITY,
    }
}
