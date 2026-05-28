//! AI provider opaque handle + per-invocation entry point.
//!
//! v0 commits the public surface; real `ort` Session loading + Python
//! sidecar spawn lands with the AI orchestration spike.

use crate::capability::Capability;
use crate::error::AiError;

/// Canonical AI-spike deferral string. Pinned so refactors cannot
/// silently drift the citation a future reader needs to find the
/// design context.
pub(crate) const SPIKE_AI_DEFERRAL_DETAIL: &str =
    "ort runtime + Python sidecar dispatch lands with the AI orchestration spike (Research 04 §3)";

/// AI provider handle.
///
/// Opaque by design — real impl grows non-clone state inside
/// (`ort::Session`, `tokio::process::Child` for sidecars, model
/// weight buffers). These traits do not survive the upgrade:
///
/// - No `Clone` (`ort::Session` is not `Clone` in general).
/// - No `Default` (initialisation requires a [`Capability`]).
/// - No `Copy`.
///
/// Same trait-discipline as `verbreel_render::Pipeline`,
/// `verbreel_wasm::EngineHandle`, codec-native `Frame`, codec-web
/// `DecodedFrame`.
#[derive(Debug)]
pub struct Provider {
    capability: Capability,
}

impl Provider {
    /// Construct a provider bound to a [`Capability`]. v0 stores the
    /// capability only; the real impl loads the corresponding model
    /// or spawns the sidecar here.
    #[must_use]
    pub fn new(capability: Capability) -> Self {
        Self { capability }
    }

    /// The [`Capability`] this provider was constructed against.
    #[must_use]
    pub fn capability(&self) -> Capability {
        self.capability
    }
}

/// Run an inference / analysis pass through the provider with the
/// caller-supplied input bytes. v0 returns
/// [`AiError::NotYetImplemented`] unconditionally with
/// [`SPIKE_AI_DEFERRAL_DETAIL`] as the body. The real impl replaces
/// the body with `ort::Session::run` (engine-side capabilities) or
/// sidecar JSON-stdio (`BeatNet` etc.).
///
/// # Errors
///
/// Returns [`AiError::NotYetImplemented`] unconditionally at v0.
/// Future variants ([`AiError::SidecarLaunchFailed`],
/// [`AiError::ModelLoadFailed`], [`AiError::ProcessExited`]) arrive
/// with the real implementation.
pub fn run(_provider: &mut Provider, _input_bytes: &[u8]) -> Result<Vec<u8>, AiError> {
    Err(AiError::NotYetImplemented {
        detail: SPIKE_AI_DEFERRAL_DETAIL.to_string(),
    })
}
