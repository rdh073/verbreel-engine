//! Browser embedding scope exported by the WASM surface.
//!
//! Research 01 fixes the browser build as preview-only so export and
//! editor mutation stay on the native/HTTP/MCP surfaces. This module
//! pins that decision in code before the eventual `wasm-bindgen`
//! wrapper freezes the JS-visible API.

/// JS-visible wire literal for the v1 WASM embedding scope.
pub const EMBEDDING_SCOPE_WIRE: &str = "preview-only";

/// WASM browser embedding scope.
///
/// v1 exposes preview playback/scrubbing only. Editor mutation and
/// export remain out of the browser embedding surface because the
/// deterministic event-log and native codec invariants live outside
/// `wasm32-unknown-unknown`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum EmbeddingScope {
    /// Browser preview only; no editor command surface is embedded.
    PreviewOnly,
}

impl EmbeddingScope {
    /// Stable JS-facing wire literal.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PreviewOnly => EMBEDDING_SCOPE_WIRE,
        }
    }

    /// Whether this scope allows browser preview playback/scrubbing.
    #[must_use]
    pub const fn supports_preview(self) -> bool {
        matches!(self, Self::PreviewOnly)
    }

    /// Whether this scope allows the full editor command surface in
    /// browser WASM.
    #[must_use]
    pub const fn supports_editor(self) -> bool {
        false
    }
}
