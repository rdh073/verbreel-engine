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
