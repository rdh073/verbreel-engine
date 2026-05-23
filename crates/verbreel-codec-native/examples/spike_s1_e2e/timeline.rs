//! Spike S1 slice C — 3-clip + 1-crossfade synthetic timeline per spec §11.
//!
//! Layout (240 frames @ 24 fps = 10.0 s):
//!
//!   frame   0..  99 : clip 0 alone           (Solo)
//!   frame 100..119 : clip 0 ↘  +  clip 1 ↗  (Crossfade, 20 frames)
//!   frame 120..179 : clip 1 alone            (Solo, 60 frames)
//!   frame 180..239 : clip 2 alone            (Solo, 60 frames)
//!
//! "Synthetic" = generated programmatically (no asset files). Each
//! generator constrains Y to [16, 235] and UV to 128 so the BT.709
//! limited-range matrix roundtrips bit-exact on the GPU (slice B
//! finding) and the resulting YUV is a valid limited-range signal that
//! libx264 won't have to crush.
//!
//!   clip 0 (120 frames): horizontal Y-gradient
//!   clip 1 ( 80 frames): vertical   Y-gradient
//!   clip 2 ( 60 frames): diagonal-stripe Y-pattern

pub const WIDTH: u32 = 1920;
pub const HEIGHT: u32 = 1080;
pub const FPS: u32 = 24;
pub const TOTAL_FRAMES: u32 = 240;
pub const CROSSFADE_START: u32 = 100;
pub const CROSSFADE_END: u32 = 120; // exclusive
pub const FRAME_BYTES: usize = (WIDTH * HEIGHT * 3 / 2) as usize;

pub enum FrameSource<'a> {
    /// One clip plays at this frame index — feed YUV through GPU as Solo.
    Solo(&'a [u8]),
    /// Two clips active — feed both, blend with `weight` on GPU.
    Cross {
        a: &'a [u8],
        b: &'a [u8],
        weight: f32,
    },
}

pub struct Timeline {
    clip0: Vec<u8>, // 120 frames (covers solo 0..99 + cross-A 100..119)
    clip1: Vec<u8>, // 80  frames (covers cross-B 100..119 + solo 120..179)
    clip2: Vec<u8>, // 60  frames (covers solo 180..239)
}

impl Timeline {
    pub fn new() -> Self {
        Self {
            clip0: gen_clip_horizontal(120),
            clip1: gen_clip_vertical(80),
            clip2: gen_clip_diagonal(60),
        }
    }

    /// What plays at frame index `i` (relative to the master timeline)?
    pub fn frame_at(&self, i: u32) -> FrameSource<'_> {
        let f = FRAME_BYTES;
        debug_assert!(i < TOTAL_FRAMES, "frame index {i} out of range");
        if i < CROSSFADE_START {
            // Solo clip0 — local index == master index
            let local = i as usize;
            FrameSource::Solo(&self.clip0[local * f..(local + 1) * f])
        } else if i < CROSSFADE_END {
            // Crossfade region — clip0[i] over clip1[i - CROSSFADE_START]
            let weight = (i - CROSSFADE_START) as f32 / (CROSSFADE_END - CROSSFADE_START) as f32;
            let a_idx = i as usize;
            let b_idx = (i - CROSSFADE_START) as usize;
            let a = &self.clip0[a_idx * f..(a_idx + 1) * f];
            let b = &self.clip1[b_idx * f..(b_idx + 1) * f];
            FrameSource::Cross { a, b, weight }
        } else if i < 180 {
            // Solo clip1 — local index = i - CROSSFADE_START
            let local = (i - CROSSFADE_START) as usize;
            FrameSource::Solo(&self.clip1[local * f..(local + 1) * f])
        } else {
            // Solo clip2 — local index = i - 180
            let local = (i - 180) as usize;
            FrameSource::Solo(&self.clip2[local * f..(local + 1) * f])
        }
    }
}

/// Generate a clip of `n_frames` 1920×1080 YUV420P frames where the Y
/// plane is a horizontal gradient clamped to [16, 235], UV planes are
/// constant 128. Frame index is encoded in the top-left 4 bytes for
/// differentiation (libx264 won't collapse to I+skip).
fn gen_clip_horizontal(n_frames: usize) -> Vec<u8> {
    gen_clip_with(n_frames, |x, _y| 16 + ((x as u32 * 219 / 1920) as u8))
}

fn gen_clip_vertical(n_frames: usize) -> Vec<u8> {
    gen_clip_with(n_frames, |_x, y| 16 + ((y as u32 * 219 / 1080) as u8))
}

fn gen_clip_diagonal(n_frames: usize) -> Vec<u8> {
    gen_clip_with(n_frames, |x, y| {
        // Diagonal stripes — Y oscillates as a function of (x + y).
        let v = ((x + y) as u32 % 256) as u8;
        // Quantize to [16, 235]
        16 + (((v as u32) * 219) / 255) as u8
    })
}

fn gen_clip_with(n_frames: usize, y_at: impl Fn(usize, usize) -> u8) -> Vec<u8> {
    let w = WIDTH as usize;
    let h = HEIGHT as usize;
    let y_plane = w * h;
    let uv_plane = (w / 2) * (h / 2);
    let frame_bytes = y_plane + 2 * uv_plane;
    let mut out = vec![0u8; frame_bytes * n_frames];

    for f in 0..n_frames {
        let base = f * frame_bytes;
        for y in 0..h {
            let row_start = base + y * w;
            for x in 0..w {
                out[row_start + x] = y_at(x, y);
            }
        }
        // Encode frame index in top-left 4-byte block (overwrites those
        // 4 gradient cells — fine, the clip-identification matters more).
        let counter = (f as u32).to_le_bytes();
        out[base..base + 4].copy_from_slice(&counter);

        // UV = 128 (chroma midpoint). Both planes.
        out[base + y_plane..base + y_plane + uv_plane].fill(128);
        out[base + y_plane + uv_plane..base + y_plane + 2 * uv_plane].fill(128);
    }

    out
}
