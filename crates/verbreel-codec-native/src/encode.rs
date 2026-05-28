//! [`encode`] — v0 entry point.
//!
//! The body returns [`CodecError::NotYetImplemented`] in v0. The real
//! rsmpeg+libx264 pipeline lands with Spike S1 per Research 01 §11:
//! rsmpeg decode → wgpu YUV→RGB → wgpu composite → wgpu RGB→YUV →
//! rsmpeg libx264 encode.
//!
//! Surface shape is stable now so downstream crates (`verbreel-render`,
//! `verbreel-state::verbs::render_*`) can wire against this signature
//! without waiting for S1 to land.

use crate::error::CodecError;
use crate::frame::Frame;
use crate::params::EncodeParams;

/// Spike S1 deferral marker — surfaced verbatim in
/// [`CodecError::NotYetImplemented::detail`] so callers (and the
/// orchestrator) can grep for the exact string.
pub(crate) const SPIKE_S1_DEFERRAL_DETAIL: &str =
    "rsmpeg+libx264 path lands with Spike S1 (Research 01 §11)";

/// Encode a sequence of frames into a container byte blob.
///
/// # Errors
///
/// v0 stub always returns [`CodecError::NotYetImplemented`] with
/// [`SPIKE_S1_DEFERRAL_DETAIL`] as the `detail`. Real error variants
/// ([`CodecError::InvalidParams`], [`CodecError::EncoderInternal`],
/// [`CodecError::InputExhausted`]) come online when Spike S1 lands
/// the real pipeline.
pub fn encode(_params: &EncodeParams, _frames: &[Frame]) -> Result<Vec<u8>, CodecError> {
    Err(CodecError::NotYetImplemented {
        detail: SPIKE_S1_DEFERRAL_DETAIL.to_string(),
    })
}
