//! [`Frame`] — opaque v0 placeholder so [`encode`](crate::encode()) has a
//! complete signature.
//!
//! The real frame shape lives in `verbreel-render` and depends on the
//! wgpu color-buffer layout that has not yet been pinned. v0 carries
//! only the three fields Spike S1 will need at the boundary:
//! dimensions and a flat plane byte buffer. The plane interpretation
//! (YUV vs RGB, plane offsets, stride) is deliberately left
//! unspecified — Spike S1 will tighten this when it picks the
//! decoder→encoder pixel-format negotiation path.

/// Opaque pixel buffer for one frame.
///
/// Fields are private by design — the buffer layout (flat `Vec<u8>`
/// in v0) is unpinned and will gain per-plane offset / stride metadata
/// once `verbreel-render` lands its color-buffer layout. Keeping the
/// fields private means Spike S1 can tighten the shape without a
/// public API break; callers go through [`Frame::new`] and the
/// borrowed accessors only.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Frame {
    width: u32,
    height: u32,
    planes: Vec<u8>,
}

impl Frame {
    /// Build a frame from raw dimensions and a plane buffer.
    #[must_use]
    pub const fn new(width: u32, height: u32, planes: Vec<u8>) -> Self {
        Self {
            width,
            height,
            planes,
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
}
