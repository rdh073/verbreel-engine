//! [`DecodedFrame`] — opaque v0 placeholder so [`crate::decode`] has a
//! complete signature.
//!
//! The real frame shape lives behind the `WebCodecs` `VideoFrame`
//! handle, which carries an opaque GPU-backed surface that v0 cannot
//! model without `wasm-bindgen` / `web_sys` in scope. v0 carries the
//! four fields Spike S2 will need at the boundary: dimensions, a flat
//! plane byte buffer, and the per-frame presentation timestamp in
//! microseconds (the `WebCodecs` `VideoFrame.timestamp` unit). The
//! plane interpretation (YUV vs RGB, plane offsets, stride) is
//! deliberately left unspecified — Spike S2 will tighten this when it
//! picks the `WebCodecs`→wgpu pixel-format negotiation path.

/// Opaque decoded video frame.
///
/// Fields are private by design — the buffer layout (flat `Vec<u8>`
/// in v0) is unpinned and will gain per-plane offset / stride metadata
/// once Spike S2 lands the `WebCodecs` binding. Keeping the fields
/// private means Spike S2 can tighten the shape without a public API
/// break; callers go through [`DecodedFrame::new`] and the borrowed
/// accessors only.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DecodedFrame {
    width: u32,
    height: u32,
    planes: Vec<u8>,
    pts_micros: u64,
}

impl DecodedFrame {
    /// Build a decoded frame from raw dimensions, a plane buffer, and
    /// the per-frame presentation timestamp in microseconds.
    #[must_use]
    pub const fn new(width: u32, height: u32, planes: Vec<u8>, pts_micros: u64) -> Self {
        Self {
            width,
            height,
            planes,
            pts_micros,
        }
    }

    /// Pixel width.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Pixel height.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Borrowed view of the plane buffer.
    #[must_use]
    pub fn planes(&self) -> &[u8] {
        &self.planes
    }

    /// Presentation timestamp in microseconds. Matches the `WebCodecs`
    /// `VideoFrame.timestamp` unit so Spike S2 can adopt it without a
    /// unit conversion at the boundary.
    #[must_use]
    pub const fn pts_micros(&self) -> u64 {
        self.pts_micros
    }
}

/// Convert a `WebCodecs` `VideoFrame.timestamp` (`f64` microseconds,
/// which may be fractional, negative, or non-finite) into the
/// non-negative integer-microsecond unit [`DecodedFrame`] carries.
///
/// Lives here (not in the wasm32-only `webcodecs` module) so the pure
/// arithmetic is exercised by native unit tests. Negative or non-finite
/// inputs clamp to `0`; an out-of-`u64`-range positive value saturates
/// to `u64::MAX` — both are sensible clamps for a presentation stamp.
///
// Called from the wasm32-only `webcodecs` decode loop and from the
// native unit tests below; unused on a native non-test lib build.
#[cfg_attr(not(any(target_arch = "wasm32", test)), allow(dead_code))]
#[must_use]
pub(crate) fn timestamp_to_micros(timestamp: f64) -> u64 {
    if timestamp <= 0.0 || !timestamp.is_finite() {
        0
    } else {
        // Guarded above: value is finite and strictly positive, so the
        // sign-loss cast cannot lose information, and the float-to-int
        // `as` saturates to `u64::MAX` for the overflow case.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let micros = timestamp.round() as u64;
        micros
    }
}

#[cfg(test)]
mod timestamp_tests {
    use super::timestamp_to_micros;

    #[test]
    fn positive_rounds_to_nearest_micro() {
        assert_eq!(timestamp_to_micros(1_000.4), 1_000);
        assert_eq!(timestamp_to_micros(1_000.6), 1_001);
    }

    #[test]
    fn negative_clamps_to_zero() {
        assert_eq!(timestamp_to_micros(-5.0), 0);
    }

    #[test]
    fn zero_is_zero() {
        assert_eq!(timestamp_to_micros(0.0), 0);
    }

    #[test]
    fn non_finite_clamps_to_zero() {
        assert_eq!(timestamp_to_micros(f64::NAN), 0);
        assert_eq!(timestamp_to_micros(f64::INFINITY), 0);
        assert_eq!(timestamp_to_micros(f64::NEG_INFINITY), 0);
    }

    #[test]
    fn long_preview_timestamp_survives_past_i32_micros() {
        // ~40 minutes in µs — past the old i32-µs cap (~35.8 min) that
        // wrongly rejected long previews. Must round-trip intact.
        let forty_minutes_micros = 40.0 * 60.0 * 1_000_000.0;
        assert_eq!(timestamp_to_micros(forty_minutes_micros), 2_400_000_000);
    }
}
