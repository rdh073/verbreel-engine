//! Synthetic input PNGs for the multiply-blend harness.
//!
//! Input A: 1920×1080 horizontal gradient (R = x/W, G = 0.5, B = 1 - x/W).
//! Input B: 1920×1080 vertical gradient   (R = 0.5,   G = y/H, B = 1 - y/H).
//!
//! These two PNGs are written once on native, then byte-identically loaded
//! by both the native and the wasm runners — so any drift in the diff
//! must come from the GPU + WGSL pipeline, not from input decode.

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

use anyhow::{Context, Result};

pub const WIDTH: u32 = 1920;
pub const HEIGHT: u32 = 1080;

fn quantize(x: f32) -> u8 {
    // Round-to-nearest, clamp to [0, 255]. Match wgpu's rgba8unorm
    // store semantics (also round-to-nearest) so the input bytes are
    // a clean predictable function of (x, y).
    let v = (x.clamp(0.0, 1.0) * 255.0).round();
    v as u8
}

fn pixels_a(w: u32, h: u32) -> Vec<u8> {
    let mut buf = Vec::with_capacity((w * h * 4) as usize);
    let fw = (w - 1).max(1) as f32;
    for _y in 0..h {
        for x in 0..w {
            let fx = x as f32 / fw;
            buf.push(quantize(fx)); // R = x/W
            buf.push(quantize(0.5)); // G = 0.5
            buf.push(quantize(1.0 - fx)); // B = 1 - x/W
            buf.push(255); // A = 1.0
        }
    }
    buf
}

fn pixels_b(w: u32, h: u32) -> Vec<u8> {
    let mut buf = Vec::with_capacity((w * h * 4) as usize);
    let fh = (h - 1).max(1) as f32;
    for y in 0..h {
        for _x in 0..w {
            let fy = y as f32 / fh;
            buf.push(quantize(0.5)); // R = 0.5
            buf.push(quantize(fy)); // G = y/H
            buf.push(quantize(1.0 - fy)); // B = 1 - y/H
            buf.push(255); // A = 1.0
        }
    }
    buf
}

fn write_png(path: &Path, w: u32, h: u32, rgba: &[u8]) -> Result<()> {
    let file =
        File::create(path).with_context(|| format!("create PNG output {}", path.display()))?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    // Deterministic settings — no timestamp, fixed compression.
    encoder.set_compression(png::Compression::Balanced);
    let mut writer = encoder.write_header().context("write PNG header")?;
    writer
        .write_image_data(rgba)
        .context("write PNG image data")?;
    Ok(())
}

/// Write the horizontal-gradient input PNG.
pub fn write_input_a(path: &Path) -> Result<()> {
    write_png(path, WIDTH, HEIGHT, &pixels_a(WIDTH, HEIGHT))
}

/// Write the vertical-gradient input PNG.
pub fn write_input_b(path: &Path) -> Result<()> {
    write_png(path, WIDTH, HEIGHT, &pixels_b(WIDTH, HEIGHT))
}

/// Decode an RGBA8 PNG into a (width, height, bytes) tuple.
/// Caller asserts the PNG is RGBA8 — used for both the native and wasm
/// runners. The wasm runner uses `include_bytes!` + an in-memory Decoder
/// instead of file I/O but the byte layout is identical.
pub fn decode_rgba8_png(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>)> {
    // png 0.18 requires BufRead + Seek for in-memory decoding.
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info().context("PNG read_info")?;
    let buf_size = reader
        .output_buffer_size()
        .context("PNG output_buffer_size: image too large for usize")?;
    let mut buf = vec![0u8; buf_size];
    reader.next_frame(&mut buf).context("PNG next_frame")?;
    let info = reader.info();
    anyhow::ensure!(
        info.color_type == png::ColorType::Rgba,
        "expected RGBA PNG, got {:?}",
        info.color_type
    );
    anyhow::ensure!(
        info.bit_depth == png::BitDepth::Eight,
        "expected 8-bit PNG, got {:?}",
        info.bit_depth
    );
    Ok((info.width, info.height, buf))
}
