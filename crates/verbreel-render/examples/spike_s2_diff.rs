//! Spike S2 diff — compares `tmp/spike_s2/native_frame.png` and
//! `tmp/spike_s2/wasm_frame.png` pixel-by-pixel and emits diff_report.txt
//! plus a stdout summary.
//!
//! Usage:
//!   cargo run --release -p verbreel-render --features spike-s2 --example spike_s2_diff

use std::path::Path;

fn read_rgba(p: &Path) -> anyhow::Result<(u32, u32, Vec<u8>)> {
    let bytes = std::fs::read(p)?;
    verbreel_render::spike_s2::synth::decode_rgba8_png(&bytes)
}

fn main() -> anyhow::Result<()> {
    let native = Path::new("tmp/spike_s2/native_frame.png");
    let wasm = Path::new("tmp/spike_s2/wasm_frame.png");
    let (nw, nh, na) = read_rgba(native)?;
    let (ww, wh, wa) = read_rgba(wasm)?;
    anyhow::ensure!(
        nw == ww && nh == wh,
        "size mismatch: native {nw}×{nh} vs wasm {ww}×{wh}"
    );
    anyhow::ensure!(na.len() == wa.len(), "byte-len mismatch");

    let n_px = (nw * nh) as usize;
    let mut max_dr = 0u8;
    let mut max_dg = 0u8;
    let mut max_db = 0u8;
    let mut max_pixel_delta = 0u8;
    let mut px_with_any_drift = 0usize;
    let mut px_above_1 = 0usize;
    let mut px_above_2 = 0usize;
    let mut dr_gt1 = 0usize;
    let mut dg_gt1 = 0usize;
    let mut db_gt1 = 0usize;

    for i in 0..n_px {
        let n = &na[i * 4..i * 4 + 4];
        let w = &wa[i * 4..i * 4 + 4];
        let dr = n[0].abs_diff(w[0]);
        let dg = n[1].abs_diff(w[1]);
        let db = n[2].abs_diff(w[2]);
        max_dr = max_dr.max(dr);
        max_dg = max_dg.max(dg);
        max_db = max_db.max(db);
        let px_max = dr.max(dg).max(db);
        max_pixel_delta = max_pixel_delta.max(px_max);
        if px_max > 0 {
            px_with_any_drift += 1;
        }
        if px_max > 1 {
            px_above_1 += 1;
        }
        if px_max > 2 {
            px_above_2 += 1;
        }
        if dr > 1 {
            dr_gt1 += 1;
        }
        if dg > 1 {
            dg_gt1 += 1;
        }
        if db > 1 {
            db_gt1 += 1;
        }
    }

    let drift_pct = 100.0 * px_with_any_drift as f64 / n_px as f64;
    let above1_pct = 100.0 * px_above_1 as f64 / n_px as f64;
    let above2_pct = 100.0 * px_above_2 as f64 / n_px as f64;
    let pct_clean = 100.0 * (n_px - px_with_any_drift) as f64 / n_px as f64;
    let pct_within_1 = 100.0 * (n_px - px_above_1) as f64 / n_px as f64;
    let pct_within_2 = 100.0 * (n_px - px_above_2) as f64 / n_px as f64;

    // §11 PASS: |ΔRGB| ≤ 1/255 for ≥99.9% pixels AND max |Δ| ≤ 2/255.
    let pass_within_1 = pct_within_1 >= 99.9;
    let pass_max_2 = max_pixel_delta <= 2;
    let verdict = if pass_within_1 && pass_max_2 {
        "PASS"
    } else if pass_within_1 {
        "PASS-PROVISIONAL (max >2)"
    } else if pass_max_2 {
        "FAIL (≥0.1% drift > 1)"
    } else {
        "FAIL (both criteria)"
    };

    let report = format!(
        "SPIKE S2 — Cross-target pixel diff\n\
Native:  tmp/spike_s2/native_frame.png ({nw}×{nh})\n\
Wasm:    tmp/spike_s2/wasm_frame.png\n\
\n\
Pixels total:               {n_px}\n\
Pixels with any |Δ| > 0:    {px_with_any_drift} ({drift_pct:.4}%)\n\
Pixels with any |Δ| > 1:    {px_above_1} ({above1_pct:.4}%)\n\
Pixels with any |Δ| > 2:    {px_above_2} ({above2_pct:.4}%)\n\
Pixels clean (no drift):    {pct_clean:.4}%\n\
Pixels within ≤1 tolerance: {pct_within_1:.4}%\n\
Pixels within ≤2 tolerance: {pct_within_2:.4}%\n\
\n\
Per-channel |Δ| > 1 counts:\n\
  |ΔR| > 1: {dr_gt1}\n\
  |ΔG| > 1: {dg_gt1}\n\
  |ΔB| > 1: {db_gt1}\n\
\n\
Max channel deltas:\n\
  |ΔR| max: {max_dr}\n\
  |ΔG| max: {max_dg}\n\
  |ΔB| max: {max_db}\n\
  per-pixel max: {max_pixel_delta}\n\
\n\
§11 S2 Pass criteria:\n\
  ≥99.9% pixels within ≤1:  {pct_within_1:.4}%  → {p1}\n\
  max per-pixel ≤2:         {max_pixel_delta}     → {p2}\n\
\n\
Verdict: {verdict}\n",
        p1 = if pass_within_1 { "PASS" } else { "FAIL" },
        p2 = if pass_max_2 { "PASS" } else { "FAIL" },
    );

    std::fs::write("tmp/spike_s2/diff_report.txt", &report)?;
    print!("{report}");
    Ok(())
}
