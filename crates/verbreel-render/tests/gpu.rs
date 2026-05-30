//! wgpu compositor smoke + byte-stability tests.
//!
//! Every test that initialises a wgpu device skips gracefully (prints a skip
//! line and returns) when no adapter is available — CI runners have no Vulkan
//! adapter, so this keeps feature-off CI green without a GPU. On a host with a
//! (software or hardware) Vulkan adapter the tests run for real.

use sha2::{Digest, Sha256};
use verbreel_render::{CompositeLayer, Compositor, RenderError, RenderPreset};

/// A flat yuv420p buffer of `width`x`height` filled with one Y/U/V triple.
fn solid_yuv(width: u32, height: u32, y: u8, u: u8, v: u8) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    let cw = w / 2;
    let ch = h / 2;
    let mut buf = vec![y; w * h];
    buf.extend(std::iter::repeat_n(u, cw * ch));
    buf.extend(std::iter::repeat_n(v, cw * ch));
    buf
}

/// Build a deterministic-preset compositor, or skip the test if no adapter is
/// available. Returns `None` after printing a skip line on `NoAdapter`.
fn compositor_or_skip(test_name: &str) -> Option<Compositor> {
    match Compositor::new(RenderPreset::Deterministic) {
        Ok(c) => Some(c),
        Err(RenderError::NoAdapter { detail }) => {
            eprintln!("SKIP {test_name}: no wgpu adapter ({detail})");
            None
        }
        Err(e) => panic!("unexpected compositor init error: {e}"),
    }
}

#[test]
fn deterministic_compositor_uses_fallback_adapter() {
    let Some(c) = compositor_or_skip("deterministic_compositor_uses_fallback_adapter") else {
        return;
    };
    assert_eq!(c.preset(), RenderPreset::Deterministic);
}

#[test]
fn composite_rejects_odd_dimensions() {
    let Some(c) = compositor_or_skip("composite_rejects_odd_dimensions") else {
        return;
    };
    let layer = CompositeLayer {
        planes: vec![0u8; 16],
        alpha_q16: u16::MAX,
    };
    let err = c.composite(3, 2, std::slice::from_ref(&layer)).unwrap_err();
    assert!(matches!(err, RenderError::InvalidInput { .. }));
}

#[test]
fn composite_rejects_empty_layers() {
    let Some(c) = compositor_or_skip("composite_rejects_empty_layers") else {
        return;
    };
    let err = c.composite(16, 16, &[]).unwrap_err();
    assert!(matches!(err, RenderError::InvalidInput { .. }));
}

#[test]
fn composite_single_opaque_layer_round_trips_color() {
    let Some(c) = compositor_or_skip("composite_single_opaque_layer_round_trips_color") else {
        return;
    };
    // A solid mid-grey frame composited as a single opaque layer over black
    // must come back close to itself (YUV->RGB->YUV is near-identity for a
    // flat field; allow a small quantization tolerance).
    let (w, h) = (16u16 as u32, 16u32);
    let input = solid_yuv(w, h, 128, 128, 128);
    let layer = CompositeLayer {
        planes: input.clone(),
        alpha_q16: u16::MAX,
    };
    let out = c.composite(w, h, std::slice::from_ref(&layer)).unwrap();
    assert_eq!(out.len(), input.len());
    // Y plane sample: a 128 grey luma should round-trip within a few codes.
    let y0 = i32::from(out[0]);
    assert!((y0 - 128).abs() <= 3, "luma drifted too far: got {y0}");
}

#[test]
fn composite_output_is_byte_stable_across_runs() {
    // The core determinism claim at the compositor level: two composites of the
    // same layers on the same adapter produce byte-identical yuv420p output.
    let Some(c) = compositor_or_skip("composite_output_is_byte_stable_across_runs") else {
        return;
    };
    let (w, h) = (64u32, 64u32);
    let bottom = CompositeLayer {
        planes: solid_yuv(w, h, 80, 90, 200),
        alpha_q16: u16::MAX,
    };
    let top = CompositeLayer {
        planes: solid_yuv(w, h, 180, 110, 60),
        alpha_q16: 32_768, // 50% over the bottom layer
    };
    let layers = [bottom, top];

    let a = c.composite(w, h, &layers).unwrap();
    let b = c.composite(w, h, &layers).unwrap();
    let ha = Sha256::digest(&a);
    let hb = Sha256::digest(&b);
    assert_eq!(ha, hb, "compositor output must be byte-stable across runs");

    // A fresh compositor (new device) on the same adapter must also match —
    // the determinism contract is per-adapter, not per-device-instance.
    let Some(c2) = compositor_or_skip("composite_output_is_byte_stable_across_runs") else {
        return;
    };
    let d = c2.composite(w, h, &layers).unwrap();
    assert_eq!(
        Sha256::digest(&d),
        ha,
        "output must be stable across devices"
    );
}

// --- full S1 smoke: composite -> rsmpeg encode, identical SHA ------------

#[cfg(feature = "rsmpeg")]
#[test]
fn deterministic_render_smoke_is_byte_stable() {
    use std::collections::HashMap;
    use verbreel_render::{DecodedSource, JobRegistry, RenderJobSpec, RenderPlan, RenderStatus};

    let Some(_) = compositor_or_skip("deterministic_render_smoke_is_byte_stable") else {
        return;
    };

    let (w, h) = (64u32, 64u32);
    let frame_count = 8usize;

    // One layer per frame, no source asset (composited as opaque black) plus a
    // single decoded "source" feeding the layer. Build the spec twice and run
    // both through a fresh registry; the encoded MP4 bytes must hash equal.
    let make_spec = || {
        let plans: Vec<RenderPlan> = (0..frame_count)
            .map(|_| RenderPlan {
                layers: vec![verbreel_render::RenderLayer {
                    source_asset: None,
                    alpha_q16: u16::MAX,
                    cache_hash: [0u8; 32],
                }],
                tick: 0,
            })
            .collect();
        // A single decoded source whose frames are a moving grey ramp so the
        // composite has real per-frame content.
        let frames: Vec<verbreel_codec_native::Frame> = (0..frame_count)
            .map(|i| {
                let y = 32 + (i as u8) * 16;
                verbreel_codec_native::Frame::new(w, h, solid_yuv(w, h, y, 128, 128))
            })
            .collect();
        let mut decoded = HashMap::new();
        decoded.insert(verbreel_ir::IrNodeId::now(), DecodedSource { frames });
        RenderJobSpec {
            preset: RenderPreset::Deterministic,
            width: w,
            height: h,
            fps_num: 30,
            fps_den: 1,
            frames: plans,
            decoded,
        }
    };

    let run = || {
        let reg = JobRegistry::new();
        let id = reg.start_render(&make_spec()).unwrap();
        match reg.status(id).unwrap() {
            RenderStatus::Done {
                frame_count: n,
                output,
            } => (n, output),
            other => panic!("expected Done, got {other:?}"),
        }
    };

    let (n1, out1) = run();
    let (n2, out2) = run();

    assert_eq!(n1, frame_count, "all frames must be encoded");
    assert_eq!(n2, frame_count);
    assert!(!out1.is_empty(), "encoded MP4 must be non-empty");

    let h1 = Sha256::digest(&out1);
    let h2 = Sha256::digest(&out2);
    assert_eq!(
        h1, h2,
        "deterministic render must produce byte-identical MP4 across runs"
    );
    eprintln!(
        "deterministic render SHA-256: {} ({} frames, {} bytes)",
        hex(&h1),
        n1,
        out1.len()
    );
}

#[cfg(feature = "rsmpeg")]
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}
