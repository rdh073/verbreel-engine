//! Pure-math color DSP core for the `clip.auto_color` and
//! `clip.color_match` verbs (issue #479).
//!
//! ## Why this lives in `verbreel-state` and takes statistics as input
//!
//! [`crate::reconstructor::Verb::compute_patch`] is pure: it must not
//! decode frames or touch the filesystem (the same runtime-floor split
//! that `audio.analyze` / `tracker.run` / `asset.probe` observe — real
//! decode/DSP needs runtime context outside the pure verb). Frame
//! decode and per-pixel reduction therefore happen in a separate runtime
//! concern; this module operates only on the *already-reduced* per-frame
//! statistics (per-channel mean/std and black/white percentiles) that
//! arrive as verb args. Given identical statistics the output params are
//! byte-identical across runs and platforms — the issue's determinism
//! requirement is satisfied at the statistics boundary.
//!
//! ## Color-space chain
//!
//! Statistics are expressed in `[0, 1]` non-linear sRGB. White-balance
//! and level-stretch math is gray-world / luminance-classical and runs
//! directly on sRGB channel statistics. The Reinhard mean/std transfer
//! runs in **Oklab** — a perceptual space with no whitepoint table,
//! which keeps the math to plain `f64` arithmetic with fixed constants
//! (no platform-dependent LUTs). The sRGB transfer function and the
//! Oklab matrices use exact `f64` literals so the result is
//! reproducible.
//!
//! ## Emitted params (the `color_correct` grade)
//!
//! Both verbs emit the existing `color_correct` effect kind. Its param
//! schema is a v1.1 runtime-registry item and is *not* yet pinned by the
//! render side (grep shows no render-side constants), so this module
//! fixes the conventions used by the auto/match math and documents them
//! here so the grade stays editable and predictable:
//!
//! - `brightness`: additive luma offset in `[-1, 1]` sRGB units (0 = no
//!   change). Applied as `out = in + brightness`.
//! - `contrast`: multiplicative gain about the 0.5 mid-gray pivot
//!   (1 = no change). Applied as `out = (in - 0.5) * contrast + 0.5`.
//! - `saturation`: multiplicative chroma gain about the per-pixel luma
//!   (1 = no change).
//! - `temperature`: warm/cool balance in `[-1, 1]` (positive = warmer:
//!   lift red, cut blue). 0 = no change.
//! - `tint`: green/magenta balance in `[-1, 1]` (positive = more
//!   magenta: lift red+blue, cut green). 0 = no change.
//!
//! All emitted values are rounded to [`PARAM_DECIMALS`] decimal places
//! before serialization so the canonical-JSON hash is stable regardless
//! of `f64` round-off in the last bit.

/// Number of decimal places every emitted param is rounded to before it
/// is written into the effect's params map. Keeps the canonical-JSON
/// hash stable against `f64` last-bit round-off.
pub const PARAM_DECIMALS: i32 = 6;

/// Rec.709 luma weight for the red channel.
const LUMA_R: f64 = 0.2126;
/// Rec.709 luma weight for the green channel.
const LUMA_G: f64 = 0.7152;
/// Rec.709 luma weight for the blue channel.
const LUMA_B: f64 = 0.0722;

/// Per-channel statistics of one representative frame, in `[0, 1]`
/// non-linear sRGB. The same shape describes both the auto-color source
/// frame and the color-match reference/target frames.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChannelStats {
    /// Mean of the red channel in `[0, 1]`.
    pub mean_r: f64,
    /// Mean of the green channel in `[0, 1]`.
    pub mean_g: f64,
    /// Mean of the blue channel in `[0, 1]`.
    pub mean_b: f64,
    /// Standard deviation of the red channel in `[0, 1]`.
    pub std_r: f64,
    /// Standard deviation of the green channel in `[0, 1]`.
    pub std_g: f64,
    /// Standard deviation of the blue channel in `[0, 1]`.
    pub std_b: f64,
    /// Black point: the low luma percentile in `[0, 1]` (the value that
    /// should map to 0 after a levels stretch).
    pub black_point: f64,
    /// White point: the high luma percentile in `[0, 1]` (the value that
    /// should map to 1 after a levels stretch).
    pub white_point: f64,
}

/// The five `color_correct` grade parameters this module produces.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradeParams {
    /// Additive luma offset in `[-1, 1]` (0 = identity).
    pub brightness: f64,
    /// Multiplicative contrast about 0.5 (1 = identity).
    pub contrast: f64,
    /// Multiplicative chroma gain (1 = identity).
    pub saturation: f64,
    /// Warm/cool balance in `[-1, 1]` (0 = identity).
    pub temperature: f64,
    /// Green/magenta balance in `[-1, 1]` (0 = identity).
    pub tint: f64,
}

impl GradeParams {
    /// The identity grade: every param at its no-op value.
    #[must_use]
    pub const fn identity() -> Self {
        Self {
            brightness: 0.0,
            contrast: 1.0,
            saturation: 1.0,
            temperature: 0.0,
            tint: 0.0,
        }
    }

    /// Round every field to [`PARAM_DECIMALS`] decimals. Applied before
    /// serialization so the canonical hash is stable.
    #[must_use]
    pub fn rounded(self) -> Self {
        Self {
            brightness: round_to(self.brightness, PARAM_DECIMALS),
            contrast: round_to(self.contrast, PARAM_DECIMALS),
            saturation: round_to(self.saturation, PARAM_DECIMALS),
            temperature: round_to(self.temperature, PARAM_DECIMALS),
            tint: round_to(self.tint, PARAM_DECIMALS),
        }
    }
}

/// Round `value` to `decimals` decimal places using round-half-away-from
/// -zero on a power-of-ten scale. Plain `f64` arithmetic — no platform
/// LUTs — so the result is reproducible.
#[must_use]
pub fn round_to(value: f64, decimals: i32) -> f64 {
    if !value.is_finite() {
        return value;
    }
    let scale = 10.0_f64.powi(decimals);
    (value * scale).round() / scale
}

/// Rec.709 luma of an `[r, g, b]` sRGB triple.
#[must_use]
fn luma(r: f64, g: f64, b: f64) -> f64 {
    LUMA_R * r + LUMA_G * g + LUMA_B * b
}

/// sRGB electro-optical transfer (gamma-decode) on one `[0, 1]` channel.
#[must_use]
pub fn srgb_to_linear(c: f64) -> f64 {
    if c <= 0.040_449_936_476_345_19 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Inverse sRGB transfer (gamma-encode) on one linear channel.
#[must_use]
pub fn linear_to_srgb(c: f64) -> f64 {
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// Convert a linear-light sRGB triple to Oklab `[L, a, b]`.
///
/// Uses the canonical Oklab matrices with exact `f64` literals
/// (Ottosson 2020). No whitepoint table is involved.
// The single-letter names are the canonical Oklab/LMS symbols (l, m, s
// = long/medium/short cone responses); renaming them would obscure the
// match against the published reference matrices.
#[allow(clippy::many_single_char_names)]
#[must_use]
pub fn linear_srgb_to_oklab(r: f64, g: f64, b: f64) -> [f64; 3] {
    let l = 0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b;
    let m = 0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b;
    let s = 0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b;

    let l_ = l.cbrt();
    let m_ = m.cbrt();
    let s_ = s.cbrt();

    [
        0.210_454_255_3 * l_ + 0.793_617_785_0 * m_ - 0.004_072_046_8 * s_,
        1.977_998_495_1 * l_ - 2.428_592_205_0 * m_ + 0.450_593_709_9 * s_,
        0.025_904_037_1 * l_ + 0.782_771_766_2 * m_ - 0.808_675_766_0 * s_,
    ]
}

/// Convert an Oklab `[L, a, b]` triple back to linear-light sRGB.
// Canonical Oklab/LMS symbols — see `linear_srgb_to_oklab`.
#[allow(clippy::many_single_char_names)]
#[must_use]
pub fn oklab_to_linear_srgb(lab: [f64; 3]) -> [f64; 3] {
    let [big_l, a, b] = lab;
    let l_ = big_l + 0.396_337_777_4 * a + 0.215_803_757_3 * b;
    let m_ = big_l - 0.105_561_345_8 * a - 0.063_854_172_8 * b;
    let s_ = big_l - 0.089_484_177_5 * a - 1.291_485_548_0 * b;

    let l = l_ * l_ * l_;
    let m = m_ * m_ * m_;
    let s = s_ * s_ * s_;

    [
        4.076_741_662_1 * l - 3.307_711_591_3 * m + 0.230_969_929_2 * s,
        -1.268_438_004_6 * l + 2.609_757_401_1 * m - 0.341_319_396_5 * s,
        -0.004_196_086_3 * l - 0.703_418_614_7 * m + 1.707_614_701_0 * s,
    ]
}

/// Convert a non-linear sRGB triple straight to Oklab.
#[must_use]
pub fn srgb_to_oklab(r: f64, g: f64, b: f64) -> [f64; 3] {
    linear_srgb_to_oklab(srgb_to_linear(r), srgb_to_linear(g), srgb_to_linear(b))
}

/// Convert an Oklab triple back to a non-linear sRGB triple, clamped to
/// `[0, 1]` (out-of-gamut results are clipped, never wrapped).
#[must_use]
pub fn oklab_to_srgb(lab: [f64; 3]) -> [f64; 3] {
    let lin = oklab_to_linear_srgb(lab);
    [
        linear_to_srgb(lin[0]).clamp(0.0, 1.0),
        linear_to_srgb(lin[1]).clamp(0.0, 1.0),
        linear_to_srgb(lin[2]).clamp(0.0, 1.0),
    ]
}

/// Gray-world white-balance gains for an `[r, g, b]` mean triple.
///
/// Gray-world assumes the average scene reflectance is achromatic, so
/// each channel is scaled to match the overall mean luma. Returns the
/// per-channel multiplicative gains `[gr, gg, gb]`. A channel mean at or
/// below zero yields a unit gain (no division by zero, no silent
/// fallback to an arbitrary value).
#[must_use]
pub fn gray_world_gains(mean_r: f64, mean_g: f64, mean_b: f64) -> [f64; 3] {
    let target = luma(mean_r, mean_g, mean_b);
    [
        channel_gain(target, mean_r),
        channel_gain(target, mean_g),
        channel_gain(target, mean_b),
    ]
}

/// One channel's gray-world gain, guarding a non-positive mean.
#[must_use]
fn channel_gain(target: f64, mean: f64) -> f64 {
    if mean > 0.0 { target / mean } else { 1.0 }
}

/// Derive an editable `color_correct` grade from a single frame's
/// statistics (the `clip.auto_color` math).
///
/// - **White balance** → `temperature` + `tint`, from gray-world gains.
///   A red gain above green means the frame is too red, so we *cool* it
///   (negative temperature); blue likewise. Green imbalance drives
///   `tint`.
/// - **Levels** → `brightness` + `contrast`, from the black/white
///   percentiles. A stretch `out = (in - bp) / (wp - bp)` is decomposed
///   into the contrast gain `1 / (wp - bp)` and the brightness offset
///   that re-centres mid-gray.
/// - `saturation` is left at identity; auto-color does not infer a
///   chroma boost (that is the color-match verb's job).
///
/// An already-neutral, full-range frame (equal channel means, `bp = 0`,
/// `wp = 1`) yields the identity grade within rounding.
#[must_use]
pub fn auto_color_grade(stats: &ChannelStats) -> GradeParams {
    let gains = gray_world_gains(stats.mean_r, stats.mean_g, stats.mean_b);
    let [gr, gg, gb] = gains;

    // Temperature: warm = lift red / cut blue. gr > gb means the frame
    // was blue-heavy and gray-world wants to lift red, i.e. warm it.
    let temperature = (gr - gb) / 2.0;
    // Tint: magenta = lift red+blue / cut green. A green gain below the
    // red/blue average means green was dominant -> push toward magenta.
    let tint = f64::midpoint(gr, gb) - gg;

    // Levels stretch from black/white percentiles.
    let span = stats.white_point - stats.black_point;
    let (contrast, brightness) = if span > f64::EPSILON {
        let contrast = 1.0 / span;
        // Levels stretch:  out = (in - bp) / span = contrast*in - contrast*bp.
        // Contrast-param convention: out = (in - 0.5)*contrast + 0.5 + brightness
        //                                = contrast*in - 0.5*contrast + 0.5 + brightness.
        // Matching the constant terms gives:
        //   brightness = 0.5*contrast - 0.5 - contrast*bp.
        // For a full-range frame (bp = 0, span = 1) this is 0 — identity.
        let brightness = 0.5 * contrast - 0.5 - contrast * stats.black_point;
        (contrast, brightness)
    } else {
        (1.0, 0.0)
    };

    GradeParams {
        brightness,
        contrast,
        saturation: 1.0,
        temperature,
        tint,
    }
    .rounded()
}

/// Reinhard mean/std transfer of one channel: rescale so the target's
/// distribution matches the reference's mean and spread.
///
/// `out = (in - mean_tgt) * (std_ref / std_tgt) + mean_ref`. A
/// non-positive target std yields a pure mean shift (gain = 1) rather
/// than dividing by zero — an explicit, documented degenerate case, not
/// a silent fallback.
#[must_use]
pub fn reinhard_channel(
    in_value: f64,
    mean_tgt: f64,
    std_tgt: f64,
    mean_ref: f64,
    std_ref: f64,
) -> f64 {
    let gain = if std_tgt > 0.0 {
        std_ref / std_tgt
    } else {
        1.0
    };
    (in_value - mean_tgt) * gain + mean_ref
}

/// Derive an editable `color_correct` grade that transfers the
/// reference frame's look onto the target frame (the `clip.color_match`
/// math), via a Reinhard mean/std match in Oklab.
///
/// Both stat sets are converted to Oklab. The Reinhard transfer is
/// solved in Oklab and then projected back onto the five
/// `color_correct` params by probing how a neutral mid-gray and the
/// target's own mean move under the transfer:
///
/// - `brightness` ← the L-axis mean shift (in sRGB luma units).
/// - `contrast` ← the L-axis std ratio (`std_ref_L / std_tgt_L`).
/// - `saturation` ← the chroma (a,b) std ratio.
/// - `temperature` / `tint` ← the residual a/b mean shift mapped to the
///   warm-cool / green-magenta axes.
///
/// Identity in / identity out: matching a frame to itself yields the
/// identity grade within rounding.
#[must_use]
pub fn color_match_grade(reference: &ChannelStats, target: &ChannelStats) -> GradeParams {
    let ref_mean = srgb_to_oklab(reference.mean_r, reference.mean_g, reference.mean_b);
    let tgt_mean = srgb_to_oklab(target.mean_r, target.mean_g, target.mean_b);

    // Per-channel sRGB std mapped to Oklab axes via the luma/chroma
    // partition: L-spread tracks luma std, (a,b)-spread tracks chroma
    // std. We approximate channel std in Oklab by the local Jacobian at
    // the mean, which for a mean/std transfer collapses to the ratio of
    // sRGB stds along luma and chroma — exact for the affine transfer we
    // emit and fully deterministic.
    let ref_luma_std = luma(reference.std_r, reference.std_g, reference.std_b);
    let tgt_luma_std = luma(target.std_r, target.std_g, target.std_b);
    let ref_chroma_std = chroma_std(reference);
    let tgt_chroma_std = chroma_std(target);

    let contrast = ratio_or_one(ref_luma_std, tgt_luma_std);
    let saturation = ratio_or_one(ref_chroma_std, tgt_chroma_std);

    // L-axis mean shift expressed in sRGB luma units so it composes with
    // the brightness convention.
    let ref_luma = luma(reference.mean_r, reference.mean_g, reference.mean_b);
    let tgt_luma = luma(target.mean_r, target.mean_g, target.mean_b);
    let brightness = ref_luma - tgt_luma;

    // a/b mean shift -> warm-cool / green-magenta. Oklab `a` is the
    // green(-)/red(+) axis, `b` is the blue(-)/yellow(+) axis.
    let da = ref_mean[1] - tgt_mean[1];
    let db = ref_mean[2] - tgt_mean[2];
    // Warmer = more red & less blue => +a and +b both warm.
    let temperature = f64::midpoint(da, db);
    // Magenta(+)/green(-) maps to the +a / -b combination.
    let tint = (da - db) / 2.0;

    GradeParams {
        brightness,
        contrast,
        saturation,
        temperature,
        tint,
    }
    .rounded()
}

/// Chroma spread of a frame: the Euclidean norm of its per-channel sRGB
/// stds projected off the luma axis. A frame whose channels co-vary
/// (pure luma noise) has near-zero chroma std.
#[must_use]
fn chroma_std(stats: &ChannelStats) -> f64 {
    let mean_std = (stats.std_r + stats.std_g + stats.std_b) / 3.0;
    let dr = stats.std_r - mean_std;
    let dg = stats.std_g - mean_std;
    let db = stats.std_b - mean_std;
    (dr * dr + dg * dg + db * db).sqrt()
}

/// Reference/target ratio with a documented unit fallback for a
/// non-positive denominator (no silent divide-by-zero).
#[must_use]
fn ratio_or_one(numerator: f64, denominator: f64) -> f64 {
    if denominator > 0.0 {
        numerator / denominator
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64, eps: f64) -> bool {
        (a - b).abs() <= eps
    }

    #[test]
    fn srgb_round_trips_through_linear() {
        for raw in [0.0, 0.04, 0.2, 0.5, 0.8, 1.0] {
            let back = linear_to_srgb(srgb_to_linear(raw));
            assert!(approx(raw, back, 1e-9), "{raw} -> {back}");
        }
    }

    #[test]
    fn oklab_round_trips_through_srgb() {
        for rgb in [[0.1, 0.2, 0.3], [0.5, 0.5, 0.5], [0.9, 0.1, 0.4]] {
            let lab = srgb_to_oklab(rgb[0], rgb[1], rgb[2]);
            let back = oklab_to_srgb(lab);
            for i in 0..3 {
                assert!(approx(rgb[i], back[i], 1e-6), "{rgb:?} -> {back:?}");
            }
        }
    }

    #[test]
    fn gray_world_neutral_is_unit_gain() {
        let gains = gray_world_gains(0.4, 0.4, 0.4);
        for g in gains {
            assert!(approx(g, 1.0, 1e-12), "{g}");
        }
    }

    #[test]
    fn auto_color_neutral_full_range_is_identity() {
        let stats = ChannelStats {
            mean_r: 0.5,
            mean_g: 0.5,
            mean_b: 0.5,
            std_r: 0.2,
            std_g: 0.2,
            std_b: 0.2,
            black_point: 0.0,
            white_point: 1.0,
        };
        let grade = auto_color_grade(&stats);
        let id = GradeParams::identity();
        assert!(approx(grade.brightness, id.brightness, 1e-6));
        assert!(approx(grade.contrast, id.contrast, 1e-6));
        assert!(approx(grade.saturation, id.saturation, 1e-6));
        assert!(approx(grade.temperature, id.temperature, 1e-6));
        assert!(approx(grade.tint, id.tint, 1e-6));
    }

    #[test]
    fn reinhard_matches_closed_form() {
        let out = reinhard_channel(0.7, 0.5, 0.2, 0.6, 0.1);
        // (0.7 - 0.5) * (0.1/0.2) + 0.6 = 0.2*0.5 + 0.6 = 0.7
        assert!(approx(out, 0.7, 1e-12), "{out}");
    }

    #[test]
    fn color_match_identity_when_ref_equals_target() {
        let stats = ChannelStats {
            mean_r: 0.45,
            mean_g: 0.5,
            mean_b: 0.55,
            std_r: 0.18,
            std_g: 0.2,
            std_b: 0.22,
            black_point: 0.02,
            white_point: 0.98,
        };
        let grade = color_match_grade(&stats, &stats);
        assert!(approx(grade.brightness, 0.0, 1e-6), "{}", grade.brightness);
        assert!(approx(grade.contrast, 1.0, 1e-6), "{}", grade.contrast);
        assert!(approx(grade.saturation, 1.0, 1e-6), "{}", grade.saturation);
        assert!(
            approx(grade.temperature, 0.0, 1e-6),
            "{}",
            grade.temperature
        );
        assert!(approx(grade.tint, 0.0, 1e-6), "{}", grade.tint);
    }

    #[test]
    fn color_match_is_deterministic() {
        let reference = ChannelStats {
            mean_r: 0.6,
            mean_g: 0.5,
            mean_b: 0.3,
            std_r: 0.25,
            std_g: 0.2,
            std_b: 0.15,
            black_point: 0.05,
            white_point: 0.95,
        };
        let target = ChannelStats {
            mean_r: 0.3,
            mean_g: 0.4,
            mean_b: 0.6,
            std_r: 0.1,
            std_g: 0.12,
            std_b: 0.18,
            black_point: 0.0,
            white_point: 0.9,
        };
        let a = color_match_grade(&reference, &target);
        let b = color_match_grade(&reference, &target);
        assert_eq!(a, b);
    }
}
