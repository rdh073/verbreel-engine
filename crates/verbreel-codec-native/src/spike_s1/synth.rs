//! Synthetic raw YUV420P frame generator.
//!
//! Pure-Rust, no IO. Produces a deterministic test pattern: a horizontal
//! gradient on the Y plane with the frame index encoded as a 32-bit LE
//! integer in the top-left 64×64 block. The encoded counter ensures
//! every frame differs from its neighbors — otherwise x264 would emit
//! one I-frame followed by skip P-frames, defeating the determinism
//! test on the encoder.
//!
//! UV planes are held at chroma midpoint (128), i.e. grayscale.
//! Output is tightly packed planar YUV420P: `W*H` bytes Y, then
//! `(W/2)*(H/2)` bytes U, then same for V.

/// Generate `frame_count` raw YUV420P frames at `width × height`.
///
/// Total length = `frame_count * width * height * 3 / 2` bytes.
///
/// # Panics
///
/// Panics if `width` or `height` is not divisible by 2 (YUV420P requires
/// even dimensions for chroma subsampling), or if the requested size
/// overflows `usize`.
pub fn generate_raw_yuv420p(width: u32, height: u32, frame_count: u32) -> Vec<u8> {
    assert!(width.is_multiple_of(2), "width must be even for YUV420P");
    assert!(height.is_multiple_of(2), "height must be even for YUV420P");

    let w = width as usize;
    let h = height as usize;
    let y_plane = w * h;
    let uv_plane = (w / 2) * (h / 2);
    let frame_bytes = y_plane + 2 * uv_plane;

    let total = frame_bytes
        .checked_mul(frame_count as usize)
        .expect("output size overflow");
    let mut out = vec![0u8; total];

    for f in 0..frame_count as usize {
        let base = f * frame_bytes;

        // Y plane: horizontal gradient (column index, wrapped).
        for y in 0..h {
            let row_start = base + y * w;
            for x in 0..w {
                out[row_start + x] = (x as u8).wrapping_add(y as u8);
            }
        }

        // Encode frame index as 32-bit LE in the top-left 4-byte cell.
        // This single mutation is enough to differentiate the bitstream
        // per frame; the surrounding 64×64 block stays gradient so the
        // change is small but always non-zero.
        let counter = (f as u32).to_le_bytes();
        for (i, b) in counter.iter().enumerate() {
            out[base + i] = *b;
        }

        // U and V planes: chroma midpoint (grayscale).
        let u_off = base + y_plane;
        let v_off = u_off + uv_plane;
        out[u_off..u_off + uv_plane].fill(128);
        out[v_off..v_off + uv_plane].fill(128);
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_length_matches_yuv420p_formula() {
        let bytes = generate_raw_yuv420p(64, 64, 3);
        assert_eq!(bytes.len(), 64 * 64 * 3 / 2 * 3);
    }

    #[test]
    fn frame_counter_differs_per_frame() {
        let bytes = generate_raw_yuv420p(64, 64, 5);
        let frame_bytes = 64 * 64 * 3 / 2;
        let counters: Vec<u32> = (0..5)
            .map(|f| {
                let base = f * frame_bytes;
                u32::from_le_bytes([
                    bytes[base],
                    bytes[base + 1],
                    bytes[base + 2],
                    bytes[base + 3],
                ])
            })
            .collect();
        assert_eq!(counters, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn uv_planes_are_chroma_midpoint() {
        let bytes = generate_raw_yuv420p(64, 64, 1);
        let y = 64 * 64;
        let uv = 32 * 32;
        assert!(bytes[y..y + uv].iter().all(|&b| b == 128));
        assert!(bytes[y + uv..y + 2 * uv].iter().all(|&b| b == 128));
    }
}
