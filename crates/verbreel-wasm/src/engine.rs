//! `EngineHandle` — opaque entry-point for the browser preview engine.
//!
//! v0 holds only the project schema version mirror so JS callers can
//! confirm the wasm bundle's schema compatibility before passing
//! project.json bytes in. Future Spike S2 grows internals (wgpu
//! `Surface`, project graph reference, frame counter, async runtime)
//! without breaking the public method surface — the opaque-fields
//! pattern mirrors codec-native `Frame` / codec-web `DecodedFrame`.

use verbreel_state::SCHEMA_VERSION;

/// Browser-side engine lifecycle handle.
///
/// Constructed via [`EngineHandle::new`]. Holds the engine's schema
/// version so JS callers can branch on compatibility before issuing
/// project mutations. All compute-heavy methods (frame render, asset
/// upload, project apply) live on the bound `frame::*` / `project::*`
/// free functions and return [`crate::WasmError::NotYetImplemented`]
/// at v0.
///
/// **No `Clone`, no `Default`, no `Copy`** on purpose: Spike S2 grows
/// internals that cannot satisfy those traits (wgpu `Surface` is not
/// `Clone`; engine initialisation reads project data so there is no
/// meaningful default). Committing to a bare `new()` + accessor
/// surface keeps S2 free to add non-clone fields without breaking JS
/// callers.
#[derive(Debug)]
pub struct EngineHandle {
    schema_version: &'static str,
}

impl EngineHandle {
    /// Construct a fresh handle bound to the current
    /// [`verbreel_state::SCHEMA_VERSION`].
    // Default is intentionally NOT derived — see type-level doc.
    #[allow(clippy::new_without_default)]
    #[must_use]
    pub fn new() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
        }
    }

    /// Project schema version this engine recognises (`SemVer` string).
    /// JS callers branch on this before loading a project.
    #[must_use]
    pub fn schema_version(&self) -> &'static str {
        self.schema_version
    }
}
