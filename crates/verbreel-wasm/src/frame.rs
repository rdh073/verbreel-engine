//! Per-tick frame render entry point. v0 returns
//! [`WasmError::NotYetImplemented`]; the real wgpu render path lands
//! with Spike S2 (Research 01 §11).

use crate::engine::EngineHandle;
use crate::error::WasmError;

/// Canonical Spike-S2 deferral string. Pinned so refactors cannot
/// silently drift the citation a future reader needs to find the
/// design context.
pub(crate) const SPIKE_S2_DEFERRAL_DETAIL: &str =
    "browser preview render path lands with Spike S2 (Research 01 §11)";

/// Drive one tick of the browser preview render loop at `at_tk`.
///
/// v0 always returns
/// [`WasmError::NotYetImplemented`](crate::error::WasmError::NotYetImplemented)
/// with [`SPIKE_S2_DEFERRAL_DETAIL`] as the body. The signature is
/// committed at v0 so JS callers can wire up their render loop today;
/// behaviour arrives when the real wgpu + `WebCodecs` glue lands.
///
/// `at_tk` is the engine-tick timestamp (240,000 Hz base per §0.2).
///
/// # Errors
///
/// Returns [`WasmError::NotYetImplemented`] unconditionally at v0.
/// Future variants ([`WasmError::EngineInternal`] for wgpu surface
/// failures, etc.) arrive with Spike S2.
pub fn run_frame(_handle: &mut EngineHandle, _at_tk: i64) -> Result<(), WasmError> {
    Err(WasmError::NotYetImplemented {
        detail: SPIKE_S2_DEFERRAL_DETAIL.to_string(),
    })
}
