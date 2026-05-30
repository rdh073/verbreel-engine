//! `EngineHandle` — opaque entry-point for the browser preview engine.
//!
//! v0 holds only the project schema version mirror so JS callers can
//! confirm the wasm bundle's schema compatibility before passing
//! project.json bytes in. Future Spike S2 grows internals (wgpu
//! `Surface`, project graph reference, frame counter, async runtime)
//! without breaking the public method surface — the opaque-fields
//! pattern mirrors codec-native `Frame` / codec-web `DecodedFrame`.

use verbreel_codec_web::{BrowserFamily, PreviewClientCapabilities, WebDecoder};
use verbreel_state::SCHEMA_VERSION;
use wasm_bindgen::prelude::wasm_bindgen;

use crate::error::WasmError;
use crate::scope::EmbeddingScope;
use crate::session::PreviewSession;

/// Browser-side engine lifecycle handle.
///
/// Constructed via [`EngineHandle::new`]. Holds the engine's schema
/// version and browser embedding scope so JS callers can branch on
/// compatibility before loading preview code. All compute-heavy
/// methods (frame render, asset upload, project apply) live on the
/// bound `frame::*` / `project::*` free functions and return
/// [`crate::WasmError::NotYetImplemented`] at v0.
///
/// **No `Clone`, no `Default`, no `Copy`** on purpose: Spike S2 grows
/// internals that cannot satisfy those traits (wgpu `Surface` is not
/// `Clone`; engine initialisation reads project data so there is no
/// meaningful default). Committing to a bare `new()` + accessor
/// surface keeps S2 free to add non-clone fields without breaking JS
/// callers.
#[wasm_bindgen]
#[derive(Debug)]
pub struct EngineHandle {
    schema_version: &'static str,
    embedding_scope: EmbeddingScope,
}

#[wasm_bindgen]
impl EngineHandle {
    /// Construct a fresh handle bound to the current
    /// [`verbreel_state::SCHEMA_VERSION`].
    // Default is intentionally NOT derived — see type-level doc.
    #[wasm_bindgen(constructor)]
    #[allow(clippy::new_without_default)]
    #[must_use]
    pub fn new() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            embedding_scope: EmbeddingScope::PreviewOnly,
        }
    }

    /// Open a browser preview decode session against
    /// `verbreel-codec-web`.
    ///
    /// `has_webcodecs_decode` and `is_safari` are the client's reported
    /// capabilities (the JS bridge fills them from
    /// `verbreel_codec_web::capability::detect()` or a wire report);
    /// codec-web resolves them into a `webcodecs`-or-`mse` transport.
    /// `prefer_h265` selects the H.265 decode codec over the H.264
    /// baseline. The returned [`PreviewSession`] owns the live decoder
    /// on wasm32 and drives `seek` / `frameAt`.
    ///
    /// Mutation stays off this surface (decision #404): there is no
    /// `apply` / persistence method on the browser handle, and any future
    /// in-memory preview path must surface
    /// [`WasmError::BrowserNoPersistence`].
    ///
    /// # Errors
    ///
    /// Returns [`WasmError::PreviewDecode`] if the browser refuses to
    /// construct or configure the `VideoDecoder` (wasm32). Infallible on
    /// native targets, where no decoder is built.
    #[wasm_bindgen(js_name = openPreviewSession)]
    pub fn open_preview_session(
        &self,
        has_webcodecs_decode: bool,
        is_safari: bool,
        prefer_h265: bool,
    ) -> Result<PreviewSession, WasmError> {
        let caps = PreviewClientCapabilities {
            browser_family: if is_safari {
                BrowserFamily::Safari
            } else {
                BrowserFamily::Other
            },
            has_webcodecs_decode,
        };
        let codec = if prefer_h265 {
            WebDecoder::H265
        } else {
            WebDecoder::H264
        };
        PreviewSession::open(caps, codec)
    }
}

impl EngineHandle {
    /// Project schema version this engine recognises (`SemVer` string).
    /// JS callers branch on this before loading a project.
    #[must_use]
    pub fn schema_version(&self) -> &'static str {
        self.schema_version
    }

    /// Browser embedding scope exported by this wasm bundle.
    #[must_use]
    pub const fn embedding_scope(&self) -> EmbeddingScope {
        self.embedding_scope
    }

    /// Stable JS-facing embedding-scope literal.
    #[must_use]
    pub const fn embedding_scope_wire(&self) -> &'static str {
        self.embedding_scope.as_str()
    }

    /// Whether this wasm bundle supports browser preview embedding.
    #[must_use]
    pub const fn supports_preview_embedding(&self) -> bool {
        self.embedding_scope.supports_preview()
    }

    /// Whether this wasm bundle embeds the full editor command surface.
    #[must_use]
    pub const fn supports_editor_embedding(&self) -> bool {
        self.embedding_scope.supports_editor()
    }
}
