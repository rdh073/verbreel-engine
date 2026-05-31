//! §0.2 duration-string → integer ticks, resolved CLI-side before the
//! engine sees the value.
//!
//! The tick rate is fixed at [`verbreel_state::TICK_RATE_HZ`] (240,000
//! Hz). The four accepted duration forms (spec §0.2):
//!
//! - `5s` — seconds (decimal allowed): `value * 240000`.
//! - `5000ms` — milliseconds (decimal allowed): `value * 240`.
//! - `1200000tk` — raw ticks (decimal allowed but pointless): `value`.
//! - `00:00:05.000` — `HH:MM:SS` or `HH:MM:SS.mmm` clock form.
//!
//! Conversion rounds to the **nearest** integer tick. A bare integer
//! (`1200000`) is deliberately **not** matched here — it is already a
//! tick count and is handled by the plain `i64` coercion branch, so the
//! caller never double-converts it.

use verbreel_state::TICK_RATE_HZ;

/// Parse a §0.2 duration string to its integer tick count, or `None` if
/// `raw` does not match the duration grammar.
///
/// `None` is the signal "this is not a duration" — the caller falls
/// through to numeric / string coercion. This is intentionally a *try*:
/// the CLI has no schema telling it which flags are time-typed, so it
/// converts by grammar match, not by flag name.
#[must_use]
pub fn duration_str_to_ticks(raw: &str) -> Option<i64> {
    if let Some(ticks) = parse_clock(raw) {
        return Some(ticks);
    }
    parse_suffixed(raw)
}

/// `<number><suffix>` where suffix is `s` / `ms` / `tk`. The number may
/// be a non-negative decimal. A bare number with no suffix returns
/// `None` (it is already ticks; the i64 branch owns it).
fn parse_suffixed(raw: &str) -> Option<i64> {
    let rate = f64::from(TICK_RATE_HZ);
    // Order matters: `ms` must be tested before `s`, since `s` is a
    // suffix of `ms`.
    let (number, ticks_per_unit) = if let Some(n) = raw.strip_suffix("ms") {
        (n, rate / 1000.0)
    } else if let Some(n) = raw.strip_suffix("tk") {
        (n, 1.0)
    } else if let Some(n) = raw.strip_suffix('s') {
        (n, rate)
    } else {
        return None;
    };

    let value: f64 = number.parse().ok()?;
    if !value.is_finite() || value < 0.0 {
        return None;
    }
    Some(round_to_i64(value * ticks_per_unit))
}

/// `HH:MM:SS` or `HH:MM:SS.mmm`. Requires exactly three colon-separated
/// fields; the seconds field may carry a fractional part.
fn parse_clock(raw: &str) -> Option<i64> {
    let mut parts = raw.split(':');
    let hh = parts.next()?;
    let mm = parts.next()?;
    let ss = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    // Reject empty fields and a stray leading sign — clock time is
    // non-negative and fully specified.
    if hh.is_empty() || mm.is_empty() || ss.is_empty() {
        return None;
    }

    let hours: i64 = hh.parse().ok()?;
    let minutes: i64 = mm.parse().ok()?;
    let seconds: f64 = ss.parse().ok()?;
    if !seconds.is_finite() || seconds < 0.0 {
        return None;
    }

    // Keep the integer hours/minutes in integer arithmetic (exact at
    // 240,000 Hz) and only the fractional-seconds field through f64, so
    // there is no u64→f64 precision loss on the whole-time path.
    let rate = i64::from(TICK_RATE_HZ);
    let whole_ticks = (hours.checked_mul(3600)? + minutes.checked_mul(60)?).checked_mul(rate)?;
    let frac_ticks = round_to_i64(seconds * f64::from(TICK_RATE_HZ));
    Some(whole_ticks.saturating_add(frac_ticks))
}

/// Round a non-negative tick float to the nearest `i64`, saturating
/// rather than wrapping on the (physically unreachable) overflow path.
///
/// `2^63` as an exact `f64` literal is the saturation threshold — a
/// rounded value at or above it does not fit in `i64`. Comparing against
/// the literal (not `i64::MAX as f64`) keeps the bound exact and avoids
/// a precision-loss cast.
#[allow(clippy::cast_possible_truncation)]
fn round_to_i64(value: f64) -> i64 {
    const I64_MAX_AS_F64: f64 = 9_223_372_036_854_775_808.0; // 2^63
    let rounded = value.round();
    if rounded >= I64_MAX_AS_F64 {
        i64::MAX
    } else {
        rounded as i64
    }
}
