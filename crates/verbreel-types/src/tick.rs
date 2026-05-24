//! Time and ticks per spec §0.2.
//!
//! All time in Verbreel is integer ticks at 240,000 Hz. Never floats.

use serde::{Deserialize, Serialize};

/// Spec §0.2 fixed tick rate (240,000 Hz). LCM of common video framerates so
/// 23.976/24/25/29.97/30/48/50/59.94/60 all land on integer tick boundaries.
pub const TICK_RATE_HZ: u32 = 240_000;

/// An integer tick count at [`TICK_RATE_HZ`]. Range covers ~1.19 trillion years
/// at 240 kHz; `i64` is exactly the spec-mandated representation (spec §0.2
/// "≥53-bit signed integer").
///
/// `Tick` is a transparent newtype: serialized as a JSON number, not an object.
/// This matches the project-schema.json shape where tick fields are bare integers.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Tick(pub i64);

impl Tick {
    /// Tick value 0. Same as `Tick::default()`.
    pub const ZERO: Tick = Tick(0);

    /// Construct from a raw tick count.
    #[must_use]
    pub const fn new(raw: i64) -> Self {
        Tick(raw)
    }

    /// Inner i64 value.
    #[must_use]
    pub const fn get(self) -> i64 {
        self.0
    }

    /// Convert to seconds as f64. Lossy by design — for display only, never for
    /// engine arithmetic. Engine math always stays in i64 tick-space.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn as_seconds_f64(self) -> f64 {
        self.0 as f64 / f64::from(TICK_RATE_HZ)
    }
}

impl std::ops::Add for Tick {
    type Output = Tick;
    fn add(self, rhs: Self) -> Self::Output {
        Tick(self.0 + rhs.0)
    }
}

impl std::ops::Sub for Tick {
    type Output = Tick;
    fn sub(self, rhs: Self) -> Self::Output {
        Tick(self.0 - rhs.0)
    }
}

/// A frame rate expressed as a rational `fps_num/fps_den` (per project-schema.json).
///
/// 23.976 is `TickRate { fps_num: 24000, fps_den: 1001 }`, 30 is `{ 30, 1 }`, etc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TickRate {
    /// Numerator (e.g. 24000, 30, 60).
    pub fps_num: u32,
    /// Denominator (e.g. 1001 for NTSC, 1 for integer rates).
    pub fps_den: u32,
}

impl TickRate {
    /// Spec §0.2: ticks per frame at this rate. Returns `None` if the divisor is
    /// zero (impossible from spec-valid input but kept honest at type level).
    ///
    /// Formula: `TICK_RATE_HZ × fps_den / fps_num`.
    /// Exact iff `fps_num` divides `TICK_RATE_HZ × fps_den`; spec §0.2 documents
    /// `W_FPS_INEXACT` for the inexact case.
    #[must_use]
    pub fn ticks_per_frame(self) -> Option<Tick> {
        if self.fps_num == 0 {
            return None;
        }
        let n = i64::from(TICK_RATE_HZ) * i64::from(self.fps_den);
        Some(Tick(n / i64::from(self.fps_num)))
    }

    /// Spec §0.2: the divisibility check `fps_num | (TICK_RATE_HZ × fps_den)`.
    /// If false, callers must emit `W_FPS_INEXACT`.
    #[must_use]
    pub fn is_exact_at_240k(self) -> bool {
        if self.fps_num == 0 {
            return false;
        }
        let n = u64::from(TICK_RATE_HZ) * u64::from(self.fps_den);
        n % u64::from(self.fps_num) == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_serde_is_bare_integer() {
        let t = Tick(8008);
        let s = serde_json::to_string(&t).unwrap();
        assert_eq!(
            s, "8008",
            "Tick must serialize as a bare i64, not an object"
        );
        let back: Tick = serde_json::from_str("8008").unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn ticks_per_frame_matches_spec_table() {
        // Spec §0.2 §Time and ticks reference table:
        let cases = [
            ((24000, 1001), 10010),
            ((24, 1), 10000),
            ((25, 1), 9600),
            ((30000, 1001), 8008),
            ((30, 1), 8000),
            ((48, 1), 5000),
            ((50, 1), 4800),
            ((60000, 1001), 4004),
            ((60, 1), 4000),
        ];
        for ((num, den), expected) in cases {
            let rate = TickRate {
                fps_num: num,
                fps_den: den,
            };
            assert_eq!(
                rate.ticks_per_frame(),
                Some(Tick(expected)),
                "spec §0.2 table: {num}/{den} should be {expected} ticks/frame"
            );
            assert!(
                rate.is_exact_at_240k(),
                "{num}/{den} should be exact at 240kHz"
            );
        }
    }

    #[test]
    fn inexact_rate_is_flagged() {
        // Spec §0.2 example: 7/1 fails divisibility (240000 / 7 = 34285.71…)
        let rate = TickRate {
            fps_num: 7,
            fps_den: 1,
        };
        assert!(!rate.is_exact_at_240k());
    }

    #[test]
    fn arithmetic_is_pure_integer() {
        let a = Tick(8008);
        let b = Tick(4004);
        assert_eq!((a + b).get(), 12012);
        assert_eq!((a - b).get(), 4004);
    }
}
