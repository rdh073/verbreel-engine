//! Error type for the wasm browser preview engine.

use thiserror::Error;

/// Errors surfaced from the wasm engine surface.
///
/// v0 ships three variants. The implementation body of every public
/// method returns [`Self::NotYetImplemented`] until Spike S2 lands the
/// real wgpu + `WebCodecs` path. The other two variants are committed
/// now so JS callers can pattern-match against the eventual surface
/// without a breaking change at S2 ship.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum WasmError {
    /// Method is not implemented at the v0 type surface; arrives with
    /// Spike S2. The `detail` payload cites the spec section so the
    /// JS-side fallback can branch on it.
    #[error("verbreel-wasm method not yet implemented: {detail}")]
    NotYetImplemented {
        /// Free-form pointer to where the real impl lands (e.g. the
        /// canonical Spike S2 deferral string).
        detail: String,
    },

    /// Caller passed a project.json blob that fails pre-validation
    /// (schema mismatch, malformed JSON, version skew). Distinct from
    /// `EngineInternal` so JS callers can present a user-facing
    /// "your project file is invalid" instead of a generic crash.
    #[error("invalid project json supplied to verbreel-wasm: {detail}")]
    InvalidProjectJson {
        /// What about the project JSON was malformed.
        detail: String,
    },

    /// wgpu / `WebCodecs` / runtime errors mapped here. v0 path produces
    /// no real instances of this; included to pin the eventual error
    /// surface so JS callers do not see a breaking add at S2 ship.
    #[error("verbreel-wasm engine internal error: {detail}")]
    EngineInternal {
        /// Underlying error message bubbled up from wgpu / `WebCodecs`
        /// / the host runtime.
        detail: String,
    },
}
