//! Error type for the wasm browser preview engine.

use thiserror::Error;
use wasm_bindgen::JsValue;

/// Stable wire literal for the browser-no-persistence rejection.
///
/// Surfaced verbatim in [`WasmError::BrowserNoPersistence`] so JS
/// callers branch on a fixed token rather than free-form prose. The
/// decision (#404) keeps mutation off the browser surface — any future
/// in-memory browser preview path must surface this code.
pub const W_BROWSER_NO_PERSISTENCE: &str = "W_BROWSER_NO_PERSISTENCE";

/// Errors surfaced from the wasm engine surface.
///
/// The preview-session bridge (`open` / `seek` / `frame_at`) maps
/// codec-web decode failures onto [`Self::PreviewDecode`] and rejects
/// any mutation attempt with [`Self::BrowserNoPersistence`]. The legacy
/// `frame::run_frame` stub still returns [`Self::NotYetImplemented`]
/// until the wgpu composite path lands; `InvalidProjectJson` /
/// `EngineInternal` pin the rest of the JS-visible surface.
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

    /// A codec-web preview-session decode call failed. The `detail`
    /// carries the underlying [`verbreel_codec_web::DecodeError`]
    /// `Display` string so JS callers can present the browser decode
    /// reason. Mapped at the bridge boundary
    /// ([`crate::session::PreviewSession`]) so the wasm surface owns one
    /// flat error type instead of leaking codec-web's enum.
    #[error("verbreel-wasm preview decode failed: {detail}")]
    PreviewDecode {
        /// `Display` of the underlying codec-web `DecodeError`.
        detail: String,
    },

    /// A mutation / persistence path was attempted on the browser
    /// preview surface. Decision #404 keeps editing on native/HTTP/MCP;
    /// the browser embeds preview only. `Display` is the stable
    /// [`W_BROWSER_NO_PERSISTENCE`] token so JS callers route the user
    /// to a persistence-capable surface.
    #[error("{}", W_BROWSER_NO_PERSISTENCE)]
    BrowserNoPersistence,
}

/// `wasm-bindgen` requires a fallible exported method's error type to be
/// `Into<JsValue>`. JS callers receive the `Display` string so they can
/// branch on the stable error prefix (e.g. [`W_BROWSER_NO_PERSISTENCE`]).
impl From<WasmError> for JsValue {
    fn from(err: WasmError) -> Self {
        JsValue::from_str(&err.to_string())
    }
}
